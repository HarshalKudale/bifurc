//! Bifurc JSON imports of mappings and proxy rules.

use super::lists::{ListSpec, preflight_list, run_list};
use crate::importexport::types::{ImportOutcome, ImportSource, PreflightOutcome, Strategy};

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

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::{
        testing::scratch_data_root,
        workspace::fs::{load_enabled_set, read_all_entities, read_entity},
    };

    fn source(content: &Value) -> ImportSource {
        ImportSource::text(content.to_string(), "x.json")
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
}
