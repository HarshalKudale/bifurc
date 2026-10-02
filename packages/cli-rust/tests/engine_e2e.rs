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

    let (ok, out, err) = bifurc(data, &["--no-start", "call", "config.get", "{}"]);
    assert!(ok, "{out}{err}");
    assert!(out.contains("activeWorkspaceId"), "{out}");

    let (ok, out, err) = bifurc(data, &["--no-start", "workspaces"]);
    assert!(ok && out.starts_with("ACTIVE  ID"), "{out}{err}");

    let (ok, out, err) = bifurc(data, &["engine", "stop"]);
    assert!(ok && out.starts_with("stopped  pid "), "{out}{err}");

    let (ok, out, _) = bifurc(data, &["engine", "status"]);
    assert!(ok && out.starts_with("not running for "), "{out}");
}
