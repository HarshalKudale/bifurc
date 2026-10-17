//! Request imports: Bifurc JSON, and the files of other tools (cURL commands, HAR, Insomnia, OpenAPI and
//! Postman), each read into a [`Collection`] of typed requests and stored the one way.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value, json};

use crate::{
    importexport::{
        convert::{Collection, Draft, Http, form_encoded, store, with_query},
        formats::{insomnia::parse_insomnia, postman::parse_postman_requests},
        support::{
            FolderSpec, ImportRoot, add_import_folders, commit_config, io_failure, js_str, js_trim, ms_from_iso,
            parse_any, parse_json, present, text_or, thrown,
        },
        types::{ImportOutcome, ImportSource, Imported, Preflight, PreflightOutcome, Strategy},
    },
    kernel::kinds::to_engine_kind,
    rest::from_curl::request_entity,
    utils::{
        curl::{CurlPart, parse_curl_strict},
        id::generate_id,
        json::{array_of, is_truthy, merged, opt_str, str_of},
    },
    workspace::{
        config::load_config,
        fs::write_entity,
        tree::{self, Scope},
    },
};

/// The preflight and the run of a format that is read into a [`Collection`].
macro_rules! collection_import {
    ($parse:ident, $preflight:ident, $run:ident) => {
        pub fn $preflight(_ws_id: &str, source: &ImportSource) -> PreflightOutcome {
            Ok(Preflight::counted($parse(source)?.drafts.len()))
        }

        pub async fn $run(ws_id: String, source: ImportSource, _strategy: Strategy) -> ImportOutcome {
            store(&ws_id, $parse(&source)?)
        }
    };
}

collection_import!(parse_curl_file, curl_preflight, curl_run);
collection_import!(parse_har, har_preflight, har_run);
collection_import!(parse_insomnia_file, insomnia_preflight, insomnia_run);
collection_import!(parse_openapi, openapi_preflight, openapi_run);
collection_import!(parse_postman_requests, postman_preflight, postman_run);

// ── Bifurc JSON ────────────────────────────────────────────────────────────────────────────────────────

const NOT_REQUESTS: &str = "Not a valid lp-requests-v1 file";

/// The requests and folders of a requests file. A WebSocket file of before, when they were exported apart,
/// is read as the requests it holds.
fn parse_requests_file(content: &str) -> Result<(Vec<Value>, Vec<Value>), String> {
    let data = parse_json(content)?;
    let requests = match opt_str(&data, "schema") {
        Some("lp-requests-v1") if data.get("requests").is_some_and(Value::is_array) => {
            array_of(&data, "requests").to_vec()
        }
        Some("lp-websockets-v1") if data.get("wsConnections").is_some_and(Value::is_array) => {
            array_of(&data, "wsConnections")
                .iter()
                .map(|connection| merged(connection, &json!({ "type": "websocket" })))
                .collect()
        }
        _ => return Err(thrown(NOT_REQUESTS)),
    };
    Ok((requests, array_of(&data, "folders").to_vec()))
}

/// The store's kind for a request of a file, by the `type` it names.
pub fn kind_of_request(request: &Value) -> &'static str {
    Scope::Requests.kind_of_type(opt_str(request, "type").unwrap_or("rest")).unwrap_or("requests")
}

fn request_ids(ws_id: &str) -> HashSet<String> {
    tree::entries(ws_id, Scope::Requests).into_iter().map(|entry| entry.id).collect()
}

pub fn json_preflight(ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    let (requests, _) = parse_requests_file(&source.content)?;
    let existing = request_ids(ws_id);
    let collision_ids =
        requests.iter().map(|r| str_of(r, "id")).filter(|id| existing.contains(*id)).map(str::to_string).collect();
    Ok(Preflight { item_count: requests.len(), collision_ids })
}

pub async fn json_run(ws_id: String, source: ImportSource, strategy: Strategy) -> ImportOutcome {
    let (requests, folders) = parse_requests_file(&source.content)?;
    let mut cfg = load_config();
    let existing = request_ids(&ws_id);
    let skips = |request: &Value| existing.contains(str_of(request, "id")) && strategy == Strategy::Keep;

    let mut root = ImportRoot::new("requestFolders", "requests");
    let mut folder_ids: HashMap<String, String> = HashMap::new();
    if requests.iter().any(|request| !skips(request)) {
        let (root_id, _) = root.get(&mut cfg, &ws_id)?;
        let specs: Vec<FolderSpec> = folders
            .iter()
            .map(|f| FolderSpec {
                key: str_of(f, "id").to_string(),
                name: str_of(f, "name").to_string(),
                parent: opt_str(f, "parentId").filter(|p| !p.is_empty()).map(str::to_string),
            })
            .collect();
        folder_ids = add_import_folders(&mut cfg, "requestFolders", "requests", &ws_id, &root_id, &specs)?;
    }

    let (mut imported, mut skipped) = (0, 0);
    for request in &requests {
        if skips(request) {
            skipped += 1;
            continue;
        }
        let source_id = str_of(request, "id");
        let new_id =
            if strategy == Strategy::New || source_id.is_empty() { generate_id() } else { source_id.to_string() };
        let (root_id, _) = root.get(&mut cfg, &ws_id)?;
        let in_folder = opt_str(request, "folderId").filter(|f| !f.is_empty()).and_then(|f| folder_ids.get(f)).cloned();
        let folder_id = in_folder.unwrap_or(root_id);
        let stored = merged(request, &json!({ "id": new_id, "folderId": folder_id, "workspaceId": ws_id }));
        write_entity(&ws_id, kind_of_request(request), &new_id, &stored, Some(&folder_id)).map_err(io_failure)?;
        imported += 1;
    }

    commit_config(&cfg)?;
    Ok(Imported::with_skipped(imported, skipped))
}

// ── cURL ───────────────────────────────────────────────────────────────────────────────────────────────

fn lines_of(text: &str) -> Vec<&str> {
    text.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)).collect()
}

/// The `curl …` commands in a file with the `# comment` above each: blank lines and comments end a command,
/// backslash continuations stay in it.
fn split_curl_blocks(text: &str) -> Vec<(String, String)> {
    let mut blocks = Vec::new();
    let (mut current, mut comment, mut named) = (String::new(), String::new(), String::new());
    let mut close = |current: &mut String, named: &mut String| {
        if js_trim(current).starts_with("curl") {
            blocks.push((js_trim(current).to_string(), std::mem::take(named)));
        }
        current.clear();
    };
    for line in lines_of(text) {
        let stripped = js_trim(line);
        if stripped.is_empty() || stripped.starts_with('#') {
            close(&mut current, &mut named);
            if let Some(text) = stripped.strip_prefix('#') {
                comment = js_trim(text.trim_start_matches('!')).to_string();
            }
            continue;
        }
        if current.is_empty() {
            named = std::mem::take(&mut comment);
        } else {
            current.push('\n');
        }
        current.push_str(stripped);
    }
    close(&mut current, &mut named);
    blocks
}

fn parse_curl_file(source: &ImportSource) -> Result<Collection, String> {
    let mut out = Collection::default();
    for (block, name) in split_curl_blocks(&source.content) {
        let Ok(call) = parse_curl_strict(&block) else { continue };
        let (kind, mut entity) = request_entity(&call);
        if !name.is_empty() {
            entity["name"] = json!(name);
        }
        out.drafts.push(Draft { kind: to_engine_kind(kind), entity, folder: None });
    }
    Ok(out)
}

// ── HAR ────────────────────────────────────────────────────────────────────────────────────────────────

const NOT_HAR: &str = "Not a valid HAR file";

fn named_pairs(list: &[Value]) -> Vec<(String, String)> {
    list.iter()
        .filter(|item| is_truthy(item.get("name").unwrap_or(&Value::Null)))
        .map(|item| (js_str(&item["name"]), text_or(item, "value", "")))
        .collect()
}

fn parse_har(source: &ImportSource) -> Result<Collection, String> {
    let data = parse_json(&source.content)?;
    // A file may also be one request of a HAR on its own, as tools that make code from a request write it.
    let alone = json!({ "entries": [{ "request": data }] });
    let log = match data.get("log").filter(|l| is_truthy(l)) {
        Some(log) => log,
        None if data.get("method").is_some_and(is_truthy) && data.get("url").is_some_and(is_truthy) => &alone,
        None => return Err(NOT_HAR.to_string()),
    };
    let mut out = Collection { name: present(log, "comment").map(js_str).unwrap_or_default(), ..Collection::default() };
    for entry in array_of(log, "entries") {
        let Some(request) = entry.get("request").filter(|r| is_truthy(r)) else { continue };
        if !request.get("url").is_some_and(is_truthy) || !request.get("method").is_some_and(is_truthy) {
            continue;
        }
        let mut call = Http {
            method: js_str(&request["method"]),
            url: js_str(&request["url"]),
            // HTTP/2 writes the request line as headers named `:method`, `:path`, …
            headers: named_pairs(array_of(request, "headers"))
                .into_iter()
                .filter(|(name, _)| !name.starts_with(':'))
                .collect(),
            ..Http::default()
        };
        if let Some(post) = request.get("postData").filter(|p| p.is_object()) {
            let mime = text_or(post, "mimeType", "");
            let params = array_of(post, "params");
            match present(post, "text").map(js_str).filter(|text| !text.is_empty()) {
                Some(text) => call.body = text,
                None if mime.starts_with("multipart/")
                    || params.iter().any(|p| p.get("fileName").is_some_and(is_truthy)) =>
                {
                    call.parts = params
                        .iter()
                        .map(|p| CurlPart {
                            name: text_or(p, "name", ""),
                            value: text_or(p, "value", ""),
                            file: p.get("fileName").is_some_and(is_truthy),
                            file_name: text_or(p, "fileName", "").rsplit(['/', '\\']).next().unwrap_or("").to_string(),
                            content_type: text_or(p, "contentType", ""),
                        })
                        .collect();
                }
                None => call.body = form_encoded(&named_pairs(params)),
            }
            if !mime.is_empty() && !call.body.is_empty() {
                call.default_header("Content-Type", &mime);
            }
        }
        call.created_at = opt_str(entry, "startedDateTime").and_then(ms_from_iso).map(|ms| json!(ms));
        out.drafts.push(call.draft(None));
    }
    Ok(out)
}

// ── Insomnia ───────────────────────────────────────────────────────────────────────────────────────────

fn parse_insomnia_file(source: &ImportSource) -> Result<Collection, String> {
    parse_insomnia(&source.content)
}

// ── OpenAPI ────────────────────────────────────────────────────────────────────────────────────────────

const OPEN_API_METHODS: [&str; 8] = ["get", "post", "put", "patch", "delete", "head", "options", "trace"];
const NOT_OPENAPI: &str = "Not a valid OpenAPI specification";

/// `node`, or what its `$ref` into the same document points at.
fn resolve<'a>(spec: &'a Value, mut node: &'a Value) -> &'a Value {
    for _ in 0..16 {
        let Some(target) = opt_str(node, "$ref").and_then(|r| r.strip_prefix('#')).and_then(|p| spec.pointer(p)) else {
            break;
        };
        node = target;
    }
    node
}

/// A value that fits a schema: its example or default when it has one, else one made from its type.
fn sample(spec: &Value, schema: &Value, depth: usize) -> Value {
    let schema = resolve(spec, schema);
    if depth > 8 || !schema.is_object() {
        return Value::Null;
    }
    for key in ["example", "default"] {
        if let Some(value) = present(schema, key) {
            return value.clone();
        }
    }
    if let Some(first) = schema.get("enum").and_then(|e| e.get(0)) {
        return first.clone();
    }
    if let Some(parts) = schema.get("allOf").and_then(Value::as_array) {
        let mut all = Map::new();
        for part in parts {
            if let Value::Object(fields) = sample(spec, part, depth + 1) {
                all.extend(fields);
            }
        }
        return Value::Object(all);
    }
    if let Some(first) = ["oneOf", "anyOf"].iter().find_map(|key| schema.get(key).and_then(|list| list.get(0))) {
        return sample(spec, first, depth + 1);
    }
    let ty = match schema.get("type") {
        Some(Value::String(ty)) => ty.as_str(),
        Some(Value::Array(types)) => types.iter().filter_map(Value::as_str).find(|ty| *ty != "null").unwrap_or(""),
        _ => "",
    };
    match ty {
        "array" => json!([sample(spec, schema.get("items").unwrap_or(&Value::Null), depth + 1)]),
        "string" => json!(match opt_str(schema, "format") {
            Some("date-time") => "2024-01-01T00:00:00Z",
            Some("date") => "2024-01-01",
            Some("uuid") => "00000000-0000-0000-0000-000000000000",
            Some("email") => "user@example.com",
            _ => "string",
        }),
        "integer" | "number" => json!(0),
        "boolean" => json!(false),
        _ if ty == "object" || schema.get("properties").is_some() => {
            let mut fields = Map::new();
            for (name, property) in schema.get("properties").and_then(Value::as_object).into_iter().flatten() {
                if !is_truthy(resolve(spec, property).get("readOnly").unwrap_or(&Value::Null)) {
                    fields.insert(name.clone(), sample(spec, property, depth + 1));
                }
            }
            Value::Object(fields)
        }
        _ => Value::Null,
    }
}

/// The address every path of a spec is under. One the spec leaves open is the variable `baseUrl`.
fn base_url(spec: &Value) -> String {
    let base = if let Some(server) = spec.get("servers").and_then(|s| s.get(0)) {
        let mut url = text_or(server, "url", "");
        for (name, variable) in server.get("variables").and_then(Value::as_object).into_iter().flatten() {
            url = url.replace(&format!("{{{name}}}"), &text_or(variable, "default", ""));
        }
        url
    } else if let Some(host) = opt_str(spec, "host").filter(|h| !h.is_empty()) {
        let scheme = spec.get("schemes").and_then(|s| s.get(0)).and_then(Value::as_str).unwrap_or("https");
        format!("{scheme}://{host}{}", text_or(spec, "basePath", ""))
    } else {
        text_or(spec, "basePath", "")
    };
    let base = base.trim_end_matches('/').to_string();
    if base.contains("://") { base } else { format!("{{{{baseUrl}}}}{base}") }
}

/// A body value as the text that is sent for a content type.
fn body_text(value: &Value, content_type: &str) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Object(fields) if content_type.contains("x-www-form-urlencoded") => {
            form_encoded(&fields.iter().map(|(key, value)| (key.clone(), js_str(value))).collect::<Vec<_>>())
        }
        other if content_type.contains("json") => serde_json::to_string_pretty(other).unwrap_or_default(),
        _ => String::new(),
    }
}

fn operation(spec: &Value, base: &str, op_path: &str, method: &str, path_item: &Value, op: &Value) -> Http {
    let url = format!("{base}{}", op_path.replace('{', "{{").replace('}', "}}"));
    let name = present(op, "summary")
        .or_else(|| present(op, "operationId"))
        .map(js_str)
        .unwrap_or_else(|| format!("{} {op_path}", method.to_uppercase()));
    let mut call = Http { name, method: method.to_uppercase(), url, ..Http::default() };

    // The path's parameters, then the operation's own over them.
    let mut parameters: Vec<&Value> = Vec::new();
    for parameter in array_of(path_item, "parameters").iter().chain(array_of(op, "parameters")) {
        let parameter = resolve(spec, parameter);
        let same = |other: &&Value| {
            str_of(other, "name") == str_of(parameter, "name") && str_of(other, "in") == str_of(parameter, "in")
        };
        parameters.retain(|other| !same(other));
        parameters.push(parameter);
    }
    let value_of = |parameter: &Value| -> String {
        let schema = resolve(spec, parameter.get("schema").unwrap_or(&Value::Null));
        ["example", "default"]
            .iter()
            .find_map(|key| present(parameter, key).or_else(|| present(schema, key)))
            .map(js_str)
            .unwrap_or_else(|| format!("{{{{{}}}}}", str_of(parameter, "name")))
    };
    let required = |parameter: &Value| is_truthy(parameter.get("required").unwrap_or(&Value::Null));
    let mut query: Vec<(String, String)> = Vec::new();
    let mut form: Vec<(String, String)> = Vec::new();
    for parameter in parameters {
        let name = str_of(parameter, "name").to_string();
        match str_of(parameter, "in") {
            "query" if required(parameter) => query.push((name, value_of(parameter))),
            "header" if required(parameter) => call.headers.push((name, value_of(parameter))),
            "formData" => form.push((name, value_of(parameter))),
            // Swagger 2 names the body as a parameter.
            "body" => {
                let consumes = op.get("consumes").or_else(|| spec.get("consumes"));
                let content_type =
                    consumes.and_then(|c| c.get(0)).and_then(Value::as_str).unwrap_or("application/json");
                call.body = body_text(&sample(spec, parameter.get("schema").unwrap_or(&Value::Null), 0), content_type);
                call.default_header("Content-Type", content_type);
            }
            _ => {}
        }
    }
    call.url = with_query(&call.url, &query);
    if !form.is_empty() {
        call.body = form_encoded(&form);
        call.default_header("Content-Type", "application/x-www-form-urlencoded");
    }

    let body = resolve(spec, op.get("requestBody").unwrap_or(&Value::Null));
    if let Some(content) = body.get("content").and_then(Value::as_object) {
        let chosen =
            content.iter().find(|(content_type, _)| content_type.contains("json")).or_else(|| content.iter().next());
        if let Some((content_type, media)) = chosen {
            let example = present(media, "example").cloned().or_else(|| {
                let first =
                    media.get("examples").and_then(Value::as_object).and_then(|examples| examples.values().next())?;
                present(resolve(spec, first), "value").cloned()
            });
            let value = example.unwrap_or_else(|| sample(spec, media.get("schema").unwrap_or(&Value::Null), 0));
            call.headers.push(("Content-Type".into(), content_type.clone()));
            if content_type.starts_with("multipart/") {
                call.parts = value
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(name, value)| CurlPart { name: name.clone(), value: js_str(value), ..CurlPart::default() })
                    .collect();
            } else {
                call.body = body_text(&value, content_type);
            }
        }
    }
    call
}

fn parse_openapi(source: &ImportSource) -> Result<Collection, String> {
    let spec = parse_any(&source.content)?;
    if !spec.get("openapi").is_some_and(is_truthy) && !spec.get("swagger").is_some_and(is_truthy) {
        return Err(NOT_OPENAPI.to_string());
    }
    let title =
        spec.get("info").and_then(|i| present(i, "title")).map(js_str).unwrap_or_else(|| "Imported".to_string());
    let mut out = Collection { name: title, ..Collection::default() };
    let base = base_url(&spec);
    if base.starts_with("{{baseUrl}}") {
        out.variables.push(("baseUrl".into(), String::new()));
    }

    let mut tags: HashSet<String> = HashSet::new();
    for (op_path, path_item) in spec.get("paths").and_then(Value::as_object).into_iter().flatten() {
        let path_item = resolve(&spec, path_item);
        for method in OPEN_API_METHODS {
            let Some(op) = path_item.get(method).filter(|op| op.is_object()) else { continue };
            // An operation goes in the folder of its first tag.
            let tag = op.get("tags").and_then(|t| t.get(0)).and_then(Value::as_str).filter(|t| !t.is_empty());
            if let Some(tag) = tag.filter(|tag| tags.insert(tag.to_string())) {
                out.folders.push(FolderSpec { key: tag.to_string(), name: tag.to_string(), parent: None });
            }
            let call = operation(&spec, &base, op_path, method, path_item, op);
            out.drafts.push(call.draft(tag.map(str::to_string)));
        }
    }
    Ok(out)
}

// ── OpenAPI as a contract source ───────────────────────────────────────────────────────────────────────

/// The path prefix the spec's server (OpenAPI 3) or `basePath` (Swagger 2) puts before its paths.
fn base_path(spec: &Value) -> String {
    let base = base_url(spec);
    let rest = base.strip_prefix("{{baseUrl}}").map(str::to_string).unwrap_or_else(|| {
        let after = base.split_once("://").map_or(base.as_str(), |(_, rest)| rest);
        after.find('/').map_or(String::new(), |at| after[at..].to_string())
    });
    rest.trim_end_matches('/').to_string()
}

/// The skeleton of a JSON value that fits `schema`: strings `""`, numbers `0`, booleans `false`, arrays
/// `[element]`, objects of their properties; `None` when the schema says nothing (the key is then left out).
/// Rule: an object with a `required` list is held to those properties only (a mock may omit an optional one);
/// without a `required` list every property counts. allOf is merged, oneOf/anyOf take the first branch, a `$ref`
/// is followed within the document and a cycle stops at the reference that closes it.
fn skeleton_of(spec: &Value, schema: &Value, at: &mut At) -> Option<Value> {
    if at.stack.len() > 12 || !schema.is_object() {
        return None;
    }
    let reference = opt_str(schema, "$ref").map(str::to_string);
    if let Some(r) = &reference {
        if at.stack.contains(r) {
            return None;
        }
        at.stack.push(r.clone());
    }
    let out = skeleton_body(spec, resolve(spec, schema), at);
    if reference.is_some() {
        at.stack.pop();
    }
    out
}

/// Where `skeleton_of` is: the `$ref`s being followed, the path as the CLI's shape diff prints it, and the
/// declared-but-not-required properties met so far (a mock may carry those, so a `-` line for them is no drift).
#[derive(Default)]
struct At {
    stack: Vec<String>,
    path: String,
    optional: Vec<String>,
}

/// A key appended to a shape path, as `cli/src/shape.rs` writes it.
fn key_path(path: &str, key: &str) -> String {
    let bare = !key.is_empty() && !key.chars().any(|c| c.is_whitespace() || ".[]|=!<>\"".contains(c));
    if bare { format!("{path}.{key}") } else { format!("{path}.[{}]", Value::String(key.into())) }
}

fn skeleton_body(spec: &Value, schema: &Value, at: &mut At) -> Option<Value> {
    let types: Vec<&str> = match schema.get("type") {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let mut fields = Map::new();
    let mut is_object = types.contains(&"object") || schema.get("properties").is_some();
    if let Some(parts) = schema.get("allOf").and_then(Value::as_array) {
        for part in parts {
            if let Some(Value::Object(inner)) = skeleton_of(spec, part, at) {
                is_object = true;
                fields.extend(inner);
            }
        }
    }
    if is_object {
        let required: Option<Vec<&str>> =
            schema.get("required").and_then(Value::as_array).map(|r| r.iter().filter_map(Value::as_str).collect());
        for (name, property) in schema.get("properties").and_then(Value::as_object).into_iter().flatten() {
            if required.as_ref().is_some_and(|r| !r.contains(&name.as_str())) {
                at.optional.push(key_path(&at.path, name));
                continue;
            }
            let inner = key_path(&at.path, name);
            let outer = std::mem::replace(&mut at.path, inner);
            let value = skeleton_of(spec, property, at);
            at.path = outer;
            if let Some(value) = value {
                fields.insert(name.clone(), value);
            }
        }
        return Some(Value::Object(fields));
    }
    if let Some(first) = ["oneOf", "anyOf"].iter().find_map(|key| schema.get(key).and_then(|list| list.get(0))) {
        return skeleton_of(spec, first, at);
    }
    // No type: an `items` is an array, an enum is typed by its first value.
    let ty = types.iter().find(|t| **t != "null").copied().or_else(|| {
        if schema.get("items").is_some() {
            Some("array")
        } else {
            schema.get("enum").and_then(|e| e.get(0)).map(|v| match v {
                Value::String(_) => "string",
                Value::Bool(_) => "boolean",
                _ => "number",
            })
        }
    });
    match ty {
        Some("string") => Some(json!("")),
        Some("number" | "integer") => Some(json!(0)),
        Some("boolean") => Some(json!(false)),
        Some("array") => {
            let inner = format!("{}[]", at.path);
            let outer = std::mem::replace(&mut at.path, inner);
            let element = schema.get("items").and_then(|items| skeleton_of(spec, items, at));
            at.path = outer;
            Some(Value::Array(element.into_iter().collect()))
        }
        None if types == ["null"] => Some(Value::Null),
        _ => None,
    }
}

/// Every 2xx response of every operation as `{method, path, status, contentType, body}`: `path` as the spec writes
/// it (`/orders/{id}`, without the base path), `contentType` the media type of the JSON body or else the first
/// one (empty when none), `body` its skeleton (null when there is no JSON schema), `optional` the shape paths of the properties the
/// schema declares but does not require (left out of the skeleton).
pub fn contract_operations(text: &str) -> Result<Value, String> {
    let spec = parse_any(text)?;
    if !spec.get("openapi").is_some_and(is_truthy) && !spec.get("swagger").is_some_and(is_truthy) {
        return Err(NOT_OPENAPI.to_string());
    }
    let mut operations = Vec::new();
    for (path, item) in spec.get("paths").and_then(Value::as_object).into_iter().flatten() {
        for method in OPEN_API_METHODS {
            let Some(responses) = item.get(method).and_then(|op| op.get("responses")).and_then(Value::as_object) else {
                continue;
            };
            for (code, response) in responses {
                let status = match code.as_bytes() {
                    [b'2', rest @ ..] if rest.len() == 2 && rest.iter().all(u8::is_ascii_digit) => {
                        code.parse().unwrap_or(200)
                    }
                    [b'2', b'X' | b'x', b'X' | b'x'] => 200,
                    _ => continue,
                };
                let response = resolve(&spec, response);
                // Swagger 2 has the schema on the response, OpenAPI 3 under each media type.
                let (media, schema) = match response.get("content").and_then(Value::as_object) {
                    Some(content) => match content.keys().find(|k| k.contains("json")).or(content.keys().next()) {
                        Some(key) => (key.clone(), content[key].get("schema").cloned()),
                        None => (String::new(), None),
                    },
                    None => (String::new(), response.get("schema").cloned()),
                };
                let media = if media.is_empty() && schema.is_some() { "application/json".into() } else { media };
                let mut at = At::default();
                let body = schema
                    .filter(|_| media.contains("json"))
                    .and_then(|s| skeleton_of(&spec, &s, &mut at))
                    .unwrap_or(Value::Null);
                operations.push(json!({
                    "method": method.to_uppercase(), "path": path, "status": status, "contentType": media, "body": body, "optional": at.optional,
                }));
            }
        }
    }
    Ok(json!({ "basePath": base_path(&spec), "operations": operations }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        testing::scratch_data_root,
        workspace::fs::{entity_dir, read_all_entities, read_entity, read_names_index},
    };

    fn source(content: &str, filename: &str) -> ImportSource {
        ImportSource::text(content, filename)
    }

    /// An import puts everything in one new top-level folder named after the moment it ran.
    fn default_folder_id(ws: &str) -> Value {
        let folders = tree::folders(ws, Scope::Requests);
        let made: Vec<_> = folders.iter().filter(|f| f.name.starts_with("import-")).collect();
        assert_eq!(made.len(), 1, "one import folder per run");
        assert_eq!(made[0].parent_id, None);
        json!(made[0].id)
    }

    /// The directory of the folder named `name`, wherever in the request tree it is.
    fn folder_dir(ws: &str, name: &str) -> std::path::PathBuf {
        let folders = tree::folders(ws, Scope::Requests);
        let folder = folders.iter().find(|f| f.name == name).expect("the folder is made");
        assert!(folder.path.starts_with("import-"), "inside the import folder");
        entity_dir(ws, "requests").join(&folder.path)
    }

    fn of_kind(ws: &str, kind: &str) -> Vec<Value> {
        let mut all = read_all_entities(ws, kind);
        all.sort_by_key(|r| str_of(r, "name").to_string());
        all
    }

    const CURLS: &str = "# Create user\ncurl -X POST https://a.test/users \\\n  -H 'Content-Type: application/json' \\\n  -d '{\"a\":1}'\n\n# not a command\n\ncurl https://a.test/ping\nwget nothing\n\n# Feed\ncurl wss://a.test/feed\n";

    #[tokio::test]
    async fn curl_blocks_become_requests_named_by_their_comment() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        assert_eq!(curl_preflight(&ws, &source(CURLS, "x.sh")).unwrap().item_count, 3);
        let done = curl_run(ws.clone(), source(CURLS, "x.sh"), Strategy::Keep).await.unwrap();
        assert_eq!(done.imported, 3);
        let all = of_kind(&ws, "requests");
        let create = all.iter().find(|r| r["name"] == "Create user").unwrap();
        assert_eq!(create["method"], "POST");
        assert_eq!(create["url"], "https://a.test/users");
        assert_eq!(create["headers"], json!({ "Content-Type": "application/json" }));
        assert_eq!(create["body"], "{\"a\":1}");
        assert_eq!(create["workspaceId"], ws.as_str());
        let ping = all.iter().find(|r| r["name"] == "not a command").unwrap();
        assert_eq!(ping["folderId"], default_folder_id(&ws));
        assert_eq!(read_names_index(&ws, "requests").len(), 2);
        assert_eq!(of_kind(&ws, "sockets")[0]["name"], "Feed", "a WebSocket address is a WebSocket request");
    }

    #[tokio::test]
    async fn captured_traffic_exported_as_har_imports_back_as_the_same_requests() {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let b64 = |s: &str| STANDARD.encode(s);
        let entries = [
            json!({ "id": "1", "ts": 1_700_000_000_000_i64, "method": "GET", "url": "http://h/a?x=1&y=two", "reqHeaders": { "X-A": "1" },
                    "reqBody": "", "resHeaders": { "content-type": "text/plain" }, "resBody": b64("ok"), "resStatus": 200, "durationMs": 5 }),
            json!({ "id": "2", "ts": 1_700_000_001_000_i64, "method": "POST", "url": "http://h/b", "reqHeaders": { "Content-Type": "application/json" },
                    "reqBody": b64("{\"a\":1}"), "resHeaders": {}, "resBody": STANDARD.encode([0xff, 0xfe, 0x00]), "resStatus": 201 }),
        ];
        let har = crate::capture::captured_to_har(&entries).to_string();
        assert_eq!(
            har_run(ws.clone(), source(&har, "captured-requests.har"), Strategy::Keep).await.unwrap().imported,
            2
        );
        let all = of_kind(&ws, "requests");
        let get = all.iter().find(|r| r["method"] == "GET").unwrap();
        assert_eq!(
            (&get["url"], &get["headers"], &get["body"]),
            (&json!("http://h/a?x=1&y=two"), &json!({ "X-A": "1" }), &json!(""))
        );
        assert_eq!(get["createdAt"], 1_700_000_000_000_i64);
        let post = all.iter().find(|r| r["method"] == "POST").unwrap();
        assert_eq!((&post["url"], &post["body"]), (&json!("http://h/b"), &json!("{\"a\":1}")));
        assert_eq!(post["headers"], json!({ "Content-Type": "application/json" }));
        let har: Value = serde_json::from_str(&har).unwrap();
        let content = &har["log"]["entries"][1]["response"]["content"];
        assert_eq!((&content["encoding"], &content["size"]), (&json!("base64"), &json!(3)));
    }

    #[tokio::test]
    async fn har_entries_go_into_a_folder_named_by_the_comment() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let har = json!({ "log": { "comment": "Session", "entries": [
            { "startedDateTime": "2023-11-14T22:13:20.000Z", "request": { "method": "post", "url": "http://h/x",
              "headers": [{ "name": "Host", "value": "h" }, { "name": ":authority", "value": "h" }, { "name": "X-A", "value": "1" }],
              "postData": { "mimeType": "text/plain", "text": "body" } } },
            { "request": { "method": "POST", "url": "http://h/form", "headers": [],
              "postData": { "mimeType": "application/x-www-form-urlencoded", "params": [{ "name": "a", "value": "1 2" }] } } },
            { "request": { "method": "GET" } },
        ] } })
        .to_string();
        assert_eq!(har_preflight(&ws, &source(&har, "a.har")).unwrap().item_count, 2);
        assert_eq!(har_run(ws.clone(), source(&har, "a.har"), Strategy::Keep).await.unwrap().imported, 2);
        let all = of_kind(&ws, "requests");
        let form = &all[0];
        assert_eq!((&form["name"], &form["body"]), (&json!("POST /form"), &json!("a=1%202")));
        let posted = &all[1];
        assert_eq!(posted["name"], "POST /x");
        assert_eq!(posted["method"], "POST");
        assert_eq!(posted["headers"], json!({ "X-A": "1", "Content-Type": "text/plain" }));
        assert_eq!(posted["body"], "body");
        assert_eq!(posted["createdAt"], 1_700_000_000_000_i64);
        assert!(folder_dir(&ws, "Session").join(format!("{}.json", posted["id"].as_str().unwrap())).exists());
        assert_eq!(har_preflight(&ws, &source("{}", "a.har")).err().unwrap(), "Not a valid HAR file");
    }

    #[tokio::test]
    async fn insomnia_requests_are_stored_by_type_with_their_variables() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let export = json!({ "__export_format": 4, "resources": [
            { "_id": "wrk_1", "_type": "workspace", "name": "W" },
            { "_id": "env_1", "_type": "environment", "parentId": "wrk_1", "data": { "base-url": "http://h" } },
            { "_id": "fld_1", "_type": "request_group", "parentId": "wrk_1", "name": "Group" },
            { "_id": "req_1", "_type": "request", "parentId": "fld_1", "name": "One", "method": "put",
              "url": "{{ _.base-url }}/1", "body": { "text": "t" }, "created": 5 },
            { "_id": "greq_1", "_type": "grpc_request", "parentId": "wrk_1", "name": "Rpc", "url": "h:1", "protoMethodName": "/a.B/C" },
        ] })
        .to_string();
        assert_eq!(insomnia_preflight(&ws, &source(&export, "i.json")).unwrap().item_count, 2);
        assert_eq!(insomnia_run(ws.clone(), source(&export, "i.json"), Strategy::Keep).await.unwrap().imported, 2);
        let one = &of_kind(&ws, "requests")[0];
        assert_eq!((&one["method"], &one["body"], &one["createdAt"]), (&json!("PUT"), &json!("t"), &json!(5)));
        assert_eq!(one["url"], "{{base_url}}/1", "a variable is written the way Bifurc reads it");
        assert!(folder_dir(&ws, "Group").join(format!("{}.json", one["id"].as_str().unwrap())).exists());
        assert!(folder_dir(&ws, "W").is_dir(), "the export's name is a folder");
        assert_eq!(of_kind(&ws, "grpcRequests")[0]["methodName"], "C");
        let global = read_entity(&ws, "environments", "__global__").unwrap();
        assert_eq!(global["variables"][0]["key"], "base_url");
        assert_eq!(global["variables"][0]["value"], "http://h");
        assert_eq!(insomnia_preflight(&ws, &source("{}", "i.json")).err().unwrap(), "Not a valid Insomnia export file");
    }

    const SPEC_YAML: &str = "openapi: 3.0.0\ninfo:\n  title: Pets\nservers:\n  - url: https://{env}.p.test/\n    variables:\n      env:\n        default: api\npaths:\n  /pets:\n    get:\n      summary: List pets\n      tags: [pets]\n      parameters:\n        - name: limit\n          in: query\n          required: true\n          schema:\n            type: integer\n            default: 20\n        - name: tag\n          in: query\n        - name: X-Key\n          in: header\n          required: true\n      responses:\n        200:\n          description: ok\n    post:\n      operationId: addPet\n      requestBody:\n        content:\n          application/json:\n            example: '{\"n\":1}'\n    put:\n      summary: Replace\n      requestBody:\n        content:\n          application/json:\n            schema:\n              $ref: '#/components/schemas/Pet'\n  /pets/{id}:\n    delete: {}\n    parameters: []\ncomponents:\n  schemas:\n    Pet:\n      type: object\n      properties:\n        id:\n          type: integer\n          readOnly: true\n        name:\n          type: string\n          example: Rex\n        tags:\n          type: array\n          items:\n            type: string\n";

    #[tokio::test]
    async fn openapi_operations_get_an_address_parameters_and_a_body() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        assert_eq!(openapi_preflight(&ws, &source(SPEC_YAML, "spec.yaml")).unwrap().item_count, 4);
        assert_eq!(openapi_run(ws.clone(), source(SPEC_YAML, "spec.yaml"), Strategy::Keep).await.unwrap().imported, 4);
        let all = of_kind(&ws, "requests");
        let by_name = |name: &str| all.iter().find(|r| r["name"] == name).unwrap();
        let post = by_name("addPet");
        assert_eq!(post["method"], "POST");
        assert_eq!(post["url"], "https://api.p.test/pets");
        assert_eq!(post["headers"], json!({ "Content-Type": "application/json" }));
        assert_eq!(post["body"], "{\"n\":1}");
        let list = by_name("List pets");
        assert_eq!(list["url"], "https://api.p.test/pets?limit=20", "required parameters only");
        assert_eq!(list["headers"], json!({ "X-Key": "{{X_Key}}" }));
        assert!(folder_dir(&ws, "pets").join(format!("{}.json", list["id"].as_str().unwrap())).exists());
        assert!(folder_dir(&ws, "Pets").is_dir());
        let body: Value = serde_json::from_str(by_name("Replace")["body"].as_str().unwrap()).unwrap();
        assert_eq!(body, json!({ "name": "Rex", "tags": ["string"] }), "a body made from the schema");
        assert_eq!(by_name("DELETE /pets/{id}")["url"], "https://api.p.test/pets/{{id}}");
        assert_eq!(
            openapi_preflight(&ws, &source("{\"a\":1}", "spec.json")).err().unwrap(),
            "Not a valid OpenAPI specification"
        );
    }

    #[test]
    fn a_swagger_2_spec_has_its_address_and_body_parameters() {
        let spec = json!({ "swagger": "2.0", "info": { "title": "Old" }, "host": "o.test", "basePath": "/v2", "schemes": ["http"],
            "paths": { "/pet": { "post": { "consumes": ["application/json"], "parameters": [
                { "in": "body", "name": "body", "schema": { "$ref": "#/definitions/Pet" } }] } },
              "/login": { "post": { "parameters": [{ "in": "formData", "name": "user", "type": "string" }] } } },
            "definitions": { "Pet": { "type": "object", "properties": { "name": { "type": "string" } } } } })
        .to_string();
        let out = parse_openapi(&source(&spec, "s.json")).unwrap();
        let pet = &out.drafts.iter().find(|d| d.entity["url"] == "http://o.test/v2/pet").unwrap().entity;
        assert_eq!(pet["body"], "{\n  \"name\": \"string\"\n}");
        let login = &out.drafts.iter().find(|d| d.entity["url"] == "http://o.test/v2/login").unwrap().entity;
        assert_eq!(login["body"], "user={{user}}");
        let open = parse_openapi(&source("{\"openapi\":\"3.1.0\",\"paths\":{\"/a\":{\"get\":{}}}}", "s.json")).unwrap();
        assert_eq!(open.drafts[0].entity["url"], "{{baseUrl}}/a");
        assert_eq!(open.variables, [("baseUrl".to_string(), String::new())]);
    }

    #[tokio::test]
    async fn a_postman_collection_is_stored_in_a_folder_of_its_name() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let col = json!({ "info": { "name": "C", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
            "variable": [{ "key": "host", "value": "h" }],
            "item": [
                { "name": "Top", "request": { "method": "GET", "url": "http://{{host}}/top" } },
                { "name": "Group", "item": [
                    { "name": "In", "request": { "method": "post", "url": { "raw": "http://h/in" },
                      "header": [{ "key": "K", "value": "V" }], "body": { "mode": "raw", "raw": "b" } },
                      "event": [{ "listen": "test", "script": { "exec": ["a", "pm.test('b')"] } }] },
                    { "name": "Q", "request": { "method": "POST", "url": "http://h/graphql",
                      "body": { "mode": "graphql", "graphql": { "query": "{ a }" } } } } ] } ] })
        .to_string();
        assert_eq!(postman_preflight(&ws, &source(&col, "c.json")).unwrap().item_count, 3);
        assert_eq!(postman_run(ws.clone(), source(&col, "c.json"), Strategy::Keep).await.unwrap().imported, 3);
        let all = of_kind(&ws, "requests");
        let inside = all.iter().find(|r| r["name"] == "In").unwrap();
        assert_eq!(inside["method"], "POST");
        assert_eq!(inside["testScript"], "a\nbc.test('b')");
        assert!(inside.get("preScript").is_none());
        assert!(folder_dir(&ws, "Group").join(format!("{}.json", inside["id"].as_str().unwrap())).exists());
        let top = all.iter().find(|r| r["name"] == "Top").unwrap();
        let collection = tree::folders(&ws, Scope::Requests).into_iter().find(|f| f.name == "C").unwrap();
        assert_eq!(top["folderId"], collection.id.as_str());
        assert_eq!(collection.parent_id.map(Value::from), Some(default_folder_id(&ws)));
        assert_eq!(of_kind(&ws, "graphqlRequests")[0]["query"], "{ a }");
        assert_eq!(read_entity(&ws, "environments", "__global__").unwrap()["variables"][0]["key"], "host");
    }

    #[tokio::test]
    async fn bifurc_json_brings_every_type_and_honours_the_collision_strategy() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let file = json!({ "schema": "lp-requests-v1", "requests": [
            { "id": "r1", "type": "rest", "name": "Ping", "method": "GET", "url": "http://a/", "headers": {}, "body": "", "folderId": "f1", "workspaceId": "old" },
            { "id": "g1", "type": "grpc", "name": "Rpc", "serverAddress": "h:1", "serviceName": "a.B", "methodName": "C", "folderId": null, "workspaceId": "old" },
            { "id": "s1", "type": "websocket", "name": "Feed", "url": "ws://x", "headers": {}, "folderId": "f1", "workspaceId": "old" },
        ], "folders": [{ "id": "f1", "name": "Live", "parentId": null }] })
        .to_string();
        assert_eq!(json_preflight(&ws, &source(&file, "r.json")).unwrap().collision_ids.len(), 0);
        let done = json_run(ws.clone(), source(&file, "r.json"), Strategy::Keep).await.unwrap();
        assert_eq!((done.imported, done.skipped), (3, Some(0)));
        assert_eq!(read_entity(&ws, "requests", "r1").unwrap()["workspaceId"], ws.as_str());
        assert_eq!(read_entity(&ws, "grpcRequests", "g1").unwrap()["folderId"], default_folder_id(&ws));
        assert!(folder_dir(&ws, "Live").join("s1.json").exists(), "a WebSocket is a request, in its folder");
        assert!(read_entity(&ws, "sockets", "s1").is_some());

        assert_eq!(json_preflight(&ws, &source(&file, "r.json")).unwrap().collision_ids, ["r1", "g1", "s1"]);
        let kept = json_run(ws.clone(), source(&file, "r.json"), Strategy::Keep).await.unwrap();
        assert_eq!((kept.imported, kept.skipped), (0, Some(3)));
        let both = json_run(ws.clone(), source(&file, "r.json"), Strategy::New).await.unwrap();
        assert_eq!(both.imported, 3);
        assert_eq!(of_kind(&ws, "grpcRequests").len(), 2);

        let old = json!({ "schema": "lp-websockets-v1", "wsConnections": [{ "id": "c9", "name": "Old", "url": "ws://y", "headers": {} }] }).to_string();
        assert_eq!(json_run(ws.clone(), source(&old, "w.json"), Strategy::Keep).await.unwrap().imported, 1);
        assert!(read_entity(&ws, "sockets", "c9").is_some());
        assert_eq!(
            json_preflight(&ws, &source("{\"schema\":\"x\"}", "r.json")).err().unwrap(),
            "Error: Not a valid lp-requests-v1 file"
        );
    }

    fn ops(yaml: &str) -> Value {
        contract_operations(yaml).unwrap()
    }

    #[test]
    fn contract_skeletons_follow_refs_allof_required_and_stop_at_cycles() {
        let spec = "openapi: 3.0.0\nservers:\n  - url: https://h.test/v1\npaths:\n  /orders/{id}:\n    get:\n      responses:\n        '404':\n          description: no\n        '200':\n          description: ok\n          content:\n            application/json:\n              schema:\n                $ref: '#/components/schemas/Order'\n  /tree:\n    get:\n      responses:\n        200:\n          content:\n            application/problem+json:\n              schema:\n                $ref: '#/components/schemas/Node'\n  /pets:\n    get:\n      responses:\n        2XX:\n          content:\n            application/json:\n              schema:\n                oneOf:\n                  - type: array\n                    items: {type: object, properties: {n: {type: integer}, flag: {type: boolean}}}\n                  - type: string\n  /empty:\n    delete:\n      responses:\n        204:\n          description: none\ncomponents:\n  schemas:\n    Base:\n      type: object\n      properties:\n        id: {type: integer}\n    Order:\n      allOf:\n        - $ref: '#/components/schemas/Base'\n        - type: object\n          required: [total, note]\n          properties:\n            total: {type: number}\n            note: {type: string, nullable: true}\n            optional: {type: string}\n            gone: {type: 'null'}\n            anything: {}\n    Node:\n      type: object\n      properties:\n        name: {type: string}\n        children:\n          type: array\n          items:\n            $ref: '#/components/schemas/Node'\n";
        let got = ops(spec);
        assert_eq!(got["basePath"], "/v1");
        let list = got["operations"].as_array().unwrap();
        let find = |path: &str| list.iter().find(|o| o["path"] == path).unwrap().clone();
        // allOf merged, `required` keeps total and note only (not the optional, not the untyped one).
        assert_eq!(find("/orders/{id}")["body"], json!({"id": 0, "total": 0, "note": ""}));
        assert_eq!(find("/orders/{id}")["status"], 200);
        assert_eq!(find("/orders/{id}")["optional"], json!([".optional", ".gone", ".anything"]));
        assert_eq!(find("/tree")["body"], json!({"name": "", "children": []}));
        assert_eq!(find("/tree")["contentType"], "application/problem+json");
        assert_eq!(find("/pets")["body"], json!([{"n": 0, "flag": false}]));
        assert_eq!(find("/pets")["status"], 200);
        let empty = find("/empty");
        assert_eq!(
            (empty["status"].clone(), empty["body"].clone(), empty["contentType"].clone()),
            (json!(204), Value::Null, json!(""))
        );
        // A null-only type is null; `required` absent keeps every property.
        let only_null = ops(
            "openapi: 3.0.0\npaths:\n  /a:\n    get:\n      responses:\n        200:\n          content:\n            application/json:\n              schema: {type: object, properties: {a: {type: 'null'}, b: {type: [string, 'null']}}}\n",
        );
        assert_eq!(only_null["operations"][0]["body"], json!({"a": null, "b": ""}));
        assert_eq!(only_null["basePath"], "");
    }

    #[test]
    fn contract_operations_read_swagger_2_and_refuse_other_files() {
        let swagger = ops(
            "swagger: '2.0'\nbasePath: /api\npaths:\n  /x:\n    get:\n      responses:\n        200:\n          schema:\n            $ref: '#/definitions/X'\ndefinitions:\n  X:\n    type: object\n    properties:\n      n: {type: integer}\n",
        );
        assert_eq!(swagger["basePath"], "/api");
        assert_eq!(swagger["operations"][0]["body"], json!({"n": 0}));
        assert_eq!(contract_operations("{\"a\":1}").unwrap_err(), "Not a valid OpenAPI specification");
    }

    #[test]
    fn optional_paths_cover_nested_objects_array_elements_and_refs() {
        let got = ops(
            "openapi: 3.0.0\npaths:\n  /a:\n    get:\n      responses:\n        200:\n          content:\n            application/json:\n              schema:\n                type: array\n                items:\n                  $ref: '#/components/schemas/Item'\ncomponents:\n  schemas:\n    Item:\n      type: object\n      required: [id, meta]\n      properties:\n        id: {type: integer}\n        note: {type: string}\n        x.y: {type: string}\n        meta:\n          type: object\n          required: [a]\n          properties:\n            a: {type: string}\n            b: {type: string}\n",
        );
        let op = &got["operations"][0];
        assert_eq!(op["body"], json!([{"id": 0, "meta": {"a": ""}}]));
        assert_eq!(op["optional"], json!(["[].note", "[].[\"x.y\"]", "[].meta.b"]));
    }
}
