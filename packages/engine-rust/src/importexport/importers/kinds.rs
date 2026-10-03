//! Bifurc JSON imports of mappings, proxy rules and WebSocket connections.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::lists::{ListSpec, preflight_list, run_list};
use crate::{
    importexport::{
        support::{commit_config, folder_name, io_failure, parse_json, push_to, thrown},
        types::{ImportOutcome, ImportSource, Imported, Preflight, PreflightOutcome, Strategy},
    },
    utils::{
        id::generate_id,
        json::{array_of, merged, opt_str, str_of},
    },
    workspace::{
        config::load_config,
        fs::{read_all_entities, upsert_name_entry, write_entity},
    },
};

const MAPPINGS: ListSpec = ListSpec {
    schema: "lp-mappings-v1",
    list_key: "mappings",
    cfg_key: "mappings",
    fs_kind: "mappings",
    tracks_enabled: true,
};

const PROXY_RULES: ListSpec = ListSpec {
    schema: "lp-proxy-rules-v1",
    list_key: "proxyRules",
    cfg_key: "proxyRules",
    fs_kind: "rules",
    tracks_enabled: true,
};

pub fn mappings_preflight(ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    preflight_list(&MAPPINGS, ws_id, source)
}

pub async fn mappings_run(ws_id: String, source: ImportSource, strategy: Strategy) -> ImportOutcome {
    run_list(&MAPPINGS, ws_id, source, strategy)
}

pub fn proxy_rules_preflight(ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    preflight_list(&PROXY_RULES, ws_id, source)
}

pub async fn proxy_rules_run(ws_id: String, source: ImportSource, strategy: Strategy) -> ImportOutcome {
    run_list(&PROXY_RULES, ws_id, source, strategy)
}

fn parse_websockets(content: &str) -> Result<(Vec<Value>, Vec<Value>), String> {
    let data = parse_json(content)?;
    if opt_str(&data, "schema") != Some("lp-websockets-v1") || !data.get("wsConnections").is_some_and(Value::is_array) {
        return Err(thrown("Not a valid lp-websockets-v1 file"));
    }
    Ok((array_of(&data, "wsConnections").to_vec(), array_of(&data, "folders").to_vec()))
}

pub fn websockets_preflight(ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    let (connections, _) = parse_websockets(&source.content)?;
    let existing: std::collections::HashSet<String> =
        read_all_entities(ws_id, "sockets").iter().map(|c| str_of(c, "id").to_string()).collect();
    let collision_ids =
        connections.iter().map(|c| str_of(c, "id")).filter(|id| existing.contains(*id)).map(str::to_string).collect();
    Ok(Preflight { item_count: connections.len(), collision_ids })
}

pub async fn websockets_run(ws_id: String, source: ImportSource, strategy: Strategy) -> ImportOutcome {
    let (connections, folders) = parse_websockets(&source.content)?;
    let mut cfg = load_config();
    let existing: std::collections::HashSet<String> =
        read_all_entities(&ws_id, "sockets").iter().map(|c| str_of(c, "id").to_string()).collect();

    let mut folder_ids: HashMap<String, String> = HashMap::new();
    for folder in &folders {
        let new_id = if strategy == Strategy::New { generate_id() } else { str_of(folder, "id").to_string() };
        folder_ids.insert(str_of(folder, "id").to_string(), new_id.clone());
        if !array_of(&cfg, "wsFolders").iter().any(|f| str_of(f, "id") == new_id) {
            push_to(&mut cfg, "wsFolders", merged(folder, &json!({ "id": new_id, "workspaceId": ws_id })));
        }
    }

    let (mut imported, mut skipped) = (0, 0);
    for connection in &connections {
        if existing.contains(str_of(connection, "id")) && strategy == Strategy::Keep {
            skipped += 1;
            continue;
        }
        let new_id = if strategy == Strategy::New { generate_id() } else { str_of(connection, "id").to_string() };
        let folder_id =
            opt_str(connection, "folderId").filter(|f| !f.is_empty()).and_then(|f| folder_ids.get(f)).cloned();
        let folder = folder_name(&cfg, "wsFolders", folder_id.as_deref());
        let stored = merged(connection, &json!({ "id": new_id, "folderId": folder_id, "workspaceId": ws_id }));
        write_entity(&ws_id, "sockets", &new_id, &stored, folder.as_deref()).map_err(io_failure)?;
        let entry = json!({
            "name": stored.get("name").cloned().unwrap_or(Value::Null),
            "url": stored.get("url").cloned().unwrap_or(Value::Null),
        });
        upsert_name_entry(&ws_id, "sockets", &new_id, entry).map_err(io_failure)?;
        imported += 1;
    }

    commit_config(&cfg)?;
    Ok(Imported::with_skipped(imported, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        testing::scratch_data_root,
        workspace::fs::{entity_dir, load_enabled_set, read_entity},
    };

    fn source(content: &Value) -> ImportSource {
        ImportSource { content: content.to_string(), filename: "x.json".to_string() }
    }

    #[tokio::test]
    async fn mappings_import_sets_enabled_state_and_workspace() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let file = json!({ "schema": "lp-mappings-v1", "mappings": [
            { "id": "a", "domain": "a.localhost", "target": "localhost:1", "enabled": true, "workspaceId": "old" },
            { "id": "b", "domain": "b.localhost", "target": "localhost:2", "enabled": false, "workspaceId": "old" },
        ] });
        let done = mappings_run(ws.clone(), source(&file), Strategy::Keep).await.unwrap();
        assert_eq!((done.imported, done.skipped), (2, Some(0)));
        assert_eq!(read_entity(&ws, "mappings", "a").unwrap()["workspaceId"], ws.as_str());
        let enabled = load_enabled_set(&ws, "mappings");
        assert!(enabled.contains("a") && !enabled.contains("b"));
        assert_eq!(mappings_preflight(&ws, &source(&file)).unwrap().collision_ids, ["a", "b"]);
        let skipped = mappings_run(ws.clone(), source(&file), Strategy::Keep).await.unwrap();
        assert_eq!((skipped.imported, skipped.skipped), (0, Some(2)));
        assert_eq!(
            mappings_preflight(&ws, &source(&json!({ "schema": "lp-mappings-v1" }))).err().unwrap(),
            "Error: Not a valid lp-mappings-v1 file"
        );
    }

    #[tokio::test]
    async fn proxy_rules_import_writes_rules_and_enabled_state() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let file = json!({ "schema": "lp-proxy-rules-v1", "proxyRules": [
            { "id": "p1", "name": "R", "pattern": "x", "enabled": true, "workspaceId": "old" }] });
        let done = proxy_rules_run(ws.clone(), source(&file), Strategy::Override).await.unwrap();
        assert_eq!(done.imported, 1);
        assert_eq!(read_entity(&ws, "rules", "p1").unwrap()["workspaceId"], ws.as_str());
        assert!(load_enabled_set(&ws, "rules").contains("p1"));
        assert_eq!(proxy_rules_preflight(&ws, &source(&file)).unwrap().collision_ids, ["p1"]);
        let renamed = proxy_rules_run(ws.clone(), source(&file), Strategy::New).await.unwrap();
        assert_eq!(renamed.imported, 1);
        assert_eq!(read_all_entities(&ws, "rules").len(), 2);
    }

    #[tokio::test]
    async fn websocket_connections_import_into_their_folders() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let file = json!({ "schema": "lp-websockets-v1", "wsConnections": [
            { "id": "c1", "name": "Feed", "url": "ws://x", "headers": {}, "createdAt": 1, "folderId": "f1", "workspaceId": "old" },
            { "id": "c2", "name": "Loose", "url": "ws://y", "headers": {}, "createdAt": 2, "folderId": null, "workspaceId": "old" },
        ], "folders": [{ "id": "f1", "name": "Live", "parentId": null, "createdAt": 1, "workspaceId": "old" }] });
        let done = websockets_run(ws.clone(), source(&file), Strategy::Keep).await.unwrap();
        assert_eq!((done.imported, done.skipped), (2, Some(0)));
        assert!(entity_dir(&ws, "sockets").join("Live").join("c1.json").exists());
        let stored = read_entity(&ws, "sockets", "c1").unwrap();
        assert_eq!(stored["folderId"], "f1");
        assert_eq!(stored["workspaceId"], ws.as_str());
        assert_eq!(websockets_preflight(&ws, &source(&file)).unwrap().collision_ids, ["c1", "c2"]);
        let again = websockets_run(ws.clone(), source(&file), Strategy::New).await.unwrap();
        assert_eq!(again.imported, 2);
        assert_eq!(read_all_entities(&ws, "sockets").len(), 4);
    }
}
