//! Request exports: Postman, OpenAPI, cURL, HAR and Insomnia.

use serde_json::{Map, Value, json};
use url::Url;

use crate::{
    importexport::{
        formats::postman::export_requests_to_postman,
        support::{in_workspace, iso_from_value, js_str, js_trim, pretty, text_or, workspace_name},
        types::{ExportFile, ExportOutcome},
    },
    utils::{
        id::{generate_id, now_ms},
        json::{opt_str, str_of},
    },
    workspace::{config::load_config, fs::read_all_entities},
};

/// The requests of workspace `ws_id`.
fn workspace_requests(ws_id: &str) -> Vec<Value> {
    read_all_entities(ws_id, "requests").into_iter().filter(|r| str_of(r, "workspaceId") == ws_id).collect()
}

/// The entries of a header record, in order.
fn header_entries(request: &Value) -> Vec<(&String, &Value)> {
    request.get("headers").and_then(Value::as_object).map(|h| h.iter().collect()).unwrap_or_default()
}

/// `headers["Content-Type"] ?? headers["content-type"]`.
fn content_type_header(request: &Value) -> Option<&Value> {
    let headers = request.get("headers")?;
    ["Content-Type", "content-type"].iter().find_map(|key| headers.get(key).filter(|v| !v.is_null()))
}

/// The body when it has anything but whitespace in it.
fn visible_body(request: &Value) -> Option<&str> {
    opt_str(request, "body").filter(|body| !js_trim(body).is_empty())
}

pub fn postman(ws_id: &str) -> ExportOutcome {
    let cfg = load_config();
    let requests = workspace_requests(ws_id);
    let folders = in_workspace(&cfg, "requestFolders", ws_id);
    let content = export_requests_to_postman(&requests, &folders, &workspace_name(&cfg, ws_id, "Bifurc Requests"));
    Ok(ExportFile { content, suggested_name: "requests-export.json" })
}

// ── OpenAPI ────────────────────────────────────────────────────────────────────────────────────────────

fn extract_path_params(url: &str) -> Vec<String> {
    let Ok(parsed) = Url::parse(url) else { return Vec::new() };
    let path = parsed.path();
    let mut params = Vec::new();
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) if close > 0 && !after[..close].contains('{') => {
                params.push(after[..close].to_string());
                rest = &after[close + 1..];
            }
            Some(close) => rest = &after[close..],
            None => break,
        }
    }
    params
}

fn to_open_api_path(url: &str) -> String {
    if let Ok(parsed) = Url::parse(url) {
        return parsed.path().to_string();
    }
    let origin_end = ["https://", "http://"].iter().find(|scheme| url.starts_with(**scheme)).map(|scheme| {
        let host_start = scheme.len();
        url[host_start..].find('/').map(|slash| host_start + slash)
    });
    let rest = match origin_end {
        Some(Some(at)) => &url[at..],
        Some(None) => return url.to_string(),
        None => url,
    };
    if !rest.starts_with('/') {
        return url.to_string();
    }
    rest.split('?').next().unwrap_or(rest).to_string()
}

fn detect_content_type(body: &str) -> &'static str {
    let trimmed = js_trim(body);
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        "application/json"
    } else if trimmed.starts_with('<') {
        "application/xml"
    } else {
        "text/plain"
    }
}

pub fn openapi(ws_id: &str) -> ExportOutcome {
    let cfg = load_config();
    let requests = workspace_requests(ws_id);
    let mut paths: Map<String, Value> = Map::new();

    for request in &requests {
        let op_path = to_open_api_path(&text_or(request, "url", "undefined"));
        let method = text_or(request, "method", "undefined");
        let entry = paths.entry(op_path.clone()).or_insert_with(|| json!({}));

        let mut parameters: Vec<Value> = extract_path_params(&op_path)
            .into_iter()
            .map(|name| json!({ "name": name, "in": "path", "required": true, "schema": { "type": "string" } }))
            .collect();
        parameters.extend(header_entries(request).into_iter().filter(|(k, _)| k.to_lowercase() != "content-type").map(
            |(name, example)| json!({ "name": name, "in": "header", "schema": { "type": "string" }, "example": example }),
        ));

        let mut operation = Map::new();
        operation.insert("operationId".into(), request.get("id").cloned().unwrap_or(Value::Null));
        operation.insert("summary".into(), request.get("name").cloned().unwrap_or(Value::Null));
        operation.insert("parameters".into(), Value::Array(parameters));
        operation.insert("responses".into(), json!({ "200": { "description": "OK" } }));

        if let Some(body) = visible_body(request) {
            if !["GET", "HEAD", "DELETE"].contains(&method.to_uppercase().as_str()) {
                let content_type =
                    content_type_header(request).map(js_str).unwrap_or_else(|| detect_content_type(body).to_string());
                let mut content = Map::new();
                content.insert(content_type, json!({ "schema": { "type": "string" }, "example": body }));
                operation.insert("requestBody".into(), json!({ "content": Value::Object(content) }));
            }
        }
        entry[method.to_lowercase()] = Value::Object(operation);
    }

    let spec = json!({
        "openapi": "3.0.3",
        "info": { "title": workspace_name(&cfg, ws_id, "Bifurc"), "version": "1.0.0" },
        "paths": Value::Object(paths),
    });
    Ok(ExportFile { content: pretty(&spec), suggested_name: "requests-export.json" })
}

// ── cURL ───────────────────────────────────────────────────────────────────────────────────────────────

fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

fn to_curl(request: &Value) -> String {
    let mut parts = vec![format!("curl -X {}", text_or(request, "method", "undefined"))];
    for (key, value) in header_entries(request) {
        parts.push(format!("  -H {}", quoted(&format!("{key}: {}", js_str(value)))));
    }
    if let Some(body) = visible_body(request) {
        parts.push(format!("  -d {}", quoted(body)));
    }
    parts.push(format!("  {}", quoted(&text_or(request, "url", "undefined"))));
    parts.join(" \\\n")
}

pub fn curl(ws_id: &str) -> ExportOutcome {
    let mut lines: Vec<String> = Vec::new();
    for request in workspace_requests(ws_id) {
        lines.push(format!("# {}", text_or(&request, "name", "undefined")));
        lines.push(to_curl(&request));
        lines.push(String::new());
    }
    Ok(ExportFile { content: lines.join("\n"), suggested_name: "requests-export.sh" })
}

// ── HAR ────────────────────────────────────────────────────────────────────────────────────────────────

fn to_har_entry(request: &Value) -> Result<Value, String> {
    let headers: Vec<Value> =
        header_entries(request).into_iter().map(|(name, value)| json!({ "name": name, "value": value })).collect();
    let body_size = opt_str(request, "body").map_or(0, |body| body.encode_utf16().count());

    let mut har_request = Map::new();
    har_request.insert("method".into(), request.get("method").cloned().unwrap_or(Value::Null));
    har_request.insert("url".into(), request.get("url").cloned().unwrap_or(Value::Null));
    har_request.insert("httpVersion".into(), json!("HTTP/1.1"));
    har_request.insert("headers".into(), Value::Array(headers));
    har_request.insert("queryString".into(), json!([]));
    har_request.insert("cookies".into(), json!([]));
    har_request.insert("headersSize".into(), json!(-1));
    har_request.insert("bodySize".into(), json!(body_size));
    if let Some(body) = visible_body(request) {
        let mime_type = content_type_header(request).cloned().unwrap_or_else(|| json!("text/plain"));
        har_request.insert("postData".into(), json!({ "mimeType": mime_type, "text": body }));
    }

    Ok(json!({
        "startedDateTime": iso_from_value(request.get("createdAt"))?,
        "time": 0,
        "request": Value::Object(har_request),
        "response": {
            "status": 0,
            "statusText": "",
            "httpVersion": "HTTP/1.1",
            "headers": [],
            "cookies": [],
            "content": { "size": 0, "mimeType": "text/plain" },
            "redirectURL": "",
            "headersSize": -1,
            "bodySize": -1,
        },
        "cache": {},
        "timings": { "send": 0, "wait": 0, "receive": 0 },
    }))
}

pub fn har(ws_id: &str) -> ExportOutcome {
    let cfg = load_config();
    let entries = workspace_requests(ws_id).iter().map(to_har_entry).collect::<Result<Vec<_>, _>>()?;
    let har = json!({
        "log": {
            "version": "1.2",
            "creator": { "name": "Bifurc", "version": "1.0" },
            "comment": workspace_name(&cfg, ws_id, "Bifurc Requests"),
            "entries": entries,
        }
    });
    Ok(ExportFile { content: pretty(&har), suggested_name: "requests-export.har" })
}

// ── Insomnia ───────────────────────────────────────────────────────────────────────────────────────────

pub fn insomnia(ws_id: &str) -> ExportOutcome {
    let cfg = load_config();
    let requests = workspace_requests(ws_id);
    let folders = in_workspace(&cfg, "requestFolders", ws_id);

    let workspace_id = format!("wrk_{}", generate_id());
    let now = now_ms();
    let mut resources = vec![json!({
        "_id": workspace_id,
        "_type": "workspace",
        "name": workspace_name(&cfg, ws_id, "Bifurc"),
        "created": now,
        "modified": now,
    })];

    let mut folder_ids: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for folder in &folders {
        let insomnia_id = format!("fld_{}", generate_id());
        folder_ids.insert(js_str(&folder["id"]), insomnia_id.clone());
        let parent = opt_str(folder, "parentId")
            .filter(|p| !p.is_empty())
            .and_then(|p| folder_ids.get(p).cloned())
            .unwrap_or_else(|| workspace_id.clone());
        let created = folder.get("createdAt").cloned().unwrap_or(Value::Null);
        resources.push(json!({
            "_id": insomnia_id,
            "_type": "request_group",
            "parentId": parent,
            "name": folder.get("name").cloned().unwrap_or(Value::Null),
            "created": created,
            "modified": created,
        }));
    }

    for request in &requests {
        let headers: Vec<Value> =
            header_entries(request).into_iter().map(|(name, value)| json!({ "name": name, "value": value })).collect();
        let parent = opt_str(request, "folderId")
            .filter(|f| !f.is_empty())
            .and_then(|f| folder_ids.get(f).cloned())
            .unwrap_or_else(|| workspace_id.clone());
        let body = match visible_body(request) {
            Some(text) => json!({ "mimeType": "text/plain", "text": text }),
            None => json!({}),
        };
        let created = request.get("createdAt").cloned().unwrap_or(Value::Null);
        resources.push(json!({
            "_id": format!("req_{}", generate_id()),
            "_type": "request",
            "parentId": parent,
            "name": request.get("name").cloned().unwrap_or(Value::Null),
            "method": request.get("method").cloned().unwrap_or(Value::Null),
            "url": request.get("url").cloned().unwrap_or(Value::Null),
            "headers": headers,
            "body": body,
            "created": created,
            "modified": created,
        }));
    }

    let output = json!({
        "__export_format": 4,
        "__export_date": iso_from_value(Some(&json!(now)))?,
        "__export_source": "bifurc",
        "resources": resources,
    });
    Ok(ExportFile { content: pretty(&output), suggested_name: "requests-export.json" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        testing::scratch_data_root,
        workspace::fs::{upsert_name_entry, write_entity},
    };

    fn seed(ws: &str) {
        let a = json!({ "id": "r1", "name": "List", "method": "GET", "url": "https://api.test/users?x=1",
            "headers": { "Accept": "application/json", "Content-Type": "text/plain" }, "body": "",
            "createdAt": 1_700_000_000_000_i64, "folderId": null, "workspaceId": ws });
        let b = json!({ "id": "r2", "name": "Create", "method": "POST", "url": "https://api.test/users",
            "headers": { "X-Key": "k" }, "body": "{\"name\":\"a\"}",
            "createdAt": 1_700_000_000_500_i64, "folderId": null, "workspaceId": ws });
        for r in [&a, &b] {
            write_entity(ws, "requests", str_of(r, "id"), r, None).unwrap();
            upsert_name_entry(ws, "requests", str_of(r, "id"), json!({ "name": str_of(r, "name") })).unwrap();
        }
    }

    #[test]
    fn curl_export_quotes_headers_body_and_url() {
        let root = scratch_data_root();
        seed(root.ws_id);
        let out = curl(root.ws_id).unwrap();
        assert_eq!(out.suggested_name, "requests-export.sh");
        assert!(out.content.contains("# Create\ncurl -X POST \\\n  -H \"X-Key: k\" \\\n  -d \"{\\\"name\\\":\\\"a\\\"}\" \\\n  \"https://api.test/users\"\n"));
        assert!(out.content.contains("curl -X GET \\\n  -H \"Accept: application/json\""));
    }

    #[test]
    fn curl_export_imports_back_through_the_curl_parser() {
        let root = scratch_data_root();
        seed(root.ws_id);
        let out = curl(root.ws_id).unwrap();
        let block = out.content.split("# Create\n").nth(1).unwrap().split("\n\n").next().unwrap();
        let parsed = crate::utils::curl::parse_curl_command(block.trim()).unwrap();
        assert_eq!(parsed.method, "POST");
        assert_eq!(parsed.url, "https://api.test/users");
        assert_eq!(parsed.body, "{\"name\":\"a\"}");
    }

    #[test]
    fn har_export_has_the_log_shape() {
        let root = scratch_data_root();
        seed(root.ws_id);
        let har: Value = serde_json::from_str(&har(root.ws_id).unwrap().content).unwrap();
        assert_eq!(har["log"]["version"], "1.2");
        assert_eq!(har["log"]["comment"], "Workspace 1");
        let entries = har["log"]["entries"].as_array().unwrap();
        let create = entries.iter().find(|e| e["request"]["method"] == "POST").unwrap();
        assert_eq!(create["startedDateTime"], "2023-11-14T22:13:20.500Z");
        assert_eq!(create["request"]["postData"], json!({ "mimeType": "text/plain", "text": "{\"name\":\"a\"}" }));
        assert_eq!(create["request"]["bodySize"], 12);
        assert_eq!(create["response"]["content"]["mimeType"], "text/plain");
        let keys: Vec<_> = create.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["startedDateTime", "time", "request", "response", "cache", "timings"]);
    }

    #[test]
    fn openapi_export_groups_operations_by_path() {
        let root = scratch_data_root();
        seed(root.ws_id);
        let spec: Value = serde_json::from_str(&openapi(root.ws_id).unwrap().content).unwrap();
        assert_eq!(spec["openapi"], "3.0.3");
        assert_eq!(spec["info"], json!({ "title": "Workspace 1", "version": "1.0.0" }));
        let users = &spec["paths"]["/users"];
        assert_eq!(users["get"]["summary"], "List");
        assert_eq!(
            users["get"]["parameters"],
            json!([{ "name": "Accept", "in": "header", "schema": { "type": "string" }, "example": "application/json" }])
        );
        assert!(users["get"].get("requestBody").is_none());
        assert_eq!(users["post"]["requestBody"]["content"]["application/json"]["example"], "{\"name\":\"a\"}");
    }

    #[test]
    fn open_api_paths_come_from_urls_the_way_javascript_reads_them() {
        assert_eq!(to_open_api_path("https://h.test/a/b?x=1"), "/a/b");
        assert_eq!(to_open_api_path("/a/b?x=1"), "/a/b");
        assert_eq!(to_open_api_path("{{base}}/users"), "{{base}}/users");
        assert_eq!(detect_content_type(" <a/>"), "application/xml");
        assert!(extract_path_params("/users/{id}").is_empty());
    }

    #[test]
    fn insomnia_export_links_requests_to_folders() {
        let root = scratch_data_root();
        seed(root.ws_id);
        let out: Value = serde_json::from_str(&insomnia(root.ws_id).unwrap().content).unwrap();
        assert_eq!(out["__export_format"], 4);
        assert_eq!(out["__export_source"], "bifurc");
        let resources = out["resources"].as_array().unwrap();
        assert_eq!(resources[0]["_type"], "workspace");
        assert!(resources[0]["_id"].as_str().unwrap().starts_with("wrk_"));
        let create = resources.iter().find(|r| r["name"] == "Create").unwrap();
        assert_eq!(create["parentId"], resources[0]["_id"]);
        assert_eq!(create["body"], json!({ "mimeType": "text/plain", "text": "{\"name\":\"a\"}" }));
        let list = resources.iter().find(|r| r["name"] == "List").unwrap();
        assert_eq!(list["body"], json!({}));
    }

    #[test]
    fn postman_export_uses_the_workspace_name() {
        let root = scratch_data_root();
        seed(root.ws_id);
        let col: Value = serde_json::from_str(&postman(root.ws_id).unwrap().content).unwrap();
        assert_eq!(col["info"]["name"], "Workspace 1");
        assert_eq!(col["item"].as_array().unwrap().len(), 2);
    }
}
