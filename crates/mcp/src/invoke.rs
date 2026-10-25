//! Running a tool: its arguments become the command line of the command it stands for, and the CLI's own
//! `run` executes that line, finding (or starting) the engine exactly as `bifurc` does. What the command would
//! have printed is the tool's result; a non-zero exit code makes it an error result.

use std::ffi::OsString;

use serde_json::{Map, Value};

use crate::tools::{Place, Tool, Type};

/// What `bifurc-mcp` was started with; it applies to every tool.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    /// `--data-dir`: the data directory whose engine the tools talk to.
    pub data_dir: Option<String>,
    /// `--no-start`: fail instead of starting an engine when none is running.
    pub no_start: bool,
    /// `--session`: the named session every tool call works in (else `BIFURC_SESSION`, which the CLI reads itself).
    pub session: Option<String>,
    /// The value of `BIFURC_DATA_DIR`.
    pub env_data_dir: Option<OsString>,
}

/// What a tool answered.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub text: String,
    pub failed: bool,
}

/// An argument as one word of a command line: text as it is, anything else as its JSON.
fn word(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn wrong(tool: &Tool, name: &str, expected: &str) -> String {
    format!("{}: \"{name}\" must be {expected}", tool.name)
}

/// The value of one argument, checked against what its parameter is.
fn checked(tool: &Tool, name: &str, ty: Type, value: &Value) -> Result<(), String> {
    let ok = match ty {
        Type::Text => value.is_string() || value.is_number() || value.is_boolean(),
        Type::Integer => value.is_i64() || value.is_u64() || value.as_str().is_some_and(|t| t.parse::<i64>().is_ok()),
        Type::Number => value.is_number() || value.as_str().is_some_and(|t| t.parse::<f64>().is_ok()),
        Type::Boolean => value.is_boolean(),
        Type::Choice(words) => value.as_str().is_some_and(|t| words.contains(&t)),
        Type::Json => value.is_object() || value.is_array() || value.is_string(),
        Type::TextMap | Type::AnyMap => value.is_object(),
        Type::TextList => value.as_array().is_some_and(|items| items.iter().all(Value::is_string)),
    };
    if ok {
        return Ok(());
    }
    Err(wrong(
        tool,
        name,
        &match ty {
            Type::Text => "text".to_string(),
            Type::Integer => "a whole number".to_string(),
            Type::Number => "a number".to_string(),
            Type::Boolean => "true or false".to_string(),
            Type::Choice(words) => format!("one of {}", words.join(", ")),
            Type::Json => "a JSON object or array".to_string(),
            Type::TextMap | Type::AnyMap => "an object".to_string(),
            Type::TextList => "a list of text".to_string(),
        },
    ))
}

/// The command line a call of `tool` with `arguments` stands for, after the program name. An argument the tool
/// does not have, one of the wrong type and a missing required one are errors; a `null` is an argument left out.
pub fn command_line(tool: &Tool, arguments: &Map<String, Value>) -> Result<Vec<String>, String> {
    if let Some(unknown) = arguments.keys().find(|name| !tool.params.iter().any(|param| param.name == *name)) {
        let known: Vec<&str> = tool.params.iter().map(|param| param.name).collect();
        return Err(format!("{}: unknown argument \"{unknown}\"; it takes: {}", tool.name, known.join(", ")));
    }
    let mut line: Vec<String> = tool.words.iter().map(|word| word.to_string()).collect();
    let mut operands = Vec::new();
    for param in &tool.params {
        let Some(value) = arguments.get(param.name).filter(|value| !value.is_null()) else {
            if param.required {
                return Err(format!("{}: missing \"{}\" ({})", tool.name, param.name, param.about));
            }
            if let Place::OptionOr(option, default) = param.place {
                line.extend([format!("--{option}"), default.to_string()]);
            }
            continue;
        };
        checked(tool, param.name, param.ty, value)?;
        match param.place {
            Place::Operand => operands.push(word(value)),
            Place::Operands => operands.extend(value.as_array().into_iter().flatten().map(word)),
            Place::Option(option) | Place::OptionOr(option, _) => line.extend([format!("--{option}"), word(value)]),
            Place::Switch(on, off) => match (value.as_bool(), off) {
                (Some(true), _) => line.push(format!("--{on}")),
                (_, Some(off)) => line.push(format!("--{off}")),
                _ => {}
            },
            Place::Pairs(option, separator) => {
                for (name, value) in value.as_object().into_iter().flatten() {
                    if param.ty == Type::TextMap && !value.is_string() {
                        return Err(wrong(tool, &format!("{}.{name}", param.name), "text"));
                    }
                    line.extend([format!("--{option}"), format!("{name}{separator}{}", word(value))]);
                }
            }
            Place::Each(option) => {
                for value in value.as_array().into_iter().flatten() {
                    line.extend([format!("--{option}"), word(value)]);
                }
            }
        }
    }
    // An MCP call cannot block, so the mock server is always detached.
    if tool.name == "mocks_serve" {
        line.push("--detach".to_string());
    }
    // After `--` nothing is an option, so an operand may start with a dash.
    if !operands.is_empty() {
        line.push("--".to_string());
        line.extend(operands);
    }
    Ok(line)
}

/// The error line of the CLI without the program's name in front.
fn said(stderr: &str) -> &str {
    stderr.strip_prefix("bifurc: ").unwrap_or(stderr)
}

/// Run `tool` with `arguments` and return what the command printed.
pub fn run(tool: &Tool, arguments: &Map<String, Value>, settings: &Settings) -> Outcome {
    let mut line = match command_line(tool, arguments) {
        Ok(line) => line,
        Err(message) => return Outcome { text: message, failed: true },
    };
    // The server's own options go before the `--` that ends the options.
    let at = tool.words.len();
    let mut ours = Vec::new();
    if let Some(dir) = &settings.data_dir {
        ours.extend(["--data-dir".to_string(), dir.clone()]);
    }
    if let Some(name) = &settings.session {
        ours.extend(["--session".to_string(), name.clone()]);
    }
    if settings.no_start {
        ours.push("--no-start".to_string());
    }
    line.splice(at..at, ours);

    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let code = bifurc_cli::cli::run_with_env(&line, settings.env_data_dir.clone(), &mut stdout, &mut stderr);
    let (stdout, stderr) = (String::from_utf8_lossy(&stdout), String::from_utf8_lossy(&stderr));
    let parts: Vec<&str> =
        [stdout.trim_end(), said(stderr.trim_end())].into_iter().filter(|part| !part.is_empty()).collect();
    Outcome { text: parts.join("\n"), failed: code != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::catalog;
    use serde_json::json;

    fn tool(name: &str) -> Tool {
        catalog().into_iter().find(|tool| tool.name == name).unwrap_or_else(|| panic!("no tool {name}"))
    }

    fn line(name: &str, arguments: Value) -> Result<Vec<String>, String> {
        command_line(&tool(name), arguments.as_object().unwrap())
    }

    #[test]
    fn options_come_before_the_operands_which_follow_a_double_dash() {
        let built = line("mappings_create", json!({"domain": "api", "target": "127.0.0.1:3000", "label": "My API"}));
        assert_eq!(built.unwrap(), ["mappings", "create", "--label", "My API", "--", "api", "127.0.0.1:3000"]);
        // So an operand that looks like an option is still an operand.
        let built = line("rules_create", json!({"pattern": "--help", "target": "http://x"}));
        assert_eq!(built.unwrap(), ["rules", "create", "--target", "http://x", "--", "--help"]);
    }

    #[test]
    fn a_tool_without_arguments_is_its_command() {
        assert_eq!(line("status", json!({})).unwrap(), ["status"]);
        assert_eq!(
            line("mocks_list", json!({"workspace": "w2", "json": true})).unwrap(),
            ["mocks", "list", "--workspace", "w2", "--json"]
        );
    }

    #[test]
    fn switches_take_their_on_or_off_form() {
        let built = line("rules_update", json!({"rule": "r1", "regex": false})).unwrap();
        assert_eq!(built, ["rules", "update", "--no-regex", "--", "r1"]);
        let built = line("rules_update", json!({"rule": "r1", "regex": true})).unwrap();
        assert_eq!(built, ["rules", "update", "--regex", "--", "r1"]);
        // A switch with no off form is left out when false.
        let built = line("mocks_create", json!({"url": "http://x/a", "disabled": false, "regex": false})).unwrap();
        assert_eq!(built, ["mocks", "create", "--url", "http://x/a"]);
    }

    #[test]
    fn headers_lists_and_tool_arguments_repeat_their_option() {
        let built = line(
            "requests_send",
            json!({
                "url": "http://x", "type": "socketio", "headers": {"Accept": "a/b", "X-Gone": ""},
                "listen": ["tick", "tock"], "wait": 1.5,
            }),
        )
        .unwrap();
        assert_eq!(
            built,
            [
                "requests",
                "send",
                "--type",
                "socketio",
                "--url",
                "http://x",
                "--header",
                "Accept: a/b",
                "--header",
                "X-Gone: ",
                "--listen",
                "tick",
                "--listen",
                "tock",
                "--wait",
                "1.5",
            ]
        );
        let built = line(
            "requests_send",
            json!({"request": "Files", "tool": "read", "arguments": {"path": "/a", "depth": 2, "deep": true}}),
        );
        assert_eq!(
            built.unwrap(),
            [
                "requests",
                "send",
                "--tool",
                "read",
                "--arg",
                "path=/a",
                "--arg",
                "depth=2",
                "--arg",
                "deep=true",
                "--",
                "Files",
            ]
        );
    }

    #[test]
    fn a_saved_send_takes_variables_an_environment_a_body_and_patches() {
        let built = line(
            "requests_send",
            json!({"request": "Order", "vars": {"host": "a.test"}, "env": "staging", "body": "{}", "patch": [".a=1", ".b=2"]}),
        );
        assert_eq!(
            built.unwrap(),
            [
                "requests",
                "send",
                "--data",
                "{}",
                "--var",
                "host=a.test",
                "--env",
                "staging",
                "--patch",
                ".a=1",
                "--patch",
                ".b=2",
                "--",
                "Order",
            ]
        );
    }

    #[test]
    fn json_arguments_are_given_as_an_object_or_as_its_text() {
        for variables in [json!({"id": 1}), json!("{\"id\":1}")] {
            let built = line(
                "requests_create",
                json!({"type": "graphql", "url": "http://x", "query": "{a}", "variables": variables}),
            );
            assert_eq!(built.unwrap()[8..], ["--variables".to_string(), "{\"id\":1}".to_string()]);
        }
    }

    #[test]
    fn the_mock_server_tools_are_always_detached_or_stopping() {
        let built = line("mocks_serve", json!({"port": 4010})).unwrap();
        assert_eq!(built, ["mocks", "serve", "--port", "4010", "--detach"]);
        assert_eq!(line("mocks_serve_stop", json!({})).unwrap(), ["mocks", "serve", "stop"]);
    }

    #[test]
    fn numbers_are_written_as_the_command_line_reads_them() {
        let built = line("mocks_update", json!({"mock": "m1", "status": 404, "delay": "250"})).unwrap();
        assert_eq!(built, ["mocks", "update", "--status", "404", "--delay", "250", "--", "m1"]);
    }

    #[test]
    fn a_flow_run_is_always_bounded() {
        assert_eq!(line("flows_run", json!({"flow": "f1"})).unwrap(), ["flows", "run", "--timeout", "120", "--", "f1"]);
        assert_eq!(
            line("flows_run", json!({"flow": "f1", "timeout": 0, "inputs": {"a": "b"}})).unwrap(),
            ["flows", "run", "--input", "a=b", "--timeout", "0", "--", "f1"]
        );
    }

    #[test]
    fn the_newer_verbs_write_their_command_lines() {
        let built =
            line("contract_check", json!({"openapi": "o.yaml", "ignore": [".meta.*", ".a[].b"], "all": true})).unwrap();
        assert_eq!(
            built,
            ["contract", "check", "--all", "--ignore", ".meta.*", "--ignore", ".a[].b", "--openapi", "o.yaml"]
        );
        let built = line("capture_diff", json!({"entry": "#k3f9", "other": "x1"})).unwrap();
        assert_eq!(built, ["capture", "diff", "--", "#k3f9", "x1"]);
        let built = line("capture_diff", json!({"entry": "k3f9", "mock": "Get order", "shape": true})).unwrap();
        assert_eq!(built, ["capture", "diff", "--mock", "Get order", "--shape", "--", "k3f9"]);
        let built = line("claim", json!({"kind": "mock", "name": "Get orders", "release": true})).unwrap();
        assert_eq!(built, ["claim", "--release", "--", "mock", "Get orders"]);
        let built = line("env_auth", json!({"environment": "dev", "login": "Login", "retry": "false"})).unwrap();
        assert_eq!(built, ["env", "auth", "--login", "Login", "--retry", "false", "--", "dev"]);
        let built = line("changes", json!({"since": "10m", "mine": true})).unwrap();
        assert_eq!(built, ["changes", "--since", "10m", "--mine"]);
        let built = line("capture_pin", json!({"entries": ["a", "b"]})).unwrap();
        assert_eq!(built, ["capture", "pin", "--", "a", "b"]);
        let built = line("contract_update_mock", json!({"mock": "M", "force": true})).unwrap();
        assert_eq!(built, ["contract", "update-mock", "--force", "--", "M"]);
    }

    #[test]
    fn a_null_is_an_argument_left_out() {
        assert_eq!(line("health_run", json!({"check": null})).unwrap(), ["health", "run"]);
    }

    #[test]
    fn missing_unknown_and_mistyped_arguments_are_refused() {
        let err = line("requests_show", json!({})).unwrap_err();
        assert_eq!(err, "requests_show: missing \"request\" (The request's id or name.)");
        let err = line("status", json!({"verbose": true})).unwrap_err();
        assert_eq!(err, "status: unknown argument \"verbose\"; it takes: json");
        for (arguments, expected) in [
            (
                json!({"type": "carrier-pigeon"}),
                "requests_list: \"type\" must be one of rest, graphql, grpc, soap, websocket, sse, socketio, mcp",
            ),
            (json!({"json": "yes"}), "requests_list: \"json\" must be true or false"),
            (json!({"workspace": ["w1"]}), "requests_list: \"workspace\" must be text"),
        ] {
            assert_eq!(line("requests_list", arguments).unwrap_err(), expected);
        }
        let err = line("mocks_update", json!({"mock": "m1", "status": "teapot"})).unwrap_err();
        assert_eq!(err, "mocks_update: \"status\" must be a whole number");
        let err = line("mocks_update", json!({"mock": "m1", "headers": {"a": 1}})).unwrap_err();
        assert_eq!(err, "mocks_update: \"headers.a\" must be text");
        let err = line("requests_send", json!({"url": "http://x", "listen": "tick"})).unwrap_err();
        assert_eq!(err, "requests_send: \"listen\" must be a list of text");
    }

    #[test]
    fn curl_takes_the_whole_command_after_the_double_dash_and_its_json_is_ours() {
        let built = line("curl", json!({"command": "curl -s http://x/u", "save": "S", "json": true})).unwrap();
        assert_eq!(built, ["curl", "--save", "S", "--bifurc-json", "--", "curl -s http://x/u"]);
    }

    /// Arguments that use every parameter of every tool at least once, in sets the command line accepts.
    fn examples() -> Vec<(&'static str, Value)> {
        let all = json!({"workspace": "w1", "json": true});
        let with = |extra: Value| {
            let mut merged = all.as_object().cloned().unwrap();
            merged.extend(extra.as_object().cloned().unwrap());
            Value::Object(merged)
        };
        vec![
            ("status", json!({"json": true})),
            ("requests_list", with(json!({"type": "grpc"}))),
            ("requests_show", with(json!({"request": "Ping", "type": "rest"}))),
            ("session_show", all.clone()),
            ("session_reset", all.clone()),
            ("vars_list", all.clone()),
            ("vars_set", with(json!({"variables": ["a=1"]}))),
            ("vars_clear", with(json!({"variables": ["a"]}))),
            ("requests_send", with(json!({"request": "Ping", "type": "rest"}))),
            (
                "requests_send",
                with(json!({"request": "Ping", "vars": {"host": "a.test"}, "env": "staging", "patch": [".qty=2"]})),
            ),
            ("requests_send", with(json!({"url": "http://x", "method": "POST", "body": "{}", "headers": {"a": "b"}}))),
            (
                "requests_send",
                with(
                    json!({"url": "http://x", "select": ".a", "show_body": true, "show_headers": true, "max_body": 10, "expect_status": 200, "expect": ["status < 300", ".a == 1"], "extract": ["id=.id"]}),
                ),
            ),
            (
                "requests_send",
                with(
                    json!({"type": "graphql", "url": "http://x", "query": "{a}", "variables": {"a": 1}, "operation": "A"}),
                ),
            ),
            ("requests_send", with(json!({"type": "grpc", "url": "x:1", "method": "p.S/M", "proto": "p1"}))),
            ("requests_send", with(json!({"type": "grpc", "url": "x:1", "method": "p.S/M", "reflection": true}))),
            ("requests_send", with(json!({"type": "soap", "url": "http://x", "action": "urn:a", "body": "<a/>"}))),
            (
                "requests_send",
                with(
                    json!({"type": "socketio", "url": "http://x", "namespace": "/n", "path": "/s", "auth": {"t": 1}, "listen": ["a"], "event": "e", "body": "[1]", "ack": true, "wait": 1}),
                ),
            ),
            (
                "requests_send",
                with(json!({"type": "mcp", "url": "npx x", "transport": "stdio", "tool": "t", "arguments": {"a": 1}})),
            ),
            ("requests_send", with(json!({"type": "mcp", "url": "http://x/mcp", "prompt": "p"}))),
            ("requests_send", with(json!({"type": "mcp", "url": "http://x/mcp", "resource": "file:///a"}))),
            (
                "requests_create",
                with(json!({"name": "N", "url": "http://x", "method": "POST", "body": "{}", "headers": {"a": "b"}})),
            ),
            ("requests_create", with(json!({"curl": "curl http://x", "name": "N"}))),
            (
                "requests_create",
                with(
                    json!({"type": "graphql", "url": "http://x", "query": "{a}", "variables": "{}", "operation": "A"}),
                ),
            ),
            ("requests_create", with(json!({"type": "grpc", "url": "x:1", "method": "p.S/M", "proto": "p1"}))),
            ("requests_create", with(json!({"type": "grpc", "url": "x:1", "method": "p.S/M", "reflection": true}))),
            ("requests_create", with(json!({"type": "soap", "url": "http://x", "action": "urn:a"}))),
            (
                "requests_create",
                with(
                    json!({"type": "socketio", "url": "http://x", "namespace": "/n", "path": "/s", "auth": {"t": 1}, "listen": ["a"]}),
                ),
            ),
            ("requests_create", with(json!({"type": "mcp", "url": "npx x", "transport": "stdio"}))),
            (
                "requests_update",
                with(
                    json!({"request": "Ping", "type": "rest", "name": "N", "url": "http://x", "headers": {"a": ""}, "method": "PUT", "body": "b", "query": "{a}", "variables": {}, "operation": "A", "proto": "p", "reflection": true, "action": "a", "namespace": "/n", "path": "/p", "auth": {}, "listen": ["a"], "transport": "http"}),
                ),
            ),
            ("requests_reflect", with(json!({"address": "localhost:50051"}))),
            ("mocks_list", all.clone()),
            ("mocks_show", with(json!({"mock": "m1"}))),
            ("mocks_enable", with(json!({"mock": "m1"}))),
            ("mocks_disable", with(json!({"mock": "m1"}))),
            (
                "mocks_create",
                with(
                    json!({"url": "http://x/a", "method": "*", "name": "N", "status": 201, "headers": {"a": "b"}, "body": "{}", "delay": 10, "regex": true, "disabled": true}),
                ),
            ),
            ("mocks_create", with(json!({"curl": "curl http://x"}))),
            ("mocks_scenario", with(json!({"mock": "m1", "scenario": "slow", "delay": 500}))),
            ("mocks_status", all.clone()),
            ("mocks_hits", with(json!({"mock": "m1", "last": 3}))),
            ("mocks_serve", with(json!({"port": 0, "host": "127.0.0.1", "fallthrough": "http://localhost:3000"}))),
            ("mocks_serve_stop", with(json!({"port": 4010}))),
            (
                "mocks_update",
                with(
                    json!({"mock": "m1", "url": "http://x/b", "method": "GET", "name": "N", "status": 200, "headers": {"a": ""}, "body": "", "delay": 0, "regex": false}),
                ),
            ),
            ("mappings_list", all.clone()),
            ("mappings_show", with(json!({"mapping": "api"}))),
            ("mappings_enable", with(json!({"mapping": "api"}))),
            ("mappings_disable", with(json!({"mapping": "api"}))),
            (
                "mappings_create",
                with(json!({"domain": "api", "target": "127.0.0.1:1", "label": "L", "disabled": true})),
            ),
            ("mappings_update", with(json!({"mapping": "api", "domain": "b", "target": "127.0.0.1:2", "label": "L"}))),
            ("rules_list", all.clone()),
            ("rules_show", with(json!({"rule": "r1"}))),
            ("rules_enable", with(json!({"rule": "r1"}))),
            ("rules_disable", with(json!({"rule": "r1"}))),
            (
                "rules_create",
                with(json!({"pattern": "a.com", "mapping": "api", "name": "N", "regex": true, "disabled": true})),
            ),
            ("rules_create", with(json!({"pattern": "a.com", "target": "http://x"}))),
            (
                "rules_update",
                with(json!({"rule": "r1", "pattern": "b.com", "regex": false, "mapping": "api", "name": "N"})),
            ),
            ("rules_update", with(json!({"rule": "r1", "target": "http://x"}))),
            (
                "curl",
                with(json!({
                    "command": "curl -X POST http://x/u -d '{}'", "save": "Create", "folder": "auth", "update": true,
                    "save_only": true, "vars": {"a": "b"}, "env": "dev", "extract": ["id=.id"], "select": ".id", "show_body": true,
                    "show_headers": true, "max_body": 100, "expect_status": 201, "expect": ["status == 201"],
                })),
            ),
            ("env_list", all.clone()),
            ("env_show", all.clone()),
            ("env_use", with(json!({"environment": "Dev"}))),
            ("health_list", all.clone()),
            ("health_show", with(json!({"check": "h1"}))),
            ("health_create", with(json!({"name": "Users", "url": "http://x/health", "auto": true}))),
            ("health_update", with(json!({"check": "h1", "name": "N", "url": "http://y", "auto": false}))),
            ("health_run", with(json!({"check": "h1"}))),
            ("flows_list", all.clone()),
            ("flows_show", with(json!({"flow": "f1"}))),
            // A flow is written as JSON (the table's example) or as the text of a step file.
            ("flows_create", with(json!({"steps": "name: A\nsteps: []"}))),
            ("flows_update", with(json!({"flow": "f1", "steps": "name: A\nsteps: []"}))),
            (
                "flows_run",
                with(json!({"flow": "f1", "inputs": {"user": "bob"}, "timeout": 30, "max_events": 5, "vars": {"a": "b"},
                            "env": "dev", "fresh_session": true, "verbose": true, "max_body": 100})),
            ),
            ("flows_loadtest", with(json!({"flow": "f1", "instances": 5, "inputs": {"user": "bob"}, "timeout": 30}))),
            ("workspaces_list", json!({"json": true})),
            ("workspaces_show", json!({"workspace": "w1", "json": true})),
            ("workspaces_create", json!({"name": "Mine", "account": "octocat", "json": true})),
            ("workspaces_update", json!({"workspace": "w1", "name": "New", "json": true})),
            ("workspaces_use", json!({"workspace": "w1", "json": true})),
            ("workspaces_sync", with(json!({"message": "Add the login flow"}))),
            ("vault_list", all.clone()),
            ("account_list", json!({"json": true})),
            ("account_repos", json!({"account": "octocat", "json": true})),
            ("services_list", json!({"json": true})),
            ("folders_list", json!({"tree": "mock", "json": true})),
            ("env_set", with(json!({"environment": "Dev", "variables": ["HOST=http://x"]}))),
            ("env_unset", with(json!({"environment": "Dev", "variables": ["HOST"]}))),
            (
                "capture_list",
                with(json!({
                    "last": 5, "status": "4xx", "method": "POST", "host": "api.*", "path": "/orders*",
                    "since": "2m", "origin": "mock", "grep": "qty", "pinned": true,
                })),
            ),
            (
                "capture_show",
                with(json!({
                    "entry": "k3f9", "select": ".id", "show_body": true, "show_headers": true, "req_body": true,
                    "req_headers": true, "max_body": 100,
                })),
            ),
            ("capture_show", with(json!({"entry": "k3f9", "logs": true, "context": 2, "service": "api"}))),
            ("capture_mock", with(json!({"entry": "c1"}))),
            ("capture_request", with(json!({"entry": "c1"}))),
            (
                "runner_run",
                with(json!({"folder": "f1", "bail": true, "vars": {"a": "b"}, "env": "dev", "fresh_session": true,
                            "verbose": true, "max_body": 100})),
            ),
            ("settings_show", json!({"json": true})),
            ("wait", with(json!({"target": "http://x/health", "status": "200", "timeout": "1s", "interval": "100ms"}))),
            (
                "wait",
                with(json!({
                    "capture": true, "status": "4xx", "host": "api.*", "method": "POST", "path": "/orders*",
                    "origin": "mock", "grep": "qty",
                })),
            ),
            ("wait", json!({"port": 3000, "host": "127.0.0.1", "timeout": "0", "json": true})),
            ("health_delete", with(json!({"check": "h1"}))),
            ("flows_show", with(json!({"flow": "f1", "steps": true}))),
            ("capture_diff", with(json!({"entry": "k3f9", "other": "x1", "shape": true}))),
            ("capture_diff", with(json!({"entry": "k3f9", "mock": "M"}))),
            (
                "capture_diff",
                with(json!({"entry": "k3f9", "request": "R", "vars": {"a": "b"}, "env": "dev", "fresh_session": true})),
            ),
            ("capture_pin", with(json!({"entries": ["k3f9"]}))),
            ("capture_unpin", with(json!({"entries": ["k3f9"]}))),
            (
                "contract_check",
                with(json!({"mock": "M", "folder": "shop", "all": true, "ignore": [".a.*"], "openapi": "o.yaml", "vars": {"a": "b"}, "env": "dev", "fresh_session": true})),
            ),
            ("contract_check", with(json!({"from_capture": true, "from_snapshot": true}))),
            (
                "contract_update_mock",
                with(json!({"mock": "M", "from_capture": true, "vars": {"a": "b"}, "env": "dev", "fresh_session": true, "force": true})),
            ),
            (
                "contract_snapshot",
                with(json!({"mock": "M", "folder": "shop", "all": true, "vars": {"a": "b"}, "env": "dev", "fresh_session": true})),
            ),
            (
                "services",
                with(json!({"verb": "define", "name": "api", "cmd": "npm run dev", "port": 3000, "cwd": ".", "vault": "dev", "env": {"A": "b"}, "log_file": "app.log", "mapping": "api"})),
            ),
            ("services", with(json!({"verb": "up", "all": true, "wait": true, "timeout": "30s"}))),
            ("logs", with(json!({"name": "api", "errors": true, "since": "2m", "grep": "x", "last": 20}))),
            ("port_free", with(json!({"port": 3000, "kill": true}))),
            (
                "mail",
                json!({"verb": "last", "to": "a@b.co", "link": true, "link_match": "/verify", "wait": "30s", "json": true}),
            ),
            ("mail", json!({"verb": "last", "otp": true})),
            ("mail", json!({"verb": "list", "to": "a@b.co", "last": 5})),
            ("mail", json!({"verb": "show", "id": "#k3f9", "html": true, "text": false})),
            ("webhooks", json!({"verb": "list", "name": "stripe", "last": 5, "json": true})),
            (
                "webhooks",
                json!({"verb": "listen", "name": "Stripe in", "folder": "hooks", "url": "http://api.localhost/hook"}),
            ),
            ("webhooks", json!({"verb": "stop", "id": "stripe"})),
            (
                "webhooks",
                json!({"verb": "trigger", "id": "stripe", "vault": "hooks", "secret": "K", "patch": [".type=x"], "to": "http://localhost:3000"}),
            ),
            (
                "webhooks",
                json!({"verb": "trigger", "id": "github.push", "show_body": true, "show_headers": true, "select": ".ok", "max_body": 0, "expect_status": 200}),
            ),
            ("webhooks", json!({"verb": "save", "id": "#k3f9", "as_request": true, "folder": "hooks"})),
            ("webhooks", json!({"verb": "save", "id": "#k3f9", "as_request": true, "name": "Stripe hook"})),
            ("jwt", json!({"verb": "decode", "token": "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2ln", "check": true, "json": true})),
            ("jwt", json!({"verb": "decode", "from_jar": "api.localhost"})),
            (
                "jwt",
                json!({"verb": "mint", "key_from_vault": "jwt_dev", "secret": "JWT_KEY", "claims": "{\"role\":\"admin\"}", "exp": "5m", "alg": "HS256", "header": "{\"kid\":\"dev\"}"}),
            ),
            ("approvals_list", with(json!({"all": true}))),
            ("policy_show", with(json!({}))),
            ("policy_check", with(json!({"method": "DELETE", "url": "https://api.prod.x/orders/1", "env": "prod"}))),
            ("types", with(json!({"request": "Get orders", "lang": "zod", "out": "src/o.ts", "name": "Order", "send": true, "env": "Dev"}))),
            ("openapi_infer", with(json!({"folder": "shop", "out": "o.json", "title": "Shop"}))),
            ("record", with(json!({"verb": "start", "hosts": "api.stripe.com,api.openai.com", "cassette": "checkout"}))),
            ("record", with(json!({"verb": "list"}))),
            ("faults", json!({"verb": "add", "target": "api.localhost/orders", "status": "429", "rate": "30%"})),
            ("faults", json!({"verb": "add", "target": "api.localhost/orders", "drop": "10%"})),
            ("faults", json!({"verb": "add", "target": "api.localhost/orders", "latency": "2s", "reset": "5%"})),
            ("faults", json!({"verb": "clear", "target": "#1k3f", "json": true})),
            ("replay", with(json!({"target": "checkout", "passthrough": true}))),
            ("replay", with(json!({"target": "stop"}))),
            ("changes", with(json!({"since": "2m", "mine": true}))),
            ("claim", with(json!({"kind": "mock", "name": "M", "release": true}))),
            ("claim", with(json!({"kind": "mock", "name": "M", "force": true}))),
            ("claims", all.clone()),
            ("env_auth", with(json!({"environment": "Dev", "login": "L", "flow": "F", "retry": "false", "off": true, "force": true}))),
            ("requests_update", with(json!({"request": "Ping", "force": true}))),
            ("mocks_update", with(json!({"mock": "m1", "force": true}))),
            ("mappings_update", with(json!({"mapping": "api", "force": true}))),
            ("rules_update", with(json!({"rule": "r1", "force": true}))),
            ("env_set", with(json!({"environment": "Dev", "variables": ["A=1"], "force": true}))),
            ("env_unset", with(json!({"environment": "Dev", "variables": ["A"], "force": true}))),
        ]
        .into_iter()
        .chain(table_examples())
        .collect()
    }

    /// Every argument of every tool that is a row of the command line's own table, with a value of its type.
    fn table_examples() -> Vec<(&'static str, Value)> {
        let verbs = bifurc_cli::direct::verbs();
        let example = |ty: Type| match ty {
            Type::Text => json!("x"),
            Type::Integer | Type::Number => json!(1),
            Type::Boolean => json!(true),
            Type::Choice(words) => json!(words[0]),
            Type::Json | Type::AnyMap => json!({"a": 1}),
            Type::TextMap => json!({"a": "b"}),
            Type::TextList => json!(["a"]),
        };
        catalog()
            .into_iter()
            .filter(|tool| verbs.iter().any(|verb| verb.words == tool.words))
            .map(|tool| {
                let arguments: Map<String, Value> = tool
                    .params
                    .iter()
                    .filter(|param| param.name != "steps")
                    .map(|param| (param.name.to_string(), example(param.ty)))
                    .collect();
                (tool.name, Value::Object(arguments))
            })
            .collect()
    }

    #[test]
    fn help_is_the_manual_of_the_command_line_and_needs_no_engine() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            data_dir: Some(dir.path().to_string_lossy().into_owned()),
            no_start: true,
            session: None,
            env_data_dir: None,
        };
        let overview = run(&tool("help"), &Map::new(), &settings);
        assert!(!overview.failed && overview.text.starts_with("Usage: bifurc <group> <verb>"), "{}", overview.text);
        let flows = run(&tool("help"), json!({"group": "flows", "json": true}).as_object().unwrap(), &settings);
        assert!(flows.text.starts_with("Usage: bifurc flows <verb>") && flows.text.contains("\n  validate <flow>\n"));
        assert!(flows.text.contains("The JSON of a flow"));
    }

    #[test]
    fn the_examples_use_every_parameter_of_every_tool() {
        let examples = examples();
        for tool in catalog().into_iter().filter(|tool| tool.name != "help") {
            for param in &tool.params {
                let used =
                    examples.iter().any(|(name, arguments)| *name == tool.name && arguments.get(param.name).is_some());
                assert!(used, "no example uses {}.{}", tool.name, param.name);
            }
        }
    }

    /// The command line is the judge of what a command takes: every example gets as far as needing the engine,
    /// which means the CLI found no unknown option, no extra operand and no missing one.
    #[test]
    fn the_command_line_accepts_what_every_tool_writes() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            data_dir: Some(dir.path().to_string_lossy().into_owned()),
            no_start: true,
            session: None,
            env_data_dir: None,
        };
        for (name, arguments) in examples() {
            // Waiting for a port needs no engine: it tries the port. Nor does decoding a token.
            if arguments.get("port").is_some() || arguments.get("token").is_some() {
                continue;
            }
            let outcome = run(&tool(name), arguments.as_object().unwrap(), &settings);
            assert!(outcome.failed, "{name}");
            assert!(outcome.text.starts_with("No engine is running for"), "{name} {arguments}: {}", outcome.text);
        }
        assert!(!dir.path().join("runtime.json").exists());
    }

    #[test]
    fn what_the_command_line_refuses_is_the_tools_error() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            data_dir: Some(dir.path().to_string_lossy().into_owned()),
            no_start: true,
            session: None,
            env_data_dir: None,
        };
        let arguments = json!({"type": "rest", "url": "http://x", "query": "{a}"});
        let outcome = run(&tool("requests_create"), arguments.as_object().unwrap(), &settings);
        assert_eq!(
            outcome,
            Outcome {
                text: "--query is not an option of a rest request (see: bifurc requests --help)".to_string(),
                failed: true,
            }
        );
    }
}
