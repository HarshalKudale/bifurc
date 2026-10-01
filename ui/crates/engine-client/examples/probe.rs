//! Starts (or attaches to) the engine for a data directory and prints what it says. A manual smoke test of
//! the real wire: `cargo run -p bifurc-engine-client --example probe -- <data-dir>`.

use bifurc_engine_client::{SidecarConfig, attach_or_start};
use serde_json::json;

fn main() {
    let mut config = SidecarConfig::default();
    if let Some(dir) = std::env::args().nth(1) {
        config.data_dir = dir.into();
    }
    println!("data dir: {}", config.data_dir.display());

    let (client, hello, mut sidecar) = attach_or_start(&config, env!("CARGO_PKG_VERSION")).expect("engine");
    println!(
        "hello: engine {} protocol {} session {} (owned: {})",
        hello.engine_version,
        hello.protocol_version,
        hello.session_id,
        sidecar.is_owned()
    );

    let cfg = client.call_blocking("config.get", json!({})).expect("config.get");
    println!("workspaces: {}", cfg["workspaces"]);
    println!("active workspace: {}", cfg["activeWorkspaceId"]);

    let status = client.call_blocking("server.status", json!({})).expect("server.status");
    println!("server.status: {status}");

    let missing = client.call_blocking("entity.load", json!({"workspaceId": "nope", "kind": "requests", "id": "x"}));
    println!("entity.load on a missing id: {missing:?}");

    sidecar.release(&client);
    println!("released");
}
