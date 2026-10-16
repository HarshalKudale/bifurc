//! Postman collections read into Bifurc requests and mocks.
//!
//! Two formats hold a collection. Collection v2.0 / v2.1 is one JSON file and carries HTTP requests only (a
//! GraphQL call is an HTTP request whose body mode is `graphql`). The v3 format is a directory of YAML files,
//! one per request, and is the one that carries gRPC, WebSocket, Socket.IO and MCP requests; it is read from
//! a single `*.request.yaml` or from a zip of the directory.

use std::{
    collections::{BTreeMap, HashMap},
    io::{Cursor, Read},
};

use serde_json::{Value, json};

use crate::{
    importexport::{
        convert::{
            AuthParts, Collection, Draft, Http, auth_parts, form_encoded, grpc_draft, header_record,
            with_path_variables, with_query,
        },
        support::{FolderSpec, js_str, js_trim, parse_any, parse_json, present, text_or, thrown},
        types::ImportSource,
    },
    utils::{
        curl::CurlPart,
        json::{array_of, is_truthy, opt_str},
    },
};

/// Folders nest this deep at most; a deeper (or cyclic) tree is cut off.
const MAX_DEPTH: usize = 64;

const NOT_A_COLLECTION: &str = "Not a Postman collection";

fn enabled(item: &Value) -> bool {
    !is_truthy(item.get("disabled").unwrap_or(&Value::Null)) && item.get("enabled") != Some(&Value::Bool(false))
}

/// The enabled `{ key, value }` entries of a list, as pairs.
fn pairs(list: Option<&Value>) -> Vec<(String, String)> {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| enabled(item) && is_truthy(item.get("key").unwrap_or(&Value::Null)))
        .map(|item| (js_str(&item["key"]), text_or(item, "value", "")))
        .collect()
}

/// The enabled, named headers of a Postman header list as a record.
fn h_to_record(headers: Option<&Value>) -> Value {
    Value::Object(header_record(&headers_of(headers)))
}

/// A header list, or the `Name: value` lines Collection v2.0 may hold instead.
fn headers_of(headers: Option<&Value>) -> Vec<(String, String)> {
    match headers {
        Some(Value::String(lines)) => lines
            .lines()
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (js_trim(name).to_string(), js_trim(value).to_string()))
            .filter(|(name, _)| !name.is_empty() && !name.starts_with("//"))
            .collect(),
        other => pairs(other),
    }
}

fn url_raw(url: Option<&Value>) -> String {
    match url {
        Some(Value::String(s)) => s.clone(),
        Some(other) => text_or(other, "raw", ""),
        None => String::new(),
    }
}

/// A request's address: the text Postman shows, with its `:name` path variables given their values.
fn address(url: Option<&Value>) -> String {
    let Some(url) = url else { return String::new() };
    let mut raw = url_raw(Some(url));
    if raw.is_empty() && url.is_object() {
        // An address kept only in parts.
        let joined = |key: &str, with: &str| match url.get(key) {
            Some(Value::Array(parts)) => parts.iter().map(js_str).collect::<Vec<_>>().join(with),
            Some(Value::String(text)) => text.clone(),
            _ => String::new(),
        };
        let (host, path) = (joined("host", "."), joined("path", "/"));
        let scheme = opt_str(url, "protocol").map(|p| format!("{p}://")).unwrap_or_default();
        let port = present(url, "port").map(|p| format!(":{}", js_str(p))).unwrap_or_default();
        raw = with_query(&format!("{scheme}{host}{port}/{path}"), &pairs(url.get("query")));
    }
    with_path_variables(&raw, &pairs(url.get("variable")))
}

/// Postman's names in a script and what Bifurc calls them: `pm` is `bc`, and the calls of the older `postman`
/// object have a `bc` counterpart. Insomnia's `insomnia` object is shaped like `pm` too.
const SCRIPT_RENAMES: [(&str, &str); 9] = [
    ("pm.", "bc."),
    ("insomnia.", "bc."),
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
pub fn to_bifurc_script(source: &str) -> String {
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

/// The scripts a collection, a folder and then the request itself run around a call, oldest first.
#[derive(Clone, Default)]
struct Scripts {
    pre: Vec<String>,
    test: Vec<String>,
}

impl Scripts {
    fn add(&mut self, before: bool, code: &str) {
        let code = to_bifurc_script(code);
        if !js_trim(&code).is_empty() {
            if before { self.pre.push(code) } else { self.test.push(code) }
        }
    }

    /// `self` followed by the scripts of a v2 item's `event` list.
    fn with_events(&self, events: Option<&Value>) -> Scripts {
        let mut out = self.clone();
        for event in events.and_then(Value::as_array).into_iter().flatten() {
            let code = match event.get("script").and_then(|s| s.get("exec")) {
                Some(Value::Array(lines)) => lines.iter().map(js_str).collect::<Vec<_>>().join("\n"),
                Some(Value::String(text)) => text.clone(),
                _ => continue,
            };
            match opt_str(event, "listen") {
                Some("prerequest") => out.add(true, &code),
                Some("test") => out.add(false, &code),
                _ => {}
            }
        }
        out
    }

    /// `self` followed by the scripts of a v3 file's `scripts` list.
    fn with_v3(&self, scripts: Option<&Value>) -> Scripts {
        let mut out = self.clone();
        for script in scripts.and_then(Value::as_array).into_iter().flatten() {
            let kind = text_or(script, "type", "");
            let kind = kind.rsplit(':').next().unwrap_or("");
            out.add(kind.starts_with("before"), &text_or(script, "code", ""));
        }
        out
    }

    fn apply(&self, call: &mut Http) {
        call.pre_script = self.pre.join("\n\n");
        call.test_script = self.test.join("\n\n");
    }
}

/// What an `auth` object adds to a call. Collection v2.1 lists a scheme's settings as `{ key, value }`, v2.0
/// keeps them as an object, v3 lists them under `credentials`.
fn auth_of(auth: Option<&Value>) -> AuthParts {
    let Some(auth) = auth.filter(|a| a.is_object()) else { return AuthParts::default() };
    let kind = text_or(auth, "type", "");
    let settings = auth.get("credentials").or_else(|| auth.get(&kind));
    auth_parts(&kind, &|name| match settings {
        Some(Value::Array(list)) => list
            .iter()
            .find(|entry| opt_str(entry, "key") == Some(name))
            .map(|entry| text_or(entry, "value", ""))
            .unwrap_or_default(),
        Some(other) => text_or(other, name, ""),
        None => String::new(),
    })
}

/// The authorization in force for an item: its own unless it has none or says to inherit.
fn auth_in_force<'a>(own: Option<&'a Value>, inherited: Option<&'a Value>) -> Option<&'a Value> {
    match own.filter(|a| a.is_object()) {
        Some(own) if opt_str(own, "type") == Some("inherit") => inherited,
        Some(own) => Some(own),
        None => inherited,
    }
}

fn apply_auth(call: &mut Http, auth: Option<&Value>) {
    let parts = auth_of(auth);
    for (name, value) in parts.headers {
        call.default_header(&name, &value);
    }
    call.url = with_query(&call.url, &parts.query);
}

fn file_name(src: Option<&Value>) -> String {
    let path = match src {
        Some(Value::Array(paths)) => paths.first().map(js_str).unwrap_or_default(),
        Some(Value::String(path)) => path.clone(),
        _ => String::new(),
    };
    path.rsplit(['/', '\\']).next().unwrap_or("").to_string()
}

/// The fields of a multipart body. A file is on the machine the collection was made on: its field is kept, empty.
fn form_parts(list: Option<&Value>) -> Vec<CurlPart> {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|field| enabled(field) && is_truthy(field.get("key").unwrap_or(&Value::Null)))
        .map(|field| {
            let file = opt_str(field, "type") == Some("file");
            CurlPart {
                name: js_str(&field["key"]),
                value: if file { String::new() } else { text_or(field, "value", "") },
                file,
                file_name: if file { file_name(field.get("src")) } else { String::new() },
                content_type: text_or(field, "contentType", ""),
            }
        })
        .collect()
}

fn language_type(language: &str) -> Option<&'static str> {
    match language {
        "json" => Some("application/json"),
        "xml" => Some("application/xml"),
        "html" => Some("text/html"),
        "javascript" => Some("application/javascript"),
        _ => None,
    }
}

fn graphql_parts(graphql: &Value) -> (String, String, String) {
    let variables = match graphql.get("variables") {
        Some(Value::String(text)) => text.clone(),
        Some(other) if !other.is_null() => serde_json::to_string_pretty(other).unwrap_or_default(),
        _ => String::new(),
    };
    (text_or(graphql, "query", ""), variables, text_or(graphql, "operationName", ""))
}

/// Put a v2 `body` into a call.
fn apply_body(call: &mut Http, body: Option<&Value>) {
    let Some(body) = body.filter(|b| is_truthy(b) && enabled(b)) else { return };
    match opt_str(body, "mode") {
        Some("raw") => {
            call.body = text_or(body, "raw", "");
            let language = body.pointer("/options/raw/language").and_then(Value::as_str).unwrap_or("");
            if let Some(content_type) = language_type(language).filter(|_| !call.body.is_empty()) {
                call.default_header("Content-Type", content_type);
            }
        }
        Some("urlencoded") => {
            call.body = form_encoded(&pairs(body.get("urlencoded")));
            call.default_header("Content-Type", "application/x-www-form-urlencoded");
        }
        Some("formdata") => call.parts = form_parts(body.get("formdata")),
        Some("graphql") => call.graphql = body.get("graphql").filter(|g| g.is_object()).map(graphql_parts),
        _ => {}
    }
}

// ── Collection v2.0 / v2.1 ─────────────────────────────────────────────────────────────────────────────

fn collection_json(json_text: &str) -> Result<Value, String> {
    let col = parse_json(json_text)?;
    // `info.schema` names the version; a draft of v2 had `_postman_schema` instead. Either way it has items.
    let is_collection = col.get("info").is_some_and(|info| {
        ["schema", "_postman_schema"]
            .iter()
            .any(|key| opt_str(info, key).is_some_and(|schema| schema.contains("postman")))
    }) && col.get("item").is_some_and(Value::is_array);
    if !is_collection {
        return Err(thrown(NOT_A_COLLECTION));
    }
    Ok(col)
}

fn upper_method(request: &Value, default: &str) -> String {
    present(request, "method").map(|m| js_str(m).to_uppercase()).unwrap_or_else(|| default.to_string())
}

fn v2_request(item: &Value, auth: Option<&Value>, scripts: &Scripts) -> Http {
    let request = &item["request"];
    let mut call = Http { name: text_or(item, "name", ""), ..Http::default() };
    match request {
        // Collection v2 lets a request be only its address.
        Value::String(url) => call.url = url.clone(),
        _ => {
            call.method = upper_method(request, "GET");
            call.url = address(request.get("url"));
            call.headers = headers_of(request.get("header"));
            apply_body(&mut call, request.get("body"));
            apply_auth(&mut call, auth_in_force(request.get("auth"), auth));
        }
    }
    scripts.with_events(item.get("event")).apply(&mut call);
    call
}

fn walk_v2(
    items: &[Value],
    parent: Option<&str>,
    auth: Option<&Value>,
    scripts: &Scripts,
    depth: usize,
    out: &mut Collection,
) {
    if depth > MAX_DEPTH {
        return;
    }
    for (index, item) in items.iter().enumerate() {
        if let Some(children) = item.get("item").and_then(Value::as_array) {
            // Two folders may share a name: a folder is known by where it is.
            let key = format!("{}/{index}", parent.unwrap_or(""));
            out.folders.push(FolderSpec {
                key: key.clone(),
                name: text_or(item, "name", "Folder"),
                parent: parent.map(str::to_string),
            });
            let scripts = scripts.with_events(item.get("event"));
            walk_v2(children, Some(&key), auth_in_force(item.get("auth"), auth), &scripts, depth + 1, out);
        } else if item.get("request").is_some_and(is_truthy) {
            out.drafts.push(v2_request(item, auth, scripts).draft(parent.map(str::to_string)));
        }
    }
}

fn parse_v2(col: &Value) -> Collection {
    let mut out = Collection {
        name: col.get("info").map(|info| text_or(info, "name", "")).unwrap_or_default(),
        variables: pairs(col.get("variable")),
        ..Collection::default()
    };
    let scripts = Scripts::default().with_events(col.get("event"));
    walk_v2(array_of(col, "item"), None, col.get("auth"), &scripts, 0, &mut out);
    out
}

// ── v3 ─────────────────────────────────────────────────────────────────────────────────────────────────

/// What the `definition.yaml` files above a request give it.
#[derive(Clone, Default)]
struct Scope {
    auth: Option<Value>,
    scripts: Scripts,
}

fn v3_body(call: &mut Http, body: Option<&Value>) {
    let Some(body) = body.filter(|b| b.is_object()) else { return };
    let content = body.get("content");
    let text = || {
        content.map(|c| c.as_str().map_or_else(|| serde_json::to_string_pretty(c).unwrap_or_default(), str::to_string))
    };
    match text_or(body, "type", "").as_str() {
        "formdata" => call.parts = form_parts(content),
        "urlencoded" => {
            call.body = form_encoded(&pairs(content));
            call.default_header("Content-Type", "application/x-www-form-urlencoded");
        }
        "file" | "none" => {}
        kind => {
            call.body = text().unwrap_or_default();
            if let Some(content_type) = language_type(kind).filter(|_| !call.body.is_empty()) {
                call.default_header("Content-Type", content_type);
            }
        }
    }
}

/// One v3 request file as a request, or `None` for a kind Bifurc has no request type for (MQTT, LLM).
fn v3_request(file: &Value, fallback_name: &str, scope: &Scope) -> Option<(&'static str, Value)> {
    let name = text_or(file, "name", fallback_name);
    let url = with_query(&text_or(file, "url", ""), &pairs(file.get("queryParams")));
    let headers = pairs(file.get("headers"));
    let auth = auth_in_force(file.get("auth"), scope.auth.as_ref());
    let with_auth = |mut call: Http| {
        apply_auth(&mut call, auth);
        call
    };
    let scripts = scope.scripts.with_v3(file.get("scripts"));
    match text_or(file, "$kind", "").as_str() {
        "http-request" => {
            let mut call = Http {
                name,
                method: text_or(file, "method", "GET"),
                url: with_path_variables(&url, &pairs(file.get("pathVariables"))),
                headers,
                ..Http::default()
            };
            v3_body(&mut call, file.get("body"));
            scripts.apply(&mut call);
            let draft = with_auth(call).draft(None);
            Some((draft.kind, draft.entity))
        }
        "graphql-request" => {
            let mut call = Http { name, method: "POST".into(), url, headers, ..Http::default() };
            call.graphql = Some(graphql_parts(file));
            scripts.apply(&mut call);
            let draft = with_auth(call).draft(None);
            Some((draft.kind, draft.entity))
        }
        "grpc-request" => {
            let call = with_auth(Http { headers: pairs(file.get("metadata")), ..Http::default() });
            let message = file.get("message").map(|m| text_or(m, "content", "")).unwrap_or_default();
            let method = text_or(file, "methodPath", "");
            Some(("grpcRequests", grpc_draft(&name, &url, &method, &message, header_record(&call.headers))))
        }
        "websocket-request" => {
            let call = with_auth(Http { url, headers, ..Http::default() });
            Some(("sockets", json!({ "name": name, "url": call.url, "headers": header_record(&call.headers) })))
        }
        "socket.io-request" => {
            let listeners: Vec<Value> = array_of(file, "events")
                .iter()
                .filter(|event| event.get("subscribeOnConnect") != Some(&Value::Bool(false)))
                .map(|event| json!({ "event": text_or(event, "name", ""), "description": text_or(event, "description", "") }))
                .collect();
            let path = file.get("settings").map(|s| text_or(s, "path", "")).unwrap_or_default();
            Some((
                "socketioRequests",
                json!({
                    "name": name, "url": url, "headers": header_record(&headers), "path": path, "namespace": "",
                    "auth": "", "listeners": listeners,
                }),
            ))
        }
        "mcp-request" => {
            let stdio = opt_str(file, "transport") == Some("stdio");
            let (address, record) = if stdio {
                (text_or(file, "command", ""), header_record(&pairs(file.get("env"))))
            } else {
                let call = with_auth(Http { url, headers, ..Http::default() });
                (call.url.clone(), header_record(&call.headers))
            };
            Some((
                "mcpRequests",
                json!({
                    "name": name, "transport": if stdio { "stdio" } else { "http" }, "url": address, "headers": record,
                }),
            ))
        }
        _ => None,
    }
}

/// The request files of a zipped v3 tree, by the directory they are in. `postman/collections/` on top of
/// the tree, when the archive has it, is not a folder.
fn parse_v3_zip(bytes: &[u8]) -> Result<Collection, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|err| thrown(&err.to_string()))?;
    // directory -> its definition; (directory, file stem) -> request
    let mut definitions: HashMap<String, Value> = HashMap::new();
    let mut requests: BTreeMap<(String, String), Value> = BTreeMap::new();
    for index in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(index) else { continue };
        let Some(path) = entry.enclosed_name().map(|p| p.to_string_lossy().replace('\\', "/")) else { continue };
        let lower = path.to_lowercase();
        let is_request = lower.ends_with(".request.yaml") || lower.ends_with(".request.yml");
        let is_definition = lower.ends_with(".resources/definition.yaml");
        if entry.is_dir() || !(is_request || is_definition) {
            continue;
        }
        let mut text = String::new();
        if entry.read_to_string(&mut text).is_err() {
            continue;
        }
        let Ok(file) = serde_yaml_ng::from_str::<Value>(&text) else { continue };
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", &path));
        if is_definition {
            definitions.insert(dir.trim_end_matches(".resources").trim_end_matches('/').to_string(), file);
        } else if !dir.split('/').any(|part| part.ends_with(".resources")) {
            let stem = name.rsplit_once(".request.").map_or(name, |(stem, _)| stem);
            requests.insert((dir.to_string(), stem.to_string()), file);
        }
    }

    let mut out = Collection::default();
    let mut scopes: HashMap<String, Scope> = HashMap::new();
    let mut ordered: Vec<(&(String, String), &Value)> = requests.iter().collect();
    ordered.sort_by(|a, b| {
        let order = |file: &Value| file.get("order").and_then(Value::as_f64).unwrap_or(f64::MAX);
        (&a.0.0, order(a.1)).partial_cmp(&(&b.0.0, order(b.1))).unwrap_or(std::cmp::Ordering::Equal)
    });
    for ((dir, stem), file) in ordered {
        // Every directory above the request is a folder, and may hold a definition the request inherits.
        let mut scope = Scope::default();
        let mut walked = String::new();
        let mut folder: Option<String> = None;
        for part in dir.split('/').filter(|part| !part.is_empty()) {
            walked = if walked.is_empty() { part.to_string() } else { format!("{walked}/{part}") };
            if matches!(walked.as_str(), "postman" | "postman/collections") {
                continue;
            }
            scope = scopes
                .entry(walked.clone())
                .or_insert_with(|| {
                    let definition = definitions.get(&walked);
                    let mut scope = scope.clone();
                    if let Some(definition) = definition {
                        scope.auth = auth_in_force(definition.get("auth"), scope.auth.as_ref()).cloned();
                        scope.scripts = scope.scripts.with_v3(definition.get("scripts"));
                        out.variables.extend(pairs(definition.get("variables")));
                    }
                    out.folders.push(FolderSpec {
                        key: walked.clone(),
                        name: definition.map(|d| text_or(d, "name", part)).unwrap_or_else(|| part.to_string()),
                        parent: folder.clone(),
                    });
                    scope
                })
                .clone();
            folder = Some(walked.clone());
        }
        match v3_request(file, stem, &scope) {
            Some((kind, entity)) => out.drafts.push(Draft { kind, entity, folder }),
            None => out.skipped += 1,
        }
    }
    Ok(out)
}

/// The requests of a Postman collection, whichever of its formats the file is.
pub fn parse_postman_requests(source: &ImportSource) -> Result<Collection, String> {
    if source.bytes.starts_with(b"PK") {
        return parse_v3_zip(&source.bytes);
    }
    let file = parse_any(&source.content)?;
    if file.get("$kind").is_some() {
        let stem = source.filename.rsplit(['/', '\\']).next().unwrap_or("");
        let stem = stem.split(".request.").next().unwrap_or(stem);
        let mut out = Collection::default();
        match v3_request(&file, stem, &Scope::default()) {
            Some((kind, entity)) => out.drafts.push(Draft { kind, entity, folder: None }),
            None => out.skipped += 1,
        }
        return Ok(out);
    }
    Ok(parse_v2(&collection_json(&source.content)?))
}

// ── mocks ──────────────────────────────────────────────────────────────────────────────────────────────

/// A folder found in a collection; `parent` is the parent folder's name.
pub struct ParsedFolder {
    pub name: String,
    pub parent: Option<String>,
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

fn b64_encode(text: &str) -> String {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    if js_trim(text).is_empty() { String::new() } else { STANDARD.encode(text.as_bytes()) }
}

/// The folders and mocks of a Postman collection: each request with the first response saved for it. A
/// collection Bifurc exported in the past carries a `_bifurc` extension with the regex and enabled state.
pub fn parse_postman_mocks(json_text: &str) -> Result<(Vec<ParsedFolder>, Vec<ParsedMock>), String> {
    let col = collection_json(json_text)?;
    let mut folders = Vec::new();
    let mut mocks = Vec::new();
    walk(
        array_of(&col, "item"),
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
    use std::io::Write;

    use super::*;

    const SCHEMA: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

    fn parsed(col: &Value) -> Collection {
        parse_postman_requests(&ImportSource::text(col.to_string(), "c.json")).unwrap()
    }

    #[test]
    fn a_collection_becomes_typed_requests_in_its_folders() {
        let col = json!({ "info": { "name": "Shop", "schema": SCHEMA },
            "auth": { "type": "bearer", "bearer": [{ "key": "token", "value": "{{token}}" }] },
            "variable": [{ "key": "base-url", "value": "https://shop.test" }, { "key": "off", "value": "1", "disabled": true }],
            "event": [{ "listen": "prerequest", "script": { "exec": ["pm.variables.set('a', 1);"] } }],
            "item": [
                { "name": "Top", "request": { "method": "GET", "url": "http://h/top" } },
                { "name": "Group", "auth": { "type": "noauth" }, "item": [
                    { "name": "In", "request": { "method": "post",
                        "url": { "raw": "{{base-url}}/users/:id?x=1", "variable": [{ "key": "id", "value": "7" }] },
                        "header": [{ "key": "K", "value": "V" }, { "key": "Off", "value": "1", "disabled": true }],
                        "body": { "mode": "raw", "raw": "{\"a\":1}", "options": { "raw": { "language": "json" } } } },
                      "event": [{ "listen": "test", "script": { "exec": ["a", "pm.test('b')"] } }] },
                    { "name": "Group", "item": [
                        { "name": "Query", "request": { "method": "POST", "url": "http://h/graphql",
                            "body": { "mode": "graphql", "graphql": { "query": "{ me { id } }", "variables": "{\"a\":1}" } } } },
                        { "name": "Feed", "request": { "method": "GET", "url": "wss://h/feed" } },
                        { "name": "Form", "request": { "method": "POST", "url": "http://h/form",
                            "auth": { "type": "apikey", "apikey": [{ "key": "key", "value": "k" }, { "key": "value", "value": "v" }, { "key": "in", "value": "query" }] },
                            "body": { "mode": "formdata", "formdata": [{ "key": "a", "value": "1" },
                                { "key": "f", "type": "file", "src": "/tmp/pic.png" }] } } },
                    ] },
                ] },
            ] });
        let out = parsed(&col);
        assert_eq!(out.name, "Shop");
        assert_eq!(out.variables, [("base-url".to_string(), "https://shop.test".to_string())]);
        let keys: Vec<_> = out.folders.iter().map(|f| (f.key.as_str(), f.name.as_str(), f.parent.as_deref())).collect();
        assert_eq!(keys, [("/1", "Group", None), ("/1/1", "Group", Some("/1"))]);
        let by_name = |name: &str| out.drafts.iter().find(|d| d.entity["name"] == name).unwrap();

        let top = by_name("Top");
        assert_eq!((top.kind, top.folder.as_deref()), ("requests", None));
        assert_eq!(top.entity["headers"], json!({ "Authorization": "Bearer {{token}}" }), "the collection's auth");
        assert_eq!(top.entity["preScript"], "bc.variables.set('a', 1);");

        let inside = by_name("In");
        assert_eq!(inside.folder.as_deref(), Some("/1"));
        assert_eq!(inside.entity["method"], "POST");
        assert_eq!(inside.entity["url"], "{{base-url}}/users/7?x=1");
        assert_eq!(
            inside.entity["headers"],
            json!({ "K": "V", "Content-Type": "application/json" }),
            "the folder says no auth"
        );
        assert_eq!(inside.entity["testScript"], "a\nbc.test('b')");

        let query = by_name("Query");
        assert_eq!((query.kind, query.folder.as_deref()), ("graphqlRequests", Some("/1/1")));
        assert_eq!(query.entity["endpointUrl"], "http://h/graphql");
        assert_eq!(query.entity["variables"], "{\"a\":1}");
        assert_eq!(by_name("Feed").kind, "sockets");

        let form = by_name("Form");
        assert_eq!(form.entity["url"], "http://h/form?k=v");
        assert_eq!(
            form.entity["bodyParts"][1],
            json!({ "name": "f", "kind": "file", "value": "", "fileName": "pic.png", "contentType": "" })
        );
    }

    #[test]
    fn collection_v2_0_shapes_are_read() {
        let col = json!({ "info": { "schema": "https://schema.getpostman.com/json/collection/v2.0.0/collection.json" },
            "item": [
                { "name": "Plain", "request": "http://h/plain" },
                { "name": "Old", "request": { "url": "http://h/old", "method": "PUT", "header": "A: b\nC: d",
                    "auth": { "type": "basic", "basic": { "username": "u", "password": "p" } },
                    "body": { "mode": "urlencoded", "urlencoded": [{ "key": "a b", "value": "x&y" }, { "key": "c" }] } } },
            ] });
        let out = parsed(&col);
        assert_eq!(out.drafts[0].entity["method"], "GET");
        let old = &out.drafts[1].entity;
        assert_eq!(old["body"], "a%20b=x%26y&c=");
        assert_eq!(
            old["headers"],
            json!({ "A": "b", "C": "d", "Content-Type": "application/x-www-form-urlencoded",
            "Authorization": "Basic dTpw" })
        );
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
        let scripts = Scripts::default().with_events(Some(&events));
        assert_eq!(scripts.pre, ["bc.environment.set('a', npm.pm.x);"]);
        assert_eq!(
            scripts.test,
            ["bc.test('ok', () => bc.expect(bc.response.code).to.eql(200));\n\
             if(bc.globals.has('rpm.limit')) x=[bc.info, bc.response.code];"]
        );
    }

    #[test]
    fn a_non_collection_is_rejected() {
        assert_eq!(
            parse_postman_requests(&ImportSource::text("{}", "c.json")).err().unwrap(),
            "Error: Not a Postman collection"
        );
        assert!(parse_postman_mocks("nope").err().unwrap().starts_with("SyntaxError"));
    }

    fn zipped(files: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, text) in files {
            writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(text.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn a_v3_tree_brings_every_protocol() {
        let bytes = zipped(&[
            (
                "postman/collections/Shop/.resources/definition.yaml",
                "$kind: collection\nname: Shop API\nvariables:\n  - key: host\n    value: shop.test\nauth:\n  type: bearer\n  credentials:\n    - key: token\n      value: '{{token}}'\n",
            ),
            (
                "postman/collections/Shop/list.request.yaml",
                "$kind: http-request\nmethod: POST\nurl: 'https://{{host}}/items/:id'\norder: 2000\npathVariables:\n  - key: id\n    value: '4'\nqueryParams:\n  - key: page\n    value: '2'\nbody:\n  type: json\n  content: |-\n    {\"a\":1}\nscripts:\n  - type: afterResponse\n    code: pm.test('ok')\n",
            ),
            (
                "postman/collections/Shop/rpc/say.request.yaml",
                "$kind: grpc-request\nname: Say hello\nurl: grpc.shop.test:443\nmethodPath: shop.Greeter/SayHello\nmessage:\n  content: '{\"name\":\"a\"}'\nmetadata:\n  - key: x-trace\n    value: '1'\n",
            ),
            (
                "postman/collections/Shop/rpc/feed.request.yaml",
                "$kind: websocket-request\nurl: wss://shop.test/feed\norder: 1\n",
            ),
            (
                "postman/collections/Shop/rpc/chat.request.yaml",
                "$kind: socket.io-request\nurl: https://shop.test\nevents:\n  - name: message\n    subscribeOnConnect: true\n  - name: quiet\n    subscribeOnConnect: false\nsettings:\n  path: /io\n",
            ),
            (
                "postman/collections/Shop/rpc/tools.request.yaml",
                "$kind: mcp-request\ntransport: stdio\ncommand: npx server\nenv:\n  - key: A\n    value: b\n",
            ),
            (
                "postman/collections/Shop/rpc/q.request.yaml",
                "$kind: graphql-request\nurl: https://shop.test/graphql\nquery: '{ me }'\n",
            ),
            ("postman/collections/Shop/rpc/topic.request.yaml", "$kind: mqtt-request\nurl: mqtt://x\n"),
            ("postman/collections/Shop/.resources/list.resources/examples/ok.example.yaml", "$kind: http-example\n"),
        ]);
        let source = ImportSource { content: String::new(), filename: "shop.zip".into(), bytes };
        let out = parse_postman_requests(&source).unwrap();
        assert_eq!(out.skipped, 1, "MQTT has no request type here");
        assert_eq!(out.variables, [("host".to_string(), "shop.test".to_string())]);
        let folders: Vec<_> = out.folders.iter().map(|f| (f.name.as_str(), f.parent.as_deref())).collect();
        assert_eq!(folders, [("Shop API", None), ("rpc", Some("postman/collections/Shop"))]);
        let kind_of = |name: &str| out.drafts.iter().find(|d| d.entity["name"] == name).map(|d| d.kind);
        let entity = |name: &str| &out.drafts.iter().find(|d| d.entity["name"] == name).unwrap().entity;

        assert_eq!(kind_of("list"), Some("requests"));
        assert_eq!(entity("list")["url"], "https://{{host}}/items/4?page=2");
        assert_eq!(
            entity("list")["headers"],
            json!({ "Content-Type": "application/json", "Authorization": "Bearer {{token}}" })
        );
        assert_eq!(entity("list")["testScript"], "bc.test('ok')");

        let grpc = entity("Say hello");
        assert_eq!(kind_of("Say hello"), Some("grpcRequests"));
        assert_eq!(
            (&grpc["serverAddress"], &grpc["serviceName"], &grpc["methodName"]),
            (&json!("grpc.shop.test:443"), &json!("shop.Greeter"), &json!("SayHello"))
        );
        assert_eq!(grpc["requestBody"], "{\"name\":\"a\"}");
        assert_eq!(grpc["metadata"], json!({ "x-trace": "1", "Authorization": "Bearer {{token}}" }));
        assert_eq!(grpc["useReflection"], true);

        assert_eq!(kind_of("feed"), Some("sockets"));
        assert_eq!(entity("chat")["listeners"], json!([{ "event": "message", "description": "" }]));
        assert_eq!(entity("chat")["path"], "/io");
        assert_eq!(entity("tools")["transport"], "stdio");
        assert_eq!(entity("tools")["headers"], json!({ "A": "b" }));
        assert_eq!(kind_of("q"), Some("graphqlRequests"));
        assert_eq!(out.drafts[1].entity["name"], "feed", "requests of a folder follow their order");
    }

    #[test]
    fn one_v3_request_file_is_one_request() {
        let source = ImportSource::text("$kind: grpc-request\nurl: h:1\nmethodPath: /a.B/C\n", "Call it.request.yaml");
        let out = parse_postman_requests(&source).unwrap();
        assert_eq!(out.drafts[0].entity["name"], "Call it");
        assert_eq!(out.drafts[0].entity["serviceName"], "a.B");
    }

    #[test]
    fn a_saved_response_is_a_mock() {
        let text = json!({ "info": { "schema": SCHEMA }, "item": [
            { "name": "x", "request": { "method": "post", "url": "http://h/p" } },
            { "name": "M", "request": { "method": "GET", "url": "http://h/q", "body": { "mode": "raw", "raw": "{\"a\":1}" } },
              "response": [{ "code": 201, "header": [{ "key": "X-R", "value": "y" }], "body": "{\"ok\":true}" }],
              "_bifurc": { "urlPattern": "/users/\\d+", "useRegex": true, "enabled": true } },
        ] })
        .to_string();
        let (_, parsed) = parse_postman_mocks(&text).unwrap();
        assert_eq!(parsed[0].method, "POST");
        assert_eq!(parsed[0].enabled, json!(false));
        assert_eq!(parsed[0].response_status, json!(200));
        assert_eq!(parsed[0].response_body, json!("{}"));
        assert_eq!(parsed[1].url_pattern, "/users/\\d+");
        assert_eq!(parsed[1].use_regex, json!(true));
        assert_eq!(parsed[1].captured_body, "eyJhIjoxfQ==");
        assert_eq!(parsed[1].response_status, json!(201));
        assert_eq!(parsed[1].response_headers, json!({ "X-R": "y" }));
    }
}
