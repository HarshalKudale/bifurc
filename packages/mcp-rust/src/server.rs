//! The MCP server: `tools/list` and `tools/call` over stdio.
//!
//! `tools/list` is a fixed list, as the protocol asks (it must not change because of an earlier request): the
//! tools marked upfront, then three that reach the rest — `search_tools` finds a tool by what it is for,
//! `describe_tool` gives its arguments, `call_tool` runs it. With `--all-tools` every tool is listed and those
//! three are not, for a host that does its own tool search.

use std::sync::Arc;

use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorCode, Implementation,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool as McpTool, ToolAnnotations,
    },
    service::RequestContext,
};
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;

use crate::{
    invoke::{Outcome, Settings, run},
    search::{DEFAULT_LIMIT, search},
    tools::{Effect, Tool, catalog},
};

pub const SEARCH_TOOLS: &str = "search_tools";
pub const DESCRIBE_TOOL: &str = "describe_tool";
pub const CALL_TOOL: &str = "call_tool";

/// What a client is told about the server as a whole.
const INSTRUCTIONS: &str = "Bifurc is a local API workbench: saved requests of every protocol, mocks, <sub>.localhost \
    mappings, forward-proxy rules, environments and health checks, kept per workspace by an engine that also runs \
    the proxy. These tools are the commands of its `bifurc` command line and act on the same engine as the desktop \
    app, so what they change shows there at once. An item is named by its id or by its name. Deleting anything, \
    writing scripts, entering secrets and signing in are left to the user in the desktop app.";

const ALL_LISTED: &str = "";
const SOME_LISTED: &str = " Only the everyday tools are listed; find the others with search_tools (for example \
    \"health check\", \"environment\", \"proxy rule\", \"workspace\"), read one's arguments with describe_tool and run \
    it with call_tool.";

/// What a host is told about a tool's effect.
fn annotations(effect: Effect) -> ToolAnnotations {
    let hints = ToolAnnotations::new();
    match effect {
        Effect::Reads => hints.read_only(true).open_world(false),
        Effect::Adds => hints.read_only(false).destructive(false).open_world(false),
        Effect::Changes => hints.read_only(false).destructive(true).idempotent(true).open_world(false),
        Effect::Runs => hints.read_only(false).destructive(false).open_world(true),
    }
}

fn listed(tool: &Tool) -> McpTool {
    McpTool::new(tool.name, tool.description(), Arc::new(tool.input_schema()))
        .with_annotations(annotations(tool.effect))
}

fn schema(value: Value) -> Arc<Map<String, Value>> {
    Arc::new(value.as_object().cloned().unwrap_or_default())
}

/// The three tools that reach the tools that are not listed.
fn meta_tools() -> Vec<McpTool> {
    vec![
        McpTool::new(
            SEARCH_TOOLS,
            "Find Bifurc tools by what they are for. Returns the name and a one-line summary of each match; run a \
             match with call_tool. An empty query lists every tool.",
            schema(json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "What you want to do, in a few words, e.g. \"create health check\"." },
                    "detail": {
                        "type": "string",
                        "enum": ["names", "summary", "full"],
                        "description": "names: only the names. summary (default): name and one line. full: with every argument, so describe_tool is not needed.",
                    },
                    "limit": { "type": "integer", "minimum": 1, "description": "The most tools to return. Default: 10." },
                },
                "additionalProperties": false,
            })),
        )
        .with_annotations(ToolAnnotations::new().read_only(true).open_world(false)),
        McpTool::new(
            DESCRIBE_TOOL,
            "Show the full description and the arguments (JSON Schema) of one Bifurc tool found with search_tools.",
            schema(json!({
                "type": "object",
                "properties": { "name": { "type": "string", "description": "The tool's name." } },
                "required": ["name"],
                "additionalProperties": false,
            })),
        )
        .with_annotations(ToolAnnotations::new().read_only(true).open_world(false)),
        McpTool::new(
            CALL_TOOL,
            "Run a Bifurc tool found with search_tools. What it does is what that tool does: one whose name ends in \
             _list or _show only reads; _create adds; _update, _enable, _disable and _use change something.",
            schema(json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "The tool's name." },
                    "arguments": { "type": "object", "description": "The tool's arguments, as describe_tool shows them." },
                },
                "required": ["name"],
                "additionalProperties": false,
            })),
        )
        .with_annotations(ToolAnnotations::new().read_only(false).destructive(true).open_world(true)),
    ]
}

/// One tool as `search_tools` (with `full`) and `describe_tool` show it.
fn described(tool: &Tool) -> Value {
    json!({ "name": tool.name, "description": tool.description(), "inputSchema": tool.input_schema() })
}

fn failed(text: impl Into<String>) -> Outcome {
    Outcome { text: text.into(), failed: true }
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

/// The server: the catalog and what it was started with.
pub struct Server {
    tools: Vec<Tool>,
    settings: Settings,
    /// `--all-tools`: list every tool and leave out the three that search.
    all_tools: bool,
    /// One command at a time, as on a command line: two unsaved requests sent at once would share a connection id.
    turn: Mutex<()>,
}

impl Server {
    pub fn new(settings: Settings, all_tools: bool) -> Self {
        Self { tools: catalog(), settings, all_tools, turn: Mutex::new(()) }
    }

    /// What `tools/list` answers.
    pub fn listing(&self) -> Vec<McpTool> {
        if self.all_tools {
            return self.tools.iter().map(listed).collect();
        }
        let mut tools: Vec<McpTool> = self.tools.iter().filter(|tool| tool.upfront).map(listed).collect();
        tools.extend(meta_tools());
        tools
    }

    fn tool(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|tool| tool.name == name)
    }

    /// The error for a name that is no tool, with the tools it may have meant.
    fn unknown(&self, name: &str) -> Outcome {
        let near: Vec<&str> = search(&self.tools, name, 5).into_iter().map(|tool| tool.name).collect();
        let hint =
            if near.is_empty() { "see search_tools".to_string() } else { format!("did you mean: {}", near.join(", ")) };
        failed(format!("no tool \"{name}\"; {hint}"))
    }

    fn search_tools(&self, arguments: &Map<String, Value>) -> Outcome {
        let query = arguments.get("query").and_then(Value::as_str).unwrap_or_default();
        let limit = arguments.get("limit").and_then(Value::as_u64).map_or(DEFAULT_LIMIT, |limit| limit.max(1) as usize);
        let detail = arguments.get("detail").and_then(Value::as_str).unwrap_or("summary");
        if !["names", "summary", "full"].contains(&detail) {
            return failed("search_tools: \"detail\" must be one of names, summary, full");
        }
        let hits = search(&self.tools, query, limit);
        if hits.is_empty() {
            return Outcome {
                text: format!(
                    "No tool matches \"{query}\". The tools are about: requests, mocks, mappings, rules, env, health, \
                     workspaces, vault, account, services. An empty query lists them all."
                ),
                failed: false,
            };
        }
        let text = match detail {
            "names" => hits.iter().map(|tool| tool.name).collect::<Vec<_>>().join("\n"),
            "full" => pretty(&Value::Array(hits.iter().map(|tool| described(tool)).collect())),
            _ => hits.iter().map(|tool| format!("{}: {}", tool.name, tool.summary)).collect::<Vec<_>>().join("\n"),
        };
        Outcome { text, failed: false }
    }

    /// Run the tool `name` — one of the catalog or one of the three that reach it — and return what it answered.
    /// Blocks while the command runs.
    pub fn call(&self, name: &str, arguments: &Map<String, Value>) -> Option<Outcome> {
        let wanted = |arguments: &Map<String, Value>| arguments.get("name").and_then(Value::as_str).map(str::to_string);
        Some(match name {
            SEARCH_TOOLS if !self.all_tools => self.search_tools(arguments),
            DESCRIBE_TOOL if !self.all_tools => match wanted(arguments) {
                None => failed("describe_tool: missing \"name\""),
                Some(name) => match self.tool(&name) {
                    Some(tool) => Outcome { text: pretty(&described(tool)), failed: false },
                    None => self.unknown(&name),
                },
            },
            CALL_TOOL if !self.all_tools => match wanted(arguments) {
                None => failed("call_tool: missing \"name\""),
                Some(name) => {
                    let inner = match arguments.get("arguments") {
                        None | Some(Value::Null) => Map::new(),
                        Some(Value::Object(inner)) => inner.clone(),
                        Some(_) => return Some(failed("call_tool: \"arguments\" must be an object")),
                    };
                    match self.tool(&name) {
                        Some(tool) => run(tool, &inner, &self.settings),
                        None => self.unknown(&name),
                    }
                }
            },
            // An unlisted tool called by its own name still runs: a host may have learned it from a search.
            _ => run(self.tool(name)?, arguments, &self.settings),
        })
    }
}

/// The server as the SDK drives it; a call takes its own handle to run off the reading thread.
struct Handler(Arc<Server>);

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        let more = if self.0.all_tools { ALL_LISTED } else { SOME_LISTED };
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("bifurc-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(format!("{INSTRUCTIONS}{more}"))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.0.listing()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let server = Arc::clone(&self.0);
        let name = request.name.to_string();
        let arguments = request.arguments.unwrap_or_default();
        let _turn = self.0.turn.lock().await;
        // A command blocks on the engine's socket, so it runs off the thread that reads stdin.
        let outcome = tokio::task::spawn_blocking(move || server.call(&name, &arguments).ok_or(name))
            .await
            .map_err(|error| ErrorData::internal_error(format!("the tool did not finish: {error}"), None))?
            .map_err(|name| ErrorData::new(ErrorCode::INVALID_PARAMS, format!("Unknown tool: {name}"), None))?;
        let content = vec![ContentBlock::text(outcome.text)];
        Ok(if outcome.failed { CallToolResult::error(content) } else { CallToolResult::success(content) }.into())
    }
}

/// Serve on this process's stdin and stdout until the client closes them.
pub async fn serve(settings: Settings, all_tools: bool) -> Result<(), String> {
    let handler = Handler(Arc::new(Server::new(settings, all_tools)));
    let running = handler.serve(rmcp::transport::stdio()).await.map_err(|error| format!("could not start: {error}"))?;
    running.waiting().await.map_err(|error| format!("stopped unexpectedly: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(all_tools: bool) -> (tempfile::TempDir, Server) {
        let dir = tempfile::tempdir().unwrap();
        let settings =
            Settings { data_dir: Some(dir.path().to_string_lossy().into_owned()), no_start: true, env_data_dir: None };
        (dir, Server::new(settings, all_tools))
    }

    fn call(server: &Server, name: &str, arguments: Value) -> Outcome {
        server.call(name, arguments.as_object().unwrap()).unwrap_or_else(|| panic!("{name} is not a tool"))
    }

    fn names(server: &Server) -> Vec<String> {
        server.listing().iter().map(|tool| tool.name.to_string()).collect()
    }

    #[test]
    fn the_everyday_tools_are_listed_then_the_three_that_reach_the_rest() {
        let (_dir, server) = server(false);
        assert_eq!(
            names(&server),
            [
                "status",
                "requests_list",
                "requests_show",
                "requests_send",
                "mocks_list",
                "mocks_create",
                "mappings_list",
                "mappings_create",
                "services_list",
                "search_tools",
                "describe_tool",
                "call_tool",
            ]
        );
    }

    #[test]
    fn all_tools_lists_the_catalog_and_nothing_else() {
        let (_dir, server) = server(true);
        let listed = names(&server);
        assert_eq!(listed.len(), catalog().len());
        assert!(!listed.iter().any(|name| name == SEARCH_TOOLS || name == CALL_TOOL));
        assert!(server.call(SEARCH_TOOLS, &Map::new()).is_none());
    }

    #[test]
    fn a_listed_tool_carries_its_schema_and_what_it_does() {
        let (_dir, server) = server(false);
        let listing = serde_json::to_value(server.listing()).unwrap();
        let send = listing.as_array().unwrap().iter().find(|tool| tool["name"] == "requests_send").unwrap();
        assert_eq!(send["inputSchema"]["type"], "object");
        assert_eq!(send["annotations"]["openWorldHint"], true);
        let list = listing.as_array().unwrap().iter().find(|tool| tool["name"] == "mocks_list").unwrap();
        assert_eq!(list["annotations"]["readOnlyHint"], true);
        assert!(list["description"].as_str().unwrap().starts_with("List the mocks"));
    }

    #[test]
    fn search_gives_names_summaries_or_whole_tools() {
        let (_dir, server) = server(false);
        let found = call(&server, SEARCH_TOOLS, json!({"query": "create health check", "limit": 1}));
        assert_eq!(found, Outcome { text: "health_create: Create a health check for a URL.".into(), failed: false });
        let found = call(&server, SEARCH_TOOLS, json!({"query": "rules", "detail": "names"}));
        assert_eq!(found.text.lines().count(), 6);
        let found = call(&server, SEARCH_TOOLS, json!({"query": "health_run", "detail": "full", "limit": 1}));
        let parsed: Value = serde_json::from_str(&found.text).unwrap();
        assert_eq!(parsed[0]["inputSchema"]["properties"]["check"]["type"], "string");
        let found = call(&server, SEARCH_TOOLS, json!({"limit": 100}));
        assert_eq!(found.text.lines().count(), catalog().len());
        let found = call(&server, SEARCH_TOOLS, json!({"query": "xyzzy"}));
        assert!(!found.failed && found.text.starts_with("No tool matches \"xyzzy\""));
    }

    #[test]
    fn describe_gives_one_tools_arguments() {
        let (_dir, server) = server(false);
        let shown = call(&server, DESCRIBE_TOOL, json!({"name": "env_use"}));
        let parsed: Value = serde_json::from_str(&shown.text).unwrap();
        assert_eq!(parsed["name"], "env_use");
        assert_eq!(parsed["inputSchema"]["required"], json!(["environment"]));
        let shown = call(&server, DESCRIBE_TOOL, json!({"name": "env_usee"}));
        assert!(shown.failed && shown.text.starts_with("no tool \"env_usee\"; did you mean: env_"), "{}", shown.text);
    }

    #[test]
    fn call_tool_runs_the_tool_it_names_with_its_arguments() {
        let (_dir, server) = server(false);
        let ran = call(
            &server,
            CALL_TOOL,
            json!({"name": "health_create", "arguments": {"name": "Users", "url": "http://x"}}),
        );
        assert!(ran.failed && ran.text.starts_with("No engine is running for"), "{}", ran.text);
        let ran = call(&server, CALL_TOOL, json!({"name": "env_list"}));
        assert!(ran.text.starts_with("No engine is running for"), "{}", ran.text);
        let ran = call(&server, CALL_TOOL, json!({"name": "health_create", "arguments": {"name": "Users"}}));
        assert!(ran.text.starts_with("health_create: missing \"url\""), "{}", ran.text);
        let ran = call(&server, CALL_TOOL, json!({"name": "health_create", "arguments": "name=Users"}));
        assert_eq!(ran.text, "call_tool: \"arguments\" must be an object");
        let ran = call(&server, CALL_TOOL, json!({"name": "vault_unseal"}));
        assert_eq!(ran.text, "no tool \"vault_unseal\"; did you mean: vault_list");
        assert_eq!(call(&server, CALL_TOOL, json!({})).text, "call_tool: missing \"name\"");
    }

    #[test]
    fn a_tool_runs_by_its_own_name_listed_or_not_and_another_name_is_no_tool() {
        let (_dir, server) = server(false);
        for name in ["status", "env_list"] {
            assert!(call(&server, name, json!({})).text.starts_with("No engine is running for"), "{name}");
        }
        assert!(server.call("engine_stop", &Map::new()).is_none());
    }
}
