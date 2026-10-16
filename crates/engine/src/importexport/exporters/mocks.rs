//! The mock export: Bifurc JSON.

use serde_json::json;

use crate::{
    importexport::{
        support::{in_workspace, pretty, workspace_name},
        types::{ExportFile, ExportOutcome},
    },
    workspace::config::load_config,
};

pub fn json(ws_id: &str) -> ExportOutcome {
    let cfg = load_config();
    let payload = json!({
        "schema": "lp-mocks-v1",
        "name": workspace_name(&cfg, ws_id, "Bifurc"),
        "mocks": in_workspace(&cfg, "mocks", ws_id),
        "folders": in_workspace(&cfg, "mockFolders", ws_id),
    });
    Ok(ExportFile { content: pretty(&payload), suggested_name: "mocks-export.json" })
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::{
        testing::scratch_data_root,
        workspace::fs::{sync_enabled_set, write_entity},
    };

    #[test]
    fn json_export_lists_mocks_and_folders() {
        let root = scratch_data_root();
        let ws = root.ws_id;
        let exact = json!({ "id": "m1", "name": "Exact", "method": "*", "urlPattern": "/a", "useRegex": false,
            "responseStatus": 200, "responseHeaders": { "X": "1" }, "responseBody": "ok", "responseDelay": 50,
            "createdAt": 1, "workspaceId": ws });
        let regex = json!({ "id": "m2", "name": "Rx", "method": "GET", "urlPattern": "/b/.*", "useRegex": true,
            "responseStatus": 404, "responseHeaders": {}, "responseBody": "", "createdAt": 2, "workspaceId": ws });
        for m in [&exact, &regex] {
            write_entity(ws, "mocks", m["id"].as_str().unwrap(), m, None).unwrap();
            sync_enabled_set(ws, "mocks", m["id"].as_str().unwrap(), true).unwrap();
        }
        let out: Value = serde_json::from_str(&json(ws).unwrap().content).unwrap();
        assert_eq!(out["schema"], "lp-mocks-v1");
        assert_eq!(out["name"], "Workspace 1");
        assert_eq!(out["mocks"].as_array().unwrap().len(), 2);
        assert_eq!(out["folders"], json!([]));
        let keys: Vec<_> = out.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["schema", "name", "mocks", "folders"]);
    }
}
