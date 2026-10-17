//! The tools: one per command of the `bifurc` command line, each a row of [`catalog`].
//!
//! A row says which command it runs and where each of its arguments goes on that command's line (an operand,
//! an option, a switch). Nothing here knows what a command does: `invoke.rs` writes the command line and the
//! CLI's own code runs it, so a tool can never do what the command line cannot. Most rows are not written
//! here: they are the rows of the command line's own table of verbs (`bifurc_cli::direct`). There is no tool
//! that reads or takes a secret or a passphrase, none that signs in and none that stops the engine.

use bifurc_cli::direct::{self, Scope, Take, Verb};
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
    /// Removes something.
    Deletes,
}

/// Where an argument goes on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// The command's next operand, in the order the parameters are listed.
    Operand,
    /// A list: each value is an operand, after the others.
    Operands,
    /// `--<option> <value>`.
    Option(&'static str),
    /// `--<option> <value>`, and `--<option> <default>` when the argument is left out.
    OptionOr(&'static str, &'static str),
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
    pub details: String,
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
fn json_switch(option: &'static str) -> Param {
    switch("json", option, None, "Return the engine's raw JSON instead of the readable text.")
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
        // `--json` is curl's own option (data to send) in `bifurc curl`.
        params.push(json_switch(if self.words == ["curl"] { "bifurc-json" } else { "json" }));
        Tool {
            name: self.name,
            words: self.words,
            summary: self.summary,
            details: self.details.to_string(),
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
            "data",
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

/// What a send of a request that answers once can lay over the request and the environment.
fn override_options() -> Vec<Param> {
    vec![
        Param {
            name: "vars",
            about: "rest, graphql, soap, grpc: variables for {{name}}, by name, over the environment's.",
            ty: Type::TextMap,
            place: Place::Pairs("var", "="),
            required: false,
        },
        text(
            "env",
            "env",
            "rest, graphql, soap, grpc: the id or name of the environment to use instead of the selected one.",
        ),
        Param {
            name: "patch",
            about: "A saved request: set a field of its JSON body (or of body), each '<path>=<value>' like '.qty=2'.",
            ty: Type::TextList,
            place: Place::Each("patch"),
            required: false,
        },
    ]
}

/// What a send of a request that answers once returns; the response is a summary line unless asked for more.
fn output_options() -> Vec<Param> {
    vec![
        text("select", "select", "A jq-style path into the JSON body: return only the values it picks."),
        switch("show_body", "body", None, "Include the response body (text only), cut at max_body."),
        switch("show_headers", "headers", None, "Include the response headers; secrets are masked."),
        typed("max_body", "max-body", Type::Integer, "Bytes of body returned with show_body (default 2048; 0 = all)."),
        typed("expect_status", "expect-status", Type::Integer, "Fail unless the response has this status."),
        Param {
            name: "expect",
            about: "Fail unless each '<path> <op> <value>' holds, e.g. 'status < 300' or '.id != null'.",
            ty: Type::TextList,
            place: Place::Each("expect"),
            required: false,
        },
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
                override_options(),
                output_options(),
            ]),
        },
        Row {
            name: "curl",
            words: &["curl"],
            summary: "Run a curl command through Bifurc (shared cookie jar, environments), optionally saving it as a request.",
            details: "`command` is curl's arguments as you would write them. `save` saves the request first,                 with the environment's values as {{variables}}, and sends the saved one; `save_only` does not send.",
            effect: Effect::Runs,
            upfront: true,
            scoped: true,
            params: concat(vec![
                vec![
                    operand(
                        "command",
                        "The whole curl command, e.g. curl -X POST https://api.localhost/users -d '{}'.",
                    )
                    .required(),
                    text("save", "save", "Save the request under this name."),
                    text("folder", "folder", "The folder of the saved request, e.g. auth/admin."),
                    switch("update", "update", None, "Replace the saved request of that name."),
                    switch("save_only", "save-only", None, "Save without sending."),
                ],
                override_options().into_iter().take(2).collect(),
                output_options(),
            ]),
        },
        Row {
            name: "requests_create",
            words: &["requests", "create"],
            summary: "Save a new request of any type.",
            details: "Give `url` and the options of the type, or `curl` to save the request a cURL command describes. \
                Scripts, tests and any other field are set with requests_patch.",
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
            details: "Its scripts and tests are left as they are; requests_patch sets them.",
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
        details: "Give one of `mapping` and `target`. A rule's request and response scripts are set with rules_patch.",
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
        details: "Its scripts are left as they are; rules_patch sets them.",
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
        // -- Flows --------------------------------------------------------------------------------------------
        Row {
            name: "flows_list",
            words: &["flows", "list"],
            summary: "List the saved flows: chains of requests, each node's output feeding the next.",
            details: "A flow is written as JSON with flows_create and flows_update; help with group flows describes it.",
            effect: Effect::Reads,
            upfront: true,
            scoped: true,
            params: vec![],
        },
        Row {
            name: "flows_show",
            words: &["flows", "show"],
            summary: "Show a flow: its inputs, its nodes with the request each runs, and what is wrong with it.",
            details: "",
            effect: Effect::Reads,
            upfront: false,
            scoped: true,
            params: vec![operand("flow", "The flow's id or name.").required()],
        },
        Row {
            name: "flows_run",
            words: &["flows", "run"],
            summary: "Run a flow and return how each node did, the tests that failed and the run's status.",
            details: "The result is an error when the run fails or a test does. A run is stopped after `timeout` \
                seconds (default 120).",
            effect: Effect::Runs,
            upfront: true,
            scoped: true,
            params: vec![
                operand("flow", "The flow's id or name.").required(),
                Param {
                    name: "inputs",
                    about: "Values for the inputs the flow declares, by name.",
                    ty: Type::TextMap,
                    place: Place::Pairs("input", "="),
                    required: false,
                },
                Param {
                    name: "timeout",
                    about: "Seconds before the run is stopped. Default 120; 0 is no limit.",
                    ty: Type::Number,
                    place: Place::OptionOr("timeout", "120"),
                    required: false,
                },
                typed("max_events", "max-events", Type::Integer, "Stop after this many messages of a stream node."),
            ],
        },
        Row {
            name: "flows_loadtest",
            words: &["flows", "loadtest"],
            summary: "Load test an API: run a flow many times at once and return how the instances did.",
            details: "Returns how many instances passed and failed, the times (p50, p95, p99), each node's times and \
                what failed how often. It sends real traffic: every instance sends every request of the flow. The result is an \
                error when an instance fails. An instance is stopped after `timeout` seconds (default 120).",
            effect: Effect::Runs,
            upfront: false,
            scoped: true,
            params: vec![
                operand("flow", "The flow's id or name.").required(),
                typed("instances", "instances", Type::Integer, "How many runs to start together, 1 to 10000.")
                    .required(),
                Param {
                    name: "inputs",
                    about: "Values for the inputs the flow declares, by name; every instance gets the same.",
                    ty: Type::TextMap,
                    place: Place::Pairs("input", "="),
                    required: false,
                },
                Param {
                    name: "timeout",
                    about: "Seconds before an instance is stopped. Default 120; 0 is no limit.",
                    ty: Type::Number,
                    place: Place::OptionOr("timeout", "120"),
                    required: false,
                },
            ],
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
            details: "workspaces_publish gives it a GitHub repository.",
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
            details: "Pushes when the workspace has a remote. Pending deletions are left: sync_confirm commits one, \
                sync_restore undoes it.",
            effect: Effect::Runs,
            upfront: false,
            scoped: true,
            params: vec![text("message", "message", "The commit message. Default: one the engine writes.")],
        },
        // -- The rest -----------------------------------------------------------------------------------------
        Row {
            name: "vault_list",
            words: &["vault", "list"],
            summary: "List the secret vaults, whether each is sealed, and the names of their secrets (never the values).",
            details: "A secret is used in a request as {{vault.SECRET}}. Creating a vault, setting a secret, unsealing \
                and changing the passphrase take a secret, so the user does them: in the desktop app or with the \
                bifurc command (bifurc vault --help).",
            effect: Effect::Reads,
            upfront: false,
            scoped: true,
            params: vec![],
        },
        Row {
            name: "account_list",
            words: &["account", "list"],
            summary: "List the GitHub accounts signed in to Bifurc.",
            details: "Signing in shows a code to enter on GitHub, so the user does it: bifurc account login.",
            effect: Effect::Reads,
            upfront: false,
            scoped: false,
            params: vec![],
        },
        Row {
            name: "account_repos",
            words: &["account", "repos"],
            summary: "List the repositories a signed-in GitHub account owns or was given access to.",
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
        // -- The verbs beside the command line's table that take more than one engine command -----------------
        Row {
            name: "help",
            words: &["help"],
            summary: "Show the manual of the bifurc commands these tools run: every group, or one group's verbs and options.",
            details: "A tool is named <group>_<verb> and takes that verb's arguments. The manual of the group flows \
                describes the JSON of a flow: its nodes, their ports and the connections between them.",
            effect: Effect::Reads,
            upfront: true,
            scoped: false,
            params: vec![operand("group", "The group, e.g. flows, requests, sync. Default: the overview of them all.")],
        },
        Row {
            name: "folders_list",
            words: &["folders", "list"],
            summary: "List the folders of the request, mock, flow or rule tree of the active workspace.",
            details: "A folder's id is what folders_*, runner_* and an item's folderId take.",
            effect: Effect::Reads,
            upfront: false,
            scoped: false,
            params: vec![typed("tree", "tree", Type::Choice(TREES), "Which tree.").required()],
        },
        Row {
            name: "env_set",
            words: &["env", "set"],
            summary: "Set variables of one environment, adding the ones it does not have.",
            details: "The environment that is always applied is named Global.",
            effect: Effect::Changes,
            upfront: false,
            scoped: true,
            params: vec![
                operand("environment", "The environment's id or name.").required(),
                Param {
                    name: "variables",
                    about: "The variables, each as KEY=VALUE.",
                    ty: Type::TextList,
                    place: Place::Operands,
                    required: true,
                },
            ],
        },
        Row {
            name: "env_unset",
            words: &["env", "unset"],
            summary: "Remove variables of one environment.",
            details: "",
            effect: Effect::Deletes,
            upfront: false,
            scoped: true,
            params: vec![
                operand("environment", "The environment's id or name.").required(),
                Param {
                    name: "variables",
                    about: "The names of the variables.",
                    ty: Type::TextList,
                    place: Place::Operands,
                    required: true,
                },
            ],
        },
        Row {
            name: "capture_list",
            words: &["capture", "list"],
            summary: "List the traffic the proxies captured, newest first, one line per exchange.",
            details: "Each line starts with #tail, the entry's id for capture_show. Entries beyond `last` are only counted, in a last line.",
            effect: Effect::Reads,
            upfront: true,
            scoped: true,
            params: vec![
                typed("last", "last", Type::Integer, "How many entries (default 20)."),
                text("status", "status", "422, 4xx, !2xx or a list like 4xx,5xx."),
                text("method", "method", "The HTTP method."),
                text("host", "host", "The host; * matches any run of characters."),
                text("path", "path", "The path without the query; * matches any run of characters."),
                text("since", "since", "30s, 2m, 1h ago, or #tail: after that entry."),
                typed("origin", "origin", Type::Choice(&["real", "mock", "mapped", "rule", "error"]), "Who answered."),
                text("grep", "grep", "Case-insensitive text in the URL or a request or response body."),
            ],
        },
        Row {
            name: "capture_show",
            words: &["capture", "show"],
            summary: "Show one captured exchange: the request line, then the response summary.",
            details: "Like a request's response: a summary line, and only what show_body, show_headers, select, req_body asks for.",
            effect: Effect::Reads,
            upfront: true,
            scoped: true,
            params: vec![
                operand("entry", "The entry's id, or its #tail as capture_list prints it.").required(),
                text("select", "select", "A jq-style path into the JSON response body: return only the values it picks."),
                switch("show_body", "body", None, "Include the response body (text only), cut at max_body."),
                switch("show_headers", "headers", None, "Include the response headers; secrets are masked."),
                switch("req_body", "req-body", None, "Include the request body (text only), cut at max_body."),
                switch("req_headers", "req-headers", None, "Include the request headers; secrets are masked."),
                typed("max_body", "max-body", Type::Integer, "Bytes of body returned (default 2048; 0 = all)."),
            ],
        },
        Row {
            name: "capture_mock",
            words: &["capture", "mock"],
            summary: "Save a mock that answers as a captured exchange did.",
            details: "",
            effect: Effect::Adds,
            upfront: false,
            scoped: true,
            params: vec![operand("entry", "The captured entry's id (see capture_list).").required()],
        },
        Row {
            name: "capture_request",
            words: &["capture", "request"],
            summary: "Save a request like a captured one.",
            details: "",
            effect: Effect::Adds,
            upfront: false,
            scoped: true,
            params: vec![operand("entry", "The captured entry's id (see capture_list).").required()],
        },
        Row {
            name: "runner_run",
            words: &["runner", "run"],
            summary: "Send every request of a folder in order, with scripts and tests, and return how each did.",
            details: "The result is an error when a request fails or a test does.",
            effect: Effect::Runs,
            upfront: false,
            scoped: true,
            params: vec![operand("folder", "The id of a folder of the request tree (see folders_list).").required()],
        },
        Row {
            name: "settings_show",
            words: &["settings", "show"],
            summary: "Show the engine's ports and whether HTTPS, telemetry and the update check are on.",
            details: "",
            effect: Effect::Reads,
            upfront: false,
            scoped: false,
            params: vec![],
        },
        Row {
            name: "health_delete",
            words: &["health", "delete"],
            summary: "Delete a health check.",
            details: "",
            effect: Effect::Deletes,
            upfront: false,
            scoped: true,
            params: vec![operand("check", "The health check's id or name.").required()],
        },
    ]);
    let mut tools: Vec<Tool> = rows.into_iter().map(Row::tool).collect();
    tools.extend(direct::verbs().iter().map(of_verb));
    tools
}

const TREES: &[&str] = &["request", "mock", "flow", "rule"];

/// The tool of a verb that is a row of the command line's own table.
fn of_verb(verb: &Verb) -> Tool {
    let mut params: Vec<Param> = verb
        .args
        .iter()
        .map(|arg| Param {
            name: arg.name,
            about: arg.about,
            ty: match arg.take {
                Take::Int => Type::Integer,
                Take::Bool => Type::Boolean,
                Take::Choice(words) => Type::Choice(words),
                Take::Json => Type::Json,
                Take::List => Type::TextList,
                _ => Type::Text,
            },
            place: match (arg.operand, arg.take) {
                (true, Take::List) => Place::Operands,
                (true, _) => Place::Operand,
                (false, _) => Place::Option(arg.name),
            },
            required: arg.required,
        })
        .collect();
    if verb.out {
        params.push(text("out", "out", "The file to write. Default: the content is returned."));
    }
    if verb.confirm {
        params.push(switch("yes", "yes", None, "Must be true: what this does cannot be undone.").required());
    }
    if verb.scope == Scope::Workspace {
        params.push(workspace());
    }
    params.push(json_switch("json"));
    Tool {
        // The catalog is built once per server, so the name lives as long as the process.
        name: Box::leak(verb.words.join("_").into_boxed_str()),
        words: verb.words,
        summary: verb.about,
        details: verb.notes(),
        effect: match verb.effect {
            direct::Effect::Reads => Effect::Reads,
            direct::Effect::Adds => Effect::Adds,
            direct::Effect::Changes => Effect::Changes,
            direct::Effect::Runs => Effect::Runs,
            direct::Effect::Deletes => Effect::Deletes,
        },
        upfront: false,
        params,
    }
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
    fn nothing_takes_a_secret_signs_in_or_manages_the_engine() {
        for tool in catalog() {
            assert!(!["engine", "call", "commands"].contains(&tool.words[0]), "{}", tool.name);
            assert!(tool.name != "account_login", "{}", tool.name);
            // The vault verbs that ask for a passphrase or a value, or hand the secrets to a program.
            let secret = ["create", "set", "unseal", "passphrase", "run"].contains(tool.words.last().unwrap());
            assert!(tool.words[0] != "vault" || !secret, "{}", tool.name);
        }
    }

    #[test]
    fn every_verb_of_the_command_lines_table_is_a_tool_and_what_cannot_be_undone_says_so() {
        let tools = catalog();
        for verb in direct::verbs() {
            let tool = tools.iter().find(|tool| tool.words == verb.words).unwrap_or_else(|| panic!("{:?}", verb.words));
            assert_eq!(
                tool.params.iter().any(|param| param.name == "yes" && param.required),
                verb.confirm,
                "{}",
                tool.name
            );
            assert_eq!(tool.effect == Effect::Deletes, verb.effect == direct::Effect::Deletes, "{}", tool.name);
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
        assert_eq!(schema["properties"]["vars"]["additionalProperties"]["type"], "string");
        assert_eq!(schema["properties"]["patch"]["items"]["type"], "string");
        assert_eq!(schema["properties"]["env"]["type"], "string");
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
                "curl",
                "mocks_list",
                "mocks_create",
                "mappings_list",
                "mappings_create",
                "flows_list",
                "flows_run",
                "services_list",
                "help",
                "capture_list",
                "capture_show",
            ]
        );
    }
}
