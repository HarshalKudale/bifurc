//! `contract.saveSnapshots` and `contract.listSnapshots`: the shape of what a saved request answered, kept in
//! the workspace as `contracts/<requestId>.json` so `bifurc contract check` can compare a mock with it while
//! the backend is down. A snapshot holds `{status, contentType, body}` where `body` is a skeleton (types, never
//! values); the CLI builds it. Only these two commands touch the folder; nothing else reads it.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::{
    kernel::CommandRegistry,
    protocol::{EngineError, Result},
    utils::{
        id::now_ms,
        json::{array_of, read_json, str_of, write_json_pretty},
    },
    workspace::{
        fs::ws_dir,
        git_store::{CommitMutation, commit_mutation},
    },
};

const DIR: &str = "contracts";

pub(crate) fn plain(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn dir_of(params: &Value) -> Result<PathBuf> {
    let ws = str_of(params, "workspaceId");
    if !plain(ws) || !ws_dir(ws).is_dir() {
        return Err(EngineError::not_found(format!("No workspace \"{ws}\".")));
    }
    Ok(ws_dir(ws).join(DIR))
}

/// The snapshots on disk, in file-name order.
fn list(dir: &PathBuf) -> Vec<Value> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files.iter().filter_map(|p| read_json(p)).filter(Value::is_object).collect()
}

/// Whether two snapshots say the same, whenever they were saved.
fn same(a: &Value, b: &Value) -> bool {
    let strip = |v: &Value| {
        let mut v = v.clone();
        if let Some(map) = v.as_object_mut() {
            map.shift_remove("savedAt");
        }
        v
    };
    strip(a) == strip(b)
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("contract.listSnapshots", |params, _| Ok(json!({ "snapshots": list(&dir_of(&params)?) })));

    registry.register("contract.saveSnapshots", |params, _| async move {
        let dir = dir_of(&params)?;
        let ws = str_of(&params, "workspaceId");
        let (mut created, mut updated) = (0, 0);
        for snapshot in array_of(&params, "snapshots") {
            let id = str_of(snapshot, "requestId");
            if !plain(id) {
                return Err(EngineError::bad_request(format!("\"{id}\" is not a request id.")));
            }
            let file = dir.join(format!("{id}.json"));
            let before = read_json(&file);
            if before.as_ref().is_some_and(|old| same(old, snapshot)) {
                continue;
            }
            let mut snapshot = snapshot.clone();
            snapshot["savedAt"] = json!(now_ms());
            std::fs::create_dir_all(&dir)?;
            write_json_pretty(&file, &snapshot)?;
            if before.is_some() { updated += 1 } else { created += 1 }
        }
        if created + updated > 0 {
            // Best effort: a workspace without a repository keeps the files and has no audit entry.
            let _ = commit_mutation(CommitMutation {
                action: if created > 0 { "create" } else { "update" },
                entity: "contract",
                entity_id: DIR,
                entity_name: &format!("{} snapshots", created + updated),
                workspace_id: ws,
                rel_path: DIR,
                actor: None,
                changed_fields: &[],
            })
            .await;
        }
        Ok(json!({ "saved": created + updated, "unchanged": array_of(&params, "snapshots").len() - created - updated }))
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{kernel::CommandContext, testing::scratch_data_root};

    async fn call(registry: &Arc<CommandRegistry>, name: &str, payload: Value) -> Result<Value> {
        registry.invoke(name, payload, CommandContext::internal(registry.clone())).await
    }

    fn snap(id: &str, status: u64) -> Value {
        json!({"requestId": id, "method": "GET", "path": "/orders", "status": status, "contentType": "json", "body": {"id": 0}})
    }

    #[tokio::test]
    async fn snapshots_round_trip_and_a_second_save_overwrites() {
        let _root = scratch_data_root();
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        let registry = Arc::new(registry);
        let ws = json!("ws1");
        let saved = call(
            &registry,
            "contract.saveSnapshots",
            json!({"workspaceId": ws, "snapshots": [snap("r1", 200), snap("r2", 200)]}),
        )
        .await
        .unwrap();
        assert_eq!(saved["saved"], 2);
        let listed = call(&registry, "contract.listSnapshots", json!({"workspaceId": ws})).await.unwrap();
        assert_eq!(listed["snapshots"].as_array().unwrap().len(), 2);
        assert_eq!(listed["snapshots"][0]["requestId"], "r1");
        assert!(listed["snapshots"][0]["savedAt"].is_number());
        // r1 changed, r2 is the same: one overwrite, still two files.
        let again = call(
            &registry,
            "contract.saveSnapshots",
            json!({"workspaceId": ws, "snapshots": [snap("r1", 201), snap("r2", 200)]}),
        )
        .await
        .unwrap();
        assert_eq!((again["saved"].clone(), again["unchanged"].clone()), (json!(1), json!(1)));
        let listed = call(&registry, "contract.listSnapshots", json!({"workspaceId": ws})).await.unwrap();
        assert_eq!(listed["snapshots"].as_array().unwrap().len(), 2);
        assert_eq!(listed["snapshots"][0]["status"], 201);
        // A request id that would leave the folder is refused.
        assert!(
            call(&registry, "contract.saveSnapshots", json!({"workspaceId": ws, "snapshots": [snap("../x", 200)]}))
                .await
                .is_err()
        );
    }
}
