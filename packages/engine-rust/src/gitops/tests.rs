//! Tests of the git commands against real repositories in temporary directories; skipped when git is missing.

use std::{path::Path, sync::Arc};

use serde_json::{Value, json};

use super::{auto_sync, get_workspace_sync_status, register};
use crate::{
    kernel::{CommandContext, CommandRegistry, bus},
    testing::scratch_data_root,
    workspace::{
        fs::{
            add_pending_deletion, get_pending_deletions, pending_folders_key, read_names_index, write_entity, ws_dir,
        },
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

    /// With the entity and folder commands too.
    fn full() -> Engine {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        crate::workspace::entities::register(&mut registry);
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

    let folder = crate::workspace::fs::create_folder(ws, "requests", "My Folder", None, false).unwrap().0;
    write_entity(ws, "requests", "a", &request("a", "A"), Some(&folder)).unwrap();
    write_entity(ws, "requests", "b", &request("b", "B"), Some(&folder)).unwrap();
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

/// The requests of the snapshot, by name.
fn listed(key: &str) -> Vec<(String, bool)> {
    let cfg = crate::workspace::config::load_config();
    let mut out: Vec<(String, bool)> = cfg[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["name"].as_str().unwrap_or("").to_string(), e["deleted"] == true))
        .collect();
    out.sort();
    out
}

/// A folder `Outer` holding a synced request and a local one, and a folder `Child` under it holding a local
/// one. Returns the folder ids and the request ids.
async fn folder_with_synced_and_local_requests(engine: &Engine, ws: &str) -> ([String; 2], [String; 3]) {
    let folder = |name: &str, parent: Value| json!({ "kind": "request", "name": name, "parentId": parent });
    let outer = engine.call("folder.add", folder("Outer", Value::Null)).await["id"].as_str().unwrap().to_string();
    let child = engine.call("folder.add", folder("Child", json!(outer))).await["id"].as_str().unwrap().to_string();
    let mut ids = Vec::new();
    for (name, folder_id) in [("Synced", &outer), ("Local", &outer), ("In child", &child)] {
        let entity = json!({ "name": name, "method": "GET", "url": "http://a", "folderId": folder_id });
        let created = engine.call("entity.create", json!({ "kind": "requests", "entity": entity })).await;
        ids.push(created["id"].as_str().unwrap().to_string());
    }
    let path = format!("requests/Outer/{}.json", ids[0]);
    let published = engine.call("entity.publish", json!({ "workspaceId": ws, "paths": [path] })).await;
    assert_eq!(published["ok"], true);
    ([outer, child], [ids[0].clone(), ids[1].clone(), ids[2].clone()])
}

#[tokio::test]
async fn a_synced_entity_stays_listed_as_deleted_until_the_delete_is_confirmed_or_reverted() {
    if !check_git_installed().await {
        return;
    }
    let root = scratch_data_root();
    let ws = root.ws_id;
    let engine = Engine::full();
    init_ws(ws).await;
    let (_, [synced, local, _]) = folder_with_synced_and_local_requests(&engine, ws).await;
    let path = format!("requests/Outer/{synced}.json");

    // Never synced: gone at once.
    let gone = engine.call("entity.delete", json!({ "kind": "requests", "id": local })).await;
    assert_eq!(gone, json!({ "ok": true, "pending": false }));
    assert_eq!(listed("requests"), [("In child".to_string(), false), ("Synced".to_string(), false)]);

    // Synced: the file goes, the entity stays listed as deleted, at the path git knows it by.
    let kept = engine.call("entity.delete", json!({ "kind": "requests", "id": synced })).await;
    assert_eq!(kept, json!({ "ok": true, "pending": true }));
    assert!(!ws_dir(ws).join(&path).exists());
    assert_eq!(listed("requests"), [("In child".to_string(), false), ("Synced".to_string(), true)]);
    let status = engine.call("sync.getEntityStatus", json!({ "workspaceId": ws })).await;
    assert_eq!(status["files"][&path], "deleted");
    assert_eq!(status["paths"]["entities"][&synced], path.as_str());
    let load = json!({ "workspaceId": ws, "kind": "requests", "id": synced });
    assert_eq!(engine.call("entity.load", load.clone()).await, json!({ "ok": false, "deleted": true }));
    let ctx = CommandContext::internal(engine.registry.clone());
    let update = json!({ "kind": "requests", "entity": { "id": synced, "name": "x", "method": "GET", "url": "u" } });
    assert!(engine.registry.invoke("entity.update", update, ctx).await.is_err(), "a deleted entity is not written");

    // Revert: it is back as it was.
    let reverted = engine.call("entity.revertDelete", json!({ "kind": "requests", "id": synced })).await;
    assert_eq!(reverted, json!({ "ok": true }));
    assert_eq!(engine.call("entity.load", load).await["entity"]["name"], "Synced");
    assert_eq!(listed("requests"), [("In child".to_string(), false), ("Synced".to_string(), false)]);
    assert_eq!(get_workspace_sync_status(ws).await[&path], "clean");

    // Confirm: the deletion is committed and nothing of it is left.
    engine.call("entity.delete", json!({ "kind": "requests", "id": synced })).await;
    let confirmed = engine.call("entity.confirmDelete", json!({ "kind": "requests", "id": synced })).await;
    assert_eq!(confirmed["ok"], true);
    assert!(confirmed["commit"].is_string());
    let subject = run(&ws_dir(ws), &["log", "-1", "--format=%s"]).await;
    assert_eq!(subject.trim(), "delete request Synced");
    assert_eq!(listed("requests"), [("In child".to_string(), false)]);
    assert!(!get_workspace_sync_status(ws).await.contains_key(&path));
    assert!(read_names_index(ws, "requests").get(&synced).is_none());
    assert!(get_pending_deletions(ws, "requests").is_empty());
}

#[tokio::test]
async fn a_deleted_folder_stays_with_what_was_synced_in_it_and_says_so() {
    if !check_git_installed().await {
        return;
    }
    let root = scratch_data_root();
    let ws = root.ws_id;
    let engine = Engine::full();
    init_ws(ws).await;
    let ([outer, child], [synced, _, _]) = folder_with_synced_and_local_requests(&engine, ws).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let events = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = events.clone();
    let listener = bus().on_any(move |name, _| sink.lock().unwrap().push(name.to_string()));
    let deleted = engine.call("folder.delete", json!({ "kind": "request", "id": outer })).await;
    assert_eq!(deleted, json!({ "ok": true, "pending": true }));
    for _ in 0..50 {
        if events.lock().unwrap().iter().any(|name| name == "sync.entityStatus") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    bus().off(listener);
    assert!(events.lock().unwrap().iter().any(|name| name == "sync.entityStatus"), "clients are told to re-read");

    // The local requests and the folder that held only a local one are gone; the synced one waits in its folder.
    assert!(!ws_dir(ws).join("requests/Outer").exists() && !ws_dir(ws).join("requests/Child").exists());
    assert_eq!(listed("requests"), [("Synced".to_string(), true)]);
    assert_eq!(listed("requestFolders"), [("Outer".to_string(), true)]);
    let again = engine.call("folder.delete", json!({ "kind": "request", "id": outer })).await;
    assert_eq!(again, json!({ "ok": true, "pending": true }));

    // Revert: the folder is the same folder again, with the synced request in it.
    let reverted = engine.call("folder.revertDelete", json!({ "kind": "request", "id": outer })).await;
    assert_eq!(reverted, json!({ "ok": true }));
    let cfg = crate::workspace::config::load_config();
    assert_eq!(listed("requestFolders"), [("Outer".to_string(), false)]);
    assert_eq!(cfg["requestFolders"][0]["id"], outer.as_str());
    assert_eq!(cfg["requests"][0]["folderId"], outer.as_str());
    assert_eq!(listed("requests"), [("Synced".to_string(), false)]);
    assert!(get_pending_deletions(ws, &pending_folders_key("requests")).is_empty());
    assert!(!cfg["requestFolders"].as_array().unwrap().iter().any(|f| f["id"] == child.as_str()));

    // Confirm: one commit, and neither the folder nor the request is left.
    engine.call("folder.delete", json!({ "kind": "request", "id": outer })).await;
    let confirmed = engine.call("folder.confirmDelete", json!({ "kind": "request", "id": outer })).await;
    assert_eq!(confirmed["ok"], true);
    assert_eq!(run(&ws_dir(ws), &["log", "-1", "--format=%s"]).await.trim(), "delete folder Outer");
    assert!(listed("requests").is_empty() && listed("requestFolders").is_empty());
    assert!(get_pending_deletions(ws, "requests").is_empty());
    assert!(get_pending_deletions(ws, &pending_folders_key("requests")).is_empty());
    assert!(!get_workspace_sync_status(ws).await.contains_key(&format!("requests/Outer/{synced}.json")));
}

#[tokio::test]
async fn a_synced_file_that_went_missing_without_a_record_is_listed_as_deleted() {
    if !check_git_installed().await {
        return;
    }
    let root = scratch_data_root();
    let ws = root.ws_id;
    let engine = Engine::full();
    init_ws(ws).await;
    let ([outer, _], [synced, _, _]) = folder_with_synced_and_local_requests(&engine, ws).await;

    // What a hand, or an engine that kept no record, left behind: the directory is gone, and nothing says why.
    std::fs::remove_dir_all(ws_dir(ws).join("requests/Outer")).unwrap();
    let _ = outer;

    let status = engine.call("sync.getEntityStatus", json!({ "workspaceId": ws })).await;
    assert_eq!(status["files"][format!("requests/Outer/{synced}.json")], "deleted");
    assert!(listed("requests").contains(&("Synced".to_string(), true)));
    assert!(listed("requestFolders").contains(&("Outer".to_string(), true)));

    let reverted = engine.call("entity.revertDelete", json!({ "kind": "requests", "id": synced })).await;
    assert_eq!(reverted, json!({ "ok": true }));
    assert!(listed("requests").contains(&("Synced".to_string(), false)));
    assert!(listed("requestFolders").contains(&("Outer".to_string(), false)));
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
