//! Mock imports: Bifurc JSON, a Postman collection and WireMock stubs.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::{
    importexport::{
        formats::postman::parse_postman_mocks,
        support::{
            FolderSpec, ImportRoot, add_import_folders, commit_config, folder_name, io_failure, js_str, parse_json,
            present, push_to, sync_enabled, thrown, upsert_by_id, workspace_ids,
        },
        types::{ImportOutcome, ImportSource, Imported, Preflight, PreflightOutcome, Strategy},
    },
    utils::{
        id::{generate_id, now_ms},
        json::{array_of, is_truthy, merged, opt_str, str_of},
    },
    workspace::{
        config::load_config,
        fs::{read_all_entities, upsert_name_entry, write_entity},
    },
};

/// Write a mock into its folder, switch it on or off, and list it in the names index.
fn store_mock(ws_id: &str, mock: &Value, folder: Option<&str>) -> Result<(), String> {
    let id = str_of(mock, "id");
    write_entity(ws_id, "mocks", id, mock, folder).map_err(io_failure)?;
    sync_enabled(ws_id, "mocks", id, mock.get("enabled").unwrap_or(&Value::Null))?;
    let entry = json!({
        "name": mock.get("name").cloned().unwrap_or(Value::Null),
        "method": mock.get("method").cloned().unwrap_or(Value::Null),
        "url": mock.get("urlPattern").cloned().unwrap_or(Value::Null),
    });
    upsert_name_entry(ws_id, "mocks", id, entry).map_err(io_failure)
}

fn collisions(ws_id: &str, ids: impl Iterator<Item = String>) -> Vec<String> {
    let existing: std::collections::HashSet<String> =
        read_all_entities(ws_id, "mocks").iter().map(|m| str_of(m, "id").to_string()).collect();
    ids.filter(|id| existing.contains(id)).collect()
}

// ── Bifurc JSON ────────────────────────────────────────────────────────────────────────────────────────

fn parse_json_file(content: &str) -> Result<(Vec<Value>, Vec<Value>), String> {
    let data = parse_json(content)?;
    if opt_str(&data, "schema") != Some("lp-mocks-v1") || !data.get("mocks").is_some_and(Value::is_array) {
        return Err(thrown("Not a valid lp-mocks-v1 file"));
    }
    Ok((array_of(&data, "mocks").to_vec(), array_of(&data, "folders").to_vec()))
}

pub fn json_preflight(ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    let (mocks, _) = parse_json_file(&source.content)?;
    let collision_ids = collisions(ws_id, mocks.iter().map(|m| str_of(m, "id").to_string()));
    Ok(Preflight { item_count: mocks.len(), collision_ids })
}

pub async fn json_run(ws_id: String, source: ImportSource, strategy: Strategy) -> ImportOutcome {
    let (mocks, folders) = parse_json_file(&source.content)?;
    let mut cfg = load_config();
    let existing = workspace_ids(&cfg, "mocks", &ws_id);

    let skips = |mock: &Value| existing.contains(str_of(mock, "id")) && strategy == Strategy::Keep;
    let mut root = ImportRoot::new("mockFolders", "mocks");
    let mut folder_ids: HashMap<String, String> = HashMap::new();
    if mocks.iter().any(|mock| !skips(mock)) {
        let (root_id, _) = root.get(&mut cfg, &ws_id)?;
        let specs: Vec<FolderSpec> = folders
            .iter()
            .map(|f| FolderSpec {
                key: str_of(f, "id").to_string(),
                name: str_of(f, "name").to_string(),
                parent: opt_str(f, "parentId").filter(|p| !p.is_empty()).map(str::to_string),
            })
            .collect();
        folder_ids = add_import_folders(&mut cfg, "mockFolders", "mocks", &ws_id, &root_id, &specs)?;
    }

    let (mut imported, mut skipped) = (0, 0);
    for mock in &mocks {
        let new_id = if strategy == Strategy::New { generate_id() } else { str_of(mock, "id").to_string() };
        if existing.contains(str_of(mock, "id")) && strategy == Strategy::Keep {
            skipped += 1;
            continue;
        }
        let (root_id, root_name) = root.get(&mut cfg, &ws_id)?;
        let in_folder = opt_str(mock, "folderId").filter(|f| !f.is_empty()).and_then(|f| folder_ids.get(f)).cloned();
        let folder_id = in_folder.unwrap_or(root_id);
        let folder = folder_name(&cfg, "mockFolders", Some(&folder_id)).unwrap_or(root_name);
        let new_mock = merged(mock, &json!({ "id": new_id, "folderId": folder_id, "workspaceId": ws_id }));
        upsert_by_id(&mut cfg, "mocks", new_mock.clone());
        store_mock(&ws_id, &new_mock, Some(&folder))?;
        imported += 1;
    }

    commit_config(&cfg)?;
    Ok(Imported::with_skipped(imported, skipped))
}

// ── Postman ────────────────────────────────────────────────────────────────────────────────────────────

pub fn postman_preflight(_ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    let (_, mocks) = parse_postman_mocks(&source.content)?;
    Ok(Preflight::counted(mocks.len()))
}

pub async fn postman_run(ws_id: String, source: ImportSource, _strategy: Strategy) -> ImportOutcome {
    let (folders, mocks) = parse_postman_mocks(&source.content)?;
    let mut cfg = load_config();

    if mocks.is_empty() {
        return Ok(Imported::all(0));
    }
    let mut root = ImportRoot::new("mockFolders", "mocks");
    let (root_id, root_name) = root.get(&mut cfg, &ws_id)?;
    let specs: Vec<FolderSpec> = folders
        .iter()
        .map(|f| FolderSpec { key: f.name.clone(), name: f.name.clone(), parent: f.parent.clone() })
        .collect();
    let folder_ids = add_import_folders(&mut cfg, "mockFolders", "mocks", &ws_id, &root_id, &specs)?;

    let mut imported = 0;
    for parsed in &mocks {
        let folder_id =
            parsed.folder.as_ref().and_then(|f| folder_ids.get(f)).cloned().unwrap_or_else(|| root_id.clone());
        let folder = folder_name(&cfg, "mockFolders", Some(&folder_id)).unwrap_or_else(|| root_name.clone());
        let mock = json!({
            "id": generate_id(),
            "name": parsed.name,
            "method": parsed.method,
            "urlPattern": parsed.url_pattern,
            "useRegex": parsed.use_regex,
            "enabled": parsed.enabled,
            "capturedHeaders": parsed.captured_headers,
            "capturedBody": parsed.captured_body,
            "responseStatus": parsed.response_status,
            "responseHeaders": parsed.response_headers,
            "responseBody": parsed.response_body,
            "createdAt": now_ms(),
            "folderId": folder_id,
            "workspaceId": ws_id,
        });
        push_to(&mut cfg, "mocks", mock.clone());
        store_mock(&ws_id, &mock, Some(&folder))?;
        imported += 1;
    }

    commit_config(&cfg)?;
    Ok(Imported::all(imported))
}

// ── WireMock ───────────────────────────────────────────────────────────────────────────────────────────

fn wiremock_stubs(content: &str) -> Result<Vec<Value>, String> {
    let data = parse_json(content)?;
    Ok(match &data {
        Value::Array(stubs) => stubs.clone(),
        other => array_of(other, "mappings").to_vec(),
    })
}

pub fn wiremock_preflight(ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    let stubs = wiremock_stubs(&source.content)?;
    let ids = stubs.iter().filter(|s| s.get("id").is_some_and(is_truthy)).map(|s| js_str(&s["id"]));
    Ok(Preflight { item_count: stubs.len(), collision_ids: collisions(ws_id, ids) })
}

pub async fn wiremock_run(ws_id: String, source: ImportSource, strategy: Strategy) -> ImportOutcome {
    let stubs = wiremock_stubs(&source.content)?;
    let mut cfg = load_config();
    let existing = workspace_ids(&cfg, "mocks", &ws_id);

    let mut root = ImportRoot::new("mockFolders", "mocks");
    let (mut imported, mut skipped) = (0, 0);
    for stub in &stubs {
        let source_id = present(stub, "id").map(js_str).unwrap_or_else(generate_id);
        if existing.contains(&source_id) && strategy == Strategy::Keep {
            skipped += 1;
            continue;
        }
        let new_id = if strategy == Strategy::New { generate_id() } else { source_id };
        let empty = json!({});
        let request = stub.get("request").filter(|r| !r.is_null()).unwrap_or(&empty);
        let response = stub.get("response").filter(|r| !r.is_null()).unwrap_or(&empty);
        let use_regex = request.get("urlPattern").is_some_and(is_truthy);
        let url_pattern = ["urlPattern", "url", "urlPath"]
            .iter()
            .find_map(|key| present(request, key))
            .map(js_str)
            .unwrap_or_else(|| "/".to_string());
        let method = match present(request, "method").map(js_str) {
            None => "*".to_string(),
            Some(m) if m == "ANY" => "*".to_string(),
            Some(m) => m.to_uppercase(),
        };

        let mut mock = Map::new();
        mock.insert("id".into(), json!(new_id));
        mock.insert(
            "name".into(),
            json!(present(stub, "name").map(js_str).unwrap_or_else(|| format!("{method} {url_pattern}"))),
        );
        mock.insert("method".into(), json!(method));
        mock.insert("urlPattern".into(), json!(url_pattern));
        mock.insert("useRegex".into(), json!(use_regex));
        mock.insert("enabled".into(), json!(false));
        mock.insert("capturedHeaders".into(), json!({}));
        mock.insert("capturedBody".into(), json!(""));
        mock.insert("responseStatus".into(), present(response, "status").cloned().unwrap_or(json!(200)));
        mock.insert("responseHeaders".into(), present(response, "headers").cloned().unwrap_or(json!({})));
        mock.insert("responseBody".into(), present(response, "body").cloned().unwrap_or(json!("")));
        if let Some(delay) = response.get("fixedDelayMilliseconds") {
            mock.insert("responseDelay".into(), delay.clone());
        }
        mock.insert("createdAt".into(), json!(now_ms()));
        let (folder_id, folder) = root.get(&mut cfg, &ws_id)?;
        mock.insert("folderId".into(), json!(folder_id));
        mock.insert("workspaceId".into(), json!(ws_id));
        let mock = Value::Object(mock);

        upsert_by_id(&mut cfg, "mocks", mock.clone());
        store_mock(&ws_id, &mock, Some(&folder))?;
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
        workspace::fs::{entity_dir, load_enabled_set, read_names_index},
    };

    fn source(content: &str) -> ImportSource {
        ImportSource { content: content.to_string(), filename: "mocks.json".to_string() }
    }

    /// The directory of the one folder an import run made.
    fn import_dir(ws: &str) -> std::path::PathBuf {
        let index = crate::workspace::fs::read_index(ws, "mocks");
        let made: Vec<_> = index.folders.iter().filter_map(|f| f["name"].as_str()).filter(|n| n.starts_with("import-")).collect();
        assert!(!made.is_empty());
        entity_dir(ws, "mocks").join(made[0])
    }

    fn mock_file(id: &str, name: &str, folder: Option<&str>) -> Value {
        json!({ "id": id, "name": name, "method": "GET", "urlPattern": "/x", "useRegex": false, "enabled": true,
            "capturedHeaders": {}, "capturedBody": "", "responseStatus": 200, "responseHeaders": {},
            "responseBody": "ok", "createdAt": 1, "folderId": folder, "workspaceId": "other" })
    }

    #[tokio::test]
    async fn bifurc_json_honours_the_collision_strategy() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let file = json!({ "schema": "lp-mocks-v1", "name": "n",
            "mocks": [mock_file("m1", "One", Some("f1")), mock_file("m2", "Two", None)],
            "folders": [{ "id": "f1", "name": "Group", "parentId": null, "createdAt": 1, "workspaceId": "other" }] })
        .to_string();

        let pre = json_preflight(&ws, &source(&file)).unwrap();
        assert_eq!((pre.item_count, pre.collision_ids.len()), (2, 0));
        let first = json_run(ws.clone(), source(&file), Strategy::Keep).await.unwrap();
        assert_eq!((first.imported, first.skipped), (2, Some(0)));
        let stored = read_all_entities(&ws, "mocks");
        let one = stored.iter().find(|m| m["id"] == "m1").unwrap();
        assert_eq!(one["workspaceId"], ws.as_str());
        let index = crate::workspace::fs::read_index(&ws, "mocks");
        let group = index.folders.iter().find(|f| f["name"] == "Group").unwrap();
        let top = index.folders.iter().find(|f| f["id"] == group["parentId"]).unwrap();
        assert!(top["name"].as_str().unwrap().starts_with("import-"), "a folder of the file sits in the import folder");
        assert!(entity_dir(&ws, "mocks").join("Group").join("m1.json").exists());
        assert!(load_enabled_set(&ws, "mocks").contains("m1"));
        assert!(import_dir(&ws).join("m2.json").exists(), "a mock without a folder goes in the import folder");
        assert!(!entity_dir(&ws, "mocks").join("m2.json").exists());
        assert_eq!(read_names_index(&ws, "mocks")["m2"], json!({ "name": "Two", "method": "GET", "url": "/x" }));

        let pre = json_preflight(&ws, &source(&file)).unwrap();
        assert_eq!(pre.collision_ids, ["m1", "m2"]);
        let again = json_run(ws.clone(), source(&file), Strategy::Keep).await.unwrap();
        assert_eq!((again.imported, again.skipped), (0, Some(2)));
        let renewed = json_run(ws.clone(), source(&file), Strategy::New).await.unwrap();
        assert_eq!(renewed.imported, 2);
        assert_eq!(read_all_entities(&ws, "mocks").len(), 4);
        assert_eq!(
            json_preflight(&ws, &source("{\"schema\":\"x\"}")).err().unwrap(),
            "Error: Not a valid lp-mocks-v1 file"
        );
    }

    #[tokio::test]
    async fn exported_mocks_import_back() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let file = json!({ "schema": "lp-mocks-v1", "mocks": [mock_file("m9", "Nine", None)] }).to_string();
        json_run(ws.clone(), source(&file), Strategy::Override).await.unwrap();
        let exported = crate::importexport::exporters::mocks::json(&ws).unwrap();
        let back: Value = serde_json::from_str(&exported.content).unwrap();
        assert_eq!(back["mocks"][0]["id"], "m9");
        assert_eq!(back["mocks"][0]["enabled"], true);
        let again = json_run(ws.clone(), source(&exported.content), Strategy::Override).await.unwrap();
        assert_eq!(again.imported, 1);
        assert_eq!(read_all_entities(&ws, "mocks").len(), 1);
    }

    #[tokio::test]
    async fn postman_mocks_import_with_folders() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let col = json!({ "info": { "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
            "item": [{ "name": "Dir", "item": [{ "name": "M", "request": { "method": "GET", "url": "http://h/p" },
              "response": [{ "code": 418, "header": [{ "key": "A", "value": "b" }], "body": "tea" }],
              "_bifurc": { "urlPattern": "/p", "useRegex": true, "enabled": true } }] }] })
        .to_string();
        assert_eq!(postman_preflight(&ws, &source(&col)).unwrap().item_count, 1);
        assert_eq!(postman_run(ws.clone(), source(&col), Strategy::Keep).await.unwrap().imported, 1);
        let mock = read_all_entities(&ws, "mocks").remove(0);
        assert_eq!(mock["urlPattern"], "/p");
        assert_eq!(mock["useRegex"], true);
        assert_eq!(mock["responseStatus"], 418);
        assert_eq!(mock["responseHeaders"], json!({ "A": "b" }));
        assert!(entity_dir(&ws, "mocks").join("Dir").is_dir());
        assert!(load_enabled_set(&ws, "mocks").contains(mock["id"].as_str().unwrap()));
    }

    #[tokio::test]
    async fn wiremock_stubs_import_disabled() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let file = json!({ "mappings": [
            { "id": "w1", "request": { "method": "ANY", "urlPattern": "/a/.*" },
              "response": { "status": 201, "headers": { "H": "v" }, "body": "b", "fixedDelayMilliseconds": 30 } },
            { "name": "Named", "request": { "method": "post", "urlPath": "/p" } },
        ] })
        .to_string();
        assert_eq!(wiremock_preflight(&ws, &source(&file)).unwrap().item_count, 2);
        let done = wiremock_run(ws.clone(), source(&file), Strategy::Keep).await.unwrap();
        assert_eq!((done.imported, done.skipped), (2, Some(0)));
        let stored = read_all_entities(&ws, "mocks");
        let w1 = stored.iter().find(|m| m["id"] == "w1").unwrap();
        assert_eq!(w1["method"], "*");
        assert_eq!(w1["name"], "* /a/.*");
        assert_eq!(w1["useRegex"], true);
        assert_eq!(w1["responseDelay"], 30);
        assert_eq!(w1["responseStatus"], 201);
        assert!(import_dir(&ws).join("w1.json").exists());
        assert!(!entity_dir(&ws, "mocks").join("w1.json").exists());
        let named = stored.iter().find(|m| m["name"] == "Named").unwrap();
        assert_eq!(named["method"], "POST");
        assert_eq!(named["urlPattern"], "/p");
        assert_eq!(named["responseStatus"], 200);
        assert!(load_enabled_set(&ws, "mocks").is_empty());
        let wm_again = wiremock_run(ws.clone(), source(&file), Strategy::Keep).await.unwrap();
        assert_eq!(wm_again.skipped, Some(1));
    }
}
