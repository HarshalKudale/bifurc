//! The environment export: Bifurc JSON.

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
        "schema": "lp-environments-v1",
        "name": workspace_name(&cfg, ws_id, "Bifurc"),
        "environments": in_workspace(&cfg, "environments", ws_id),
    });
    Ok(ExportFile { content: pretty(&payload), suggested_name: "environments-export.json" })
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::{testing::scratch_data_root, workspace::fs::write_flat_entity};

    #[test]
    fn json_export_lists_the_environments() {
        let root = scratch_data_root();
        let ws = root.ws_id;
        let env = json!({ "id": "e1", "name": "Dev", "variables": [{ "id": "v1", "key": "HOST", "value": "a b" }],
            "createdAt": 1, "workspaceId": ws });
        write_flat_entity(ws, "environments", "e1", &env).unwrap();
        let out: Value = serde_json::from_str(&json(ws).unwrap().content).unwrap();
        assert_eq!(out["schema"], "lp-environments-v1");
        let dev = out["environments"].as_array().unwrap().iter().find(|e| e["name"] == "Dev").unwrap();
        assert_eq!(dev["variables"][0]["value"], "a b");
        assert!(out["environments"].as_array().unwrap().iter().any(|e| e["id"] == "__global__"));
    }
}
