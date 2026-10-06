//! An MCP request's live connection, as the engine reports it.
//!
//! The connection belongs to the engine (it outlives this window), keyed by `{workspaceId, connectionId}` where
//! `connectionId` is the saved request's id. The UI holds a view: `mcp.status` returns the whole snapshot with
//! the retained log, `event.mcp.status` carries the connection's state with what the server declares (tools,
//! prompts, resources), and `event.mcp.message` carries one line of the log.
//!
//! A call is a line that is reported twice: when it is made (`pending`) and when the server has answered. The
//! second report replaces the first, by id.

use serde_json::Value;

use super::socket::SocketState;

/// One argument a tool or a prompt takes.
#[derive(Debug, Clone, PartialEq)]
pub struct McpArgument {
    pub name: String,
    /// The type the tool's schema declares (`string`, `integer`, `object`, ...); empty when it declares none.
    /// A prompt's arguments are all text.
    pub kind: String,
    pub required: bool,
    pub description: String,
    /// The values it is limited to, as they are shown and typed; empty when it takes any value of its type.
    pub choices: Vec<String>,
    /// The value the server uses when none is given, if the schema says.
    pub default: Option<String>,
}

/// How an argument is filled in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgumentInput {
    /// One of the values it is limited to.
    Choice,
    /// True or false.
    Flag,
    /// A JSON object or array.
    Json,
    /// Anything typed on one row: text, a number.
    Text,
}

impl McpArgument {
    pub fn input(&self) -> ArgumentInput {
        match self.kind.as_str() {
            _ if !self.choices.is_empty() => ArgumentInput::Choice,
            "boolean" => ArgumentInput::Flag,
            "object" | "array" => ArgumentInput::Json,
            _ => ArgumentInput::Text,
        }
    }
}

/// Something the server declares: a tool, a prompt, a resource or a template for resources.
#[derive(Debug, Clone, PartialEq)]
pub struct McpItem {
    /// What it is asked for by: the name of a tool or a prompt, the URI of a resource or its template.
    pub name: String,
    /// What it is called for a reader, when that differs from `name`.
    pub title: String,
    pub description: String,
    pub arguments: Vec<McpArgument>,
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

/// The schemas a property is one of (`anyOf` / `oneOf`), or the property itself.
fn alternatives(property: &Value) -> Vec<&Value> {
    ["anyOf", "oneOf"]
        .iter()
        .find_map(|key| property.get(*key)?.as_array())
        .map(|schemas| schemas.iter().collect())
        .unwrap_or_else(|| vec![property])
}

/// The type a schema names: its `type`, or the first one that is not `null` of several.
fn named_type(schema: &Value) -> Option<&str> {
    match schema.get("type")? {
        Value::String(name) => Some(name.as_str()),
        Value::Array(names) => names.iter().filter_map(Value::as_str).find(|name| *name != "null"),
        _ => None,
    }
}

/// The type a JSON Schema gives a property, as the engine reads it: its own, or the one its alternatives
/// agree on besides `null`.
fn schema_type(property: &Value) -> String {
    if let Some(name) = named_type(property) {
        return name.to_owned();
    }
    let alternatives = alternatives(property);
    let mut named = alternatives.iter().map(|schema| named_type(schema)).filter(|name| *name != Some("null"));
    match named.next() {
        Some(Some(first)) if named.all(|name| name == Some(first)) => first.to_owned(),
        _ => String::new(),
    }
}

/// A value as it is typed and shown: text without its quotes, anything else as JSON.
fn as_typed(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The values a property is limited to, as the engine reads them: its `enum` or `const`, or those of its
/// alternatives; none when one alternative takes any value.
fn choices(property: &Value) -> Vec<String> {
    let mut allowed = Vec::new();
    for schema in alternatives(property) {
        if let Some(values) = schema.get("enum").and_then(Value::as_array) {
            allowed.extend(values.iter().filter(|value| !value.is_null()).map(as_typed));
        } else if let Some(only) = schema.get("const") {
            allowed.extend(Some(only).filter(|value| !value.is_null()).map(as_typed));
        } else if schema.get("type").and_then(Value::as_str) != Some("null") {
            return Vec::new();
        }
    }
    allowed
}

impl McpItem {
    /// A tool, with the properties of its input schema as its arguments, in the order the schema has them.
    fn tool(value: &Value) -> Option<Self> {
        let schema = value.get("inputSchema");
        let required: Vec<&str> = schema
            .and_then(|schema| schema.get("required")?.as_array())
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let arguments = schema
            .and_then(|schema| schema.get("properties")?.as_object())
            .map(|properties| {
                properties
                    .iter()
                    .map(|(name, property)| McpArgument {
                        name: name.clone(),
                        kind: schema_type(property),
                        required: required.contains(&name.as_str()),
                        description: text(property, "description"),
                        choices: choices(property),
                        default: property.get("default").filter(|value| !value.is_null()).map(as_typed),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            name: value.get("name")?.as_str()?.to_owned(),
            title: text(value, "title"),
            description: text(value, "description"),
            arguments,
        })
    }

    fn prompt(value: &Value) -> Option<Self> {
        let arguments = value
            .get("arguments")
            .and_then(Value::as_array)
            .map(|arguments| {
                arguments
                    .iter()
                    .filter_map(|argument| {
                        Some(McpArgument {
                            name: argument.get("name")?.as_str()?.to_owned(),
                            kind: "string".to_owned(),
                            required: argument.get("required").and_then(Value::as_bool).unwrap_or(false),
                            description: text(argument, "description"),
                            choices: Vec::new(),
                            default: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            name: value.get("name")?.as_str()?.to_owned(),
            title: text(value, "title"),
            description: text(value, "description"),
            arguments,
        })
    }

    /// A resource, or a template when its address is one (`uriTemplate`).
    fn resource(value: &Value) -> Option<Self> {
        let address = value.get("uri").or_else(|| value.get("uriTemplate"))?.as_str()?;
        Some(Self {
            name: address.to_owned(),
            title: text(value, "name"),
            description: text(value, "description"),
            arguments: Vec::new(),
        })
    }

    /// The first line of the description: what a list has room for.
    pub fn summary(&self) -> &str {
        self.description.lines().next().unwrap_or_default().trim()
    }
}

/// What a line of the log is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Tool,
    Prompt,
    Resource,
    /// What the server sent unasked.
    Notification,
    /// What the server asked of this client, which offers it nothing.
    Request,
    /// What the server's program wrote to its diagnostics stream.
    Stderr,
}

impl LineKind {
    fn parse(name: &str) -> Self {
        match name {
            "tool" => LineKind::Tool,
            "prompt" => LineKind::Prompt,
            "resource" => LineKind::Resource,
            "request" => LineKind::Request,
            "stderr" => LineKind::Stderr,
            _ => LineKind::Notification,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LineKind::Tool => "tool",
            LineKind::Prompt => "prompt",
            LineKind::Resource => "resource",
            LineKind::Notification => "notice",
            LineKind::Request => "request",
            LineKind::Stderr => "stderr",
        }
    }

    /// Whether the line is something this side asked for.
    pub fn is_call(self) -> bool {
        matches!(self, LineKind::Tool | LineKind::Prompt | LineKind::Resource)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStatus {
    Pending,
    Ok,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct McpLine {
    pub id: String,
    pub ts: u64,
    pub kind: LineKind,
    /// The tool or the prompt called, the URI read, or the method of what the server sent.
    pub name: String,
    /// The arguments of a call, as JSON text.
    pub arguments: String,
    pub status: CallStatus,
    pub duration_ms: Option<u64>,
    /// The answer as a person reads it: the engine's reading of what came.
    pub text: String,
}

impl McpLine {
    pub fn from_value(value: &Value) -> Option<Self> {
        let answer = text(value, "text");
        Some(Self {
            id: value.get("id")?.as_str()?.to_owned(),
            ts: value.get("ts").and_then(Value::as_u64).unwrap_or(0),
            kind: LineKind::parse(value.get("type")?.as_str()?),
            name: text(value, "name"),
            arguments: text(value, "arguments"),
            status: match value.get("status").and_then(Value::as_str) {
                Some("pending") => CallStatus::Pending,
                Some("error") => CallStatus::Error,
                _ => CallStatus::Ok,
            },
            duration_ms: value.get("durationMs").and_then(Value::as_u64),
            text: if answer.is_empty() { text(value, "data") } else { answer },
        })
    }

    /// What the line says in one row: a call by its name and arguments, anything else by what it carries.
    pub fn summary(&self) -> String {
        let arguments = Some(self.arguments.as_str()).filter(|arguments| !arguments.is_empty() && *arguments != "{}");
        match (self.kind.is_call(), arguments) {
            (true, Some(arguments)) => format!("{} {arguments}", self.name),
            (true, None) => self.name.clone(),
            (false, _) if self.name.is_empty() => self.text.clone(),
            (false, _) if self.text.is_empty() => self.name.clone(),
            (false, _) => format!("{} {}", self.name, self.text),
        }
    }

    /// What is said beside the line: how a call ended and how long it took.
    pub fn note(&self) -> Option<String> {
        if !self.kind.is_call() {
            return None;
        }
        Some(match (self.status, self.duration_ms) {
            (CallStatus::Pending, _) => "waiting…".to_owned(),
            (CallStatus::Ok, Some(took)) => format!("{took} ms"),
            (CallStatus::Ok, None) => "ok".to_owned(),
            (CallStatus::Error, Some(took)) => format!("failed · {took} ms"),
            (CallStatus::Error, None) => "failed".to_owned(),
        })
    }

    /// What the line opens to: the answer of a call, or what was sent.
    pub fn shown(&self) -> String {
        match (self.kind.is_call(), self.status) {
            (true, CallStatus::Pending) => "The server has not answered yet.".to_owned(),
            _ => self.text.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct McpView {
    pub state: SocketState,
    pub error: Option<String>,
    /// `stdio` or `http`; empty before the first connect.
    pub transport: String,
    /// The server's name and version, as it gave them in the handshake.
    pub server: String,
    pub protocol: Option<String>,
    /// How the server says it is to be used.
    pub instructions: Option<String>,
    pub tools: Vec<McpItem>,
    pub prompts: Vec<McpItem>,
    /// The resources, then the templates for resources.
    pub resources: Vec<McpItem>,
    pub lines: Vec<McpLine>,
}

/// The engine keeps at most this many lines per connection; the view mirrors the bound so a long session
/// cannot grow without limit on this side either.
pub const MAX_LINES: usize = 500;

fn items(value: &Value, key: &str, read: fn(&Value) -> Option<McpItem>) -> Vec<McpItem> {
    value.get(key).and_then(Value::as_array).map(|list| list.iter().filter_map(read).collect()).unwrap_or_default()
}

impl McpView {
    /// From a `mcp.status` result. `null` (no such connection yet) is a disconnected view with no history.
    pub fn from_status(result: &Value) -> Self {
        let mut view = Self::default();
        if result.is_null() {
            return view;
        }
        view.apply_status_event(result);
        view.lines = result
            .get("messages")
            .and_then(Value::as_array)
            .map(|lines| lines.iter().filter_map(McpLine::from_value).collect())
            .unwrap_or_default();
        view
    }

    /// Applies an `event.mcp.status` payload: the connection's state and everything the server declares.
    pub fn apply_status_event(&mut self, payload: &Value) {
        self.state = SocketState::parse(payload.get("status").and_then(Value::as_str).unwrap_or(""));
        self.error = payload.get("error").and_then(Value::as_str).map(str::to_owned);
        self.transport = text(payload, "transport");
        let server = payload.get("serverInfo").cloned().unwrap_or(Value::Null);
        self.server = format!("{} {}", text(&server, "name"), text(&server, "version")).trim().to_owned();
        self.protocol = payload.get("protocolVersion").and_then(Value::as_str).map(str::to_owned);
        self.instructions =
            payload.get("instructions").and_then(Value::as_str).filter(|i| !i.trim().is_empty()).map(str::to_owned);
        self.tools = items(payload, "tools", McpItem::tool);
        self.prompts = items(payload, "prompts", McpItem::prompt);
        self.resources = items(payload, "resources", McpItem::resource);
        self.resources.extend(items(payload, "resourceTemplates", McpItem::resource));
    }

    /// Applies an `event.mcp.message` payload. A line already held (the same id) is replaced: that is how a
    /// call gets its answer, and a client that attaches mid-stream can receive a line both in the snapshot and
    /// as an event.
    pub fn apply_message_event(&mut self, payload: &Value) {
        let Some(line) = payload.get("message").and_then(McpLine::from_value) else { return };
        if let Some(held) = self.lines.iter_mut().find(|held| held.id == line.id) {
            *held = line;
            return;
        }
        self.lines.push(line);
        if self.lines.len() > MAX_LINES {
            let excess = self.lines.len() - MAX_LINES;
            self.lines.drain(..excess);
        }
    }

    pub fn clear_lines(&mut self) {
        self.lines.clear();
    }

    /// How many calls the server has not answered yet.
    pub fn waiting(&self) -> usize {
        self.lines.iter().filter(|line| line.status == CallStatus::Pending).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn status() -> Value {
        json!({
            "status": "connected", "error": null, "transport": "stdio", "url": "npx server",
            "serverInfo": {"name": "everything", "version": "2.0.0"}, "protocolVersion": "2025-06-18",
            "instructions": "Be kind.",
            "tools": [
                {"name": "echo", "description": "Echoes.\nSecond line.", "inputSchema": {"type": "object",
                    "properties": {
                        "message": {"type": "string", "description": "What to say"},
                        "count": {"type": ["null", "integer"]},
                        "extra": {}},
                    "required": ["message"]}},
                {"name": "now"},
                {"description": "nameless"}],
            "prompts": [{"name": "greet", "arguments": [{"name": "who", "required": true}, {"name": "tone", "description": "How"}]}],
            "resources": [{"uri": "mem://a", "name": "alpha", "description": "The first"}],
            "resourceTemplates": [{"uriTemplate": "mem://{id}", "name": "by id"}],
        })
    }

    fn line(id: &str, kind: &str, status: &str) -> Value {
        json!({"id": id, "ts": 10, "type": kind, "name": "echo", "arguments": "{\"message\":\"hi\"}", "status": status,
               "durationMs": if status == "pending" { Value::Null } else { json!(12) },
               "data": "{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}",
               "text": if status == "pending" { "" } else { "hi" }})
    }

    #[test]
    fn a_status_carries_the_server_and_what_it_declares() {
        let mut snapshot = status();
        snapshot["messages"] = json!([line("c-1", "tool", "ok")]);
        let view = McpView::from_status(&snapshot);
        assert_eq!(view.state, SocketState::Connected);
        assert_eq!((view.transport.as_str(), view.server.as_str()), ("stdio", "everything 2.0.0"));
        assert_eq!(view.protocol.as_deref(), Some("2025-06-18"));
        assert_eq!(view.instructions.as_deref(), Some("Be kind."));
        assert_eq!(view.lines.len(), 1);

        // A tool's arguments are the properties of its schema, in its order; one without a name is not a tool.
        assert_eq!(view.tools.iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>(), ["echo", "now"]);
        let echo = &view.tools[0];
        assert_eq!(echo.summary(), "Echoes.");
        assert_eq!(
            echo.arguments.iter().map(|a| (a.name.as_str(), a.kind.as_str(), a.required)).collect::<Vec<_>>(),
            [("message", "string", true), ("count", "integer", false), ("extra", "", false)]
        );
        assert_eq!(echo.arguments[0].description, "What to say");
        assert!(view.tools[1].arguments.is_empty());

        let greet = &view.prompts[0];
        assert_eq!(
            greet.arguments.iter().map(|a| (a.name.as_str(), a.kind.as_str(), a.required)).collect::<Vec<_>>(),
            [("who", "string", true), ("tone", "string", false)]
        );
        // The resources, then the templates, each by its address.
        assert_eq!(
            view.resources.iter().map(|r| (r.name.as_str(), r.title.as_str())).collect::<Vec<_>>(),
            [("mem://a", "alpha"), ("mem://{id}", "by id")]
        );
    }

    #[test]
    fn an_argument_is_filled_in_the_way_its_schema_says() {
        let tool = McpItem::tool(&json!({"name": "report", "inputSchema": {"properties": {
            "interval": {"type": "string", "enum": ["day", "week"], "default": "day"},
            "level": {"enum": [1, 2]},
            "mode": {"anyOf": [{"const": "fast"}, {"const": "safe"}, {"type": "null"}]},
            "loud": {"type": "boolean", "default": false},
            "maybe": {"anyOf": [{"type": "boolean"}, {"type": "null"}]},
            "filter": {"type": "object"},
            "tags": {"type": "array", "items": {"type": "string"}},
            "count": {"type": "integer"},
            "open": {"anyOf": [{"const": "auto"}, {"type": "integer"}]},
            "note": {"type": "string"},
        }}}))
        .unwrap();
        let of = |name: &str| tool.arguments.iter().find(|a| a.name == name).unwrap();
        assert_eq!(of("interval").input(), ArgumentInput::Choice);
        assert_eq!(of("interval").choices, ["day", "week"]);
        assert_eq!(of("interval").default.as_deref(), Some("day"));
        assert_eq!((of("level").input(), of("level").choices.clone()), (ArgumentInput::Choice, vec!["1".to_owned(), "2".to_owned()]));
        assert_eq!(of("mode").choices, ["fast", "safe"]);
        assert_eq!((of("loud").input(), of("loud").default.as_deref()), (ArgumentInput::Flag, Some("false")));
        assert_eq!((of("maybe").input(), of("maybe").kind.as_str()), (ArgumentInput::Flag, "boolean"));
        assert_eq!((of("filter").input(), of("tags").input()), (ArgumentInput::Json, ArgumentInput::Json));
        assert_eq!((of("count").input(), of("note").input()), (ArgumentInput::Text, ArgumentInput::Text));
        // One alternative takes any whole number, so it is not a choice; its alternatives have no one type.
        assert_eq!((of("open").input(), of("open").kind.as_str(), of("open").choices.len()), (ArgumentInput::Text, "", 0));
    }

    #[test]
    fn no_connection_yet_reads_as_disconnected_with_nothing_declared() {
        let view = McpView::from_status(&Value::Null);
        assert_eq!(view.state, SocketState::Disconnected);
        assert!(view.tools.is_empty() && view.lines.is_empty());
        assert_eq!(view.server, "");
    }

    #[test]
    fn a_status_event_replaces_what_was_declared_and_keeps_the_log() {
        let mut view = McpView::from_status(&status());
        view.apply_message_event(&json!({"message": line("c-1", "tool", "ok")}));
        view.apply_status_event(&json!({"status": "error", "error": "The server closed the connection.",
                                        "transport": "stdio", "serverInfo": null, "tools": []}));
        assert_eq!(view.state, SocketState::Error);
        assert_eq!(view.error.as_deref(), Some("The server closed the connection."));
        assert!(view.tools.is_empty() && view.prompts.is_empty() && view.resources.is_empty());
        assert_eq!((view.server.as_str(), view.instructions.as_deref()), ("", None));
        assert_eq!(view.lines.len(), 1);
    }

    #[test]
    fn a_call_is_one_line_that_gets_its_answer() {
        let mut view = McpView::default();
        view.apply_message_event(&json!({"message": line("c-1", "tool", "pending")}));
        assert_eq!(view.waiting(), 1);
        let pending = &view.lines[0];
        assert_eq!(pending.summary(), "echo {\"message\":\"hi\"}");
        assert_eq!(pending.note().as_deref(), Some("waiting…"));
        assert_eq!(pending.shown(), "The server has not answered yet.");

        view.apply_message_event(&json!({"message": line("c-1", "tool", "ok")}));
        assert_eq!((view.lines.len(), view.waiting()), (1, 0));
        let answered = &view.lines[0];
        assert_eq!((answered.status, answered.note().as_deref(), answered.shown().as_str()), (CallStatus::Ok, Some("12 ms"), "hi"));

        view.apply_message_event(&json!({"message": line("c-2", "tool", "error")}));
        assert_eq!(view.lines[1].note().as_deref(), Some("failed · 12 ms"));
        view.apply_message_event(&json!({"nothing": true}));
        assert_eq!(view.lines.len(), 2);
    }

    #[test]
    fn what_the_server_sent_unasked_is_a_line_without_an_ending() {
        let parsed = |value: Value| McpLine::from_value(&value).unwrap();
        let notice = parsed(json!({"id": "c-1", "type": "notification", "name": "notifications/message",
                                   "arguments": "", "status": "ok", "data": "{\"level\":\"info\"}", "text": "{\"level\":\"info\"}"}));
        assert_eq!((notice.kind.label(), notice.note()), ("notice", None));
        assert_eq!(notice.summary(), "notifications/message {\"level\":\"info\"}");
        let said = parsed(json!({"id": "c-2", "type": "stderr", "name": "", "status": "ok", "data": "starting"}));
        assert_eq!((said.kind, said.summary().as_str(), said.shown().as_str()), (LineKind::Stderr, "starting", "starting"));
        let read = parsed(json!({"id": "c-3", "type": "resource", "name": "mem://a", "arguments": "", "status": "ok", "text": "alpha"}));
        assert_eq!(read.summary(), "mem://a");
        let bare = parsed(json!({"id": "c-4", "type": "tool", "name": "now", "arguments": "{}", "status": "ok"}));
        assert_eq!((bare.summary().as_str(), bare.note().as_deref()), ("now", Some("ok")));
        assert!(McpLine::from_value(&json!({"type": "tool"})).is_none());
    }

    #[test]
    fn the_log_is_bounded_like_the_engines_and_can_be_cleared() {
        let mut view = McpView::default();
        for n in 0..(MAX_LINES + 25) {
            view.apply_message_event(&json!({"message": line(&format!("c-{n}"), "tool", "ok")}));
        }
        assert_eq!(view.lines.len(), MAX_LINES);
        assert_eq!(view.lines[0].id, "c-25");
        view.clear_lines();
        assert!(view.lines.is_empty());
    }
}
