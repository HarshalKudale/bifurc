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
    switch("json", option, None, "Also return structured JSON.")
}

/// The argument of every tool that acts on one workspace.
fn workspace() -> Param {
    text("workspace", "workspace", "Workspace id (default: active).")
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

const TYPE_ABOUT: &str = "Request type (default rest); for a saved request, only to tell same-named ones apart.";

/// The options that describe a request of some type: what `requests create`, `update` and an unsaved `send` take.
fn request_options() -> Vec<Param> {
    vec![
        text("url", "url", "Where it goes: a URL, a gRPC address, or for mcp the server's command line or endpoint."),
        headers("Headers by name; an empty value removes one on update."),
        text("method", "method", "HTTP method (default GET); grpc: pkg.Service/Method."),
        text("body", "data", "The request body; grpc: the message as JSON."),
        text("query", "query", "graphql: the query."),
        typed("variables", "variables", Type::Json, "graphql: the variables."),
        text("operation", "operation", "graphql: the operation name."),
        text("proto", "proto", "grpc: the saved .proto file id."),
        switch("reflection", "reflection", None, "grpc: use server reflection."),
        text("action", "action", "soap: the SOAPAction."),
        text("namespace", "namespace", "socketio: the namespace."),
        text("path", "path", "socketio: the endpoint path."),
        typed("auth", "auth", Type::Json, "socketio: the auth payload."),
        Param {
            name: "listen",
            about: "socketio: the events to log (default all).",
            ty: Type::TextList,
            place: Place::Each("listen"),
            required: false,
        },
        typed("transport", "transport", Type::Choice(&["stdio", "http"]), "mcp: how the server is reached."),
    ]
}

/// What a send of a request that answers once can lay over the request and the environment.
fn override_options() -> Vec<Param> {
    vec![
        Param {
            name: "vars",
            about: "Variables for {{name}}, over the environment's.",
            ty: Type::TextMap,
            place: Place::Pairs("var", "="),
            required: false,
        },
        text("env", "env", "Environment to use instead of the selected one."),
        Param {
            name: "patch",
            about: "Set JSON body fields, each '<path>=<value>'.",
            ty: Type::TextList,
            place: Place::Each("patch"),
            required: false,
        },
    ]
}

/// What a run of many requests (a folder, a flow) can lay over them and how much it prints.
fn run_options() -> Vec<Param> {
    vec![
        Param {
            name: "vars",
            about: "Variables for {{name}}, over the environment's.",
            ty: Type::TextMap,
            place: Place::Pairs("var", "="),
            required: false,
        },
        text("env", "env", "Environment to use instead of the selected one."),
        switch("fresh_session", "fresh-session", None, "Run with a throwaway cookie jar."),
        switch("verbose", "verbose", None, "Include every request's body."),
        typed("max_body", "max-body", Type::Integer, "Body byte limit (default 2048; 0 = all)."),
    ]
}

/// What a send of a request that answers once returns; the response is a summary line unless asked for more.
fn output_options() -> Vec<Param> {
    vec![
        text("select", "select", "jq-style path into the JSON body: return only those values."),
        switch("show_body", "body", None, "Include the response body."),
        switch("show_headers", "headers", None, "Include the response headers."),
        typed("max_body", "max-body", Type::Integer, "Body byte limit (default 2048; 0 = all)."),
        typed("expect_status", "expect-status", Type::Integer, "Fail unless the status matches."),
        Param {
            name: "expect",
            about: "Fail unless each '<path> <op> <value>' holds.",
            ty: Type::TextList,
            place: Place::Each("expect"),
            required: false,
        },
        Param {
            name: "extract",
            about: "Keep values of the response as session variables, each 'name=<path>' (path as select, or status).",
            ty: Type::TextList,
            place: Place::Each("extract"),
            required: false,
        },
    ]
}

/// The options only a send takes.
fn exchange_options() -> Vec<Param> {
    vec![
        typed("wait", "wait", Type::Number, "websocket, sse, socketio, mcp: seconds to listen or wait."),
        text("event", "event", "socketio: the event to emit, its arguments in body."),
        switch("ack", "ack", None, "socketio: ask for an acknowledgement."),
        text("tool", "tool", "mcp: the tool to call."),
        text("prompt", "prompt", "mcp: the prompt to get."),
        text("resource", "resource", "mcp: the resource URI to read."),
        Param {
            name: "arguments",
            about: "mcp: the tool or prompt arguments.",
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
    let mut rows = [
        Row {
            name: names[0],
            words: words[0],
            summary: list,
            details: "",
            effect: Effect::Reads,
            upfront: group == "mocks",
            scoped: true,
            params: vec![],
        },
        one(names[1], words[1], show, Effect::Reads),
        one(names[2], words[2], enable, Effect::Changes),
        one(names[3], words[3], disable, Effect::Changes),
    ];
    // Agents switch mocks on and off often.
    if group == "mocks" {
        rows[2].upfront = true;
        rows[3].upfront = true;
    }
    rows
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
        text("method", "method", "HTTP method it answers; * for any (default GET)."),
        text("name", "name", "A name for the mock."),
        typed("status", "status", Type::Integer, "Response status (default 200)."),
        headers("Response headers by name."),
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
            summary: "Show the engine, proxy, workspace and mock state in two lines.",
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
            summary: "List the saved requests of every type.",
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
            summary: "Send a saved request, or an unsaved one described here.",
            details: "Give `request` for a saved one, else `url` and the type's options.",
            effect: Effect::Runs,
            upfront: true,
            scoped: true,
            params: concat(vec![
                vec![
                    operand("request", "Saved request id or name."),
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
            summary: "Run a curl command through Bifurc, optionally saving it as a request.",
            details: "",
            effect: Effect::Runs,
            upfront: true,
            scoped: true,
            params: concat(vec![
                vec![
                    operand("command", "The whole curl command.").required(),
                    text("save", "save", "Save the request under this name."),
                    text("folder", "folder", "Folder of the saved request."),
                    switch("update", "update", None, "Replace the saved request."),
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
        details: "The mock is the whole response; the real server is not asked.",
        effect: Effect::Adds,
        upfront: false,
        scoped: true,
        params: concat(vec![
            vec![
                text("url", "url", "The URL pattern it answers."),
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
    rows.push(Row {
        name: "mocks_scenario",
        words: &["mocks", "scenario"],
        summary: "Switch a mock's call to another answer, or list its answers.",
        details: "Presets, made on demand: error500, error401, notfound, empty, slow, timeout. No scenario lists them.",
        effect: Effect::Changes,
        upfront: true,
        scoped: true,
        params: vec![
            operand("mock", "Any mock of the call: id or name.").required(),
            operand("scenario", "default, a scenario's name or a preset."),
            typed("delay", "delay", Type::Integer, "Milliseconds for a new slow or timeout scenario."),
        ],
    });
    rows.push(Row {
        name: "mocks_status",
        words: &["mocks", "status"],
        summary: "One line per mocked call: active scenario, on or off, how often the app hit it.",
        details: "",
        effect: Effect::Reads,
        upfront: false,
        scoped: true,
        params: vec![],
    });
    rows.push(Row {
        name: "mocks_hits",
        words: &["mocks", "hits"],
        summary: "The captured requests a mocked call answered: to tell whether the app hit the mock.",
        details: "",
        effect: Effect::Reads,
        upfront: false,
        scoped: true,
        params: vec![
            operand("mock", "Any mock of the call: id or name.").required(),
            typed("last", "last", Type::Integer, "How many (default 5)."),
        ],
    });

    rows.push(Row {
        name: "mocks_serve",
        words: &["mocks", "serve"],
        summary: "Serve the enabled mocks over plain HTTP on a local port, for clients that ignore proxy settings.",
        details: "Runs until mocks_serve_stop or the engine exits. The host part of a mock's pattern is ignored. Unmatched requests get a 404 listing the closest mocks, or go to fallthrough.",
        effect: Effect::Changes,
        upfront: false,
        scoped: true,
        params: vec![
            typed("port", "port", Type::Integer, "Port (default 4010; 0 picks a free one)."),
            text("host", "host", "Address to listen on (default 127.0.0.1)."),
            text("fallthrough", "fallthrough", "Base URL that unmatched requests go to, e.g. http://localhost:3000."),
        ],
    });
    rows.push(Row {
        name: "mocks_serve_stop",
        words: &["mocks", "serve", "stop"],
        summary: "Stop the local mock server (every one without a port).",
        details: "",
        effect: Effect::Changes,
        upfront: false,
        scoped: false,
        params: vec![typed("port", "port", Type::Integer, "The server's port.")],
    });

    // -- Mappings ---------------------------------------------------------------------------------------------
    rows.extend(group_rows("mappings"));
    rows.push(Row {
        name: "mappings_create",
        words: &["mappings", "create"],
        summary: "Map a <sub>.localhost domain to a local target, so the proxy serves it from there.",
        details: "",
        effect: Effect::Adds,
        upfront: false,
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
        // -- Session ------------------------------------------------------------------------------------------
        Row {
            name: "session_show",
            words: &["session", "show"],
            summary: "Show this server's session: its variables, environment and cookie domains.",
            details: "Start the server with --session <name> (or BIFURC_SESSION) for cookies, variables and an environment of its own.",
            effect: Effect::Reads,
            upfront: false,
            scoped: true,
            params: vec![],
        },
        Row {
            name: "session_reset",
            words: &["session", "reset"],
            summary: "Empty the session's cookie jar and variables and forget its environment.",
            details: "",
            effect: Effect::Changes,
            upfront: false,
            scoped: true,
            params: vec![],
        },
        Row {
            name: "vars_list",
            words: &["vars", "list"],
            summary: "List the session's runtime variables (what extract and vars_set keep); secrets are masked.",
            details: "",
            effect: Effect::Reads,
            upfront: false,
            scoped: true,
            params: vec![],
        },
        Row {
            name: "vars_set",
            words: &["vars", "set"],
            summary: "Set runtime variables of the session; {{name}} resolves in its requests.",
            details: "",
            effect: Effect::Changes,
            upfront: false,
            scoped: true,
            params: vec![Param {
                name: "variables",
                about: "The variables, each as KEY=VALUE.",
                ty: Type::TextList,
                place: Place::Operands,
                required: true,
            }],
        },
        Row {
            name: "vars_clear",
            words: &["vars", "clear"],
            summary: "Remove runtime variables of the session, or all of them.",
            details: "",
            effect: Effect::Deletes,
            upfront: false,
            scoped: true,
            params: vec![Param {
                name: "variables",
                about: "The names to remove; none removes all.",
                ty: Type::TextList,
                place: Place::Operands,
                required: false,
            }],
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
            summary: "Run a flow: one line per request, then the total.",
            details: "An error when a step or test fails.",
            effect: Effect::Runs,
            upfront: true,
            scoped: true,
            params: vec![
                operand("flow", "The flow's id or name.").required(),
                Param {
                    name: "inputs",
                    about: "Values for the flow's inputs.",
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
            ]
            .into_iter()
            .chain(run_options())
            .collect(),
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
            upfront: false,
            scoped: false,
            params: vec![],
        },
        Row {
            name: "wait",
            words: &["wait"],
            summary: "Wait until a URL answers, a TCP port accepts, or a new matching request is captured; replaces sleep loops.",
            details: "Give one of target, port or capture. Ready: one line. Not ready after timeout: an error with the last reason.",
            effect: Effect::Runs,
            upfront: false,
            scoped: true,
            params: vec![
                operand("target", "A URL, or the name of a health check (its URL is used)."),
                typed("port", "port", Type::Integer, "Wait for this TCP port on host."),
                switch("capture", "capture", None, "Wait for a new captured request matching method, host, path, status, origin, grep."),
                text("status", "status", "For a URL: the status it must answer (default: any). For capture: 422, 4xx, !2xx."),
                text("host", "host", "For port: the host (default 127.0.0.1). For capture: the host filter."),
                text("method", "method", "Capture filter: the HTTP method."),
                text("path", "path", "Capture filter: the path; * matches any run of characters."),
                typed("origin", "origin", Type::Choice(&["real", "mock", "mapped", "rule", "error"]), "Capture filter: who answered."),
                text("grep", "grep", "Capture filter: text in the URL or a body."),
                text("timeout", "timeout", "500ms, 30s, 2m or seconds (default 30s)."),
                text("interval", "interval", "Time between attempts (default 500ms)."),
            ],
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
            details: "Each line starts with #tail, the id for capture_show.",
            effect: Effect::Reads,
            upfront: true,
            scoped: true,
            params: vec![
                typed("last", "last", Type::Integer, "How many entries (default 20)."),
                text("status", "status", "E.g. 422, 4xx, !2xx."),
                text("method", "method", "The HTTP method."),
                text("host", "host", "The host; * is a wildcard."),
                text("path", "path", "The path; * is a wildcard."),
                text("since", "since", "30s, 2m, 1h ago, or #tail: after that entry."),
                typed("origin", "origin", Type::Choice(&["real", "mock", "mapped", "rule", "error", "server"]), "Who answered."),
                text("grep", "grep", "Text in the URL or a body."),
                switch("pinned", "pinned", None, "Only the pinned entries (capture_pin)."),
            ],
        },
        Row {
            name: "capture_show",
            words: &["capture", "show"],
            summary: "Show one captured exchange: the request line, then the response summary.",
            details: "",
            effect: Effect::Reads,
            upfront: true,
            scoped: true,
            params: vec![
                operand("entry", "The entry's id, or its #tail as capture_list prints it.").required(),
                text("select", "select", "jq-style path into the JSON body: return only those values."),
                switch("show_body", "body", None, "Include the response body."),
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
            name: "capture_diff",
            words: &["capture", "diff"],
            summary: "Compare two responses: captured entries, a mock, or a saved request sent now. Shows only the differences.",
            details: "Give two sides: `entry` and `other`, or `entry` with `mock` or `request`. Compares status, content type, \
                header names and the body's shape and values. Sends the saved request when `request` is given.",
            effect: Effect::Runs,
            upfront: false,
            scoped: true,
            params: vec![
                operand("entry", "The first captured entry's id or #tail (see capture_list).").required(),
                operand("other", "The second captured entry; leave out when mock or request is the other side."),
                text("mock", "mock", "Compare with this mock's answer instead of a second entry."),
                text("request", "request", "Compare with the answer of this saved request, sent now."),
                switch("shape", "shape", None, "Keys and types only, no values."),
                Param {
                    name: "vars",
                    about: "Variables for {{name}} when a request is sent.",
                    ty: Type::TextMap,
                    place: Place::Pairs("var", "="),
                    required: false,
                },
                text("env", "env", "Environment to send the request with."),
                switch("fresh_session", "fresh-session", None, "Send with a throwaway cookie jar."),
            ],
        },
        Row {
            name: "capture_pin",
            words: &["capture", "pin"],
            summary: "Pin captured entries so they are kept beyond the capture buffer limit.",
            details: "Pinned entries do not count toward the limit (settings_update capturelimit). capture_unpin lets them age out.",
            effect: Effect::Changes,
            upfront: false,
            scoped: true,
            params: vec![Param {
                name: "entries",
                about: "The entries' ids or #tails (see capture_list).",
                ty: Type::TextList,
                place: Place::Operands,
                required: true,
            }],
        },
        Row {
            name: "capture_unpin",
            words: &["capture", "unpin"],
            summary: "Unpin captured entries so they age out of the capture buffer again.",
            details: "",
            effect: Effect::Changes,
            upfront: false,
            scoped: true,
            params: vec![Param {
                name: "entries",
                about: "The entries' ids or #tails (see capture_list).",
                ty: Type::TextList,
                place: Place::Operands,
                required: true,
            }],
        },
        Row {
            name: "contract_check",
            words: &["contract", "check"],
            summary: "Check for drift: whether the mocks still match the real API (or an OpenAPI file).",
            details: "Pairs each enabled mock with a saved request of the same method and path, sends it (POST and DELETE too) \
                and compares status, content type and body shape, never values. Drifted rows show `+ .x (number)` \
                (real has it, mock lacks it). The result is an error on drift.",
            effect: Effect::Runs,
            upfront: false,
            scoped: true,
            params: vec![
                text("mock", "mock", "Only this mock (also when disabled)."),
                text("folder", "folder", "Only the mocks in this mock folder and below."),
                switch("all", "all", None, "Disabled mocks too."),
                Param {
                    name: "ignore",
                    about: "Leave out differences whose path matches, e.g. .meta.* or .items[].debug.",
                    ty: Type::TextList,
                    place: Place::Each("ignore"),
                    required: false,
                },
                switch("from_capture", "from-capture", None, "Compare with the newest captured real answer; sends nothing."),
                switch("from_snapshot", "from-snapshot", None, "Compare with the stored snapshots only; sends nothing."),
                text("openapi", "openapi", "Compare with this OpenAPI 3 or Swagger 2 file (JSON or YAML); sends nothing."),
                Param {
                    name: "vars",
                    about: "Variables for {{name}} in the sent requests.",
                    ty: Type::TextMap,
                    place: Place::Pairs("var", "="),
                    required: false,
                },
                text("env", "env", "Environment to send the requests with."),
                switch("fresh_session", "fresh-session", None, "Send with a throwaway cookie jar."),
            ],
        },
        Row {
            name: "contract_update_mock",
            words: &["contract", "update-mock"],
            summary: "Fix a drifted mock: rewrite its body to the real API's shape, keeping its values where a key stays.",
            details: "Sends the paired saved request (or reads the newest capture) and updates the mock's body and status; headers stay.",
            effect: Effect::Runs,
            upfront: false,
            scoped: true,
            params: vec![
                operand("mock", "The mock's id or name.").required(),
                switch("from_capture", "from-capture", None, "Use the newest captured real answer; sends nothing."),
                Param {
                    name: "vars",
                    about: "Variables for {{name}} in the sent request.",
                    ty: Type::TextMap,
                    place: Place::Pairs("var", "="),
                    required: false,
                },
                text("env", "env", "Environment to send the request with."),
                switch("fresh_session", "fresh-session", None, "Send with a throwaway cookie jar."),
                switch("force", "force", None, "Overwrite even if the mock changed since it was read."),
            ],
        },
        Row {
            name: "contract_snapshot",
            words: &["contract", "snapshot"],
            summary: "Store the body shape of each mock's real answer in the workspace, for contract_check to compare later.",
            details: "Sends each paired request once and writes types only, never a value, to the workspace's contracts folder.",
            effect: Effect::Runs,
            upfront: false,
            scoped: true,
            params: vec![
                text("mock", "mock", "Only this mock (also when disabled)."),
                text("folder", "folder", "Only the mocks in this mock folder and below."),
                switch("all", "all", None, "Disabled mocks too."),
                Param {
                    name: "vars",
                    about: "Variables for {{name}} in the sent requests.",
                    ty: Type::TextMap,
                    place: Place::Pairs("var", "="),
                    required: false,
                },
                text("env", "env", "Environment to send the requests with."),
                switch("fresh_session", "fresh-session", None, "Send with a throwaway cookie jar."),
            ],
        },
        Row {
            name: "changes",
            words: &["changes"],
            summary: "What other agents or the app changed in the workspace since you last asked: a feed, oldest first.",
            details: "Without `since` it shows what is new and moves this session's cursor past it. Your own changes are left out.",
            effect: Effect::Reads,
            upfront: false,
            scoped: true,
            params: vec![
                text("since", "since", "A sequence number, a time ago (2m, 1h, 2d) or a UTC time; the cursor stays."),
                switch("mine", "mine", None, "Include the changes this session made itself."),
            ],
        },
        Row {
            name: "claim",
            words: &["claim"],
            summary: "Claim an item you are editing (a lock for other agents) or release the claim.",
            details: "Advisory only: nothing is blocked, but another session's update of it prints a warning. Lapses after 30 minutes.",
            effect: Effect::Changes,
            upfront: false,
            scoped: true,
            params: vec![
                Param {
                    name: "kind",
                    about: "What kind of item.",
                    ty: Type::Choice(&["request", "mock", "flow", "env"]),
                    place: Place::Operand,
                    required: true,
                },
                operand("name", "The item's name.").required(),
                switch("release", "release", None, "Release the claim instead of taking it."),
            ],
        },
        Row {
            name: "claims",
            words: &["claims"],
            summary: "List the live claims: which items other agents are editing, by whom, and how long ago.",
            details: "",
            effect: Effect::Reads,
            upfront: false,
            scoped: true,
            params: vec![],
        },
        Row {
            name: "env_auth",
            words: &["env", "auth"],
            summary: "Set the login an environment re-authenticates with when a send answers 401, or turn it off.",
            details: "Give `login` (a request, or a flow of that name) or `flow`, or `off`. With retry on, a 401 re-runs the login once and retries.",
            effect: Effect::Changes,
            upfront: false,
            scoped: true,
            params: vec![
                operand("environment", "The environment's id or name.").required(),
                text("login", "login", "The request (or flow) that logs in."),
                text("flow", "flow", "The flow that logs in, when a request has the same name."),
                typed("retry", "retry", Type::Choice(&["true", "false"]), "Retry the send after logging in (default true)."),
                switch("off", "off", None, "Remove the login."),
            ],
        },
        Row {
            name: "runner_run",
            words: &["runner", "run"],
            summary: "Send every request of a folder in order, with scripts and tests: one line per request, then the total.",
            details: "The folder is an id, a path like auth/admin or a unique name. Only a failed request shows its \
                body (512 bytes). The result is an error when a request fails or a test does.",
            effect: Effect::Runs,
            upfront: false,
            scoped: true,
            params: [
                operand("folder", "The id, path (auth/admin) or name of a folder of the request tree.").required(),
                switch("bail", "bail", None, "Stop after the first request that fails."),
            ]
            .into_iter()
            .chain(run_options())
            .collect(),
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
    // A flow can also be written as steps (the text of the step file) instead of the JSON of its nodes.
    for tool in tools.iter_mut().filter(|tool| matches!(tool.name, "flows_create" | "flows_update")) {
        tool.params.iter_mut().filter(|param| param.name == "data").for_each(|param| param.required = false);
        tool.params.push(text(
            "steps",
            "steps",
            "The flow as steps, the text of a YAML or JSON file (name, steps: request/inline, extract, vars, expect), instead of data.",
        ));
    }
    // The update verbs refuse a write over another session's change; force overwrites it.
    for tool in tools.iter_mut().filter(|tool| {
        matches!(
            tool.name,
            "requests_update"
                | "mocks_update"
                | "mappings_update"
                | "rules_update"
                | "env_set"
                | "env_unset"
                | "env_auth"
        )
    }) {
        tool.params.push(switch(
            "force",
            "force",
            None,
            "Overwrite even if another session changed it since it was read.",
        ));
    }
    // flows_show prints the flow as a step file.
    if let Some(tool) = tools.iter_mut().find(|tool| tool.name == "flows_show") {
        tool.params.push(switch(
            "steps",
            "steps",
            None,
            "Print the flow as a step file (an error when it is not a line of steps).",
        ));
    }
    tools
}

const TREES: &[&str] = &["request", "mock", "flow", "rule"];

/// The tool of a verb that is a row of the command line's own table.
fn of_verb(verb: &Verb) -> Tool {
    let mut params: Vec<Param> = verb
        .args
        .iter()
        .map(|arg| Param {
            // A tool's arguments are snake_case; the option keeps its dash.
            name: if arg.name.contains('-') {
                Box::leak(arg.name.replace('-', "_").into_boxed_str())
            } else {
                arg.name
            },
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
            assert_eq!(tool.name, tool.words.join("_").replace('-', "_"), "a tool is named after its command");
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
                "mocks_enable",
                "mocks_disable",
                "mocks_scenario",
                "flows_list",
                "flows_run",
                "help",
                "capture_list",
                "capture_show",
            ]
        );
    }
}
