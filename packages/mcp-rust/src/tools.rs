//! The tools: one per command of the `bifurc` command line, each a row of [`catalog`].
//!
//! A row says which command it runs and where each of its arguments goes on that command's line (an operand,
//! an option, a switch). Nothing here knows what a command does: `invoke.rs` writes the command line and the
//! CLI's own code runs it, so a tool can never do what the command line cannot. In particular there is no
//! tool that deletes, none that reads a secret and none that stops the engine.

use serde_json::{Map, Value, json};

/// What running a tool does, as an MCP client is told (`annotations`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Reads only.
    Reads,
    /// Adds a saved item.
    Adds,
    /// Changes a saved item or a selection.
    Changes,
    /// Runs something that reaches outside the engine: sends a request, checks a URL, pushes to a remote.
    Runs,
}

/// Where an argument goes on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// The command's next operand, in the order the parameters are listed.
    Operand,
    /// `--<option> <value>`.
    Option(&'static str),
    /// `--<on>` when true; when false `--<off>`, or nothing when the command has no such switch.
    Switch(&'static str, Option<&'static str>),
    /// An object of names to values: `--<option> "<name><separator><value>"` for each.
    Pairs(&'static str, &'static str),
    /// A list: `--<option> <value>` for each.
    Each(&'static str),
}

/// What an argument is, in the tool's input schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Text,
    Integer,
    Number,
    Boolean,
    /// One of these words.
    Choice(&'static [&'static str]),
    /// A JSON object or array, or its text.
    Json,
    /// An object whose values are text.
    TextMap,
    /// An object whose values are anything.
    AnyMap,
    /// A list of text.
    TextList,
}

/// One argument of a tool.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: &'static str,
    pub about: &'static str,
    pub ty: Type,
    pub place: Place,
    pub required: bool,
}

impl Param {
    fn required(mut self) -> Self {
        self.required = true;
        self
    }
}

/// One tool.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub name: &'static str,
    /// The words of the command it runs: `["mocks", "create"]`.
    pub words: &'static [&'static str],
    /// One line: what a search result shows.
    pub summary: &'static str,
    /// What else the model needs to use it well; may be empty.
    pub details: &'static str,
    pub effect: Effect,
    /// Whether it is listed by `tools/list`; the others are found with `search_tools` and run with `call_tool`.
    pub upfront: bool,
    pub params: Vec<Param>,
}

impl Tool {
    /// The whole description: the summary, then the details.
    pub fn description(&self) -> String {
        if self.details.is_empty() { self.summary.to_string() } else { format!("{} {}", self.summary, self.details) }
    }

    /// The JSON Schema of the tool's arguments.
    pub fn input_schema(&self) -> Map<String, Value> {
        let mut properties = Map::new();
        for param in &self.params {
            let mut schema = match param.ty {
                Type::Text => json!({ "type": "string" }),
                Type::Integer => json!({ "type": "integer" }),
                Type::Number => json!({ "type": "number" }),
                Type::Boolean => json!({ "type": "boolean" }),
                Type::Choice(words) => json!({ "type": "string", "enum": words }),
                Type::Json => json!({ "type": ["object", "array", "string"] }),
                Type::TextMap => json!({ "type": "object", "additionalProperties": { "type": "string" } }),
                Type::AnyMap => json!({ "type": "object" }),
                Type::TextList => json!({ "type": "array", "items": { "type": "string" } }),
            };
            schema["description"] = json!(param.about);
            properties.insert(param.name.to_string(), schema);
        }
        let required: Vec<&str> = self.params.iter().filter(|p| p.required).map(|p| p.name).collect();
        let mut schema = Map::new();
        schema.insert("type".into(), json!("object"));
        schema.insert("properties".into(), Value::Object(properties));
        if !required.is_empty() {
            schema.insert("required".into(), json!(required));
        }
        schema.insert("additionalProperties".into(), json!(false));
        schema
    }
}

fn operand(name: &'static str, about: &'static str) -> Param {
    Param { name, about, ty: Type::Text, place: Place::Operand, required: false }
}

fn text(name: &'static str, option: &'static str, about: &'static str) -> Param {
    Param { name, about, ty: Type::Text, place: Place::Option(option), required: false }
}

fn typed(name: &'static str, option: &'static str, ty: Type, about: &'static str) -> Param {
    Param { name, about, ty, place: Place::Option(option), required: false }
}

fn switch(name: &'static str, on: &'static str, off: Option<&'static str>, about: &'static str) -> Param {
    Param { name, about, ty: Type::Boolean, place: Place::Switch(on, off), required: false }
}

fn headers(about: &'static str) -> Param {
    Param { name: "headers", about, ty: Type::TextMap, place: Place::Pairs("header", ": "), required: false }
}

/// The argument every tool takes: the raw answer instead of the readable one.
fn json_switch() -> Param {
    switch("json", "json", None, "Return the engine's raw JSON instead of the readable text.")
}

/// The argument of every tool that acts on one workspace.
fn workspace() -> Param {
    text("workspace", "workspace", "Workspace id. Default: the active workspace.")
}

struct Row {
    name: &'static str,
    words: &'static [&'static str],
    summary: &'static str,
    details: &'static str,
    effect: Effect,
    upfront: bool,
    /// Whether it acts on one workspace, and so takes `workspace`.
    scoped: bool,
    params: Vec<Param>,
}

impl Row {
    fn tool(self) -> Tool {
        let mut params = self.params;
        if self.scoped {
            params.push(workspace());
        }
        params.push(json_switch());
        Tool {
            name: self.name,
            words: self.words,
            summary: self.summary,
            details: self.details,
            effect: self.effect,
            upfront: self.upfront,
            params,
        }
    }
}

const REQUEST_TYPES: &[&str] = &["rest", "graphql", "grpc", "soap", "websocket", "sse", "socketio", "mcp"];

const TYPE_ABOUT: &str = "The type of request. Default: rest. For a saved request it is only needed to tell apart two \
    requests of different types with the same name.";

/// The options that describe a request of some type: what `requests create`, `update` and an unsaved `send` take.
fn request_options() -> Vec<Param> {
    vec![
        text(
            "url",
            "url",
            "Where the request goes: a URL; a gRPC server address; for mcp the command line that starts \
            the server (stdio) or the address of its endpoint (http). May hold {{variables}}.",
        ),
        headers(
            "Headers by name (gRPC: metadata; an mcp program: environment variables). On an update an empty \
            value removes the header.",
        ),
        text(
            "method",
            "method",
            "rest and sse: the HTTP method (default GET). grpc: the method to call, as \
            pkg.Service/Method.",
        ),
        text(
            "body",
            "body",
            "rest, soap and sse: the request body. grpc: the message as JSON (a client-streaming \
            method takes a JSON array of messages).",
        ),
        text("query", "query", "graphql: the query or mutation."),
        typed("variables", "variables", Type::Json, "graphql: the variables, a JSON object."),
        text("operation", "operation", "graphql: the name of the operation to run."),
        text("proto", "proto", "grpc: the id of the saved .proto file that declares the method."),
        switch(
            "reflection",
            "reflection",
            None,
            "grpc: find the method through the server's reflection (what happens \
            when no proto is given).",
        ),
        text("action", "action", "soap: the SOAPAction."),
        text("namespace", "namespace", "socketio: the namespace, e.g. /chat."),
        text("path", "path", "socketio: the path of the Socket.IO endpoint. Default: /socket.io."),
        typed("auth", "auth", Type::Json, "socketio: the auth payload, a JSON object."),
        Param {
            name: "listen",
            about: "socketio: the events to log when the server sends them. Default: every event.",
            ty: Type::TextList,
            place: Place::Each("listen"),
            required: false,
        },
        typed(
            "transport",
            "transport",
            Type::Choice(&["stdio", "http"]),
            "mcp: how the server is reached. Default: \
            what the url looks like.",
        ),
    ]
}

/// The options only a send takes.
fn exchange_options() -> Vec<Param> {
    vec![
        typed(
            "wait",
            "wait",
            Type::Number,
            "websocket, sse, socketio: how many seconds to listen (default 2; sse 5). \
            mcp: how many seconds to wait for the answer (default 60).",
        ),
        text(
            "event",
            "event",
            "socketio: the event to emit; its arguments go in body (a JSON array is one argument \
            per element).",
        ),
        switch("ack", "ack", None, "socketio: ask the server to acknowledge the event."),
        text("tool", "tool", "mcp: the tool to call."),
        text("prompt", "prompt", "mcp: the prompt to get."),
        text("resource", "resource", "mcp: the URI of the resource to read."),
        Param {
            name: "arguments",
            about: "mcp: the arguments of the tool or the prompt, by name.",
            ty: Type::AnyMap,
            place: Place::Pairs("arg", "="),
            required: false,
        },
    ]
}

fn concat(parts: Vec<Vec<Param>>) -> Vec<Param> {
    parts.into_iter().flatten().collect()
}

/// `list`, `show`, `enable` and `disable` of a group whose items are named by `named`.
fn group_rows(group: &'static str) -> [Row; 4] {
    let (words, names, item): ([&'static [&'static str]; 4], [&'static str; 4], &'static str) = match group {
        "mappings" => (
            [&["mappings", "list"], &["mappings", "show"], &["mappings", "enable"], &["mappings", "disable"]],
            ["mappings_list", "mappings_show", "mappings_enable", "mappings_disable"],
            "The mapping's id or domain.",
        ),
        "rules" => (
            [&["rules", "list"], &["rules", "show"], &["rules", "enable"], &["rules", "disable"]],
            ["rules_list", "rules_show", "rules_enable", "rules_disable"],
            "The rule's id or name.",
        ),
        _ => (
            [&["mocks", "list"], &["mocks", "show"], &["mocks", "enable"], &["mocks", "disable"]],
            ["mocks_list", "mocks_show", "mocks_enable", "mocks_disable"],
            "The mock's id or name.",
        ),
    };
    let (list, show, enable, disable) = match group {
        "mappings" => (
            "List the mappings: each serves <sub>.localhost from a local target.",
            "Show one mapping in full.",
            "Enable a mapping.",
            "Disable a mapping.",
        ),
        "rules" => (
            "List the forward-proxy rules: each sends requests whose URL matches a pattern to a mapping or another server.",
            "Show one forward-proxy rule in full.",
            "Enable a forward-proxy rule; another enabled rule with the same pattern is disabled.",
            "Disable a forward-proxy rule.",
        ),
        _ => (
            "List the mocks: canned responses the proxy answers with instead of the real server.",
            "Show one mock in full: what it matches and the response it gives.",
            "Enable a mock; another enabled mock of the same call is disabled.",
            "Disable a mock.",
        ),
    };
    let one = |name, words, summary, effect| Row {
        name,
        words,
        summary,
        details: "",
        effect,
        upfront: false,
        scoped: true,
        params: vec![operand(group_item(group), item).required()],
    };
    [
        Row {
            name: names[0],
            words: words[0],
            summary: list,
            details: "",
            effect: Effect::Reads,
            upfront: group != "rules",
            scoped: true,
            params: vec![],
        },
        one(names[1], words[1], show, Effect::Reads),
        one(names[2], words[2], enable, Effect::Changes),
        one(names[3], words[3], disable, Effect::Changes),
    ]
}

/// The name of the argument that names one item of a group.
fn group_item(group: &str) -> &'static str {
    match group {
        "mappings" => "mapping",
        "rules" => "rule",
        _ => "mock",
    }
}

/// The options of a mock's response, shared by `mocks create` and `mocks update`.
fn mock_options(regex_off: bool) -> Vec<Param> {
    vec![
        text("method", "method", "The HTTP method it answers; * for any. Default: GET."),
        text("name", "name", "A name for the mock."),
        typed("status", "status", Type::Integer, "The response status. Default: 200."),
        headers("Response headers by name. On an update an empty value removes the header."),
        text("body", "body", "The response body."),
        typed("delay", "delay", Type::Integer, "Milliseconds to wait before answering."),
        switch("regex", "regex", regex_off.then_some("no-regex"), "Whether the URL pattern is a regular expression."),
    ]
}

/// Every tool, in the order they are listed.
pub fn catalog() -> Vec<Tool> {
    let mut rows = vec![
        Row {
            name: "status",
            words: &["status"],
            summary: "Show the Bifurc engine, its proxy and the active workspace.",
            details: "",
            effect: Effect::Reads,
            upfront: true,
            scoped: false,
            params: vec![],
        },
        // -- Requests -----------------------------------------------------------------------------------------
        Row {
            name: "requests_list",
            words: &["requests", "list"],
            summary: "List the saved requests of every type (REST, GraphQL, gRPC, SOAP, WebSocket, SSE, Socket.IO, MCP).",
            details: "",
            effect: Effect::Reads,
            upfront: true,
            scoped: true,
            params: vec![typed("type", "type", Type::Choice(REQUEST_TYPES), "Only the requests of this type.")],
        },
        Row {
            name: "requests_show",
            words: &["requests", "show"],
            summary: "Show one saved request in full.",
            details: "",
            effect: Effect::Reads,
            upfront: true,
            scoped: true,
            params: vec![
                operand("request", "The request's id or name.").required(),
                typed("type", "type", Type::Choice(REQUEST_TYPES), TYPE_ABOUT),
            ],
        },
        Row {
            name: "requests_send",
            words: &["requests", "send"],
            summary: "Send a saved request, or one described here without saving it, and return the response.",
            details: "Give `request` to send a saved one as it is saved: the engine resolves its {{variables}} and \
                runs its scripts and tests. Otherwise give `url` and the options of the type. A websocket, sse or \
                socketio request connects, listens for `wait` seconds and returns what went by. An mcp request \
                returns the tools, prompts and resources the server declares, or the answer to one `tool`, `prompt` \
                or `resource`. The result is an error when a test fails or the call does.",
            effect: Effect::Runs,
            upfront: true,
            scoped: true,
            params: concat(vec![
                vec![
                    operand(
                        "request",
                        "The id or name of a saved request. Leave out to send the one described by url.",
                    ),
                    typed("type", "type", Type::Choice(REQUEST_TYPES), TYPE_ABOUT),
                ],
                request_options(),
                exchange_options(),
            ]),
        },
        Row {
            name: "requests_create",
            words: &["requests", "create"],
            summary: "Save a new request of any type.",
            details: "Give `url` and the options of the type, or `curl` to save the request a cURL command describes. \
                Scripts and tests are written in the desktop app.",
            effect: Effect::Adds,
            upfront: false,
            scoped: true,
            params: concat(vec![
                vec![
                    typed("type", "type", Type::Choice(REQUEST_TYPES), TYPE_ABOUT),
                    text("name", "name", "A name for the request. Default: made from its method and URL."),
                    text("curl", "curl", "A whole cURL command; the request is what it describes. Leave out url."),
                ],
                request_options(),
            ]),
        },
        Row {
            name: "requests_update",
            words: &["requests", "update"],
            summary: "Change a saved request; only what is given changes.",
            details: "Its scripts and tests are left as they are.",
            effect: Effect::Changes,
            upfront: false,
            scoped: true,
            params: concat(vec![
                vec![
                    operand("request", "The request's id or name.").required(),
                    typed("type", "type", Type::Choice(REQUEST_TYPES), TYPE_ABOUT),
                    text("name", "name", "A new name."),
                ],
                request_options(),
            ]),
        },
        Row {
            name: "requests_reflect",
            words: &["requests", "reflect"],
            summary: "List the services and methods a gRPC server reports through reflection.",
            details: "",
            effect: Effect::Runs,
            upfront: false,
            scoped: true,
            params: vec![operand("address", "The gRPC server address, e.g. localhost:50051.").required()],
        },
    ];

    // -- Mocks ------------------------------------------------------------------------------------------------
    rows.extend(group_rows("mocks"));
    rows.push(Row {
        name: "mocks_create",
        words: &["mocks", "create"],
        summary: "Create a mock: a canned response the proxy gives to calls that match a URL pattern.",
        details: "The mock is the whole response; nothing is asked of the real server. Give `url`, or `curl` to mock \
            the call a cURL command makes (answering an empty 200 until told otherwise).",
        effect: Effect::Adds,
        upfront: true,
        scoped: true,
        params: concat(vec![
            vec![
                text("url", "url", "The URL pattern it answers, e.g. https://api.example.com/users."),
                text("curl", "curl", "A whole cURL command; the mock answers the call it makes. Leave out url."),
            ],
            mock_options(false),
            vec![switch("disabled", "disabled", None, "Create it disabled.")],
        ]),
    });
    rows.push(Row {
        name: "mocks_update",
        words: &["mocks", "update"],
        summary: "Change a mock; only what is given changes.",
        details: "",
        effect: Effect::Changes,
        upfront: false,
        scoped: true,
        params: concat(vec![
            vec![operand("mock", "The mock's id or name.").required(), text("url", "url", "A new URL pattern.")],
            mock_options(true),
        ]),
    });

    // -- Mappings ---------------------------------------------------------------------------------------------
    rows.extend(group_rows("mappings"));
    rows.push(Row {
        name: "mappings_create",
        words: &["mappings", "create"],
        summary: "Map a <sub>.localhost domain to a local target, so the proxy serves it from there.",
        details: "",
        effect: Effect::Adds,
        upfront: true,
        scoped: true,
        params: vec![
            operand("domain", "The subdomain or the whole domain, e.g. api or api.localhost.").required(),
            operand("target", "Where it is served from: host:port or an http(s):// URL, e.g. 127.0.0.1:3000.")
                .required(),
            text("label", "label", "A label for the mapping."),
            switch("disabled", "disabled", None, "Create it disabled."),
        ],
    });
    rows.push(Row {
        name: "mappings_update",
        words: &["mappings", "update"],
        summary: "Change a mapping; only what is given changes.",
        details: "",
        effect: Effect::Changes,
        upfront: false,
        scoped: true,
        params: vec![
            operand("mapping", "The mapping's id or domain.").required(),
            text("domain", "domain", "A new subdomain or domain."),
            text("target", "target", "A new target: host:port or an http(s):// URL."),
            text("label", "label", "A new label."),
        ],
    });

    // -- Forward-proxy rules ----------------------------------------------------------------------------------
    rows.extend(group_rows("rules"));
    rows.push(Row {
        name: "rules_create",
        words: &["rules", "create"],
        summary: "Create a forward-proxy rule: requests whose URL matches a pattern go to a mapping or another server.",
        details: "Give one of `mapping` and `target`. A rule's request and response scripts are written in the \
            desktop app.",
        effect: Effect::Adds,
        upfront: false,
        scoped: true,
        params: vec![
            operand("pattern", "The URL pattern, e.g. api.example.com/v1.").required(),
            text("mapping", "mapping", "The id or domain of the mapping matching requests go to."),
            text("target", "target", "The URL of the server matching requests go to."),
            text("name", "name", "A name for the rule. Default: the pattern."),
            switch("regex", "regex", None, "The pattern is a regular expression."),
            switch("disabled", "disabled", None, "Create it disabled."),
        ],
    });
    rows.push(Row {
        name: "rules_update",
        words: &["rules", "update"],
        summary: "Change a forward-proxy rule; only what is given changes.",
        details: "Its scripts are left as they are.",
        effect: Effect::Changes,
        upfront: false,
        scoped: true,
        params: vec![
            operand("rule", "The rule's id or name.").required(),
            text("pattern", "pattern", "A new URL pattern."),
            switch("regex", "regex", Some("no-regex"), "Whether the pattern is a regular expression."),
            text("mapping", "mapping", "The id or domain of the mapping matching requests go to."),
            text("target", "target", "The URL of the server matching requests go to."),
            text("name", "name", "A new name."),
        ],
    });

    rows.extend([
        // -- Environments -------------------------------------------------------------------------------------
        Row {
            name: "env_list",
            words: &["env", "list"],
            summary: "List the environments of a workspace and which one is selected.",
            details: "Global is always applied; the selected environment overrides it.",
            effect: Effect::Reads,
            upfront: false,
            scoped: true,
            params: vec![],
        },
        Row {
            name: "env_show",
            words: &["env", "show"],
            summary: "Show the merged variables a request's {{variables}} resolve against.",
            details: "",
            effect: Effect::Reads,
            upfront: false,
            scoped: true,
            params: vec![],
        },
        Row {
            name: "env_use",
            words: &["env", "use"],
            summary: "Select the environment of the active workspace.",
            details: "The selection is the engine's, so the desktop app follows it.",
            effect: Effect::Changes,
            upfront: false,
            scoped: true,
            params: vec![operand("environment", "The environment's id or name.").required()],
        },
        // -- Health checks ------------------------------------------------------------------------------------
        Row {
            name: "health_list",
            words: &["health", "list"],
            summary: "List the health checks: URLs that answer or do not.",
            details: "",
            effect: Effect::Reads,
            upfront: false,
            scoped: true,
            params: vec![],
        },
        Row {
            name: "health_show",
            words: &["health", "show"],
            summary: "Show one health check in full.",
            details: "",
            effect: Effect::Reads,
            upfront: false,
            scoped: true,
            params: vec![operand("check", "The health check's id or name.").required()],
        },
        Row {
            name: "health_create",
            words: &["health", "create"],
            summary: "Create a health check for a URL.",
            details: "",
            effect: Effect::Adds,
            upfront: false,
            scoped: true,
            params: vec![
                operand("name", "A name, e.g. Users API.").required(),
                operand("url", "The URL to check; may hold {{variables}}, e.g. {{HOST}}/health.").required(),
                switch("auto", "auto", None, "The desktop app re-checks it every 30 seconds."),
            ],
        },
        Row {
            name: "health_update",
            words: &["health", "update"],
            summary: "Change a health check; only what is given changes.",
            details: "",
            effect: Effect::Changes,
            upfront: false,
            scoped: true,
            params: vec![
                operand("check", "The health check's id or name.").required(),
                text("name", "name", "A new name."),
                text("url", "url", "A new URL."),
                switch("auto", "auto", Some("no-auto"), "Whether the desktop app re-checks it every 30 seconds."),
            ],
        },
        Row {
            name: "health_run",
            words: &["health", "run"],
            summary: "Run one health check, or all of them, and report which URLs answer.",
            details: "The result is an error when one does not answer.",
            effect: Effect::Runs,
            upfront: false,
            scoped: true,
            params: vec![operand("check", "The id or name of the check to run. Default: all of them.")],
        },
        // -- Workspaces ---------------------------------------------------------------------------------------
        Row {
            name: "workspaces_list",
            words: &["workspaces", "list"],
            summary: "List the workspaces and which one is active.",
            details: "",
            effect: Effect::Reads,
            upfront: false,
            scoped: false,
            params: vec![],
        },
        Row {
            name: "workspaces_show",
            words: &["workspaces", "show"],
            summary: "Show one workspace.",
            details: "",
            effect: Effect::Reads,
            upfront: false,
            scoped: false,
            params: vec![operand("workspace", "The workspace's id or name. Default: the active workspace.")],
        },
        Row {
            name: "workspaces_create",
            words: &["workspaces", "create"],
            summary: "Create a local workspace and make it the active one.",
            details: "Publishing it to a repository is done in the desktop app.",
            effect: Effect::Adds,
            upfront: false,
            scoped: false,
            params: vec![
                operand("name", "The new workspace's name.").required(),
                text("account", "account", "The login of the signed-in GitHub account that owns it."),
            ],
        },
        Row {
            name: "workspaces_update",
            words: &["workspaces", "update"],
            summary: "Rename a workspace.",
            details: "",
            effect: Effect::Changes,
            upfront: false,
            scoped: false,
            params: vec![
                operand("workspace", "The workspace's id or name.").required(),
                text("name", "name", "The new name.").required(),
            ],
        },
        Row {
            name: "workspaces_use",
            words: &["workspaces", "use"],
            summary: "Make a workspace the active one.",
            details: "The desktop app follows: it is the workspace the user sees.",
            effect: Effect::Changes,
            upfront: false,
            scoped: false,
            params: vec![operand("workspace", "The workspace's id or name.").required()],
        },
        Row {
            name: "workspaces_sync",
            words: &["workspaces", "sync"],
            summary: "Sync a workspace with git: commit everything that was never synced, one commit per item, and push.",
            details: "Pushes when the workspace has a remote. Deletions are left to the desktop app, where each one \
                is confirmed or reverted.",
            effect: Effect::Runs,
            upfront: false,
            scoped: true,
            params: vec![],
        },
        // -- The rest -----------------------------------------------------------------------------------------
        Row {
            name: "vault_list",
            words: &["vault", "list"],
            summary: "List the secret vaults, whether each is sealed, and the names of their secrets (never the values).",
            details: "A secret is used in a request as {{vault.SECRET}}. Creating a vault, setting a secret and \
                unsealing take a passphrase and are done by the user, in the desktop app or with the bifurc command.",
            effect: Effect::Reads,
            upfront: false,
            scoped: true,
            params: vec![],
        },
        Row {
            name: "account_list",
            words: &["account", "list"],
            summary: "List the GitHub accounts signed in to Bifurc.",
            details: "Signing in and out are done by the user.",
            effect: Effect::Reads,
            upfront: false,
            scoped: false,
            params: vec![],
        },
        Row {
            name: "account_repos",
            words: &["account", "repos"],
            summary: "List the repositories a signed-in GitHub account owns.",
            details: "",
            effect: Effect::Runs,
            upfront: false,
            scoped: false,
            params: vec![operand("account", "The account's login.").required()],
        },
        Row {
            name: "services_list",
            words: &["services", "list"],
            summary: "List the local processes listening on a port: what a mapping can be pointed at.",
            details: "",
            effect: Effect::Reads,
            upfront: true,
            scoped: false,
            params: vec![],
        },
    ]);
    rows.into_iter().map(Row::tool).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_are_unique_and_follow_the_protocols_rules() {
        let tools = catalog();
        let names: HashSet<&str> = tools.iter().map(|tool| tool.name).collect();
        assert_eq!(names.len(), tools.len());
        for tool in &tools {
            assert!(tool.name.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{}", tool.name);
            assert_eq!(tool.name, tool.words.join("_"), "a tool is named after its command");
        }
    }

    #[test]
    fn every_tool_says_what_it_does_in_one_sentence() {
        for tool in catalog() {
            assert!(tool.summary.ends_with('.') && !tool.summary.contains('\n'), "{}", tool.name);
            assert!(tool.summary.len() <= 130, "{}: {}", tool.name, tool.summary.len());
            for param in &tool.params {
                assert!(!param.about.is_empty(), "{}.{}", tool.name, param.name);
            }
        }
    }

    #[test]
    fn a_tools_parameters_are_unique_and_required_operands_come_first() {
        for tool in catalog() {
            let names: HashSet<&str> = tool.params.iter().map(|p| p.name).collect();
            assert_eq!(names.len(), tool.params.len(), "{}", tool.name);
            let operands: Vec<&Param> = tool.params.iter().filter(|p| p.place == Place::Operand).collect();
            // An operand that may be left out has to be the last one, or the next would take its place.
            let optional = operands.iter().position(|p| !p.required).unwrap_or(operands.len());
            assert!(optional + 1 >= operands.len(), "{}", tool.name);
        }
    }

    #[test]
    fn nothing_deletes_reads_a_secret_or_manages_the_engine() {
        for tool in catalog() {
            let verb = tool.words.last().copied().unwrap_or_default();
            assert!(
                !["delete", "remove", "rm", "unset", "logout", "login", "stop", "restart", "call", "run"]
                    .contains(&verb)
                    || tool.name == "health_run",
                "{}",
                tool.name
            );
            assert!(tool.words[0] != "engine" && tool.words[0] != "call", "{}", tool.name);
            assert!(tool.words[0] != "vault" || tool.name == "vault_list", "{}", tool.name);
        }
    }

    #[test]
    fn the_schema_lists_the_properties_and_the_required_ones() {
        let tools = catalog();
        let update = tools.iter().find(|tool| tool.name == "workspaces_update").unwrap();
        let schema = Value::Object(update.input_schema());
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"], json!(["workspace", "name"]));
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["json"]["type"], "boolean");

        let send = tools.iter().find(|tool| tool.name == "requests_send").unwrap();
        let schema = Value::Object(send.input_schema());
        assert_eq!(schema.get("required"), None);
        assert_eq!(schema["properties"]["type"]["enum"], json!(REQUEST_TYPES));
        assert_eq!(schema["properties"]["headers"]["additionalProperties"]["type"], "string");
        assert_eq!(schema["properties"]["listen"]["items"]["type"], "string");
        assert_eq!(schema["properties"]["workspace"]["type"], "string");
    }

    #[test]
    fn a_handful_of_tools_is_listed_upfront() {
        let upfront: Vec<&str> = catalog().iter().filter(|tool| tool.upfront).map(|tool| tool.name).collect();
        assert_eq!(
            upfront,
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
            ]
        );
    }
}
