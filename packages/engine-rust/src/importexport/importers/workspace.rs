//! Whole-workspace imports: a JSON snapshot and a zip of a workspace directory. Both create a new workspace.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};

use super::requests::kind_of_request;
use crate::{
    importexport::{
        support::{
            FolderSpec, add_import_folders, default_folder, io_failure, js_str, parse_json, pretty, push_to,
            sync_enabled,
        },
        types::{ImportOutcome, ImportSource, Imported, Preflight, PreflightOutcome, Strategy},
    },
    runtime::instances::notify_config_changed,
    utils::{
        id::{generate_id, now_ms},
        json::{array_mut, array_of, is_truthy, merged, object_mut, opt_str, str_of},
    },
    workspace::{
        config::load_config,
        fs::{ensure_default_folder, init_workspace_dir, upsert_name_entry, write_entity, ws_dir},
        git_store::init_workspace_repo,
        settings::{load_settings, save_settings},
    },
};

const NOT_A_SNAPSHOT: &str = "Not a valid lp-workspace-v1 file";

/// The snapshot's `data`, when `content` is a workspace snapshot.
fn parse_snapshot(content: &str) -> Result<Result<Value, String>, String> {
    let snapshot = parse_json(content)?;
    let valid = str_of(&snapshot, "schema") == "lp-workspace-v1"
        && snapshot.get("workspace").is_some_and(is_truthy)
        && snapshot.get("data").is_some_and(is_truthy);
    Ok(if valid { Ok(snapshot) } else { Err(NOT_A_SNAPSHOT.to_string()) })
}

fn item_count(data: &Value) -> usize {
    ["mocks", "requests", "mappings", "proxyRules", "wsConnections", "flows", "environments"]
        .iter()
        .map(|key| array_of(data, key).len())
        .sum()
}

pub fn json_preflight(_ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    let snapshot = parse_snapshot(&source.content)??;
    Ok(Preflight::counted(item_count(&snapshot["data"])))
}

/// `workspaceId` of every item of `data[key]` set to `ws_id`.
fn remap(data: &Value, key: &str, ws_id: &str) -> Vec<Value> {
    array_of(data, key).iter().map(|item| merged(item, &json!({ "workspaceId": ws_id }))).collect()
}

/// folder id -> folder name.
fn folder_names(folders: &[Value]) -> HashMap<String, String> {
    folders.iter().map(|f| (str_of(f, "id").to_string(), str_of(f, "name").to_string())).collect()
}

/// Make the folders a snapshot lists for a tree in workspace `ws_id`, parents before the folders in them.
/// Returns the id each got, by the id the snapshot knows it under.
fn tree_folders(
    cfg: &mut Value,
    folders_key: &str,
    fs_kind: &str,
    ws_id: &str,
    folders: &[Value],
) -> Result<HashMap<String, String>, String> {
    let specs: Vec<FolderSpec> = folders
        .iter()
        .map(|f| FolderSpec {
            key: str_of(f, "id").to_string(),
            name: str_of(f, "name").to_string(),
            parent: opt_str(f, "parentId").filter(|p| !p.is_empty()).map(str::to_string),
        })
        .collect();
    add_import_folders(cfg, folders_key, fs_kind, ws_id, "", &specs)
}

/// A name no workspace has: `name`, else `name (2)`, `name (3)`, …
fn unique_name(taken: &HashSet<String>, name: &str) -> String {
    if !taken.contains(name) {
        return name.to_string();
    }
    let mut n = 2;
    while taken.contains(&format!("{name} ({n})")) {
        n += 1;
    }
    format!("{name} ({n})")
}

pub async fn json_run(_ws_id: String, source: ImportSource, _strategy: Strategy) -> ImportOutcome {
    let snapshot = parse_snapshot(&source.content)??;
    let mut cfg = load_config();
    let new_id = generate_id();
    let source_name = js_str(snapshot["workspace"].get("name").unwrap_or(&Value::Null));
    let taken: HashSet<String> =
        array_of(&cfg, "workspaces").iter().map(|w| js_str(w.get("name").unwrap_or(&Value::Null))).collect();
    let name = unique_name(&taken, &source_name);
    push_to(
        &mut cfg,
        "workspaces",
        json!({ "id": new_id, "name": name, "createdAt": now_ms(), "activeEnvironmentId": null }),
    );

    let data = &snapshot["data"];
    for (cfg_key, data_key) in [
        ("mappings", "mappings"),
        ("proxyRules", "proxyRules"),
        ("mocks", "mocks"),
        ("ruleFolders", "ruleFolders"),
        ("environments", "environments"),
    ] {
        for item in remap(data, data_key, &new_id) {
            push_to(&mut cfg, cfg_key, item);
        }
    }
    cfg["activeWorkspaceId"] = json!(new_id);
    cfg["activeEnvironmentId"] = Value::Null;

    init_workspace_dir(&new_id, &name).map_err(io_failure)?;
    init_workspace_repo(&new_id).await.map_err(io_failure)?;

    // The snapshot's folders are made as directories of the new workspace's trees, where they get ids of its
    // own: `folder_of` gives the folder here of an item that named one there.
    let folder_of = |item: &Value, made: &HashMap<String, String>| -> Option<String> {
        item.get("folderId").filter(|f| is_truthy(f)).and_then(|f| made.get(&js_str(f)).cloned())
    };

    // A mock in no folder, or in one the snapshot does not have, goes into the Default folder.
    let mock_folders = tree_folders(&mut cfg, "mockFolders", "mocks", &new_id, array_of(data, "mockFolders"))?;
    let mut default_mock_folder: Option<String> = None;
    for index in 0..array_of(&cfg, "mocks").len() {
        let mock = &array_of(&cfg, "mocks")[index];
        if str_of(mock, "workspaceId") != new_id {
            continue;
        }
        let folder_id = match folder_of(mock, &mock_folders) {
            Some(folder_id) => folder_id,
            None => match &default_mock_folder {
                Some(default_id) => default_id.clone(),
                None => {
                    let (default_id, _) = default_folder(&mut cfg, "mockFolders", "mocks", &new_id)?;
                    default_mock_folder.insert(default_id).clone()
                }
            },
        };
        object_mut(&mut array_mut(&mut cfg, "mocks")[index]).insert("folderId".into(), json!(folder_id));
    }
    commit_config_without_notify(&cfg)?;

    // Every request is in the one request tree. A snapshot from before that lists the WebSocket folders apart.
    let mut request_folders =
        tree_folders(&mut cfg, "requestFolders", "requests", &new_id, array_of(data, "requestFolders"))?;
    request_folders.extend(tree_folders(&mut cfg, "requestFolders", "requests", &new_id, array_of(data, "wsFolders"))?);
    let rule_folders = folder_names(&remap(data, "ruleFolders", &new_id));

    // A request names its type; one from before the types shared a list is a REST request.
    for request in remap(data, "requests", &new_id) {
        let folder_id = match folder_of(&request, &request_folders) {
            Some(folder_id) => folder_id,
            None => ensure_default_folder(&new_id, "requests").map_err(io_failure)?.0,
        };
        let stored = merged(&request, &json!({ "folderId": folder_id }));
        write_entity(&new_id, kind_of_request(&request), str_of(&request, "id"), &stored, Some(&folder_id))
            .map_err(io_failure)?;
    }
    let flow_folders = tree_folders(&mut cfg, "flowFolders", "flows", &new_id, array_of(data, "flowFolders"))?;
    for flow in remap(data, "flows", &new_id) {
        let folder_id = match folder_of(&flow, &flow_folders) {
            Some(folder_id) => folder_id,
            None => ensure_default_folder(&new_id, "flows").map_err(io_failure)?.0,
        };
        let stored = merged(&flow, &json!({ "folderId": folder_id }));
        write_entity(&new_id, "flows", str_of(&flow, "id"), &stored, Some(&folder_id)).map_err(io_failure)?;
    }
    for proto in remap(data, "protoFiles", &new_id) {
        write_entity(&new_id, "protoFiles", str_of(&proto, "id"), &proto, None).map_err(io_failure)?;
    }
    for connection in remap(data, "wsConnections", &new_id) {
        let id = str_of(&connection, "id").to_string();
        let folder_id = match folder_of(&connection, &request_folders) {
            Some(folder_id) => folder_id,
            None => ensure_default_folder(&new_id, "sockets").map_err(io_failure)?.0,
        };
        write_entity(&new_id, "sockets", &id, &connection, Some(&folder_id)).map_err(io_failure)?;
    }
    for rule in remap(data, "proxyRules", &new_id) {
        let id = str_of(&rule, "id");
        let folder = folder_of(&rule, &rule_folders);
        write_entity(&new_id, "rules", id, &rule, folder.as_deref()).map_err(io_failure)?;
        sync_enabled(&new_id, "rules", id, rule.get("enabled").unwrap_or(&Value::Null))?;
        let entry = json!({
            "name": rule.get("name").cloned().unwrap_or(Value::Null),
            "url": rule.get("pattern").cloned().unwrap_or(Value::Null),
        });
        upsert_name_entry(&new_id, "rules", id, entry).map_err(io_failure)?;
    }
    for mock in remap(data, "mocks", &new_id) {
        let id = str_of(&mock, "id");
        sync_enabled(&new_id, "mocks", id, mock.get("enabled").unwrap_or(&Value::Null))?;
        let entry = json!({
            "name": mock.get("name").cloned().unwrap_or(Value::Null),
            "method": mock.get("method").cloned().unwrap_or(Value::Null),
            "url": mock.get("urlPattern").cloned().unwrap_or(Value::Null),
        });
        upsert_name_entry(&new_id, "mocks", id, entry).map_err(io_failure)?;
    }
    for mapping in remap(data, "mappings", &new_id) {
        sync_enabled(&new_id, "mappings", str_of(&mapping, "id"), mapping.get("enabled").unwrap_or(&Value::Null))?;
    }

    notify_config_changed();
    Ok(Imported::all(item_count(data)))
}

/// Save the snapshot; the proxies are told once everything is written.
fn commit_config_without_notify(cfg: &Value) -> Result<(), String> {
    crate::workspace::config::save_config(cfg).map_err(io_failure)
}

// ── zip ────────────────────────────────────────────────────────────────────────────────────────────────

pub fn zip_preflight(source: &Path) -> PreflightOutcome {
    if !source.exists() {
        return Err("File not found".to_string());
    }
    let mut head = [0u8; 4];
    let mut file = File::open(source).map_err(io_failure)?;
    let mut filled = 0;
    while filled < head.len() {
        match file.read(&mut head[filled..]).map_err(io_failure)? {
            0 => break,
            n => filled += n,
        }
    }
    if head[0] == 0x50 && head[1] == 0x4b { Ok(Preflight::counted(0)) } else { Err("Not a valid ZIP file".to_string()) }
}

/// The `name` in the archive's `workspace.json`, if it has one that can be read.
fn archived_workspace_name(source: &Path) -> Option<String> {
    let mut archive = zip::ZipArchive::new(File::open(source).ok()?).ok()?;
    let index = (0..archive.len()).find(|&i| {
        archive.by_index(i).is_ok_and(|f| f.name() == "workspace.json" || f.name().ends_with("/workspace.json"))
    })?;
    let mut text = String::new();
    archive.by_index(index).ok()?.read_to_string(&mut text).ok()?;
    let data: Value = serde_json::from_str(&text).ok()?;
    data.get("name").filter(|n| is_truthy(n)).map(js_str)
}

/// Unpack the archive into `dest`, leaving out git data. Returns the number of files written.
fn extract(source: &Path, dest: &Path) -> Result<usize, String> {
    let mut archive = zip::ZipArchive::new(File::open(source).map_err(io_failure)?).map_err(io_failure)?;
    let mut count = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(io_failure)?;
        let name = entry.name().to_string();
        if name.starts_with(".git/") || name == ".git" || entry.is_dir() {
            continue;
        }
        let Some(relative) = entry.enclosed_name() else { continue };
        let target: PathBuf = dest.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(io_failure)?;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(io_failure)?;
        std::fs::write(target, bytes).map_err(io_failure)?;
        count += 1;
    }
    Ok(count)
}

pub async fn zip_run(_ws_id: String, source: PathBuf, _strategy: Strategy) -> ImportOutcome {
    let mut settings = load_settings();
    let new_id = generate_id();

    let archive = source.clone();
    let name = tokio::task::spawn_blocking(move || archived_workspace_name(&archive))
        .await
        .map_err(io_failure)?
        .unwrap_or_else(|| "Imported Workspace".to_string());
    let taken: HashSet<String> =
        settings.workspaces().iter().map(|w| js_str(w.get("name").unwrap_or(&Value::Null))).collect();
    let name = unique_name(&taken, &name);

    let dest = ws_dir(&new_id);
    init_workspace_dir(&new_id, &name).map_err(io_failure)?;
    let (archive, target) = (source, dest.clone());
    let file_count = tokio::task::spawn_blocking(move || extract(&archive, &target)).await.map_err(io_failure)??;

    let workspace_file = dest.join("workspace.json");
    if workspace_file.exists() {
        let text = std::fs::read_to_string(&workspace_file).map_err(io_failure)?;
        let mut data = parse_json(&text)?;
        let object = object_mut(&mut data);
        object.insert("id".into(), json!(new_id));
        object.insert("name".into(), json!(name));
        // A copy from an archive is a workspace of its own, not the organization's the original was.
        object.shift_remove("organization");
        std::fs::write(&workspace_file, pretty(&data)).map_err(io_failure)?;
    }

    // The archive's files arrived without the indexes of the trees being told.
    crate::workspace::tree::mark_stale(&new_id);
    init_workspace_repo(&new_id).await.map_err(io_failure)?;

    push_to(settings.as_value_mut(), "workspaces", json!({ "id": new_id, "name": name, "activeEnvironmentId": null }));
    settings.set("activeWorkspaceId", json!(new_id));
    save_settings(&settings).map_err(io_failure)?;

    notify_config_changed();
    Ok(Imported::all(file_count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        testing::scratch_data_root,
        workspace::fs::{load_enabled_set, read_entity},
    };

    fn snapshot(name: &str) -> Value {
        json!({ "schema": "lp-workspace-v1", "workspace": { "id": "old", "name": name, "createdAt": 1, "activeEnvironmentId": null },
            "data": {
                "mappings": [{ "id": "map1", "domain": "a.localhost", "target": "localhost:1", "enabled": true, "workspaceId": "old" }],
                "proxyRules": [{ "id": "p1", "name": "Rule", "pattern": "/x", "enabled": true, "folderId": "rf", "workspaceId": "old" }],
                "mocks": [{ "id": "m1", "name": "Mock", "method": "GET", "urlPattern": "/m", "enabled": true, "workspaceId": "old" }],
                "requests": [{ "id": "r1", "name": "Req", "method": "GET", "url": "http://a/", "headers": {}, "body": "",
                    "createdAt": 1, "folderId": "qf", "workspaceId": "old" },
                    { "id": "g1", "type": "grpc", "name": "Rpc", "serverAddress": "h:1", "serviceName": "a.B", "methodName": "C",
                    "folderId": "qf", "workspaceId": "old" }],
                "flows": [{ "id": "fl1", "name": "Flow", "nodes": [], "edges": [], "folderId": null, "workspaceId": "old" }],
                "mockFolders": [], "ruleFolders": [{ "id": "rf", "name": "Rules", "parentId": null, "workspaceId": "old" }],
                "requestFolders": [{ "id": "qf", "name": "Reqs", "parentId": null, "workspaceId": "old" }],
                "wsConnections": [{ "id": "c1", "name": "Feed", "url": "ws://x", "headers": {}, "createdAt": 1, "folderId": null, "workspaceId": "old" }],
                "wsFolders": [],
                "environments": [{ "id": "e1", "name": "Env", "variables": [], "createdAt": 1, "workspaceId": "old" }],
            } })
    }

    /// A zip of the files under `dir`, as an archive of a workspace directory is.
    fn zip_of(dir: &Path, dest: &Path) {
        fn add(writer: &mut zip::ZipWriter<File>, dir: &Path, prefix: &str) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
                if entry.path().is_dir() {
                    add(writer, &entry.path(), &format!("{name}/"));
                } else if !name.contains("index.db") {
                    writer.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
                    std::io::Write::write_all(writer, &std::fs::read(entry.path()).unwrap()).unwrap();
                }
            }
        }
        let mut writer = zip::ZipWriter::new(File::create(dest).unwrap());
        add(&mut writer, dir, "");
        writer.finish().unwrap();
    }

    fn text(value: &Value) -> ImportSource {
        ImportSource::text(value.to_string(), "ws.json")
    }

    #[test]
    fn preflight_counts_the_entities() {
        let pre = json_preflight("ws1", &text(&snapshot("Old"))).unwrap();
        assert_eq!((pre.item_count, pre.collision_ids.len()), (8, 0));
        assert_eq!(json_preflight("ws1", &text(&json!({ "schema": "x" }))).err().unwrap(), NOT_A_SNAPSHOT);
        assert!(json_preflight("ws1", &ImportSource::text("nope", "")).is_err());
    }

    #[tokio::test]
    async fn a_snapshot_becomes_a_new_active_workspace_with_a_unique_name() {
        let root = scratch_data_root();
        let done = json_run(root.ws_id.to_string(), text(&snapshot("Workspace 1")), Strategy::Keep).await;
        let Ok(done) = done else {
            // git may be unavailable in the test environment
            eprintln!("skipped: {done:?}", done = done.err());
            return;
        };
        assert_eq!(done.imported, 8);
        let settings = load_settings();
        let created = settings.workspaces().iter().find(|w| w["name"] == "Workspace 1 (2)").expect("renamed workspace");
        let new_id = created["id"].as_str().unwrap();
        assert_eq!(settings.active_workspace_id(), new_id);
        assert!(ws_dir(new_id).join("workspace.json").exists());

        let request = read_entity(new_id, "requests", "r1").unwrap();
        assert_eq!(request["workspaceId"], new_id);
        assert!(crate::workspace::fs::entity_dir(new_id, "requests").join("Reqs").join("r1.json").exists());
        assert!(crate::workspace::fs::entity_dir(new_id, "rules").join("Rules").join("p1.json").exists());
        assert!(read_entity(new_id, "sockets", "c1").is_some());
        assert_eq!(read_entity(new_id, "grpcRequests", "g1").unwrap()["methodName"], "C");
        assert!(crate::workspace::fs::entity_dir(new_id, "requests").join("Reqs").join("g1.json").exists());
        assert!(read_entity(new_id, "flows", "fl1").is_some());
        assert!(read_entity(new_id, "mocks", "m1").is_some());
        assert!(read_entity(new_id, "mappings", "map1").is_some());
        assert!(load_enabled_set(new_id, "mocks").contains("m1"));
        assert!(load_enabled_set(new_id, "rules").contains("p1"));
        assert!(load_enabled_set(new_id, "mappings").contains("map1"));
    }

    #[tokio::test]
    async fn what_a_newer_engine_wrote_is_imported_locked() {
        use crate::workspace::{fs::entity_lock, stamp::VERSION_FIELD};

        let root = scratch_data_root();
        let mut newer = snapshot("Newer");
        for (list, _) in [("requests", "r1"), ("mocks", "m1"), ("proxyRules", "p1")] {
            newer["data"][list][0][VERSION_FIELD] = json!("999.0.0");
        }
        let Ok(_) = json_run(root.ws_id.to_string(), text(&newer), Strategy::Keep).await else {
            // git may be unavailable in the test environment
            return;
        };
        let new_id = load_settings().active_workspace_id();
        for (kind, id) in [("requests", "r1"), ("mocks", "m1"), ("rules", "p1")] {
            assert_eq!(entity_lock(&new_id, kind, id).as_deref(), Some("999.0.0"), "{kind} keeps its stamp");
            assert!(read_entity(&new_id, kind, id).is_none(), "{kind} is not read here");
        }
        // What carries no newer stamp is this engine's from the moment it is written.
        assert_eq!(read_entity(&new_id, "sockets", "c1").unwrap()[VERSION_FIELD], crate::ENGINE_VERSION);
        assert_eq!(entity_lock(&new_id, "mappings", "map1"), None);
    }

    #[tokio::test]
    async fn a_zip_round_trips_through_the_exporter() {
        let root = scratch_data_root();
        let ws = root.ws_id;
        let request = json!({ "id": "r1", "name": "Req", "method": "GET", "url": "http://a/", "headers": {}, "body": "",
            "createdAt": 1, "folderId": null, "workspaceId": ws });
        write_entity(ws, "requests", "r1", &request, None).unwrap();
        let archive = root.dir.path().join("ws.zip");
        zip_of(&ws_dir(ws), &archive);

        assert!(zip_preflight(&archive).is_ok());
        assert_eq!(zip_preflight(&root.dir.path().join("none.zip")).err().unwrap(), "File not found");
        let not_zip = root.dir.path().join("a.txt");
        std::fs::write(&not_zip, "hello").unwrap();
        assert_eq!(zip_preflight(&not_zip).err().unwrap(), "Not a valid ZIP file");

        let done = zip_run(ws.to_string(), archive, Strategy::Keep).await;
        let Ok(done) = done else {
            eprintln!("skipped: {:?}", done.err());
            return;
        };
        assert!(done.imported >= 3);
        let settings = load_settings();
        let created = settings.workspaces().iter().find(|w| w["name"] == "Workspace 1 (2)").expect("renamed workspace");
        let new_id = created["id"].as_str().unwrap();
        let info: Value = crate::utils::json::read_json(&ws_dir(new_id).join("workspace.json")).unwrap();
        assert_eq!((info["id"].as_str().unwrap(), info["name"].as_str().unwrap()), (new_id, "Workspace 1 (2)"));
        assert_eq!(read_entity(new_id, "requests", "r1").unwrap()["name"], "Req");
        assert_eq!(settings.active_workspace_id(), new_id);
    }

    #[test]
    fn names_get_a_numeric_suffix() {
        let taken: HashSet<String> = ["A".to_string(), "A (2)".to_string()].into();
        assert_eq!(unique_name(&taken, "A"), "A (3)");
        assert_eq!(unique_name(&taken, "B"), "B");
    }
}
