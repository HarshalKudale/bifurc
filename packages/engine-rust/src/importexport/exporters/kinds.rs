//! Bifurc JSON exports of mappings, proxy rules and WebSocket connections.

use serde_json::{Value, json};

use crate::{
    importexport::{
        support::{in_workspace, pretty, workspace_name},
        types::{ExportFile, ExportOutcome},
    },
    utils::json::{merged, str_of},
    workspace::{
        config::load_config,
        fs::{load_enabled_set, read_all_entities},
    },
};

pub fn mappings(ws_id: &str) -> ExportOutcome {
    let cfg = load_config();
    let payload = json!({
        "schema": "lp-mappings-v1",
        "name": workspace_name(&cfg, ws_id, "Bifurc"),
        "mappings": in_workspace(&cfg, "mappings", ws_id),
    });
    Ok(ExportFile { content: pretty(&payload), suggested_name: "mappings-export.json" })
}

pub fn proxy_rules(ws_id: &str) -> ExportOutcome {
    let cfg = load_config();
    let enabled = load_enabled_set(ws_id, "rules");
    let rules: Vec<Value> = read_all_entities(ws_id, "rules")
        .iter()
        .filter(|r| str_of(r, "workspaceId") == ws_id)
        .map(|r| merged(r, &json!({ "enabled": enabled.contains(str_of(r, "id")) })))
        .collect();
    let payload =
        json!({ "schema": "lp-proxy-rules-v1", "name": workspace_name(&cfg, ws_id, "Bifurc"), "proxyRules": rules });
    Ok(ExportFile { content: pretty(&payload), suggested_name: "proxy-rules-export.json" })
}

pub fn websockets(ws_id: &str) -> ExportOutcome {
    let cfg = load_config();
    let connections: Vec<Value> =
        read_all_entities(ws_id, "sockets").into_iter().filter(|c| str_of(c, "workspaceId") == ws_id).collect();
    let payload = json!({
        "schema": "lp-websockets-v1",
        "name": workspace_name(&cfg, ws_id, "Bifurc"),
        "wsConnections": connections,
        "folders": in_workspace(&cfg, "requestFolders", ws_id),
    });
    Ok(ExportFile { content: pretty(&payload), suggested_name: "websockets-export.json" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        testing::scratch_data_root,
        workspace::fs::{sync_enabled_set, write_entity, write_flat_entity},
    };

    #[test]
    fn mappings_export_carries_the_schema_and_workspace_name() {
        let root = scratch_data_root();
        let mapping =
            json!({ "id": "map1", "domain": "a.localhost", "target": "localhost:3000", "workspaceId": root.ws_id });
        write_flat_entity(root.ws_id, "mappings", "map1", &mapping).unwrap();
        sync_enabled_set(root.ws_id, "mappings", "map1", true).unwrap();
        let out: Value = serde_json::from_str(&mappings(root.ws_id).unwrap().content).unwrap();
        assert_eq!(out["schema"], "lp-mappings-v1");
        assert_eq!(out["name"], "Workspace 1");
        assert_eq!(out["mappings"][0]["enabled"], true);
    }

    #[test]
    fn proxy_rules_export_takes_enabled_from_the_enabled_set() {
        let root = scratch_data_root();
        let rule = json!({ "id": "p1", "name": "R", "pattern": "x", "enabled": false, "workspaceId": root.ws_id });
        write_entity(root.ws_id, "rules", "p1", &rule, None).unwrap();
        sync_enabled_set(root.ws_id, "rules", "p1", true).unwrap();
        let out: Value = serde_json::from_str(&proxy_rules(root.ws_id).unwrap().content).unwrap();
        assert_eq!(out["proxyRules"][0]["enabled"], true);
        let keys: Vec<_> = out["proxyRules"][0].as_object().unwrap().keys().collect();
        // The stamp goes with the rule: an older Bifurc that imports it keeps it locked.
        assert_eq!(keys, ["id", "name", "pattern", "workspaceId", "engineVersion", "enabled"]);
    }

    #[test]
    fn websockets_export_lists_connections_and_folders() {
        let root = scratch_data_root();
        let conn = json!({ "id": "c1", "name": "Feed", "url": "ws://x", "headers": {}, "createdAt": 1, "folderId": null, "workspaceId": root.ws_id });
        write_entity(root.ws_id, "sockets", "c1", &conn, None).unwrap();
        let out: Value = serde_json::from_str(&websockets(root.ws_id).unwrap().content).unwrap();
        assert_eq!(out["schema"], "lp-websockets-v1");
        assert_eq!(out["wsConnections"][0]["name"], "Feed");
        assert_eq!(out["folders"], json!([]));
    }
}
