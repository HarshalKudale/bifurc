//! Tests of the git commands against real repositories in temporary directories; skipped when git is missing.

use std::{path::Path, sync::Arc};

use serde_json::{Value, json};

use super::{auto_sync, get_sync_config, get_workspace_git_state, get_workspace_sync_status, register};
use crate::{
    kernel::{CommandContext, CommandRegistry},
    testing::scratch_data_root,
    workspace::{
        fs::{add_pending_deletion, write_entity, ws_dir},
        git_store::{check_git_installed, git, git_in, init_workspace_repo},
        settings::load_settings,
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
async fn remote_round_trip_against_a_bare_repository() {
    if !check_git_installed().await {
        return;
    }
    let root = scratch_data_root();
    let ws = root.ws_id;
    let engine = Engine::new();
    let scratch = tempfile::tempdir().unwrap();
    let bare = scratch.path().join("remote.git");
    std::fs::create_dir_all(&bare).unwrap();
    run(&bare, &["init", "--bare"]).await;
    let remote = bare.to_string_lossy().into_owned();

    init_ws(ws).await;
    write_entity(ws, "requests", "r1", &request("r1", "First"), None).unwrap();
    engine.call("entity.publish", json!({ "workspaceId": ws, "paths": ["requests/r1.json"] })).await;
    assert_eq!(get_workspace_git_state(ws).await["hasRemote"], false);

    let connected =
        engine.call("sync.setRemote", json!({ "workspaceId": ws, "remote": remote, "branch": "main" })).await;
    assert_eq!(connected, json!({ "ok": true, "cloned": false }));
    let config = get_sync_config(ws).unwrap();
    assert_eq!(config, json!({ "remote": remote, "branch": "main", "autoSync": false }));
    let state = engine.call("sync.getState", json!({ "workspaceId": ws })).await;
    assert_eq!(state["status"], "idle");
    assert!(state["lastPushedAt"].is_i64());
    assert_eq!(state["lastPulledAt"], Value::Null);
    assert_eq!(run(&bare, &["rev-parse", "main"]).await, run(&ws_dir(ws), &["rev-parse", "HEAD"]).await);

    let git_state = get_workspace_git_state(ws).await;
    assert_eq!(git_state, json!({ "hasRemote": true, "branch": "main", "ahead": false, "behind": false }));
    let entity_status = engine.call("sync.getEntityStatus", json!({ "workspaceId": ws })).await;
    assert_eq!(entity_status["files"]["requests/r1.json"], "clean");
    assert_eq!(entity_status["git"]["hasRemote"], true);
    assert_eq!(entity_status["paths"]["entities"]["r1"], "requests/r1.json");

    // A publish pushes by itself.
    write_entity(ws, "requests", "r2", &request("r2", "Second"), None).unwrap();
    engine.call("entity.publish", json!({ "workspaceId": ws, "paths": ["requests/r2.json"] })).await;
    assert_eq!(run(&bare, &["rev-parse", "main"]).await, run(&ws_dir(ws), &["rev-parse", "HEAD"]).await);

    // A second clone pushes a change; pulling brings it in.
    let other = scratch.path().join("other");
    run(scratch.path(), &["clone", "--branch", "main", &remote, &other.to_string_lossy()]).await;
    run(&other, &["config", "user.email", "o@example.com"]).await;
    run(&other, &["config", "user.name", "Other"]).await;
    std::fs::write(other.join("requests/r9.json"), serde_json::to_string_pretty(&request("r9", "Nine")).unwrap())
        .unwrap();
    run(&other, &["add", "-A"]).await;
    run(&other, &["commit", "-m", "create request Nine"]).await;
    run(&other, &["push", "origin", "HEAD:main"]).await;

    let pulled = engine.call("sync.pull", json!({ "workspaceId": ws })).await;
    assert_eq!(pulled, json!({ "ok": true, "updated": true, "updatedIds": ["r9"] }));
    assert!(ws_dir(ws).join("requests/r9.json").exists());
    let again = engine.call("sync.pull", json!({ "workspaceId": ws })).await;
    assert_eq!(again, json!({ "ok": true, "updated": false }));

    // A local commit is pushed by sync.push.
    run(&ws_dir(ws), &["commit", "--allow-empty", "-m", "local only"]).await;
    assert_eq!(get_workspace_git_state(ws).await["ahead"], true);
    assert_eq!(engine.call("sync.push", json!({ "workspaceId": ws })).await, json!({ "ok": true }));
    assert_eq!(run(&bare, &["rev-parse", "main"]).await, run(&ws_dir(ws), &["rev-parse", "HEAD"]).await);
    assert_eq!(get_workspace_git_state(ws).await["ahead"], false);

    // Auto-sync is persisted and starts a poller; turning it off stops it.
    assert_eq!(
        engine.call("sync.setAutoSync", json!({ "workspaceId": ws, "enabled": true })).await,
        json!({ "ok": true })
    );
    assert_eq!(get_sync_config(ws).unwrap()["autoSync"], true);
    assert_eq!(auto_sync::poller_count(), 1);
    engine.call("sync.setAutoSync", json!({ "workspaceId": ws, "enabled": false })).await;
    assert_eq!(auto_sync::poller_count(), 0);

    let gone = engine.call("sync.disconnect", json!({ "workspaceId": ws })).await;
    assert_eq!(gone, json!({ "ok": true }));
    assert!(get_sync_config(ws).is_none());
    assert_eq!(get_workspace_git_state(ws).await["hasRemote"], false);
    assert_eq!(engine.call("sync.push", json!({ "workspaceId": ws })).await["error"], "No remote configured");
}

#[tokio::test]
async fn non_empty_remote_is_refused_for_a_non_empty_workspace_and_cloned_into_an_empty_one() {
    if !check_git_installed().await {
        return;
    }
    let root = scratch_data_root();
    let ws = root.ws_id;
    let engine = Engine::new();
    let scratch = tempfile::tempdir().unwrap();
    let bare = scratch.path().join("remote.git");
    std::fs::create_dir_all(&bare).unwrap();
    run(&bare, &["init", "--bare"]).await;
    let seed = scratch.path().join("seed");
    std::fs::create_dir_all(seed.join("requests")).unwrap();
    run(&seed, &["init", "-b", "main"]).await;
    run(&seed, &["config", "user.email", "s@example.com"]).await;
    run(&seed, &["config", "user.name", "Seed"]).await;
    std::fs::write(seed.join("workspace.json"), r#"{ "id": "remote-ws", "name": "Remote" }"#).unwrap();
    std::fs::write(seed.join("requests/r1.json"), serde_json::to_string(&request("r1", "First")).unwrap()).unwrap();
    run(&seed, &["add", "-A"]).await;
    run(&seed, &["commit", "-m", "seed"]).await;
    let remote = bare.to_string_lossy().into_owned();
    run(&seed, &["push", &remote, "HEAD:main"]).await;

    // Non-empty workspace: refused, and the state says why.
    write_entity(ws, "requests", "mine", &request("mine", "Mine"), None).unwrap();
    let refused = engine.call("sync.setRemote", json!({ "workspaceId": ws, "remote": remote, "branch": "main" })).await;
    assert_eq!(refused["ok"], false);
    assert!(refused["error"].as_str().unwrap().starts_with("Remote is not empty"));
    assert_eq!(engine.call("sync.getState", json!({ "workspaceId": ws })).await["error"], "Remote is not empty");
    std::fs::remove_file(ws_dir(ws).join("requests/mine.json")).unwrap();

    // Empty workspace: the remote is cloned in and its identity adopted.
    let cloned = engine.call("sync.setRemote", json!({ "workspaceId": ws, "remote": remote, "branch": "main" })).await;
    assert_eq!(cloned, json!({ "ok": true, "cloned": true, "adoptedId": "remote-ws" }));
    assert!(ws_dir("remote-ws").join("requests/r1.json").exists());
    let settings = load_settings();
    assert!(settings.workspace("remote-ws").is_some());
    assert_eq!(settings.active_workspace_id(), "remote-ws");
    assert!(get_sync_config("remote-ws").is_some());
}

#[tokio::test]
async fn unreachable_remote_reports_an_error_as_data() {
    if !check_git_installed().await {
        return;
    }
    let root = scratch_data_root();
    let ws = root.ws_id;
    let engine = Engine::new();
    let missing = root.dir.path().join("does-not-exist.git").to_string_lossy().into_owned();
    let result = engine.call("sync.setRemote", json!({ "workspaceId": ws, "remote": missing, "branch": "main" })).await;
    assert_eq!(result["ok"], false);
    assert!(result["error"].as_str().unwrap().starts_with("Cannot access remote"));
    assert_eq!(engine.call("sync.getState", json!({ "workspaceId": ws })).await["status"], "error");
}

#[tokio::test]
async fn auto_sync_pollers_start_and_stop_without_leaking() {
    let root = scratch_data_root();
    let ws = root.ws_id;
    let before = auto_sync::poller_count();

    // Off by default: nothing starts.
    auto_sync::start_auto_sync(ws);
    assert_eq!(auto_sync::poller_count(), before);

    super::sync_state::save_sync_config(ws, Some(json!({ "remote": "nowhere", "branch": "main", "autoSync": true })));
    auto_sync::start_auto_sync(ws);
    auto_sync::start_auto_sync(ws);
    assert_eq!(auto_sync::poller_count(), before + 1);
    auto_sync::stop_auto_sync(ws);
    assert_eq!(auto_sync::poller_count(), before);

    auto_sync::start_auto_sync(ws);
    auto_sync::stop_all_auto_sync();
    assert_eq!(auto_sync::poller_count(), 0);
}
