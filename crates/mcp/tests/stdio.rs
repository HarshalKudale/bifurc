//! The built `bifurc-mcp` spoken to as an MCP host does: JSON-RPC, one message a line, on its stdin and stdout.
//! No engine is involved: the server is started with `--no-start` on an empty data directory.

use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

use serde_json::{Value, json};

struct Host {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    _dir: tempfile::TempDir,
}

impl Host {
    /// Start the server and shake hands.
    fn start(options: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_bifurc-mcp"))
            .args(["--no-start", "--data-dir"])
            .arg(dir.path())
            .args(options)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut host = Self { child, stdin, stdout, next_id: 1, _dir: dir };
        let hello = host.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "stdio-test", "version": "0" },
            }),
        );
        assert_eq!(hello["serverInfo"]["name"], "bifurc-mcp");
        assert!(hello["capabilities"]["tools"].is_object());
        assert!(hello["instructions"].as_str().unwrap().contains("Bifurc"));
        host.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        host
    }

    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Send a request and return the `result` of its response.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let mut line = String::new();
            assert!(self.stdout.read_line(&mut line).unwrap() > 0, "the server closed its stdout");
            // Every line of stdout is a protocol message: nothing else may be printed there.
            let message: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}"));
            if message["id"] == json!(id) {
                assert!(message.get("error").is_none(), "{method}: {message}");
                return message["result"].clone();
            }
        }
    }

    fn call(&mut self, name: &str, arguments: Value) -> (String, bool) {
        let result = self.request("tools/call", json!({ "name": name, "arguments": arguments }));
        let text = result["content"][0]["text"].as_str().unwrap_or_default().to_string();
        (text, result["isError"] == true)
    }

    fn tool_names(&mut self) -> Vec<String> {
        let listed = self.request("tools/list", json!({}));
        listed["tools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap().to_string()).collect()
    }

    /// Close the server's stdin and return its exit code: that is how a host ends it.
    fn close(mut self) -> Option<i32> {
        drop(self.stdin);
        self.child.wait().unwrap().code()
    }
}

#[test]
fn a_host_lists_searches_and_calls_tools() {
    let mut host = Host::start(&[]);
    let names = host.tool_names();
    assert_eq!(names.len(), 15);
    assert!(names.iter().any(|name| name == "requests_send") && names.iter().any(|name| name == "search_tools"));
    assert!(!names.iter().any(|name| name == "health_create"));

    let (found, failed) = host.call("search_tools", json!({ "query": "create health check" }));
    assert!(!failed && found.starts_with("health_create: "), "{found}");

    let (shown, failed) = host.call("describe_tool", json!({ "name": "health_create" }));
    let shown: Value = serde_json::from_str(&shown).unwrap();
    assert!(!failed);
    assert_eq!(shown["inputSchema"]["required"], json!(["name", "url"]));

    // The tool runs the command, which here has no engine to talk to and may not start one.
    let (said, failed) =
        host.call("call_tool", json!({ "name": "health_create", "arguments": { "name": "A", "url": "http://x" } }));
    assert!(failed && said.starts_with("No engine is running for"), "{said}");
    let (said, failed) = host.call("status", json!({}));
    assert!(failed && said.starts_with("No engine is running for"), "{said}");
    // The manual needs no engine, and a verb of the command line's table is a tool like the others.
    let (manual, failed) = host.call("help", json!({ "group": "sync" }));
    assert!(!failed && manual.contains("\n  discard <path> --yes\n"), "{manual}");
    let (said, failed) =
        host.call("call_tool", json!({ "name": "sync_discard", "arguments": { "path": "mocks/a.json" } }));
    assert!(failed && said.starts_with("sync_discard: missing \"yes\""), "{said}");

    // The list is the same after all that: it does not depend on what was asked before.
    assert_eq!(host.tool_names(), names);
    assert_eq!(host.close(), Some(0));
}

#[test]
fn all_tools_lists_every_tool() {
    let mut host = Host::start(&["--all-tools"]);
    let names = host.tool_names();
    assert_eq!(names.len(), bifurc_mcp::tools::catalog().len());
    assert!(names.iter().any(|name| name == "health_create") && !names.iter().any(|name| name == "search_tools"));
    assert_eq!(host.close(), Some(0));
}

#[test]
fn help_version_and_a_wrong_option_do_not_serve() {
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_bifurc-mcp")).args(args).stdin(Stdio::null()).output().unwrap()
    };
    let help = run(&["--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&help.stdout).starts_with("Usage: bifurc-mcp"));
    let version = run(&["--version"]);
    assert_eq!(String::from_utf8_lossy(&version.stdout).trim(), format!("bifurc-mcp {}", env!("CARGO_PKG_VERSION")));
    let wrong = run(&["--frobnicate"]);
    assert_eq!(wrong.status.code(), Some(2));
    assert!(wrong.stdout.is_empty());
    assert!(String::from_utf8_lossy(&wrong.stderr).starts_with("bifurc-mcp: unknown option \"--frobnicate\""));
}
