//! Postman Collection v2.1: Bifurc requests and mocks to a collection, and a collection back to requests and
//! mocks. Mocks carry a `_bifurc` extension on each item so regex and enabled state survive a round trip.

use std::collections::HashMap;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Map, Value, json};

use crate::{
    importexport::support::{js_str, js_trim, parse_json, present, pretty, put_opt, text_or, thrown},
    utils::{
        id::generate_id,
        json::{array_of, is_truthy, opt_str},
    },
    workspace::blob::decode_base64_lenient,
};

const SCHEMA: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

/// Folders nest this deep at most; a deeper (or cyclic) tree is cut off.
const MAX_DEPTH: usize = 64;

/// What `encodeURIComponent` leaves alone.
const URI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

fn encode_component(text: &str) -> String {
    utf8_percent_encode(text, URI_COMPONENT).to_string()
}

/// `{ key, value, type: "text" }` for each entry of a header record.
fn record_to_h(record: Option<&Value>) -> Value {
    let entries = record.and_then(Value::as_object).into_iter().flatten();
    Value::Array(entries.map(|(key, value)| json!({ "key": key, "value": value, "type": "text" })).collect())
}

/// The enabled, named headers of a Postman header list as a record.
fn h_to_record(headers: Option<&Value>) -> Value {
    let mut out = Map::new();
    for header in headers.and_then(Value::as_array).into_iter().flatten() {
        if !is_truthy(header.get("disabled").unwrap_or(&Value::Null))
            && is_truthy(header.get("key").unwrap_or(&Value::Null))
        {
            out.insert(js_str(&header["key"]), present(header, "value").cloned().unwrap_or_else(|| json!("")));
        }
    }
    Value::Object(out)
}

fn url_raw(url: Option<&Value>) -> String {
    match url {
        Some(Value::String(s)) => s.clone(),
        Some(other) => text_or(other, "raw", ""),
        None => String::new(),
    }
}

fn body_to_text(body: Option<&Value>) -> String {
    let Some(body) = body.filter(|b| is_truthy(b)) else { return String::new() };
    let params = |key: &str| -> Vec<&Value> {
        array_of(body, key)
            .iter()
            .filter(|p| {
                !is_truthy(p.get("disabled").unwrap_or(&Value::Null)) && is_truthy(p.get("key").unwrap_or(&Value::Null))
            })
            .collect()
    };
    match opt_str(body, "mode") {
        Some("raw") => text_or(body, "raw", ""),
        Some("urlencoded") if body.get("urlencoded").is_some_and(Value::is_array) => params("urlencoded")
            .iter()
            .map(|p| format!("{}={}", encode_component(&js_str(&p["key"])), encode_component(&text_or(p, "value", ""))))
            .collect::<Vec<_>>()
            .join("&"),
        Some("formdata") if body.get("formdata").is_some_and(Value::is_array) => params("formdata")
            .iter()
            .map(|p| format!("{}: {}", js_str(&p["key"]), text_or(p, "value", "")))
            .collect::<Vec<_>>()
            .join("\n"),
        Some("graphql") if body.get("graphql").is_some_and(|g| is_truthy(g)) => {
            let graphql = &body["graphql"];
            let mut parts = Map::new();
            for key in ["query", "variables"] {
                if let Some(value) = graphql.get(key).filter(|v| is_truthy(v)) {
                    parts.insert(key.into(), value.clone());
                }
            }
            pretty(&Value::Object(parts))
        }
        _ => String::new(),
    }
}

/// Postman's names in a script and what Bifurc calls them: `pm` is `bc`, and the calls of the older `postman`
/// object have a `bc` counterpart.
const SCRIPT_RENAMES: [(&str, &str); 8] = [
    ("pm.", "bc."),
    ("postman.setEnvironmentVariable(", "bc.environment.set("),
    ("postman.getEnvironmentVariable(", "bc.environment.get("),
    ("postman.clearEnvironmentVariable(", "bc.environment.unset("),
    ("postman.setGlobalVariable(", "bc.globals.set("),
    ("postman.getGlobalVariable(", "bc.globals.get("),
    ("postman.clearGlobalVariable(", "bc.globals.unset("),
    ("responseCode.code", "bc.response.code"),
];

/// A Postman script rewritten to run in Bifurc. A name is only rewritten where it starts a word, so `npm.x`
/// and `a.pm.b` are left alone.
// shortcut: a textual rewrite that also reaches strings and comments, parse the script if that ever bites.
fn to_bifurc_script(source: &str) -> String {
    let mut text = source.to_string();
    for (from, to) in SCRIPT_RENAMES {
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for (at, _) in text.match_indices(from) {
            let glued =
                text[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '.'));
            if !glued {
                out.push_str(&text[last..at]);
                out.push_str(to);
                last = at + from.len();
            }
        }
        out.push_str(&text[last..]);
        text = out;
    }
    text
}

/// `(preScript, testScript)` from an item's events, rewritten for Bifurc; the last of each kind wins. Postman's
/// `test` event becomes the test script, the one whose `bc.test` results are reported.
fn events_to_scripts(events: Option<&Value>) -> (String, String) {
    let (mut pre, mut test) = (String::new(), String::new());
    for event in events.and_then(Value::as_array).into_iter().flatten() {
        let source = event
            .get("script")
            .and_then(|s| s.get("exec"))
            .and_then(Value::as_array)
            .map(|lines| to_bifurc_script(&lines.iter().map(js_str).collect::<Vec<_>>().join("\n")))
            .unwrap_or_default();
        match opt_str(event, "listen") {
            Some("prerequest") => pre = source,
            Some("test") => test = source,
            _ => {}
        }
    }
    (pre, test)
}

fn b64_encode(text: &str) -> String {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    if js_trim(text).is_empty() { String::new() } else { STANDARD.encode(text.as_bytes()) }
}

fn b64_decode(text: &str) -> String {
    String::from_utf8_lossy(&decode_base64_lenient(text)).into_owned()
}

fn status_text(code: Option<&Value>) -> &'static str {
    match code.and_then(Value::as_i64) {
        Some(200) => "OK",
        Some(201) => "Created",
        Some(204) => "No Content",
        Some(301) => "Moved Permanently",
        Some(302) => "Found",
        Some(304) => "Not Modified",
        Some(400) => "Bad Request",
        Some(401) => "Unauthorized",
        Some(403) => "Forbidden",
        Some(404) => "Not Found",
        Some(405) => "Method Not Allowed",
        Some(409) => "Conflict",
        Some(422) => "Unprocessable Entity",
        Some(429) => "Too Many Requests",
        Some(500) => "Internal Server Error",
        Some(502) => "Bad Gateway",
        Some(503) => "Service Unavailable",
        _ => "Unknown",
    }
}

// ── folder tree ────────────────────────────────────────────────────────────────────────────────────────

struct Node {
    folder: Option<Value>,
    children: Vec<usize>,
    items: Vec<Value>,
}

/// Folders nested by `parentId`, items placed in their folder; anything unplaceable goes to the root (node 0).
fn build_folder_tree(folders: &[Value], items: &[Value]) -> Vec<Node> {
    let mut nodes = vec![Node { folder: None, children: Vec::new(), items: Vec::new() }];
    let mut by_id: HashMap<String, usize> = HashMap::new();
    for folder in folders {
        nodes.push(Node { folder: Some(folder.clone()), children: Vec::new(), items: Vec::new() });
        by_id.insert(js_str(&folder["id"]), nodes.len() - 1);
    }
    let lookup =
        |id: Option<&Value>| id.filter(|v| !v.is_null()).and_then(|v| by_id.get(&js_str(v)).copied()).unwrap_or(0);
    for folder in folders {
        let child = lookup(folder.get("id"));
        let parent = lookup(folder.get("parentId"));
        nodes[parent].children.push(child);
    }
    for item in items {
        let node = lookup(item.get("folderId"));
        nodes[node].items.push(item.clone());
    }
    nodes
}

fn node_to_items(nodes: &[Node], index: usize, depth: usize, leaf: &dyn Fn(&Value) -> Value) -> Vec<Value> {
    let node = &nodes[index];
    let mut out: Vec<Value> = node.items.iter().map(leaf).collect();
    if depth < MAX_DEPTH {
        for &child in &node.children {
            let name = nodes[child].folder.as_ref().and_then(|f| f.get("name")).cloned().unwrap_or(Value::Null);
            out.push(json!({ "name": name, "item": node_to_items(nodes, child, depth + 1, leaf) }));
        }
    }
    out
}

// ── export ─────────────────────────────────────────────────────────────────────────────────────────────

fn scripts_to_events(pre: Option<&str>, post: Option<&str>) -> Vec<Value> {
    let mut events = Vec::new();
    for (listen, script) in [("prerequest", pre), ("test", post)] {
        if let Some(script) = script.filter(|s| !js_trim(s).is_empty()) {
            let exec: Vec<&str> = script.split('\n').collect();
            events.push(json!({ "listen": listen, "script": { "type": "text/javascript", "exec": exec } }));
        }
    }
    events
}

fn request_to_item(r: &Value) -> Value {
    let method = text_or(r, "method", "undefined");
    let name = match opt_str(r, "name").filter(|n| !n.is_empty()) {
        Some(name) => name.to_string(),
        None => format!("{method} {}", text_or(r, "url", "undefined")),
    };
    let mut request = Map::new();
    request.insert("method".into(), r.get("method").cloned().unwrap_or(Value::Null));
    request.insert("header".into(), record_to_h(r.get("headers")));
    request.insert("url".into(), json!({ "raw": r.get("url").cloned().unwrap_or(Value::Null) }));
    if let Some(body) = opt_str(r, "body").filter(|b| !js_trim(b).is_empty()) {
        request.insert("body".into(), json!({ "mode": "raw", "raw": body }));
    }
    let mut item = Map::new();
    item.insert("name".into(), json!(name));
    item.insert("request".into(), Value::Object(request));
    item.insert("response".into(), json!([]));
    let events = scripts_to_events(opt_str(r, "preScript"), opt_str(r, "postScript"));
    if !events.is_empty() {
        item.insert("event".into(), Value::Array(events));
    }
    Value::Object(item)
}

fn info(name: &str, description: Option<&str>) -> Value {
    let mut info = Map::new();
    info.insert("name".into(), json!(name));
    info.insert("_postman_id".into(), json!(generate_id()));
    info.insert("schema".into(), json!(SCHEMA));
    if let Some(description) = description {
        info.insert("description".into(), json!(description));
    }
    Value::Object(info)
}

/// A Postman collection holding `requests` nested in `folders`.
pub fn export_requests_to_postman(requests: &[Value], folders: &[Value], name: &str) -> String {
    let nodes = build_folder_tree(folders, requests);
    pretty(&json!({ "info": info(name, None), "item": node_to_items(&nodes, 0, 0, &request_to_item) }))
}

fn mock_to_item(m: &Value) -> Value {
    let method = m.get("method").cloned().unwrap_or(Value::Null);
    let pm_method = if method == json!("*") { json!("GET") } else { method.clone() };
    let url_pattern = m.get("urlPattern").cloned().unwrap_or(Value::Null);
    let mut request = Map::new();
    request.insert("method".into(), pm_method);
    request.insert("header".into(), json!([]));
    request.insert("url".into(), json!({ "raw": url_pattern }));
    let captured = b64_decode(&text_or(m, "capturedBody", ""));
    if !js_trim(&captured).is_empty() {
        request.insert("body".into(), json!({ "mode": "raw", "raw": captured }));
    }
    let request = Value::Object(request);

    let pattern_text = js_str(&url_pattern);
    let name = opt_str(m, "name").filter(|n| !n.is_empty());
    let mut response = Map::new();
    response.insert(
        "name".into(),
        json!(name.map(str::to_string).unwrap_or_else(|| format!("{} {pattern_text}", js_str(&method)))),
    );
    response.insert("originalRequest".into(), request.clone());
    response.insert("status".into(), json!(status_text(m.get("responseStatus"))));
    put_opt(&mut response, "code", m.get("responseStatus"));
    response.insert("header".into(), record_to_h(m.get("responseHeaders")));
    put_opt(&mut response, "body", m.get("responseBody"));

    let any_method = if method == json!("*") { "ANY".to_string() } else { js_str(&method) };
    let mut bifurc = Map::new();
    bifurc.insert("urlPattern".into(), url_pattern);
    put_opt(&mut bifurc, "useRegex", m.get("useRegex"));
    put_opt(&mut bifurc, "enabled", m.get("enabled"));
    json!({
        "name": name.map(str::to_string).unwrap_or_else(|| format!("{any_method} {pattern_text}")),
        "request": request,
        "response": [Value::Object(response)],
        "_bifurc": Value::Object(bifurc),
    })
}

/// A Postman collection holding `mocks` nested in `folders`.
pub fn export_mocks_to_postman(mocks: &[Value], folders: &[Value], name: &str) -> String {
    let nodes = build_folder_tree(folders, mocks);
    let description = "Bifurc mock rules. Each item's response[0] contains the mock response. _bifurc extension preserves regex/enabled state.";
    pretty(&json!({ "info": info(name, Some(description)), "item": node_to_items(&nodes, 0, 0, &mock_to_item) }))
}

// ── import ─────────────────────────────────────────────────────────────────────────────────────────────

/// A folder found in a collection; `parent` is the parent folder's name.
pub struct ParsedFolder {
    pub name: String,
    pub parent: Option<String>,
}

/// A request found in a collection; `folder` is its folder's name.
pub struct ParsedRequest {
    pub name: String,
    pub method: String,
    pub url: String,
    pub headers: Value,
    pub body: String,
    pub pre_script: String,
    pub test_script: String,
    pub folder: Option<String>,
}

/// A mock found in a collection; `folder` is its folder's name.
pub struct ParsedMock {
    pub name: String,
    pub method: String,
    pub url_pattern: String,
    pub use_regex: Value,
    pub enabled: Value,
    pub captured_headers: Value,
    pub captured_body: String,
    pub response_status: Value,
    pub response_headers: Value,
    pub response_body: Value,
    pub folder: Option<String>,
}

fn collection_items(json_text: &str) -> Result<Vec<Value>, String> {
    let col = parse_json(json_text)?;
    let is_collection = col
        .get("info")
        .and_then(|i| i.get("schema"))
        .and_then(Value::as_str)
        .is_some_and(|schema| schema.contains("collection"));
    if !is_collection {
        return Err(thrown("Not a Postman collection"));
    }
    Ok(array_of(&col, "item").to_vec())
}

/// Visit the leaf items of a collection in order, folders first as they are met.
fn walk(
    items: &[Value],
    parent: Option<&str>,
    depth: usize,
    on_folder: &mut dyn FnMut(&str, Option<&str>),
    on_leaf: &mut dyn FnMut(&Value, Option<&str>),
) {
    if depth > MAX_DEPTH {
        return;
    }
    for item in items {
        if let Some(children) = item.get("item").and_then(Value::as_array) {
            let name = text_or(item, "name", "undefined");
            on_folder(&name, parent);
            walk(children, Some(&name), depth + 1, on_folder, on_leaf);
        } else if item.get("request").is_some_and(is_truthy) {
            on_leaf(item, parent);
        }
    }
}

fn upper_method(request: &Value, default: &str) -> String {
    present(request, "method").map(|m| js_str(m).to_uppercase()).unwrap_or_else(|| default.to_string())
}

/// The folders and requests of a Postman collection.
pub fn parse_postman_requests(json_text: &str) -> Result<(Vec<ParsedFolder>, Vec<ParsedRequest>), String> {
    let items = collection_items(json_text)?;
    let mut folders = Vec::new();
    let mut requests = Vec::new();
    walk(
        &items,
        None,
        0,
        &mut |name, parent| folders.push(ParsedFolder { name: name.to_string(), parent: parent.map(str::to_string) }),
        &mut |item, parent| {
            let request = &item["request"];
            let (pre_script, test_script) = events_to_scripts(item.get("event"));
            requests.push(ParsedRequest {
                name: text_or(item, "name", "undefined"),
                method: upper_method(request, "GET"),
                url: url_raw(request.get("url")),
                headers: h_to_record(request.get("header")),
                body: body_to_text(request.get("body")),
                pre_script,
                test_script,
                folder: parent.map(str::to_string),
            });
        },
    );
    Ok((folders, requests))
}

/// The folders and mocks of a Postman collection.
pub fn parse_postman_mocks(json_text: &str) -> Result<(Vec<ParsedFolder>, Vec<ParsedMock>), String> {
    let items = collection_items(json_text)?;
    let mut folders = Vec::new();
    let mut mocks = Vec::new();
    walk(
        &items,
        None,
        0,
        &mut |name, parent| folders.push(ParsedFolder { name: name.to_string(), parent: parent.map(str::to_string) }),
        &mut |item, parent| {
            let request = &item["request"];
            let extension = item.get("_bifurc").filter(|e| is_truthy(e));
            let response = item.get("response").and_then(|r| r.get(0)).filter(|r| !r.is_null());
            let url_pattern = extension
                .and_then(|e| present(e, "urlPattern"))
                .map(js_str)
                .unwrap_or_else(|| url_raw(request.get("url")));
            let method = if extension.is_some() { upper_method(request, "*") } else { upper_method(request, "GET") };
            let raw_body = match request.get("body") {
                Some(body) if opt_str(body, "mode") == Some("raw") => text_or(body, "raw", ""),
                _ => String::new(),
            };
            mocks.push(ParsedMock {
                name: text_or(item, "name", "undefined"),
                method: if method.is_empty() { "*".to_string() } else { method },
                url_pattern,
                use_regex: extension.and_then(|e| present(e, "useRegex")).cloned().unwrap_or(json!(false)),
                enabled: extension.and_then(|e| present(e, "enabled")).cloned().unwrap_or(json!(false)),
                captured_headers: h_to_record(request.get("header")),
                captured_body: b64_encode(&raw_body),
                response_status: response.and_then(|r| present(r, "code")).cloned().unwrap_or(json!(200)),
                response_headers: h_to_record(response.and_then(|r| r.get("header"))),
                response_body: response.and_then(|r| present(r, "body")).cloned().unwrap_or(json!("{}")),
                folder: parent.map(str::to_string),
            });
        },
    );
    Ok((folders, mocks))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_requests() -> (Vec<Value>, Vec<Value>) {
        let folders = vec![
            json!({ "id": "f1", "name": "Users", "parentId": null }),
            json!({ "id": "f2", "name": "Admin", "parentId": "f1" }),
        ];
        let requests = vec![
            json!({ "id": "r1", "name": "Root", "method": "GET", "url": "http://a/", "headers": { "X-A": "1" },
                "body": "", "folderId": null }),
            json!({ "id": "r2", "name": "", "method": "POST", "url": "http://a/u", "headers": {},
                "body": "{\"a\":1}", "preScript": "log(1)\nlog(2)", "folderId": "f1" }),
            json!({ "id": "r3", "name": "Deep", "method": "PUT", "url": "http://a/d", "headers": {}, "body": "",
                "folderId": "f2" }),
        ];
        (folders, requests)
    }

    #[test]
    fn requests_export_nests_folders_after_requests() {
        let (folders, requests) = sample_requests();
        let text = export_requests_to_postman(&requests, &folders, "My WS");
        let col: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(col["info"]["name"], "My WS");
        assert_eq!(col["info"]["schema"], SCHEMA);
        assert_eq!(col["item"][0]["name"], "Root");
        assert_eq!(col["item"][0]["request"]["header"][0], json!({ "key": "X-A", "value": "1", "type": "text" }));
        assert_eq!(col["item"][0]["response"], json!([]));
        assert_eq!(col["item"][1]["name"], "Users");
        assert_eq!(col["item"][1]["item"][0]["name"], "POST http://a/u");
        assert_eq!(col["item"][1]["item"][0]["request"]["body"], json!({ "mode": "raw", "raw": "{\"a\":1}" }));
        assert_eq!(col["item"][1]["item"][0]["event"][0]["script"]["exec"], json!(["log(1)", "log(2)"]));
        assert_eq!(col["item"][1]["item"][1]["name"], "Admin");
        assert_eq!(col["item"][1]["item"][1]["item"][0]["name"], "Deep");
        let keys: Vec<_> = col["item"][0]["request"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["method", "header", "url"]);
    }

    #[test]
    fn requests_round_trip_through_a_collection() {
        let (folders, requests) = sample_requests();
        let text = export_requests_to_postman(&requests, &folders, "WS");
        let (parsed_folders, parsed) = parse_postman_requests(&text).unwrap();
        assert_eq!(parsed_folders.len(), 2);
        assert_eq!(parsed_folders[1].parent.as_deref(), Some("Users"));
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].folder, None);
        assert_eq!(parsed[1].folder.as_deref(), Some("Users"));
        assert_eq!(parsed[1].method, "POST");
        assert_eq!(parsed[1].body, "{\"a\":1}");
        assert_eq!(parsed[1].pre_script, "log(1)\nlog(2)");
        assert_eq!(parsed[0].headers, json!({ "X-A": "1" }));
    }

    #[test]
    fn postman_scripts_are_rewritten_for_bifurc() {
        let events = json!([
            { "listen": "prerequest", "script": { "exec": ["postman.setEnvironmentVariable('a', npm.pm.x);"] } },
            { "listen": "test", "script": { "exec": [
                "pm.test('ok', () => pm.expect(pm.response.code).to.eql(200));",
                "if(pm.globals.has('rpm.limit')) x=[pm.info, responseCode.code];",
            ] } },
        ]);
        let (pre, test) = events_to_scripts(Some(&events));
        assert_eq!(pre, "bc.environment.set('a', npm.pm.x);");
        assert_eq!(
            test,
            "bc.test('ok', () => bc.expect(bc.response.code).to.eql(200));\n\
             if(bc.globals.has('rpm.limit')) x=[bc.info, bc.response.code];"
        );
    }

    #[test]
    fn a_non_collection_is_rejected() {
        assert_eq!(parse_postman_requests("{}").err().unwrap(), "Error: Not a Postman collection");
        assert!(parse_postman_mocks("nope").err().unwrap().starts_with("SyntaxError"));
    }

    #[test]
    fn bodies_convert_by_mode() {
        let urlencoded = json!({ "mode": "urlencoded", "urlencoded": [
            { "key": "a b", "value": "x&y" }, { "key": "off", "value": "1", "disabled": true }, { "key": "c" } ] });
        assert_eq!(body_to_text(Some(&urlencoded)), "a%20b=x%26y&c=");
        let formdata = json!({ "mode": "formdata", "formdata": [{ "key": "k", "value": "v" }, { "key": "z" }] });
        assert_eq!(body_to_text(Some(&formdata)), "k: v\nz: ");
        let graphql = json!({ "mode": "graphql", "graphql": { "query": "{ a }" } });
        assert_eq!(body_to_text(Some(&graphql)), "{\n  \"query\": \"{ a }\"\n}");
        assert_eq!(body_to_text(Some(&json!({ "mode": "file" }))), "");
    }

    #[test]
    fn mocks_round_trip_with_the_bifurc_extension() {
        let mocks = vec![json!({
            "id": "m1", "name": "", "method": "*", "urlPattern": "/users/\\d+", "useRegex": true, "enabled": true,
            "capturedBody": "eyJhIjoxfQ==", "responseStatus": 201, "responseHeaders": { "X-R": "y" },
            "responseBody": "{\"ok\":true}", "folderId": null,
        })];
        let text = export_mocks_to_postman(&mocks, &[], "Mocks");
        let col: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(col["item"][0]["name"], "ANY /users/\\d+");
        assert_eq!(col["item"][0]["request"]["method"], "GET");
        assert_eq!(col["item"][0]["request"]["body"]["raw"], "{\"a\":1}");
        assert_eq!(col["item"][0]["response"][0]["status"], "Created");
        assert_eq!(col["item"][0]["response"][0]["name"], "* /users/\\d+");

        let (_, parsed) = parse_postman_mocks(&text).unwrap();
        assert_eq!(parsed[0].method, "GET");
        assert_eq!(parsed[0].url_pattern, "/users/\\d+");
        assert_eq!(parsed[0].use_regex, json!(true));
        assert_eq!(parsed[0].enabled, json!(true));
        assert_eq!(parsed[0].captured_body, "eyJhIjoxfQ==");
        assert_eq!(parsed[0].response_status, json!(201));
        assert_eq!(parsed[0].response_headers, json!({ "X-R": "y" }));
        assert_eq!(parsed[0].response_body, json!("{\"ok\":true}"));
    }

    #[test]
    fn a_plain_postman_item_imports_as_a_disabled_mock() {
        let text = json!({ "info": { "schema": SCHEMA }, "item": [{ "name": "x", "request": { "method": "post",
            "url": "http://h/p" } }] })
        .to_string();
        let (_, parsed) = parse_postman_mocks(&text).unwrap();
        assert_eq!(parsed[0].method, "POST");
        assert_eq!(parsed[0].enabled, json!(false));
        assert_eq!(parsed[0].response_status, json!(200));
        assert_eq!(parsed[0].response_body, json!("{}"));
    }
}
