//! Mock exports: Bifurc JSON, a Postman collection and WireMock stubs.

use serde_json::{Map, Value, json};

use crate::{
    importexport::{
        formats::postman::export_mocks_to_postman,
        support::{in_workspace, pretty, put_opt, workspace_name},
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

pub fn postman(ws_id: &str) -> ExportOutcome {
    let cfg = load_config();
    let content = export_mocks_to_postman(
        &in_workspace(&cfg, "mocks", ws_id),
        &in_workspace(&cfg, "mockFolders", ws_id),
        &workspace_name(&cfg, ws_id, "Bifurc Mocks"),
    );
    Ok(ExportFile { content, suggested_name: "mocks-export.json" })
}

fn mock_to_wiremock(mock: &Value) -> Value {
    let pattern = mock.get("urlPattern").cloned().unwrap_or(Value::Null);
    let method = match mock.get("method") {
        Some(m) if m == "*" => json!("ANY"),
        other => other.cloned().unwrap_or(Value::Null),
    };
    let mut request = Map::new();
    request.insert("method".into(), method);
    let use_regex = mock.get("useRegex").is_some_and(crate::utils::json::is_truthy);
    request.insert(if use_regex { "urlPattern" } else { "url" }.into(), pattern);

    let mut response = Map::new();
    put_opt(&mut response, "status", mock.get("responseStatus"));
    put_opt(&mut response, "headers", mock.get("responseHeaders"));
    put_opt(&mut response, "body", mock.get("responseBody"));
    if let Some(delay) = mock.get("responseDelay").filter(|d| d.as_f64().is_some_and(|n| n > 0.0)) {
        response.insert("fixedDelayMilliseconds".into(), delay.clone());
    }

    let mut stub = Map::new();
    put_opt(&mut stub, "id", mock.get("id"));
    put_opt(&mut stub, "name", mock.get("name"));
    stub.insert("request".into(), Value::Object(request));
    stub.insert("response".into(), Value::Object(response));
    Value::Object(stub)
}

pub fn wiremock(ws_id: &str) -> ExportOutcome {
    let cfg = load_config();
    let mappings: Vec<Value> = in_workspace(&cfg, "mocks", ws_id).iter().map(mock_to_wiremock).collect();
    Ok(ExportFile { content: pretty(&json!({ "mappings": mappings })), suggested_name: "mocks-export.json" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        testing::scratch_data_root,
        workspace::fs::{sync_enabled_set, write_entity},
    };

    fn seed(ws: &str) {
        let exact = json!({ "id": "m1", "name": "Exact", "method": "*", "urlPattern": "/a", "useRegex": false,
            "responseStatus": 200, "responseHeaders": { "X": "1" }, "responseBody": "ok", "responseDelay": 50,
            "createdAt": 1, "workspaceId": ws });
        let regex = json!({ "id": "m2", "name": "Rx", "method": "GET", "urlPattern": "/b/.*", "useRegex": true,
            "responseStatus": 404, "responseHeaders": {}, "responseBody": "", "createdAt": 2, "workspaceId": ws });
        for m in [&exact, &regex] {
            write_entity(ws, "mocks", m["id"].as_str().unwrap(), m, None).unwrap();
            sync_enabled_set(ws, "mocks", m["id"].as_str().unwrap(), true).unwrap();
        }
    }

    #[test]
    fn json_export_lists_mocks_and_folders() {
        let root = scratch_data_root();
        seed(root.ws_id);
        let out: Value = serde_json::from_str(&json(root.ws_id).unwrap().content).unwrap();
        assert_eq!(out["schema"], "lp-mocks-v1");
        assert_eq!(out["name"], "Workspace 1");
        assert_eq!(out["mocks"].as_array().unwrap().len(), 2);
        assert_eq!(out["folders"], json!([]));
        let keys: Vec<_> = out.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["schema", "name", "mocks", "folders"]);
    }

    #[test]
    fn wiremock_export_picks_the_url_matcher_and_delay() {
        let root = scratch_data_root();
        seed(root.ws_id);
        let out: Value = serde_json::from_str(&wiremock(root.ws_id).unwrap().content).unwrap();
        let stubs = out["mappings"].as_array().unwrap();
        let exact = stubs.iter().find(|s| s["id"] == "m1").unwrap();
        assert_eq!(exact["request"], json!({ "method": "ANY", "url": "/a" }),);
        assert_eq!(exact["response"]["fixedDelayMilliseconds"], 50);
        let regex = stubs.iter().find(|s| s["id"] == "m2").unwrap();
        assert_eq!(regex["request"], json!({ "method": "GET", "urlPattern": "/b/.*" }));
        assert!(regex["response"].get("fixedDelayMilliseconds").is_none());
    }

    #[test]
    fn postman_export_keeps_the_bifurc_state() {
        let root = scratch_data_root();
        seed(root.ws_id);
        let col: Value = serde_json::from_str(&postman(root.ws_id).unwrap().content).unwrap();
        assert_eq!(col["info"]["name"], "Workspace 1");
        let rx = col["item"].as_array().unwrap().iter().find(|i| i["name"] == "Rx").unwrap();
        assert_eq!(rx["_bifurc"], json!({ "urlPattern": "/b/.*", "useRegex": true, "enabled": true }));
    }
}
