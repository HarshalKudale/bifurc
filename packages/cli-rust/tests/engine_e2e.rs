//! End to end against a real `bifurc-engine` on a temporary data directory. Ignored by default because it
//! needs the engine built: `cargo build -p bifurc-engine -p bifurc-cli && cargo test -p bifurc-cli -- --ignored`.

use std::{path::Path, process::Command};

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
