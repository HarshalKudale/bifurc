//! The engine's live MCP registry: one client connection per `(workspaceId, connectionId)`, what the server
//! declared on it (tools, prompts, resources), the bounded log of what was asked of it, and the `mcp.status` /
//! `mcp.message` events its life produces.
//!
//! Like a WebSocket, a connection belongs to the engine, not to the client that opened it: only
//! [`disconnect_connection`] closes one. A finished connection keeps its entry so its final state and log can
//! still be read, and a reconnect replaces the connection under the same key. The engine does not reconnect by
//! itself.
//!
//! A connection is `connected` once the handshake is done and the server's lists have been read. A call is
//! logged when it is made, as `pending`, and the same log line is reported again when the server answers.

use std::{
    collections::HashMap,
    sync::LazyLock,
    time::{Duration, Instant},
};

use reqwest::header::{HeaderName, HeaderValue};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::{
    answer,
    arguments::{self, CallKind, Catalog},
    client::{self, Heard, Prepared, TRANSPORT_HEADERS},
};
use crate::{
    env::{load_merged_environment, resolve_headers, resolve_vars},
    kernel::streams::{Event, Stream, Streams, emit, key_of, trim_log},
    rest::direct_http::header_pairs,
    utils::id::now_ms,
};

/// Max concurrent live connections per workspace.
pub const MAX_MCP_CONNECTIONS: usize = 5;

/// The retained log's ceiling per connection; older lines are dropped from the front.
pub const MAX_MCP_MESSAGES: usize = 500;

/// How long a server has to finish the handshake and answer for its lists. A program may first have to be
/// downloaded (`npx`), so it is given longer than an endpoint.
const STDIO_HANDSHAKE: Duration = Duration::from_millis(if cfg!(test) { 3_000 } else { 120_000 });
const HTTP_HANDSHAKE: Duration = Duration::from_millis(if cfg!(test) { 3_000 } else { 30_000 });

/// How long what a program wrote as it ended is waited for, so its last words can explain the end.
const LAST_WORDS: Duration = Duration::from_millis(150);

/// How long the closing of a connection may take.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(10);

/// What a caller of [`disconnect_connection`] and [`clear_connection_messages`] gets back.
pub type OkResult = Result<(), String>;

/// What a connect is asked to open, as the caller wrote it (variables unresolved).
pub struct Target<'a> {
    /// `stdio` or `http`; when empty, `http` for an `http(s)://` address and `stdio` for anything else.
    pub transport: &'a str,
    /// The command that starts the server, or the address of its endpoint.
    pub url: &'a str,
    /// The request's headers: sent with every HTTP request, or given to the program as environment variables.
    pub headers: Option<&'a Value>,
}

enum Command {
    /// Something to ask of the server, answered into the log line `message_id`.
    Call {
        message_id: String,
        kind: CallKind,
        name: String,
        arguments: Map<String, Value>,
    },
    Close,
}

struct Entry {
    workspace_id: String,
    connection_id: String,
    transport: &'static str,
    /// The command or address as the caller sent it, unresolved and trimmed.
    url: String,
    status: &'static str,
    error: Option<String>,
    /// What the server said of itself in the handshake.
    server_info: Value,
    protocol_version: Option<String>,
    capabilities: Value,
    instructions: Option<String>,
    /// What the server declared, by [`Catalog::index`].
    catalogs: [Vec<Value>; 4],
    opened_at: Option<i64>,
    messages: Vec<Value>,
    /// The live connection's task, if any.
    tx: Option<UnboundedSender<Command>>,
    /// Bumped on every connect; a task whose generation is behind the entry's no-ops, so a superseded
    /// connection's late end cannot overwrite the new connection's state.
    generation: u64,
    /// Set by a disconnect: a close the caller asked for is not an error.
    close_requested: bool,
    /// Monotonic per entry; the suffix of each log line's id.
    seq: u64,
}

/// How a call ended.
struct Answered {
    status: &'static str,
    data: String,
    text: String,
}

impl Answered {
    /// What the server answered: a result (which a tool may mark as its own failure), or an error.
    fn by_server(outcome: Result<Value, Value>) -> Self {
        match outcome {
            Ok(result) => Self {
                status: if result["isError"] == true { "error" } else { "ok" },
                text: answer::of_result(&result),
                data: result.to_string(),
            },
            Err(error) => Self { status: "error", text: answer::of_error(&error), data: error.to_string() },
        }
    }

    /// The call will not be answered, for `why`.
    fn never(why: &str) -> Self {
        Self { status: "error", text: why.to_string(), data: json!({ "message": why }).to_string() }
    }
}

/// One line of the log.
struct Logged<'a> {
    /// `tool`, `prompt`, `resource`, or what the server sent unasked: `notification`, `request`, `stderr`.
    kind: &'a str,
    name: &'a str,
    /// The arguments of a call, as JSON text.
    arguments: String,
    status: &'static str,
    data: String,
}

impl Entry {
    fn is_open(&self) -> bool {
        self.tx.is_some() && self.status == "connected" && !self.close_requested
    }

    /// The connection without its log: what a status event carries.
    fn state(&self) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("workspaceId".into(), json!(self.workspace_id));
        out.insert("connectionId".into(), json!(self.connection_id));
        out.insert("transport".into(), json!(self.transport));
        out.insert("url".into(), json!(self.url));
        out.insert("status".into(), json!(self.status));
        out.insert("error".into(), json!(self.error));
        out.insert("serverInfo".into(), self.server_info.clone());
        out.insert("protocolVersion".into(), json!(self.protocol_version));
        out.insert("capabilities".into(), self.capabilities.clone());
        out.insert("instructions".into(), json!(self.instructions));
        for catalog in Catalog::ALL {
            out.insert(catalog.key().into(), Value::Array(self.catalogs[catalog.index()].clone()));
        }
        out.insert("openedAt".into(), json!(self.opened_at));
        out
    }

    fn status_event(&self) -> Event {
        Event { name: "mcp.status", payload: Value::Object(self.state()) }
    }

    fn message_event(&self, message: &Value) -> Event {
        Event {
            name: "mcp.message",
            payload: json!({
                "workspaceId": self.workspace_id, "connectionId": self.connection_id, "message": message,
            }),
        }
    }

    /// The log is what clients and files see; a secret that came back from the server is shown as its reference.
    fn redact(&self, text: String) -> String {
        crate::vault::redact_text(&self.workspace_id, text)
    }

    /// Append a line to the log, trim it to the ceiling and return the line's id with the `mcp.message` event.
    fn push(&mut self, logged: Logged) -> (String, Event) {
        self.seq += 1;
        let id = format!("{}-{}", self.connection_id, self.seq);
        let message = json!({
            "id": id,
            "ts": now_ms(),
            "type": logged.kind,
            "name": logged.name,
            "arguments": self.redact(logged.arguments),
            "status": logged.status,
            "durationMs": Value::Null,
            // What the line says as it came, and as it is read: the same until a call is answered.
            "text": self.redact(logged.data.clone()),
            "data": self.redact(logged.data),
        });
        let event = self.message_event(&message);
        self.messages.push(message);
        trim_log(&mut self.messages, MAX_MCP_MESSAGES);
        (id, event)
    }

    /// Something the server sent unasked.
    fn note(&mut self, kind: &str, name: &str, data: String) -> Event {
        self.push(Logged { kind, name, arguments: String::new(), status: "ok", data }).1
    }

    /// Settle the pending line `message_id` with what the server answered: the answer as it came (`data`)
    /// and as it is read (`text`). `None` when the line is no longer in the log.
    fn settle(&mut self, message_id: &str, answered: Answered, took: Option<Duration>) -> Option<Event> {
        let (data, text) = (self.redact(answered.data), self.redact(answered.text));
        let message = self.messages.iter_mut().rev().find(|message| message["id"] == message_id)?;
        message["status"] = json!(answered.status);
        message["text"] = json!(text);
        message["data"] = json!(data);
        message["durationMs"] = json!(took.map(|took| took.as_millis() as u64));
        let message = message.clone();
        Some(self.message_event(&message))
    }
}

impl Stream for Entry {
    fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    fn is_live(&self) -> bool {
        self.tx.is_some()
    }

    fn generation(&self) -> u64 {
        self.generation
    }

    fn snapshot(&self) -> Value {
        let mut out = self.state();
        out.insert("messages".into(), Value::Array(self.messages.clone()));
        Value::Object(out)
    }

    fn clear_log(&mut self) {
        self.messages.clear();
    }
}

static CONNECTIONS: LazyLock<Streams<Entry>> = LazyLock::new(Streams::new);

fn no_connection(workspace_id: &str, connection_id: &str) -> String {
    format!("No connection \"{connection_id}\" in workspace \"{workspace_id}\".")
}

/// The snapshot for one connection, live or finished, or `None` if the engine never opened that id.
pub fn get_connection(workspace_id: &str, connection_id: &str) -> Option<Value> {
    CONNECTIONS.get(workspace_id, connection_id)
}

/// The live connections of a workspace. A finished connection keeps its entry but does not appear here.
pub fn list_connections(workspace_id: &str) -> Vec<Value> {
    CONNECTIONS.list(workspace_id)
}

fn live_count(workspace_id: &str) -> usize {
    CONNECTIONS.live_count(workspace_id)
}

/// The transport a target names, or the one its address implies.
fn transport_of(target: &Target, address: &str) -> Result<&'static str, String> {
    match target.transport.trim().to_ascii_lowercase().as_str() {
        "stdio" => Ok("stdio"),
        "http" => Ok("http"),
        "" if address.starts_with("http://") || address.starts_with("https://") => Ok("http"),
        "" => Ok("stdio"),
        other => Err(format!("Unknown transport \"{other}\": an MCP request is `stdio` or `http`.")),
    }
}

/// The command to run or the endpoint to call, with the request's headers, everything resolved against the
/// environment. A program gets the headers as environment variables: that is where a local server reads its
/// credentials from.
fn prepare(target: &Target, env: Option<&Value>) -> Result<Prepared, String> {
    let address = resolve_vars(target.url.trim(), env);
    let address = address.trim();
    let headers = target.headers.map(|headers| resolve_headers(headers, env)).unwrap_or_else(|| json!({}));
    if transport_of(target, address)? == "stdio" {
        if address.is_empty() {
            return Err("A command is required: what starts the MCP server, such as \
                        `npx -y @modelcontextprotocol/server-everything`."
                .to_string());
        }
        let mut env = Vec::new();
        for (name, value) in header_pairs(&headers) {
            let name = name.trim().to_string();
            if name.is_empty() || name.contains(['=', '\0']) || value.contains('\0') {
                return Err(format!("Invalid environment variable name \"{name}\"."));
            }
            env.push((name, value));
        }
        return Ok(Prepared::Stdio { command: address.to_string(), env });
    }

    const NEEDS: &str = "Invalid URL: an MCP request over HTTP needs an http:// or https:// address.";
    let url = url::Url::parse(address).map_err(|_| NEEDS.to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
        return Err(NEEDS.to_string());
    }
    let mut map = HashMap::new();
    for (name, value) in header_pairs(&headers) {
        let header =
            HeaderName::from_bytes(name.as_bytes()).map_err(|e| format!("Invalid header name \"{name}\": {e}"))?;
        if TRANSPORT_HEADERS.contains(&header.as_str()) {
            return Err(format!("The header \"{name}\" is set by the MCP transport and cannot be given."));
        }
        let value = HeaderValue::from_str(&value).map_err(|e| format!("Invalid value for header \"{name}\": {e}"))?;
        map.insert(header, value);
    }
    Ok(Prepared::Http { url, headers: map })
}

/// Open a connection, replacing any live one under the same key. Returns once the connection attempt has
/// started; the open (or the failure) arrives as `mcp.status` events.
pub fn connect_connection(workspace_id: &str, connection_id: &str, target: &Target) -> Result<(), String> {
    let key = key_of(workspace_id, connection_id);
    let env = load_merged_environment(workspace_id);

    // The limit applies only when the key has no live connection of its own: reconnecting to one you already
    // hold must not be refused by the connection you are replacing.
    let existing_live = CONNECTIONS.lock().get(&key).is_some_and(Entry::is_live);
    if !existing_live && live_count(workspace_id) >= MAX_MCP_CONNECTIONS {
        return Err(format!(
            "Max {MAX_MCP_CONNECTIONS} active connections reached. \
             Disconnect another one first (mcp.disconnect), or list them with mcp.list."
        ));
    }

    let prepared = prepare(target, env.as_ref());
    let mut events = Vec::new();
    let (tx, rx) = unbounded_channel();
    let outcome = {
        let mut connections = CONNECTIONS.lock();
        let entry = connections.entry(key.clone()).or_insert_with(|| Entry {
            workspace_id: workspace_id.to_string(),
            connection_id: connection_id.to_string(),
            transport: "stdio",
            url: String::new(),
            status: "connecting",
            error: None,
            server_info: Value::Null,
            protocol_version: None,
            capabilities: Value::Null,
            instructions: None,
            catalogs: Default::default(),
            opened_at: None,
            messages: Vec::new(),
            tx: None,
            generation: 0,
            close_requested: false,
            seq: 0,
        });

        // Supersede the old connection before closing it, so its end cannot report after the `connecting` below.
        entry.generation += 1;
        if let Some(old) = entry.tx.take() {
            let _ = old.send(Command::Close);
        }

        // The retained log deliberately survives a reconnect; what the last server declared does not.
        entry.url = target.url.trim().to_string();
        entry.status = "connecting";
        entry.error = None;
        entry.server_info = Value::Null;
        entry.protocol_version = None;
        entry.capabilities = Value::Null;
        entry.instructions = None;
        entry.catalogs = Default::default();
        entry.opened_at = None;
        entry.close_requested = false;
        let mut events_of_old = unanswered(entry, "The connection was replaced before the server answered.");
        events.append(&mut events_of_old);
        if let Ok(prepared) = &prepared {
            entry.transport = prepared.transport();
        }
        events.push(entry.status_event());

        match prepared {
            Ok(prepared) => {
                entry.tx = Some(tx);
                Ok((prepared, entry.generation))
            }
            Err(message) => {
                entry.status = "error";
                entry.error = Some(message.clone());
                events.push(entry.status_event());
                Err(message)
            }
        }
    };
    emit(events);

    let (prepared, generation) = outcome?;
    tokio::spawn(run(key, generation, prepared, rx));
    Ok(())
}

/// Close a connection. `Ok` for one that is already closed; an error only when the id was never opened.
pub fn disconnect_connection(workspace_id: &str, connection_id: &str) -> OkResult {
    let mut events = Vec::new();
    let result = {
        let mut connections = CONNECTIONS.lock();
        let Some(entry) = connections.get_mut(&key_of(workspace_id, connection_id)) else {
            return Err(no_connection(workspace_id, connection_id));
        };
        if let Some(tx) = entry.tx.clone() {
            entry.close_requested = true;
            if tx.send(Command::Close).is_err() {
                // The task is gone, so no end will ever be reported: settle the state directly.
                entry.tx = None;
                entry.status = "disconnected";
                entry.error = None;
                events.push(entry.status_event());
            }
        }
        Ok(())
    };
    emit(events);
    result
}

/// Ask the server for something: call the tool `name`, get the prompt `name` or read the resource whose URI
/// is `name`. `typed` holds what was typed for each argument (see [`arguments::tool_arguments`]); the name and every
/// text are resolved against the merged environment. Returns the id of the log line the answer will settle.
pub fn call_server(
    workspace_id: &str,
    connection_id: &str,
    kind: CallKind,
    name: &str,
    typed: Option<&Map<String, Value>>,
) -> Result<String, String> {
    let key = key_of(workspace_id, connection_id);
    // What is sent has the secrets in; what is logged has them as written (`push` also masks any value).
    let env = load_merged_environment(workspace_id);
    let display = {
        let mut display = env.clone();
        if let Some(display) = display.as_mut() {
            display["workspaceId"] = json!("");
        }
        display
    };
    let sent_name = resolve_vars(name.trim(), env.as_ref()).trim().to_string();
    let shown_name = resolve_vars(name.trim(), display.as_ref()).trim().to_string();
    if sent_name.is_empty() {
        return Err(match kind {
            CallKind::Tool => "A tool name is required.",
            CallKind::Prompt => "A prompt name is required.",
            CallKind::Resource => "A resource URI is required.",
        }
        .to_string());
    }

    // What the server declared about the tool or the prompt says how its arguments are read.
    let declared = {
        let connections = CONNECTIONS.lock();
        let Some(entry) = connections.get(&key).filter(|entry| entry.is_open()) else {
            return Err("Not connected".to_string());
        };
        let catalog = match kind {
            CallKind::Tool => Some(Catalog::Tools),
            CallKind::Prompt => Some(Catalog::Prompts),
            CallKind::Resource => None,
        };
        catalog.and_then(|catalog| {
            entry.catalogs[catalog.index()].iter().find(|item| item["name"] == sent_name.as_str()).cloned()
        })
    };
    let arguments = |env: Option<&Value>| -> Result<Map<String, Value>, String> {
        let resolve = |text: &str| resolve_vars(text, env);
        match kind {
            CallKind::Tool => {
                arguments::tool_arguments(declared.as_ref().and_then(|tool| tool.get("inputSchema")), typed, &resolve)
            }
            CallKind::Prompt => Ok(arguments::prompt_arguments(declared.as_ref(), typed, &resolve)),
            CallKind::Resource => Ok(Map::new()),
        }
    };
    let sent = arguments(env.as_ref())?;
    let shown = arguments(display.as_ref())?;
    let shown = match kind {
        CallKind::Resource => String::new(),
        _ => Value::Object(shown).to_string(),
    };

    let (message_id, event) = {
        let mut connections = CONNECTIONS.lock();
        let Some(entry) = connections.get_mut(&key).filter(|entry| entry.is_open()) else {
            return Err("Not connected".to_string());
        };
        let (message_id, event) = entry.push(Logged {
            kind: kind.name(),
            name: &shown_name,
            arguments: shown,
            status: "pending",
            data: String::new(),
        });
        let tx = entry.tx.as_ref().expect("an open connection has a task");
        let call = Command::Call { message_id: message_id.clone(), kind, name: sent_name, arguments: sent };
        if tx.send(call).is_err() {
            entry.messages.pop();
            return Err("Send failed".to_string());
        }
        (message_id, event)
    };
    emit(vec![event]);
    Ok(message_id)
}

/// Drop the retained log of one connection; the connection itself is untouched.
pub fn clear_connection_messages(workspace_id: &str, connection_id: &str) -> OkResult {
    if CONNECTIONS.clear_log(workspace_id, connection_id) {
        Ok(())
    } else {
        Err(no_connection(workspace_id, connection_id))
    }
}

/// Settle every call still waiting: the connection it was made on will not answer it.
fn unanswered(entry: &mut Entry, why: &str) -> Vec<Event> {
    let waiting: Vec<String> = entry
        .messages
        .iter()
        .filter(|message| message["status"] == "pending")
        .filter_map(|message| message["id"].as_str().map(str::to_string))
        .collect();
    waiting.iter().filter_map(|id| entry.settle(id, Answered::never(why), None)).collect()
}

// ── the connection task ───────────────────────────────────────────────────────────────────────────

/// Change the entry of a connection that has not been superseded and report what `change` returns. `false`
/// when the connection has been superseded.
fn record(key: &str, generation: u64, change: impl FnOnce(&mut Entry) -> Vec<Event>) -> bool {
    CONNECTIONS.record(key, generation, change)
}

/// The connection is over. `error` is why it failed; with none it ended cleanly. A close the caller asked for
/// is never an error.
fn record_end(key: &str, generation: u64, error: Option<String>) {
    record(key, generation, |entry| {
        entry.tx = None;
        let mut events = unanswered(entry, "The connection ended before the server answered.");
        match error.filter(|_| !entry.close_requested) {
            Some(message) => {
                entry.status = "error";
                entry.error = Some(message);
            }
            None => {
                entry.status = "disconnected";
                entry.error = None;
            }
        }
        events.push(entry.status_event());
        events
    });
}

/// Log what a connection heard. Returns the lists it says have changed, or `None` when the connection has been
/// superseded.
fn hear(key: &str, generation: u64, heard: Heard, last_words: &mut Vec<String>) -> Option<&'static [Catalog]> {
    match heard {
        Heard::Stderr(line) => {
            let alive = record(key, generation, |entry| vec![entry.note("stderr", "", line.clone())]);
            // The last two lines: a shell's complaint about a command takes both.
            if last_words.len() == 2 {
                last_words.remove(0);
            }
            last_words.push(line);
            alive.then_some(&[])
        }
        Heard::Notification { method, data } => {
            let changed = Catalog::changed_by(&method);
            record(key, generation, |entry| vec![entry.note("notification", &method, data)]).then_some(changed)
        }
    }
}

/// What a connection's task knows while it runs.
struct Running {
    key: String,
    generation: u64,
    heard: UnboundedReceiver<Heard>,
    /// The last things the program said for itself: usually why it ended.
    last_words: Vec<String>,
}

impl Running {
    /// `false` when the connection has been superseded.
    fn record(&self, change: impl FnOnce(&mut Entry) -> Vec<Event>) -> bool {
        record(&self.key, self.generation, change)
    }

    /// The connection failed for `reason`; the program's last words are added to it.
    async fn fail(mut self, reason: &str) {
        // What the program wrote as it ended may still be on its way.
        tokio::time::sleep(LAST_WORDS).await;
        while let Ok(late) = self.heard.try_recv() {
            hear(&self.key, self.generation, late, &mut self.last_words);
        }
        let message = match self.last_words.is_empty() {
            false => format!("{}: {}", reason.trim_end_matches('.'), self.last_words.join(" ")),
            true => reason.to_string(),
        };
        record_end(&self.key, self.generation, Some(message));
    }
}

/// Owns one connection from its start to its end. The registry reaches it through `rx`.
async fn run(key: String, generation: u64, prepared: Prepared, mut rx: UnboundedReceiver<Command>) {
    let handshake = match prepared {
        Prepared::Stdio { .. } => STDIO_HANDSHAKE,
        Prepared::Http { .. } => HTTP_HANDSHAKE,
    };
    let (heard_tx, heard) = unbounded_channel();
    let mut running = Running { key, generation, heard, last_words: Vec::new() };

    // Connected is said once the handshake is done and the lists the server declared have been read.
    let opening = async {
        let client = client::connect(prepared, heard_tx).await?;
        let mut lists = Vec::new();
        for catalog in Catalog::ALL {
            if client::declares(&client, catalog) {
                lists.push((catalog, client::list(client.peer(), catalog).await));
            }
        }
        Ok::<_, String>((client, lists))
    };
    tokio::pin!(opening);
    let timer = tokio::time::sleep(handshake);
    tokio::pin!(timer);
    let (client, lists) = loop {
        tokio::select! {
            opened = &mut opening => match opened {
                Ok(opened) => break opened,
                Err(reason) => return running.fail(&reason).await,
            },
            Some(item) = running.heard.recv() => {
                if hear(&running.key, generation, item, &mut running.last_words).is_none() {
                    return;
                }
            }
            command = rx.recv() => match command {
                Some(Command::Close) | None => return record_end(&running.key, generation, None),
                Some(Command::Call { .. }) => {}
            },
            _ = &mut timer => {
                return running.fail("Timed out waiting for the MCP handshake. Is this an MCP server?").await;
            }
        }
    };

    let (server_info, protocol_version, capabilities, instructions) = client::handshake(&client);
    let open = |entry: &mut Entry| {
        entry.server_info = server_info;
        entry.protocol_version = protocol_version;
        entry.capabilities = capabilities;
        entry.instructions = instructions;
        for (catalog, items) in lists {
            entry.catalogs[catalog.index()] = items;
        }
        entry.status = "connected";
        entry.error = None;
        entry.opened_at = Some(now_ms());
        vec![entry.status_event()]
    };
    if !running.record(open) {
        return;
    }

    let peer = client.peer().clone();
    let cancel = client.cancellation_token();
    let ended = client.waiting();
    tokio::pin!(ended);
    loop {
        tokio::select! {
            _ = &mut ended => return running.fail("The server closed the connection.").await,
            Some(item) = running.heard.recv() => {
                let Some(changed) = hear(&running.key, generation, item, &mut running.last_words) else {
                    return cancel.cancel();
                };
                for catalog in changed {
                    let items = client::list(&peer, *catalog).await;
                    let update = |entry: &mut Entry| {
                        entry.catalogs[catalog.index()] = items;
                        vec![entry.status_event()]
                    };
                    if !running.record(update) {
                        return cancel.cancel();
                    }
                }
            }
            command = rx.recv() => match command {
                Some(Command::Call { message_id, kind, name, arguments }) => {
                    let (peer, key) = (peer.clone(), running.key.clone());
                    tokio::spawn(async move {
                        let started = Instant::now();
                        let answered = Answered::by_server(client::call(&peer, kind, name, arguments).await);
                        let took = Some(started.elapsed());
                        record(&key, generation, |entry| {
                            entry.settle(&message_id, answered, took).into_iter().collect()
                        });
                    });
                }
                Some(Command::Close) | None => {
                    // The state is settled now; the transport gives the program a moment to leave.
                    cancel.cancel();
                    record_end(&running.key, generation, None);
                    let _ = tokio::time::timeout(CLOSE_TIMEOUT, &mut ended).await;
                    return;
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target<'a>(transport: &'a str, url: &'a str, headers: Option<&'a Value>) -> Target<'a> {
        Target { transport, url, headers }
    }

    #[test]
    fn the_transport_is_the_one_named_or_the_one_the_address_implies() {
        for (transport, url, expected) in [
            ("", "https://example.test/mcp", "http"),
            ("", "http://localhost:3000/mcp", "http"),
            ("", "npx -y some-server", "stdio"),
            ("stdio", "node server.js", "stdio"),
            ("HTTP", "https://example.test/mcp", "http"),
            // A command may well start with an address-like word; the transport named wins.
            ("stdio", "https-server --port 1", "stdio"),
        ] {
            let prepared = prepare(&target(transport, url, None), None).unwrap();
            assert_eq!(prepared.transport(), expected, "{transport} {url}");
        }
        let error = prepare(&target("sse", "http://h", None), None).err().unwrap();
        assert!(error.starts_with("Unknown transport \"sse\""), "{error}");
    }

    #[test]
    fn a_program_gets_the_headers_as_its_environment_and_an_endpoint_as_headers() {
        let env = json!({ "variables": [
            { "key": "TOKEN", "value": "secret", "enabled": true },
            { "key": "SERVER", "value": "my-server", "enabled": true },
            { "key": "HOST", "value": "example.test", "enabled": true },
        ] });
        let headers = json!({ "API_KEY": "{{TOKEN}}", "Authorization": "Bearer {{TOKEN}}" });
        match prepare(&target("stdio", " npx {{SERVER}} --flag ", Some(&headers)), Some(&env)).unwrap() {
            Prepared::Stdio { command, env } => {
                assert_eq!(command, "npx my-server --flag");
                assert_eq!(
                    env,
                    [
                        ("API_KEY".to_string(), "secret".to_string()),
                        ("Authorization".to_string(), "Bearer secret".to_string())
                    ]
                );
            }
            Prepared::Http { .. } => panic!("a command is not an endpoint"),
        }
        match prepare(&target("", "https://{{HOST}}/mcp", Some(&headers)), Some(&env)).unwrap() {
            Prepared::Http { url, headers } => {
                assert_eq!(url.as_str(), "https://example.test/mcp");
                assert_eq!(headers[&HeaderName::from_static("authorization")], "Bearer secret");
                assert_eq!(headers[&HeaderName::from_static("api_key")], "secret");
            }
            Prepared::Stdio { .. } => panic!("an address is not a command"),
        }
    }

    #[test]
    fn what_cannot_be_connected_is_refused_with_a_reason() {
        let error = prepare(&target("stdio", "  ", None), None).err().unwrap();
        assert!(error.starts_with("A command is required"), "{error}");
        let error = prepare(&target("", "", None), None).err().unwrap();
        assert!(error.starts_with("A command is required"), "{error}");
        for url in ["", "not a url", "ws://h/mcp", "localhost:3000"] {
            let error = prepare(&target("http", url, None), None).err().expect(url);
            assert!(error.starts_with("Invalid URL"), "{url}: {error}");
        }
        let headers = json!({ "bad name": "v" });
        assert!(prepare(&target("http", "http://h", Some(&headers)), None).err().unwrap().contains("header name"));
        let headers = json!({ "A=B": "v" });
        let error = prepare(&target("stdio", "server", Some(&headers)), None).err().unwrap();
        assert_eq!(error, "Invalid environment variable name \"A=B\".");
    }
}
