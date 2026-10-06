//! End to end against a real `bifurc-engine` on a temporary data directory. Ignored by default because it
//! needs the engine built: `cargo build -p bifurc-engine -p bifurc-cli && cargo test -p bifurc-cli -- --ignored`.

use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::Command,
};

/// A loopback SSE server: `/events` sends two events and ends, `/missing` is a 404. Returns its port.
fn sse_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            std::thread::spawn(move || {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&head).into_owned();
                if head.starts_with("GET /events") {
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n\
                          event: tick\ndata: one\n\ndata: two\ndata: lines\n\n",
                    );
                } else {
                    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                }
            });
        }
    });
    port
}

/// A loopback Socket.IO server (Engine.IO v4 over a WebSocket). It accepts any namespace but `/deny`, greets
/// with a `welcome` event, echoes `echo` events and acknowledges the ones that ask. Returns its port.
fn socketio_server() -> u16 {
    use tungstenite::Message;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            std::thread::spawn(move || {
                let Ok(mut ws) = tungstenite::accept(stream) else { return };
                let open = "0{\"sid\":\"eio\",\"upgrades\":[],\"pingInterval\":25000,\"pingTimeout\":20000,\"maxPayload\":1000000}";
                if ws.send(Message::text(open)).is_err() {
                    return;
                }
                while let Ok(message) = ws.read() {
                    let Message::Text(text) = message else { continue };
                    let text = text.to_string();
                    // After the two digits: the namespace with its comma (or nothing), an ack id, the JSON.
                    let rest = text.get(2..).unwrap_or_default();
                    let (ns, rest) = match rest.starts_with('/') {
                        true => rest.split_at(rest.find(',').map_or(rest.len(), |at| at + 1)),
                        false => ("", rest),
                    };
                    let digits = rest.chars().take_while(char::is_ascii_digit).count();
                    let (id, data) = rest.split_at(digits);
                    let mut replies = Vec::new();
                    if text.starts_with("40") && ns == "/deny," {
                        replies.push("44/deny,{\"message\":\"Not authorized\"}".to_string());
                    } else if text.starts_with("40") {
                        replies.push(format!("40{ns}{{\"sid\":\"sid-1\"}}"));
                        replies.push(format!("42{ns}[\"welcome\",{}]", if data.is_empty() { "null" } else { data }));
                    } else if text.starts_with("42") && data.starts_with("[\"echo\"") {
                        replies.push(format!("42{ns}{data}"));
                        if !id.is_empty() {
                            replies.push(format!("43{ns}{id}[\"got it\"]"));
                        }
                    } else if text.starts_with("41") {
                        return;
                    }
                    for reply in replies {
                        if ws.send(Message::text(reply)).is_err() {
                            return;
                        }
                    }
                }
            });
        }
    });
    port
}

fn bifurc(data_dir: &Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_bifurc"))
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .output()
        .expect("running bifurc");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
#[ignore = "needs a built bifurc-engine next to the bifurc binary or in BIFURC_ENGINE"]
fn start_status_call_and_stop_a_real_engine() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path();

    let (ok, out, err) = bifurc(data, &["engine", "start", "--no-tray"]);
    assert!(ok, "start failed: {out}{err}");
    assert!(out.starts_with("started  pid "), "{out}");

    let (ok, out, err) = bifurc(data, &["engine", "status"]);
    assert!(ok && out.starts_with("running  pid "), "{out}{err}");

    // A data directory whose first-run setup is not finished is the interface's: every command is refused,
    // except asking how far the setup is.
    for argv in [vec!["--no-start", "call", "config.get", "{}"], vec!["--no-start", "status"], vec!["--no-start", "mocks"]] {
        let (ok, out, err) = bifurc(data, &argv);
        assert!(!ok && out.is_empty(), "{argv:?}: {out}");
        assert!(err.starts_with("bifurc: Bifurc has not been set up yet."), "{argv:?}: {err}");
    }
    let (ok, out, err) = bifurc(data, &["--no-start", "call", "setup.getState"]);
    assert!(ok && out.contains("\"onboarded\": false"), "{out}{err}");

    // What the interface's setup leaves behind when it is finished.
    let settings_path = data.join("app.json");
    let mut settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
    settings["hasSeenWelcome"] = true.into();
    std::fs::write(&settings_path, settings.to_string()).unwrap();

    let (ok, out, err) = bifurc(data, &["--no-start", "call", "config.get", "{}"]);
    assert!(ok, "{out}{err}");
    assert!(out.contains("activeWorkspaceId"), "{out}");

    let (ok, out, err) = bifurc(data, &["--no-start", "workspaces", "create", "E2E"]);
    assert!(ok && out.starts_with("Created workspace "), "{out}{err}");
    let (ok, out, err) = bifurc(data, &["--no-start", "workspaces"]);
    assert!(ok && out.starts_with("ACTIVE  ID"), "{out}{err}");

    // Every group of saved items is created, read, updated and switched or run the same way, by id or name.
    for (create, update, read) in [
        (
            vec!["mappings", "create", "api", "127.0.0.1:3999"],
            vec!["mappings", "update", "api.localhost", "--target", "127.0.0.1:4000"],
            (vec!["mappings", "show", "api.localhost"], "127.0.0.1:4000"),
        ),
        (
            vec!["rules", "create", "api.example.com/v1", "--mapping", "api.localhost", "--name", "V1"],
            vec!["rules", "update", "V1", "--target", "http://localhost:9000"],
            (vec!["rules", "show", "V1"], "\"targetType\": \"external\""),
        ),
        (
            vec!["mocks", "create", "https://api.example.com/users", "--name", "Users", "--status", "201"],
            vec!["mocks", "update", "Users", "--status", "404"],
            (vec!["mocks", "show", "Users"], "\"responseStatus\": 404"),
        ),
        (
            vec!["requests", "create", "--type", "grpc", "--url", "127.0.0.1:1", "--method", "pkg.S/M", "--name", "Rpc"],
            vec!["requests", "update", "Rpc", "--method", "pkg.S/Other"],
            (vec!["requests", "show", "Rpc"], "\"methodName\": \"Other\""),
        ),
        (
            vec!["health", "create", "Dead", "http://127.0.0.1:1/"],
            vec!["health", "update", "Dead", "--auto"],
            (vec!["health", "show", "Dead"], "\"autoRefreshEnabled\": true"),
        ),
    ] {
        for argv in [&create, &update] {
            let (ok, out, err) = bifurc(data, argv);
            assert!(ok, "{argv:?}: {out}{err}");
        }
        let (ok, out, err) = bifurc(data, &read.0);
        assert!(ok && out.contains(read.1), "{:?}: {out}{err}", read.0);
        let (ok, _, err) = bifurc(data, &[create[0], "delete", "x"]);
        assert!(!ok && err.contains("deleting is done in the desktop app"), "{err}");
    }
    for group in ["mappings", "rules", "mocks"] {
        let named = if group == "mappings" { "api.localhost" } else if group == "rules" { "V1" } else { "Users" };
        let (ok, out, err) = bifurc(data, &[group, "disable", named]);
        assert!(ok && out.starts_with("Disabled "), "{out}{err}");
        let (ok, out, err) = bifurc(data, &[group, "list"]);
        assert!(ok && out.contains("false"), "{out}{err}");
    }
    let (ok, out, err) = bifurc(data, &["requests", "list", "--type", "grpc"]);
    assert!(ok && out.contains("pkg.S/Other") && out.contains("grpc"), "{out}{err}");
    let (ok, out, err) = bifurc(data, &["requests", "list", "--type", "rest"]);
    assert!(ok && out.trim() == "(none)", "{out}{err}");
    // An SSE request is saved, changed, listed and sent like the others; what it receives is printed.
    let port = sse_server();
    let url = format!("http://127.0.0.1:{port}/events");
    let (ok, out, err) = bifurc(data, &["requests", "create", "--type", "sse", "--url", &url, "--name", "Feed"]);
    assert!(ok && out.starts_with("Saved sse request "), "{out}{err}");
    let (ok, out, err) = bifurc(data, &["requests", "update", "Feed", "-H", "X-Token: abc"]);
    assert!(ok, "{out}{err}");
    let (ok, out, err) = bifurc(data, &["requests", "show", "Feed"]);
    assert!(ok && out.contains("\"X-Token\": \"abc\"") && out.contains("\"type\": \"sse\""), "{out}{err}");
    let (ok, out, err) = bifurc(data, &["requests", "list", "--type", "sse"]);
    assert!(ok && out.contains("Feed") && out.contains("GET") && out.contains(&url), "{out}{err}");
    let events = format!("SSE disconnected  {url}\n< [tick] one\n< two\n  lines");
    let (ok, out, err) = bifurc(data, &["requests", "send", "Feed", "--wait", "10"]);
    assert!(ok && out.trim_end() == events, "{out}{err}");
    let (ok, out, err) = bifurc(data, &["requests", "send", "--type", "sse", "--url", &url]);
    assert!(ok && out.trim_end() == events, "{out}{err}");
    let missing = format!("http://127.0.0.1:{port}/missing");
    let (ok, out, _) = bifurc(data, &["requests", "send", "--type", "sse", "--url", &missing]);
    assert!(!ok && out.contains("error: HTTP 404 Not Found"), "{out}");
    // Nothing is left open by the sends above.
    let (ok, out, err) = bifurc(data, &["--no-start", "call", "config.get", "{}"]);
    assert!(ok, "{out}{err}");
    let active = serde_json::from_str::<serde_json::Value>(&out).unwrap()["activeWorkspaceId"].as_str().unwrap().to_string();
    let payload = serde_json::json!({ "workspaceId": active }).to_string();
    let (ok, out, err) = bifurc(data, &["--no-start", "call", "sse.list", &payload]);
    assert!(ok && out.contains("\"streams\": []"), "{out}{err}");

    // A Socket.IO request is saved, changed, listed and sent like the others: it connects to its namespace
    // with its auth payload, emits the event asked for and prints what came back.
    let port = socketio_server();
    let url = format!("http://127.0.0.1:{port}");
    let (ok, out, err) =
        bifurc(data, &["requests", "create", "--type", "socketio", "--url", &url, "--name", "Chat", "--namespace", "/chat"]);
    assert!(ok && out.starts_with("Saved socketio request "), "{out}{err}");
    let (ok, out, err) = bifurc(data, &["requests", "update", "Chat", "--auth", "{\"token\":\"abc\"}"]);
    assert!(ok, "{out}{err}");
    let (ok, out, err) = bifurc(data, &["requests", "show", "Chat"]);
    assert!(ok && out.contains("\"namespace\": \"/chat\"") && out.contains("\"type\": \"socketio\""), "{out}{err}");
    let (ok, out, err) = bifurc(data, &["requests", "list", "--type", "socketio"]);
    assert!(ok && out.contains("Chat") && out.contains("/chat") && out.contains(&url), "{out}{err}");
    let (ok, out, err) =
        bifurc(data, &["requests", "send", "Chat", "--event", "echo", "-d", "[\"hi\", {\"n\": 1}]", "--ack", "--wait", "1"]);
    let exchange = format!(
        "Socket.IO connected  {url}  /chat\n< [welcome] [{{\"token\":\"abc\"}}]\n> [echo #0] [\"hi\",{{\"n\":1}}]\n\
         < [echo] [\"hi\",{{\"n\":1}}]\n< [ack #0] [\"got it\"]"
    );
    assert!(ok && out.trim_end() == exchange, "{out}{err}");
    let (ok, out, err) =
        bifurc(data, &["requests", "send", "--type", "socketio", "--url", &url, "--event", "echo", "-d", "plain", "--wait", "1"]);
    let exchange = format!("Socket.IO connected  {url}\n< [welcome] [null]\n> [echo] [\"plain\"]\n< [echo] [\"plain\"]");
    assert!(ok && out.trim_end() == exchange, "{out}{err}");
    let (ok, out, _) =
        bifurc(data, &["requests", "send", "--type", "socketio", "--url", &url, "--namespace", "/deny", "--event", "echo"]);
    assert!(!ok && out.contains("error: Not authorized") && !out.contains("> [echo]"), "{out}");
    // Only the events listened to are printed: the server's welcome is not among them.
    let (ok, out, err) = bifurc(
        data,
        &["requests", "send", "--type", "socketio", "--url", &url, "--listen", "echo", "--event", "echo", "-d", "plain", "--wait", "1"],
    );
    let exchange = format!("Socket.IO connected  {url}\n> [echo] [\"plain\"]\n< [echo] [\"plain\"]");
    assert!(ok && out.trim_end() == exchange, "{out}{err}");
    let (ok, out, err) = bifurc(data, &["requests", "update", "Chat", "--listen", "nothing"]);
    assert!(ok, "{out}{err}");
    let (ok, out, err) = bifurc(data, &["requests", "send", "Chat", "--event", "echo", "-d", "x", "--wait", "1"]);
    assert!(ok && out.trim_end() == format!("Socket.IO connected  {url}  /chat\n> [echo] [\"x\"]"), "{out}{err}");
    // Nothing is left open by the sends above.
    let (ok, out, err) = bifurc(data, &["--no-start", "call", "socketio.list", &payload]);
    assert!(ok && out.contains("\"connections\": []"), "{out}{err}");

    // What was run failed, so the command does: nothing listens on that port.
    let (ok, out, _) = bifurc(data, &["health", "run"]);
    assert!(!ok && out.contains("failing"), "{out}");
    let (ok, _, err) = bifurc(data, &["requests", "send", "Rpc"]);
    assert!(!ok && err.starts_with("bifurc: "), "{err}");

    let (ok, out, err) = bifurc(data, &["engine", "stop"]);
    assert!(ok && out.starts_with("stopped  pid "), "{out}{err}");

    let (ok, out, _) = bifurc(data, &["engine", "status"]);
    assert!(ok && out.starts_with("not running for "), "{out}");
}
