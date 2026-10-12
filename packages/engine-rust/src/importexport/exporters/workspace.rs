//! The whole-workspace export: a JSON snapshot.

use serde_json::{Value, json};

use super::requests::workspace_requests;
use crate::{
    importexport::{
        support::{in_workspace, pretty},
        types::{ExportFile, ExportOutcome},
    },
    utils::json::{array_of, merged, str_of},
    workspace::{
        config::load_config,
        fs::{load_enabled_set, read_all_entities},
    },
};

fn owned_by(entities: Vec<Value>, ws_id: &str) -> Vec<Value> {
    entities.into_iter().filter(|e| str_of(e, "workspaceId") == ws_id).collect()
}

/// The `lp-workspace-v1` snapshot of workspace `ws_id`.
pub fn json(ws_id: &str) -> ExportOutcome {
    let cfg = load_config();
    let Some(workspace) = array_of(&cfg, "workspaces").iter().find(|w| str_of(w, "id") == ws_id) else {
        return Err("Workspace not found".to_string());
    };

    let enabled_rules = load_enabled_set(ws_id, "rules");
    let proxy_rules: Vec<Value> = owned_by(read_all_entities(ws_id, "rules"), ws_id)
        .iter()
        .map(|r| merged(r, &json!({ "enabled": enabled_rules.contains(str_of(r, "id")) })))
        .collect();

    let snapshot = json!({
        "schema": "lp-workspace-v1",
        "workspace": workspace,
        "data": {
            "mappings": in_workspace(&cfg, "mappings", ws_id),
            "proxyRules": proxy_rules,
            "mocks": in_workspace(&cfg, "mocks", ws_id),
            // Every request, whatever its type: each names it in `type`.
            "requests": workspace_requests(ws_id),
            "mockFolders": in_workspace(&cfg, "mockFolders", ws_id),
            "ruleFolders": in_workspace(&cfg, "ruleFolders", ws_id),
            "requestFolders": in_workspace(&cfg, "requestFolders", ws_id),
            "flows": owned_by(read_all_entities(ws_id, "flows"), ws_id),
            "flowFolders": in_workspace(&cfg, "flowFolders", ws_id),
            "protoFiles": read_all_entities(ws_id, "protoFiles"),
            "environments": in_workspace(&cfg, "environments", ws_id),
        },
    });
    Ok(ExportFile { content: pretty(&snapshot), suggested_name: "workspace-export.json" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        testing::scratch_data_root,
        workspace::fs::{sync_enabled_set, write_entity},
    };

    #[test]
    fn the_snapshot_holds_the_workspace_in_the_documented_order() {
        let root = scratch_data_root();
        let ws = root.ws_id;
        let request = json!({ "id": "r1", "name": "Ping", "method": "GET", "url": "http://a/", "headers": {}, "body": "",
            "createdAt": 1, "folderId": null, "workspaceId": ws });
        write_entity(ws, "requests", "r1", &request, None).unwrap();
        let query = json!({ "id": "q1", "name": "Me", "endpointUrl": "http://a/graphql", "query": "{ me }", "workspaceId": ws });
        write_entity(ws, "graphqlRequests", "q1", &query, None).unwrap();
        let rule = json!({ "id": "p1", "name": "Rule", "pattern": "x", "workspaceId": ws });
        write_entity(ws, "rules", "p1", &rule, None).unwrap();
        sync_enabled_set(ws, "rules", "p1", true).unwrap();

        let out = json(ws).unwrap();
        assert_eq!(out.suggested_name, "workspace-export.json");
        let snapshot: Value = serde_json::from_str(&out.content).unwrap();
        assert_eq!(snapshot["schema"], "lp-workspace-v1");
        assert_eq!(snapshot["workspace"]["id"], ws);
        let keys: Vec<_> = snapshot["data"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "mappings",
                "proxyRules",
                "mocks",
                "requests",
                "mockFolders",
                "ruleFolders",
                "requestFolders",
                "flows",
                "flowFolders",
                "protoFiles",
                "environments"
            ]
        );
        assert_eq!(snapshot["data"]["requests"][0]["id"], "r1");
        assert_eq!(snapshot["data"]["requests"][1]["type"], "graphql");
        assert_eq!(snapshot["data"]["proxyRules"][0]["enabled"], true);
        assert_eq!(snapshot["data"]["environments"][0]["id"], "__global__");
        assert_eq!(json("nope").err().unwrap(), "Workspace not found");
    }
}
