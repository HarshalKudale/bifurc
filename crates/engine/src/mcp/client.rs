//! The MCP client itself, which is the official SDK (`rmcp`): this file starts it over one of the two ways a
//! server is reached and turns what it gives back into the JSON the registry keeps.
//!
//! A program is started through the OS shell and spoken to over its standard streams; an endpoint is spoken to
//! over Streamable HTTP with the engine's own HTTP client. What the server sends unasked, and what a program
//! writes to its diagnostics stream, reaches the connection's task as [`Heard`].

// The SDK marks a server's log messages as deprecated (SEP-2577); servers still send them, and they are shown.
#![allow(deprecated)]

use std::{collections::HashMap, process::Stdio};

use process_wrap::tokio::{CommandWrap, KillOnDrop};
use reqwest::header::{HeaderName, HeaderValue};
use rmcp::{
    ClientHandler, Peer, RoleClient, ServiceError, ServiceExt,
    model::{
        CallToolRequestParams, CancelledNotificationParam, ClientCapabilities, ClientConfig, CustomNotification,
        GetPromptRequestParams, Implementation, LoggingMessageNotificationParam, ProgressNotificationParam,
        ReadResourceRequestParams, ResourceUpdatedNotificationParam,
    },
    service::{ClientInitializeError, NotificationContext, RunningService},
    transport::{
        StreamableHttpClientTransport, TokioChildProcess,
        streamable_http_client::{StreamableHttpClientTransportConfig, StreamableHttpError},
    },
};
use serde::Serialize;
use serde_json::{Map, Value};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::mpsc::UnboundedSender,
};

use super::arguments::{CallKind, Catalog};
use crate::rest::direct_http::client as http_client;
use crate::utils::http::{NAME_NOT_FOUND_NO_HOST, REFUSED_NO_ADDRESS};

/// What reaches a connection's task besides the answers to what it asked.
pub enum Heard {
    /// A notification of the server, with its parameters as JSON text.
    Notification { method: String, data: String },
    /// A line the program wrote to its diagnostics stream.
    Stderr(String),
}

/// The client's side of a connection: it offers the server nothing to ask for (no sampling, no roots, no
/// elicitation) and passes on what the server says.
pub struct Handler {
    heard: UnboundedSender<Heard>,
}

impl Handler {
    fn note(&self, method: &str, params: &impl Serialize) {
        let data = match serde_json::to_value(params) {
            Ok(Value::Null) | Err(_) => String::new(),
            Ok(params) => params.to_string(),
        };
        let _ = self.heard.send(Heard::Notification { method: method.to_string(), data });
    }
}

type Context = NotificationContext<RoleClient>;

impl ClientHandler for Handler {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(ClientCapabilities::default(), Implementation::new("bifurc", env!("CARGO_PKG_VERSION")))
    }

    async fn on_cancelled(&self, params: CancelledNotificationParam, _: Context) {
        self.note("notifications/cancelled", &params);
    }

    async fn on_progress(&self, params: ProgressNotificationParam, _: Context) {
        self.note("notifications/progress", &params);
    }

    async fn on_logging_message(&self, params: LoggingMessageNotificationParam, _: Context) {
        self.note("notifications/message", &params);
    }

    async fn on_resource_updated(&self, params: ResourceUpdatedNotificationParam, _: Context) {
        self.note("notifications/resources/updated", &params);
    }

    async fn on_resource_list_changed(&self, _: Context) {
        self.note("notifications/resources/list_changed", &Value::Null);
    }

    async fn on_tool_list_changed(&self, _: Context) {
        self.note("notifications/tools/list_changed", &Value::Null);
    }

    async fn on_prompt_list_changed(&self, _: Context) {
        self.note("notifications/prompts/list_changed", &Value::Null);
    }

    async fn on_custom_notification(&self, notification: CustomNotification, _: Context) {
        self.note(&notification.method, &notification.params);
    }
}

pub type Client = RunningService<RoleClient, Handler>;

/// Everything a connect needs, already resolved.
pub enum Prepared {
    Stdio { command: String, env: Vec<(String, String)> },
    Http { url: url::Url, headers: HashMap<HeaderName, HeaderValue> },
}

impl Prepared {
    pub fn transport(&self) -> &'static str {
        match self {
            Prepared::Stdio { .. } => "stdio",
            Prepared::Http { .. } => "http",
        }
    }
}

/// The headers the transport sets itself; a request cannot give them.
pub const TRANSPORT_HEADERS: [&str; 4] = ["accept", "mcp-session-id", "mcp-protocol-version", "last-event-id"];

/// The command line interpreted by the OS shell, so `npx`, quoting and `PATH` work as they do in a terminal.
/// The program ends with everything it started, and never shows a console window.
fn shell_command(command: &str) -> CommandWrap {
    #[cfg(windows)]
    {
        use process_wrap::tokio::{CreationFlags, JobObject};
        use windows::Win32::System::Threading::CREATE_NO_WINDOW;

        let shell = std::env::var("COMSPEC").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "cmd.exe".to_string());
        let mut cmd = tokio::process::Command::new(shell);
        cmd.raw_arg("/d /s /c").raw_arg(format!("\"{command}\""));
        let mut wrapped = CommandWrap::from(cmd);
        wrapped.wrap(CreationFlags(CREATE_NO_WINDOW)).wrap(JobObject).wrap(KillOnDrop);
        wrapped
    }
    #[cfg(not(windows))]
    {
        use process_wrap::tokio::ProcessGroup;

        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.arg("-c").arg(command);
        let mut wrapped = CommandWrap::from(cmd);
        wrapped.wrap(ProcessGroup::leader()).wrap(KillOnDrop);
        wrapped
    }
}

/// Open the connection and shake hands. Fails with a sentence for the user.
pub async fn connect(prepared: Prepared, heard: UnboundedSender<Heard>) -> Result<Client, String> {
    let handler = Handler { heard: heard.clone() };
    match prepared {
        Prepared::Stdio { command, env } => {
            let mut wrapped = shell_command(&command);
            let cmd = wrapped.command_mut();
            // The engine's own directory means nothing to the user; their home is where a terminal would start.
            let home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
            if let Some(home) = std::env::var_os(home).filter(|home| !home.is_empty()) {
                cmd.current_dir(home);
            }
            cmd.envs(env.iter().map(|(name, value)| (name.as_str(), value.as_str())));
            let (transport, stderr) = TokioChildProcess::builder(wrapped)
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|error| format!("Cannot start the server: {error}"))?;
            if let Some(stderr) = stderr {
                tokio::spawn(async move {
                    let mut lines = BufReader::new(stderr).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let line = line.trim_end();
                        if !line.is_empty() && heard.send(Heard::Stderr(line.to_string())).is_err() {
                            return;
                        }
                    }
                });
            }
            handler.serve(transport).await.map_err(|error| describe_start(&error, true))
        }
        Prepared::Http { url, headers } => {
            let config = StreamableHttpClientTransportConfig::with_uri(url.as_str()).custom_headers(headers);
            let transport = StreamableHttpClientTransport::with_client(http_client()?.clone(), config);
            handler.serve(transport).await.map_err(|error| describe_start(&error, false))
        }
    }
}

/// The innermost message of an error's chain, in plain words for a refused connection or unknown host.
fn root_cause(error: &(dyn std::error::Error + 'static)) -> String {
    let mut message = String::new();
    let mut current = Some(error);
    while let Some(inner) = current {
        if inner.downcast_ref::<std::io::Error>().is_some_and(|io| io.kind() == std::io::ErrorKind::ConnectionRefused) {
            return REFUSED_NO_ADDRESS.to_string();
        }
        message = inner.to_string();
        if message.contains("dns error") {
            return NAME_NOT_FOUND_NO_HOST.to_string();
        }
        // The HTTP transport holds the HTTP client's error without naming it as its source.
        current = match inner.downcast_ref::<StreamableHttpError<reqwest::Error>>() {
            Some(StreamableHttpError::Client(of_client)) => Some(of_client),
            _ => inner.source(),
        };
    }
    message
}

/// Why a connection did not open, for the user. `program` says the server is one the engine started.
fn describe_start(error: &ClientInitializeError, program: bool) -> String {
    if error.is_authorization_required() {
        return "The server wants credentials. Add them as a header, such as Authorization.".to_string();
    }
    match error {
        ClientInitializeError::ConnectionClosed(_) if program => {
            "The program ended before the MCP handshake finished".to_string()
        }
        ClientInitializeError::ConnectionClosed(_) => {
            "The server closed the connection before the MCP handshake finished".to_string()
        }
        ClientInitializeError::JsonRpcError(error) => error.message.to_string(),
        ClientInitializeError::TransportError { error, .. } => {
            let cause = root_cause(error.error.as_ref());
            // The first line, without the page a web server answered with.
            let cause = cause.lines().next().unwrap_or_default();
            let cause = cause.split(": <").next().unwrap_or_default().trim().trim_end_matches('.');
            if cause == REFUSED_NO_ADDRESS.trim_end_matches('.') || cause == NAME_NOT_FOUND_NO_HOST.trim_end_matches('.') {
                return format!("{cause}.");
            }
            format!("{cause}. Is this an MCP server?")
        }
        other => format!("{}. Is this an MCP server?", other.to_string().trim_end_matches('.')),
    }
}

/// Whether the server said in the handshake that it has this list.
pub fn declares(client: &Client, catalog: Catalog) -> bool {
    client.peer_info().is_some_and(|info| match catalog {
        Catalog::Tools => info.capabilities.tools.is_some(),
        Catalog::Prompts => info.capabilities.prompts.is_some(),
        Catalog::Resources | Catalog::ResourceTemplates => info.capabilities.resources.is_some(),
    })
}

/// What the server said of itself in the handshake: its identity, the protocol revision, its capabilities and
/// its instructions.
pub fn handshake(client: &Client) -> (Value, Option<String>, Value, Option<String>) {
    match client.peer_info() {
        Some(info) => (
            serde_json::to_value(&info.server_info).unwrap_or(Value::Null),
            Some(info.protocol_version.to_string()),
            serde_json::to_value(&info.capabilities).unwrap_or(Value::Null),
            info.instructions.clone(),
        ),
        None => (Value::Null, None, Value::Null, None),
    }
}

fn values<T: Serialize>(items: Result<Vec<T>, ServiceError>) -> Vec<Value> {
    items.unwrap_or_default().iter().filter_map(|item| serde_json::to_value(item).ok()).collect()
}

/// Every page of one of the server's lists. A server that declares a list and then cannot produce it has an
/// empty one.
pub async fn list(peer: &Peer<RoleClient>, catalog: Catalog) -> Vec<Value> {
    match catalog {
        Catalog::Tools => values(peer.list_all_tools().await),
        Catalog::Prompts => values(peer.list_all_prompts().await),
        Catalog::Resources => values(peer.list_all_resources().await),
        Catalog::ResourceTemplates => values(peer.list_all_resource_templates().await),
    }
}

/// Call the tool `name`, get the prompt `name` or read the resource whose URI is `name`. Returns the server's
/// result, or the error (the server's error object, or a transport failure as `{ "message": .. }`), as JSON.
pub async fn call(
    peer: &Peer<RoleClient>,
    kind: CallKind,
    name: String,
    arguments: Map<String, Value>,
) -> Result<Value, Value> {
    let result = match kind {
        CallKind::Tool => {
            let params = CallToolRequestParams::new(name).with_arguments(arguments);
            peer.call_tool(params).await.map(|result| serde_json::to_value(result))
        }
        CallKind::Prompt => {
            let mut params = GetPromptRequestParams::new(name);
            params.arguments = Some(arguments);
            peer.get_prompt(params).await.map(|result| serde_json::to_value(result))
        }
        CallKind::Resource => {
            peer.read_resource(ReadResourceRequestParams::new(name)).await.map(|result| serde_json::to_value(result))
        }
    };
    let failed = |message: String| serde_json::json!({ "message": message });
    match result {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(error)) => Err(failed(error.to_string())),
        Err(ServiceError::McpError(error)) => Err(serde_json::to_value(&error).unwrap_or_else(|_| failed(error.to_string()))),
        Err(ServiceError::TransportSend(error)) => Err(failed(root_cause(error.error.as_ref()))),
        Err(ServiceError::TransportClosed) => Err(failed("The connection ended before the server answered.".to_string())),
        Err(other) => Err(failed(other.to_string())),
    }
}
