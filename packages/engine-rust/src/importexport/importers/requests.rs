//! Request imports: cURL commands, HAR, Insomnia, OpenAPI and Postman.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::{
    importexport::{
        formats::postman::parse_postman_requests,
        support::{
            FolderSpec, ImportRoot, add_import_folders, commit_config, js_str, js_trim, ms_from_iso,
            parse_json, present, store_request, text_or,
        },
        types::{ImportOutcome, ImportSource, Imported, Preflight, PreflightOutcome, Strategy},
    },
    utils::{
        curl::parse_curl_command,
        id::{generate_id, now_ms},
        json::{array_of, is_truthy, opt_str},
    },
    workspace::config::load_config,
};

/// A saved request in the field order the store has always written.
fn saved_request(
    id: &str,
    name: &str,
    method: &str,
    url: &Value,
    headers: Value,
    body: Value,
    created_at: Value,
    folder_id: Option<&str>,
    ws_id: &str,
) -> Value {
    json!({
        "id": id, "name": name, "method": method, "url": url, "headers": headers, "body": body,
        "createdAt": created_at, "folderId": folder_id, "workspaceId": ws_id,
    })
}

// ── cURL ───────────────────────────────────────────────────────────────────────────────────────────────

fn lines_of(text: &str) -> Vec<&str> {
    text.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)).collect()
}

/// The `curl …` commands in a file: blank lines and comments end a command, backslash continuations stay in it.
fn split_curl_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current = String::new();
    for line in lines_of(text) {
        let stripped = js_trim(line);
        if stripped.is_empty() || stripped.starts_with('#') {
            if js_trim(&current).starts_with("curl") {
                blocks.push(js_trim(&current).to_string());
                current.clear();
            }
            continue;
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(stripped);
    }
    if js_trim(&current).starts_with("curl") {
        blocks.push(js_trim(&current).to_string());
    }
    blocks
}

pub fn curl_preflight(_ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    let valid = split_curl_blocks(&source.content).iter().filter(|b| parse_curl_command(b).is_some()).count();
    Ok(Preflight::counted(valid))
}

/// Request names from the `# comment` above each command.
fn curl_names(text: &str) -> HashMap<String, String> {
    let lines = lines_of(text);
    let mut names = HashMap::new();
    for (i, line) in lines.iter().enumerate() {
        let line = js_trim(line);
        if let Some(comment) = line.strip_prefix('#') {
            if let Some(next) = lines[i + 1..].iter().find(|l| js_trim(l).starts_with("curl")) {
                let first = js_trim(next).split('\n').next().unwrap_or("").to_string();
                names.insert(first, js_trim(comment).to_string());
            }
        }
    }
    names
}

pub async fn curl_run(ws_id: String, source: ImportSource, _strategy: Strategy) -> ImportOutcome {
    let blocks = split_curl_blocks(&source.content);
    let mut cfg = load_config();
    let mut root = ImportRoot::new("requestFolders", "requests");
    let names = curl_names(&source.content);

    let mut imported = 0;
    for block in blocks {
        let Some(parsed) = parse_curl_command(&block) else { continue };
        let (root_id, _) = root.get(&mut cfg, &ws_id)?;
        let id = generate_id();
        let first_line = block.split('\n').next().unwrap_or("");
        let name = names
            .get(first_line)
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("{} {}", parsed.method, parsed.url));
        let request = saved_request(
            &id,
            &name,
            &parsed.method,
            &json!(parsed.url),
            json!(parsed.headers),
            json!(parsed.body),
            json!(now_ms()),
            Some(&root_id),
            &ws_id,
        );
        store_request(&ws_id, &request, Some(&root_id))?;
        imported += 1;
    }
    crate::runtime::instances::notify_config_changed();
    Ok(Imported::all(imported))
}

// ── HAR ────────────────────────────────────────────────────────────────────────────────────────────────

const NOT_HAR: &str = "Not a valid HAR file";

pub fn har_preflight(_ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    let data = parse_json(&source.content)?;
    let Some(log) = data.get("log").filter(|l| is_truthy(l)) else { return Err(NOT_HAR.to_string()) };
    Ok(Preflight::counted(array_of(log, "entries").iter().filter(|e| e.get("request").is_some_and(is_truthy)).count()))
}

pub async fn har_run(ws_id: String, source: ImportSource, _strategy: Strategy) -> ImportOutcome {
    let data = parse_json(&source.content)?;
    let Some(log) = data.get("log").filter(|l| is_truthy(l)) else { return Err(NOT_HAR.to_string()) };

    let mut cfg = load_config();
    let comment = present(log, "comment").map(js_str);
    let mut root = ImportRoot::new("requestFolders", "requests");
    let mut commented: Option<String> = None;

    let mut imported = 0;
    for entry in array_of(log, "entries") {
        let Some(request) = entry.get("request").filter(|r| is_truthy(r)) else { continue };
        if !request.get("url").is_some_and(is_truthy) || !request.get("method").is_some_and(is_truthy) {
            continue;
        }
        let mut headers = Map::new();
        for header in array_of(request, "headers") {
            let name = text_or(header, "name", "undefined");
            if !["host", "content-length", "connection"].contains(&name.to_lowercase().as_str()) {
                headers.insert(name, header.get("value").cloned().unwrap_or(Value::Null));
            }
        }
        let (root_id, _) = root.get(&mut cfg, &ws_id)?;
        let folder_id = match comment.as_ref().filter(|c| !c.is_empty()) {
            None => root_id,
            Some(comment) => {
                if commented.is_none() {
                    let spec = FolderSpec { key: "har".into(), name: comment.clone(), parent: None };
                    let made = add_import_folders(&mut cfg, "requestFolders", "requests", &ws_id, &root_id, &[spec])?;
                    commented = Some(made["har"].clone());
                }
                commented.clone().unwrap_or_default()
            }
        };
        let method = js_str(&request["method"]);
        let created_at = match opt_str(entry, "startedDateTime").filter(|s| !s.is_empty()) {
            Some(started) => ms_from_iso(started).map_or(Value::Null, |ms| json!(ms)),
            None => json!(now_ms()),
        };
        let body = request.get("postData").and_then(|p| present(p, "text")).cloned().unwrap_or(json!(""));
        let url = request["url"].clone();
        let request = saved_request(
            &generate_id(),
            &format!("{method} {}", js_str(&url)),
            &method.to_uppercase(),
            &url,
            Value::Object(headers),
            body,
            created_at,
            Some(&folder_id),
            &ws_id,
        );
        store_request(&ws_id, &request, Some(&folder_id))?;
        imported += 1;
    }

    commit_config(&cfg)?;
    Ok(Imported::all(imported))
}

// ── Insomnia ───────────────────────────────────────────────────────────────────────────────────────────

const NOT_INSOMNIA: &str = "Not a valid Insomnia export file";

pub fn insomnia_preflight(_ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    let data = parse_json(&source.content)?;
    let Some(resources) = data.get("resources").and_then(Value::as_array) else {
        return Err(NOT_INSOMNIA.to_string());
    };
    Ok(Preflight::counted(resources.iter().filter(|r| opt_str(r, "_type") == Some("request")).count()))
}

pub async fn insomnia_run(ws_id: String, source: ImportSource, _strategy: Strategy) -> ImportOutcome {
    let data = parse_json(&source.content)?;
    let Some(resources) = data.get("resources").filter(|r| is_truthy(r)) else { return Err(NOT_INSOMNIA.to_string()) };
    let Some(resources) = resources.as_array() else { return Err("TypeError: data.resources is not iterable".into()) };

    let mut cfg = load_config();
    if !resources.iter().any(|r| opt_str(r, "_type") == Some("request")) {
        return Ok(Imported::all(0));
    }
    let mut root = ImportRoot::new("requestFolders", "requests");
    let (root_id, _) = root.get(&mut cfg, &ws_id)?;
    let specs: Vec<FolderSpec> = resources
        .iter()
        .filter(|r| opt_str(r, "_type") == Some("request_group"))
        .map(|resource| FolderSpec {
            key: text_or(resource, "_id", "undefined"),
            name: text_or(resource, "name", "Folder"),
            parent: opt_str(resource, "parentId").filter(|p| !p.is_empty()).map(str::to_string),
        })
        .collect();
    let folder_ids = add_import_folders(&mut cfg, "requestFolders", "requests", &ws_id, &root_id, &specs)?;

    let mut imported = 0;
    for resource in resources.iter().filter(|r| opt_str(r, "_type") == Some("request")) {
        let mut headers = Map::new();
        for header in array_of(resource, "headers") {
            if header.get("name").is_some_and(is_truthy) {
                headers.insert(js_str(&header["name"]), present(header, "value").cloned().unwrap_or(json!("")));
            }
        }
        let in_group = opt_str(resource, "parentId").filter(|p| !p.is_empty()).and_then(|p| folder_ids.get(p)).cloned();
        let folder_id = in_group.unwrap_or_else(|| root_id.clone());
        let method = text_or(resource, "method", "GET");
        let url = present(resource, "url").cloned().unwrap_or(json!(""));
        let name = match present(resource, "name") {
            Some(name) => js_str(name),
            None => format!("{method} {}", js_str(&url)),
        };
        let body = resource.get("body").and_then(|b| present(b, "text")).cloned().unwrap_or(json!(""));
        let created_at = present(resource, "created").cloned().unwrap_or(json!(now_ms()));
        let request = saved_request(
            &generate_id(),
            &name,
            &method.to_uppercase(),
            &url,
            Value::Object(headers),
            body,
            created_at,
            Some(&folder_id),
            &ws_id,
        );
        store_request(&ws_id, &request, Some(&folder_id))?;
        imported += 1;
    }

    commit_config(&cfg)?;
    Ok(Imported::all(imported))
}

// ── OpenAPI ────────────────────────────────────────────────────────────────────────────────────────────

const OPEN_API_METHODS: [&str; 8] = ["get", "post", "put", "patch", "delete", "head", "options", "trace"];
const NOT_OPENAPI: &str = "Not a valid OpenAPI specification";

fn load_spec(source: &ImportSource) -> Result<Value, String> {
    let name = source.filename.rsplit(['/', '\\']).next().unwrap_or("").to_lowercase();
    let is_yaml = name.rfind('.').is_some_and(|dot| dot > 0 && matches!(&name[dot..], ".yaml" | ".yml"));
    if is_yaml {
        serde_yaml_ng::from_str::<Value>(&source.content).map_err(|err| format!("YAMLException: {err}"))
    } else {
        parse_json(&source.content)
    }
}

fn is_openapi(spec: &Value) -> bool {
    spec.get("openapi").is_some_and(is_truthy) || spec.get("swagger").is_some_and(is_truthy)
}

pub fn openapi_preflight(_ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    let spec = load_spec(source)?;
    if !is_openapi(&spec) {
        return Err(NOT_OPENAPI.to_string());
    }
    let count = spec
        .get("paths")
        .and_then(Value::as_object)
        .map(|paths| {
            paths
                .values()
                .filter_map(Value::as_object)
                .map(|item| item.keys().filter(|m| OPEN_API_METHODS.contains(&m.as_str())).count())
                .sum()
        })
        .unwrap_or(0);
    Ok(Preflight::counted(count))
}

pub async fn openapi_run(ws_id: String, source: ImportSource, _strategy: Strategy) -> ImportOutcome {
    let spec = load_spec(&source)?;
    if !is_openapi(&spec) {
        return Err(NOT_OPENAPI.to_string());
    }

    let mut cfg = load_config();
    let base_url =
        spec.get("servers").and_then(|s| s.get(0)).and_then(|s| present(s, "url")).map(js_str).unwrap_or_default();
    let title =
        spec.get("info").and_then(|i| present(i, "title")).map(js_str).unwrap_or_else(|| "Imported".to_string());

    let mut root = ImportRoot::new("requestFolders", "requests");
    let (root_id, _) = root.get(&mut cfg, &ws_id)?;
    let title_folder = FolderSpec { key: "spec".into(), name: title.clone(), parent: None };
    let folder_id = add_import_folders(&mut cfg, "requestFolders", "requests", &ws_id, &root_id, &[title_folder])?.remove("spec").unwrap_or_default();

    let mut imported = 0;
    for (op_path, path_item) in spec.get("paths").and_then(Value::as_object).into_iter().flatten() {
        for method in OPEN_API_METHODS {
            let Some(op) = path_item.get(method).filter(|op| is_truthy(op)) else { continue };
            let url = format!("{base_url}{op_path}");
            let mut headers = Map::new();
            let mut body = json!("");
            if let Some(content) = op.get("requestBody").and_then(|b| b.get("content")).and_then(Value::as_object) {
                if let Some((content_type, media)) = content.iter().next() {
                    headers.insert("Content-Type".into(), json!(content_type));
                    body = media.get("example").filter(|e| !e.is_null()).cloned().unwrap_or(json!(""));
                }
            }
            let name = present(op, "summary")
                .or_else(|| present(op, "operationId"))
                .map(js_str)
                .unwrap_or_else(|| format!("{} {op_path}", method.to_uppercase()));
            let request = saved_request(
                &generate_id(),
                &name,
                &method.to_uppercase(),
                &json!(url),
                Value::Object(headers),
                body,
                json!(now_ms()),
                Some(&folder_id),
                &ws_id,
            );
            store_request(&ws_id, &request, Some(&folder_id))?;
            imported += 1;
        }
    }

    commit_config(&cfg)?;
    Ok(Imported::all(imported))
}

// ── Postman ────────────────────────────────────────────────────────────────────────────────────────────

pub fn postman_preflight(_ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    let (_, requests) = parse_postman_requests(&source.content)?;
    Ok(Preflight::counted(requests.len()))
}

pub async fn postman_run(ws_id: String, source: ImportSource, _strategy: Strategy) -> ImportOutcome {
    let (folders, requests) = parse_postman_requests(&source.content)?;
    let mut cfg = load_config();

    if requests.is_empty() {
        return Ok(Imported::all(0));
    }
    let mut root = ImportRoot::new("requestFolders", "requests");
    let (root_id, _) = root.get(&mut cfg, &ws_id)?;
    let specs: Vec<FolderSpec> = folders
        .iter()
        .map(|f| FolderSpec { key: f.name.clone(), name: f.name.clone(), parent: f.parent.clone() })
        .collect();
    let folder_ids = add_import_folders(&mut cfg, "requestFolders", "requests", &ws_id, &root_id, &specs)?;

    let mut imported = 0;
    for parsed in &requests {
        let id = generate_id();
        let folder_id = parsed.folder.as_ref().and_then(|f| folder_ids.get(f)).cloned().unwrap_or_else(|| root_id.clone());
        let mut request = Map::new();
        request.insert("id".into(), json!(id));
        request.insert("name".into(), json!(parsed.name));
        request.insert("method".into(), json!(parsed.method));
        request.insert("url".into(), json!(parsed.url));
        request.insert("headers".into(), parsed.headers.clone());
        request.insert("body".into(), json!(parsed.body));
        if !parsed.pre_script.is_empty() {
            request.insert("preScript".into(), json!(parsed.pre_script));
        }
        if !parsed.post_script.is_empty() {
            request.insert("postScript".into(), json!(parsed.post_script));
        }
        request.insert("createdAt".into(), json!(now_ms()));
        request.insert("folderId".into(), json!(folder_id));
        request.insert("workspaceId".into(), json!(ws_id));
        store_request(&ws_id, &Value::Object(request), Some(&folder_id))?;
        imported += 1;
    }

    commit_config(&cfg)?;
    Ok(Imported::all(imported))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        testing::scratch_data_root,
        utils::json::str_of,
        workspace::fs::{read_all_entities, read_names_index},
    };

    fn source(content: &str, filename: &str) -> ImportSource {
        ImportSource { content: content.to_string(), filename: filename.to_string() }
    }

    /// An import puts everything in one new top-level folder named after the moment it ran.
    fn default_folder_id(ws: &str) -> Value {
        let folders = crate::workspace::tree::folders(ws, crate::workspace::tree::Scope::Requests);
        let made: Vec<_> = folders.iter().filter(|f| f.name.starts_with("import-")).collect();
        assert_eq!(made.len(), 1, "one import folder per run");
        assert_eq!(made[0].parent_id, None);
        json!(made[0].id)
    }

    /// The directory of the folder named `name`, wherever in the request tree it is.
    fn folder_dir(ws: &str, name: &str) -> std::path::PathBuf {
        let folders = crate::workspace::tree::folders(ws, crate::workspace::tree::Scope::Requests);
        let folder = folders.iter().find(|f| f.name == name).expect("the folder is made");
        assert!(folder.path.starts_with("import-"), "inside the import folder");
        crate::workspace::fs::entity_dir(ws, "requests").join(&folder.path)
    }

    fn requests(ws: &str) -> Vec<Value> {
        let mut all = read_all_entities(ws, "requests");
        all.sort_by_key(|r| str_of(r, "name").to_string());
        all
    }

    const CURLS: &str = "# Create user\ncurl -X POST https://a.test/users \\\n  -H 'Content-Type: application/json' \\\n  -d '{\"a\":1}'\n\n# not a command\n\ncurl https://a.test/ping\nwget nothing\n";

    #[tokio::test]
    async fn curl_blocks_become_requests_named_by_their_comment() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        assert_eq!(curl_preflight(&ws, &source(CURLS, "x.sh")).unwrap().item_count, 2);
        let done = curl_run(ws.clone(), source(CURLS, "x.sh"), Strategy::Keep).await.unwrap();
        assert_eq!(done.imported, 2);
        let all = requests(&ws);
        let create = all.iter().find(|r| r["name"] == "Create user").unwrap();
        assert_eq!(create["method"], "POST");
        assert_eq!(create["url"], "https://a.test/users");
        assert_eq!(create["headers"], json!({ "content-type": "application/json" }));
        assert_eq!(create["body"], "{\"a\":1}");
        assert_eq!(create["workspaceId"], ws.as_str());
        let ping = all.iter().find(|r| r["name"] == "not a command").unwrap();
        assert_eq!(ping["folderId"], default_folder_id(&ws));
        assert_eq!(read_names_index(&ws, "requests").len(), 2);
    }

    #[tokio::test]
    async fn har_entries_go_into_a_folder_named_by_the_comment() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let har = json!({ "log": { "comment": "Session", "entries": [
            { "startedDateTime": "2023-11-14T22:13:20.000Z", "request": { "method": "post", "url": "http://h/x",
              "headers": [{ "name": "Host", "value": "h" }, { "name": "X-A", "value": "1" }],
              "postData": { "text": "body" } } },
            { "request": { "method": "GET" } },
        ] } })
        .to_string();
        assert_eq!(har_preflight(&ws, &source(&har, "a.har")).unwrap().item_count, 2);
        assert_eq!(har_run(ws.clone(), source(&har, "a.har"), Strategy::Keep).await.unwrap().imported, 1);
        let all = requests(&ws);
        assert_eq!(all[0]["name"], "post http://h/x");
        assert_eq!(all[0]["method"], "POST");
        assert_eq!(all[0]["headers"], json!({ "X-A": "1" }));
        assert_eq!(all[0]["body"], "body");
        assert_eq!(all[0]["createdAt"], 1_700_000_000_000_i64);
        assert!(all[0]["folderId"].is_string());
        assert!(folder_dir(&ws, "Session").join(format!("{}.json", all[0]["id"].as_str().unwrap())).exists());
        assert_eq!(har_preflight(&ws, &source("{}", "a.har")).err().unwrap(), "Not a valid HAR file");
    }

    #[tokio::test]
    async fn insomnia_groups_become_folders() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let export = json!({ "__export_format": 4, "resources": [
            { "_id": "wrk_1", "_type": "workspace", "name": "W" },
            { "_id": "fld_1", "_type": "request_group", "parentId": "wrk_1", "name": "Group" },
            { "_id": "req_1", "_type": "request", "parentId": "fld_1", "name": "One", "method": "put", "url": "http://h/1",
              "headers": [{ "name": "A", "value": "b" }, { "name": "", "value": "z" }], "body": { "text": "t" }, "created": 5 },
            { "_id": "req_2", "_type": "request", "parentId": "wrk_1", "url": "http://h/2" },
        ] })
        .to_string();
        assert_eq!(insomnia_preflight(&ws, &source(&export, "i.json")).unwrap().item_count, 2);
        assert_eq!(insomnia_run(ws.clone(), source(&export, "i.json"), Strategy::Keep).await.unwrap().imported, 2);
        let all = requests(&ws);
        let one = all.iter().find(|r| r["name"] == "One").unwrap();
        assert_eq!(one["method"], "PUT");
        assert_eq!(one["headers"], json!({ "A": "b" }));
        assert_eq!(one["body"], "t");
        assert_eq!(one["createdAt"], 5);
        assert!(one["folderId"].is_string());
        let two = all.iter().find(|r| r["name"] == "GET http://h/2").unwrap();
        assert_eq!(two["folderId"], default_folder_id(&ws));
        assert_eq!(insomnia_preflight(&ws, &source("{}", "i.json")).err().unwrap(), "Not a valid Insomnia export file");
    }

    const SPEC_YAML: &str = "openapi: 3.0.0\ninfo:\n  title: Pets\nservers:\n  - url: https://p.test\npaths:\n  /pets:\n    get:\n      summary: List pets\n      responses:\n        200:\n          description: ok\n    post:\n      operationId: addPet\n      requestBody:\n        content:\n          application/json:\n            example: '{\"n\":1}'\n  /pets/{id}:\n    delete: {}\n    parameters: []\n";

    #[tokio::test]
    async fn openapi_yaml_imports_operations_into_a_folder() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        assert_eq!(openapi_preflight(&ws, &source(SPEC_YAML, "spec.yaml")).unwrap().item_count, 3);
        assert_eq!(openapi_run(ws.clone(), source(SPEC_YAML, "spec.yaml"), Strategy::Keep).await.unwrap().imported, 3);
        let all = requests(&ws);
        let post = all.iter().find(|r| r["name"] == "addPet").unwrap();
        assert_eq!(post["method"], "POST");
        assert_eq!(post["url"], "https://p.test/pets");
        assert_eq!(post["headers"], json!({ "Content-Type": "application/json" }));
        assert_eq!(post["body"], "{\"n\":1}");
        let list = all.iter().find(|r| r["name"] == "List pets").unwrap();
        assert!(list["folderId"].is_string());
        assert!(folder_dir(&ws, "Pets").is_dir());
        assert_eq!(
            openapi_preflight(&ws, &source("{\"a\":1}", "spec.json")).err().unwrap(),
            "Not a valid OpenAPI specification"
        );
    }

    #[tokio::test]
    async fn postman_folders_nest_by_name() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let col = json!({ "info": { "name": "C", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
            "item": [
                { "name": "Top", "request": { "method": "GET", "url": "http://h/top" } },
                { "name": "Group", "item": [
                    { "name": "In", "request": { "method": "post", "url": { "raw": "http://h/in" },
                      "header": [{ "key": "K", "value": "V" }], "body": { "mode": "raw", "raw": "b" } },
                      "event": [{ "listen": "test", "script": { "exec": ["a", "b"] } }] } ] } ] })
        .to_string();
        assert_eq!(postman_preflight(&ws, &source(&col, "c.json")).unwrap().item_count, 2);
        assert_eq!(postman_run(ws.clone(), source(&col, "c.json"), Strategy::Keep).await.unwrap().imported, 2);
        let all = requests(&ws);
        let inside = all.iter().find(|r| r["name"] == "In").unwrap();
        assert_eq!(inside["method"], "POST");
        assert_eq!(inside["postScript"], "a\nb");
        assert!(inside.get("preScript").is_none());
        assert!(inside["folderId"].is_string());
        let keys: Vec<_> = inside.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["type", "id", "name", "method", "url", "headers", "body", "postScript", "createdAt", "workspaceId", "folderId"]
        );
        let top = all.iter().find(|r| r["name"] == "Top").unwrap();
        assert_eq!(top["folderId"], default_folder_id(&ws));
    }
}
