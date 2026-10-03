//! Tests of the git commands against real repositories in temporary directories; skipped when git is missing.

use std::{path::Path, sync::Arc};

use serde_json::{Value, json};

use super::{auto_sync, get_workspace_sync_status, register};
use crate::{
    kernel::{CommandContext, CommandRegistry},
    testing::scratch_data_root,
    workspace::{
        fs::{add_pending_deletion, write_entity, ws_dir},
        git_store::{check_git_installed, git, git_in, init_workspace_repo},
    },
};

struct Engine {
    registry: Arc<CommandRegistry>,
}

impl Engine {
    fn new() -> Engine {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        Engine { registry: Arc::new(registry) }
    }

    async fn call(&self, action: &str, payload: Value) -> Value {
        let ctx = CommandContext::internal(self.registry.clone());
        self.registry.invoke(action, payload, ctx).await.unwrap_or_else(|e| panic!("{action} failed: {e:?}"))
    }
}

fn request(id: &str, name: &str) -> Value {
    json!({ "id": id, "name": name, "method": "GET", "url": "http://localhost/x" })
}

/// A repository for `ws1` on branch `main`, with the init commit.
async fn init_ws(ws: &str) {
    init_workspace_repo(ws).await.unwrap();
    git(ws, &["symbolic-ref", "HEAD", "refs/heads/main"]).await.unwrap();
}

async fn run(dir: &Path, args: &[&str]) -> String {
    git_in(dir, args).await.unwrap_or_else(|e| panic!("git {args:?} failed: {e}"))
}

#[tokio::test]
async fn status_publish_discard_diff_and_history() {
    if !check_git_installed().await {
        return;
    }
    let root = scratch_data_root();
    let ws = root.ws_id;
    let engine = Engine::new();
    init_ws(ws).await;

    write_entity(ws, "requests", "r1", &request("r1", "First"), None).unwrap();
    write_entity(ws, "requests", "r2", &request("r2", "Second"), None).unwrap();
    let status = get_workspace_sync_status(ws).await;
    assert_eq!(status["requests/r1.json"], "new");
    assert_eq!(status["requests/r2.json"], "new");
    assert!(!status.contains_key("requests/index.json"));

    // Only the selected path is committed.
    let published = engine.call("entity.publish", json!({ "workspaceId": ws, "paths": ["requests/r1.json"] })).await;
    assert_eq!(published["ok"], true);
    let sha = published["commit"].as_str().expect("a commit id").to_string();
    let files = run(&ws_dir(ws), &["show", "--name-only", "--format=%s", "HEAD"]).await;
    assert!(files.contains("create request First"), "{files}");
    assert!(files.contains("requests/r1.json") && !files.contains("r2.json"), "{files}");
    let status = get_workspace_sync_status(ws).await;
    assert_eq!(status["requests/r1.json"], "clean");
    assert_eq!(status["requests/r2.json"], "new");

    // Nothing left to publish for r1.
    let again = engine.call("git.sync", json!({ "workspaceId": ws, "paths": ["requests/r1.json"] })).await;
    assert_eq!(again, json!({ "ok": true, "commit": null }));

    // A path git has never heard of is reported, not swallowed.
    let missing = engine.call("entity.publish", json!({ "workspaceId": ws, "paths": ["requests/nope.json"] })).await;
    assert_eq!(missing["ok"], false);
    assert!(missing["error"].as_str().unwrap().contains("does not exist"));

    // Modified: diff, then discard restores the committed content.
    write_entity(ws, "requests", "r1", &request("r1", "Renamed"), None).unwrap();
    assert_eq!(get_workspace_sync_status(ws).await["requests/r1.json"], "modified");
    let diff = engine.call("git.diff", json!({ "workspaceId": ws, "relPath": "requests\\r1.json" })).await;
    assert_eq!(diff["hasDiff"], true);
    assert_eq!(diff["status"], "modified");
    assert!(diff["diff"].as_str().unwrap().contains("Renamed"));
    let new_diff = engine.call("git.diff", json!({ "workspaceId": ws, "relPath": "requests/r2.json" })).await;
    assert_eq!(new_diff["status"], "new");
    assert!(new_diff["current"].as_str().unwrap().contains("Second"));
    let clean_diff = engine.call("git.diff", json!({ "workspaceId": ws, "relPath": "requests/index.json" })).await;
    assert_eq!(clean_diff, json!({ "hasDiff": false, "status": "clean", "diff": "" }));

    let discarded = engine.call("git.discard", json!({ "workspaceId": ws, "relPath": "requests/r1.json" })).await;
    assert_eq!(discarded, json!({ "ok": true }));
    assert!(std::fs::read_to_string(ws_dir(ws).join("requests/r1.json")).unwrap().contains("First"));
    assert_eq!(get_workspace_sync_status(ws).await["requests/r1.json"], "clean");

    // Discarding an untracked file deletes it.
    engine.call("entity.restore", json!({ "workspaceId": ws, "relPath": "requests/r2.json" })).await;
    assert!(!ws_dir(ws).join("requests/r2.json").exists());

    // Deleted.
    std::fs::remove_file(ws_dir(ws).join("requests/r1.json")).unwrap();
    assert_eq!(get_workspace_sync_status(ws).await["requests/r1.json"], "deleted");
    engine.call("entity.restore", json!({ "workspaceId": ws, "relPath": "requests/r1.json" })).await;
    assert_eq!(get_workspace_sync_status(ws).await["requests/r1.json"], "clean");

    // History.
    let history = engine.call("history.list", json!({ "filePath": "requests\\r1.json" })).await;
    assert_eq!(history["total"], 1);
    assert_eq!(history["entries"][0]["action"], "create");
    assert_eq!(history["entries"][0]["entityId"], "r1");
    let by_git =
        engine.call("git.history", json!({ "workspaceId": ws, "relPath": "requests/r1.json", "limit": 5 })).await;
    assert_eq!(by_git, history);

    let hash = history["entries"][0]["commitHash"].as_str().unwrap();
    assert!(hash.starts_with(&sha));
    let diff = engine
        .call("history.diff", json!({ "commitHash": hash, "filePath": "requests/r1.json", "workspaceId": ws }))
        .await;
    assert_eq!(diff["before"], Value::Null);
    assert_eq!(diff["after"]["name"], "First");
    let audit = engine
        .call("audit.diff", json!({ "commitHash": hash, "entity": "request", "entityId": "r1", "workspaceId": ws }))
        .await;
    assert_eq!(audit["after"]["id"], "r1");
}

#[tokio::test]
async fn folder_publish_and_pending_deletions() {
    if !check_git_installed().await {
        return;
    }
    let root = scratch_data_root();
    let ws = root.ws_id;
    let engine = Engine::new();
    init_ws(ws).await;

    write_entity(ws, "requests", "a", &request("a", "A"), Some("My Folder")).unwrap();
    write_entity(ws, "requests", "b", &request("b", "B"), Some("My Folder")).unwrap();
    let status = get_workspace_sync_status(ws).await;
    assert_eq!(status["requests/My Folder/a.json"], "new");

    let result = engine
        .call("folder.publish", json!({ "workspaceId": ws, "kind": "requests", "folderName": "My Folder" }))
        .await;
    assert_eq!(result["ok"], true);
    assert!(result["commit"].is_string());
    let subject = run(&ws_dir(ws), &["log", "-1", "--format=%s%n%b"]).await;
    assert!(subject.contains("update request folder \"My Folder\""), "{subject}");
    assert!(subject.contains("entity-ids: a,b"), "{subject}");
    assert_eq!(get_workspace_sync_status(ws).await["requests/My Folder/b.json"], "clean");

    // An entity that never reached git but has a pending deletion is `deleted`.
    add_pending_deletion(ws, "mocks", json!({ "id": "gone", "folderId": null, "name": "Gone" })).unwrap();
    assert_eq!(get_workspace_sync_status(ws).await["mocks/gone.json"], "deleted");
}

#[tokio::test]
async fn auto_sync_pollers_start_and_stop_without_leaking() {
    let root = scratch_data_root();
    let ws = root.ws_id;
    let before = auto_sync::poller_count();

    // A workspace with no remote has nothing to watch.
    auto_sync::start_auto_sync(ws);
    assert_eq!(auto_sync::poller_count(), before);

    super::sync_state::save_sync_config(ws, Some(json!({ "remote": "nowhere", "branch": "main" })));
    auto_sync::start_auto_sync(ws);
    auto_sync::start_auto_sync(ws);
    assert_eq!(auto_sync::poller_count(), before + 1);
    auto_sync::stop_auto_sync(ws);
    assert_eq!(auto_sync::poller_count(), before);

    auto_sync::start_auto_sync(ws);
    auto_sync::stop_all_auto_sync();
    assert_eq!(auto_sync::poller_count(), 0);
}
