//! What every request importer ends in: the requests of another tool as Bifurc requests of the right type,
//! and the one way they are stored.
//!
//! An importer reads its format into a [`Collection`]: folders, [`Draft`]s (a request with no id yet) and the
//! variables the file defines. [`store`] writes it into an import folder of the workspace.

use std::collections::HashSet;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Map, Value, json};

use super::{
    support::{FolderSpec, ImportRoot, add_import_folders, commit_config, io_failure, upsert_by_id},
    types::{ImportOutcome, Imported},
};
use crate::{
    kernel::kinds::to_engine_kind,
    rest::from_curl::request_entity,
    utils::{
        curl::{CurlPart, CurlRequest},
        id::{generate_id, now_ms},
        json::{array_of, object_mut, str_of},
    },
    workspace::{
        config::{GLOBAL_ENV_ID, load_config},
        fs::{default_global_environment, read_entity, write_entity, write_flat_entity},
    },
};

/// A request read from a file: the store's kind for it and its fields, without an id, a folder or a workspace.
pub struct Draft {
    pub kind: &'static str,
    pub entity: Value,
    /// The key of the [`FolderSpec`] it is in; `None` puts it at the top of what the file brings.
    pub folder: Option<String>,
}

/// Everything one file brings.
#[derive(Default)]
pub struct Collection {
    /// The file's own name for itself. When it has one, everything goes in a folder of that name.
    pub name: String,
    pub folders: Vec<FolderSpec>,
    pub drafts: Vec<Draft>,
    /// Variables the file defines for its requests, in order.
    pub variables: Vec<(String, String)>,
    /// Requests of a type Bifurc has no counterpart for.
    pub skipped: usize,
}

/// An HTTP call as a source file describes it. [`Http::draft`] decides which request type it is.
#[derive(Default)]
pub struct Http {
    pub name: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub parts: Vec<CurlPart>,
    /// `(query, variables, operationName)` when the source says outright that this is a GraphQL call.
    pub graphql: Option<(String, String, String)>,
    pub pre_script: String,
    pub test_script: String,
    pub created_at: Option<Value>,
}

impl Http {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// Add a header unless the call already has one of that name.
    pub fn default_header(&mut self, name: &str, value: &str) {
        if self.header(name).is_none() {
            self.headers.push((name.to_string(), value.to_string()));
        }
    }

    pub fn draft(mut self, folder: Option<String>) -> Draft {
        if self.method.is_empty() {
            self.method = "GET".to_string();
        }
        let (kind, mut entity) = match self.graphql.take() {
            Some((query, variables, operation)) => {
                let headers: Map<String, Value> = self
                    .headers
                    .iter()
                    .filter(|(name, _)| !name.eq_ignore_ascii_case("content-type"))
                    .map(|(name, value)| (name.clone(), json!(value)))
                    .collect();
                let entity = json!({
                    "name": "", "protocol": "graphql", "endpointUrl": self.url, "operationName": operation,
                    "query": query, "variables": variables, "headers": headers,
                });
                ("graphqlRequests", entity)
            }
            None => {
                let call = CurlRequest {
                    method: self.method.to_uppercase(),
                    url: self.url.clone(),
                    headers: self.headers.clone(),
                    body: self.body.clone(),
                    parts: self.parts.clone(),
                };
                let (kind, mut entity) = request_entity(&call);
                // A cURL command with data and no type is a form; a file that names no type means none.
                if self.header("content-type").is_none() && self.parts.is_empty() {
                    if let Some(headers) = entity.get_mut("headers").and_then(Value::as_object_mut) {
                        headers.shift_remove("Content-Type");
                    }
                }
                (kind, entity)
            }
        };
        let fields = object_mut(&mut entity);
        if !self.name.is_empty() {
            fields.insert("name".into(), json!(self.name));
        }
        if !self.pre_script.trim().is_empty() {
            fields.insert("preScript".into(), json!(self.pre_script));
        }
        if !self.test_script.trim().is_empty() {
            fields.insert("testScript".into(), json!(self.test_script));
        }
        if let Some(created_at) = self.created_at.filter(|at| at.is_number()) {
            fields.insert("createdAt".into(), created_at);
        }
        Draft { kind: to_engine_kind(kind), entity, folder }
    }
}

/// `pkg.Service/Method`, with or without the leading slash, as the service and the method.
pub fn grpc_method(path: &str) -> (String, String) {
    match path.trim().trim_start_matches('/').rsplit_once('/') {
        Some((service, method)) => (service.to_string(), method.to_string()),
        None => (String::new(), path.trim().to_string()),
    }
}

/// A gRPC request. The file does not carry the `.proto`, so the method is found by server reflection.
pub fn grpc_draft(name: &str, address: &str, method_path: &str, message: &str, metadata: Map<String, Value>) -> Value {
    let (service, method) = grpc_method(method_path);
    json!({
        "name": if name.is_empty() { method.as_str() } else { name },
        "protocol": "grpc", "serverAddress": address, "serviceName": service, "methodName": method,
        "requestBody": if message.trim().is_empty() { "{}" } else { message },
        "metadata": metadata, "protoFileId": null, "useReflection": true, "streamingType": "unary",
    })
}

/// A header record from `(name, value)` pairs.
pub fn header_record(pairs: &[(String, String)]) -> Map<String, Value> {
    pairs.iter().map(|(name, value)| (name.clone(), json!(value))).collect()
}

// ── authorization ──────────────────────────────────────────────────────────────────────────────────────

/// What an authorization setting adds to a call.
#[derive(Default)]
pub struct AuthParts {
    pub headers: Vec<(String, String)>,
    pub query: Vec<(String, String)>,
}

/// The headers and query parameters an authorization of type `kind` stands for. `field` reads one of its
/// settings by the name Postman gives it. A scheme that is computed per call (Digest, AWS, OAuth 1) has no
/// fixed header and gives nothing.
pub fn auth_parts(kind: &str, field: &dyn Fn(&str) -> String) -> AuthParts {
    let mut out = AuthParts::default();
    match kind.to_lowercase().as_str() {
        "bearer" => {
            let token = field("token");
            if !token.is_empty() {
                let prefix = Some(field("prefix")).filter(|p| !p.is_empty()).unwrap_or_else(|| "Bearer".to_string());
                out.headers.push(("Authorization".into(), format!("{prefix} {token}")));
            }
        }
        "basic" => {
            let (user, password) = (field("username"), field("password"));
            // A variable cannot be encoded here: it has no value yet.
            if !(user.is_empty() && password.is_empty()) && !user.contains("{{") && !password.contains("{{") {
                let encoded = STANDARD.encode(format!("{user}:{password}"));
                out.headers.push(("Authorization".into(), format!("Basic {encoded}")));
            }
        }
        "apikey" => {
            let (key, value) = (field("key"), field("value"));
            if !key.is_empty() {
                let place = field("in") + &field("addTo");
                if place.to_lowercase().contains("query") {
                    out.query.push((key, value));
                } else {
                    out.headers.push((key, value));
                }
            }
        }
        "oauth2" => {
            let token = field("accessToken");
            if !token.is_empty() {
                let prefix = Some(field("headerPrefix"))
                    .filter(|p| !p.trim().is_empty())
                    .unwrap_or_else(|| "Bearer".to_string());
                out.headers.push(("Authorization".into(), format!("{} {token}", prefix.trim())));
            }
        }
        _ => {}
    }
    out
}

// ── addresses and bodies ───────────────────────────────────────────────────────────────────────────────

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

/// `encodeURIComponent(text)`, with every `{{variable}}` in it left as it is.
pub fn encode_component(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        let Some(close) = rest[open..].find("}}") else { break };
        out.push_str(&utf8_percent_encode(&rest[..open], URI_COMPONENT).to_string());
        out.push_str(&rest[open..open + close + 2]);
        rest = &rest[open + close + 2..];
    }
    out.push_str(&utf8_percent_encode(rest, URI_COMPONENT).to_string());
    out
}

/// `a=1&b=2` of the pairs, encoded.
pub fn form_encoded(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", encode_component(key), encode_component(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// `url` with the query parameters it does not already name added to it.
pub fn with_query(url: &str, pairs: &[(String, String)]) -> String {
    let (base, fragment) = match url.split_once('#') {
        Some((base, fragment)) => (base, Some(fragment)),
        None => (url, None),
    };
    let named: HashSet<&str> = base
        .split_once('?')
        .map(|(_, query)| query.split('&').map(|pair| pair.split('=').next().unwrap_or(pair)).collect())
        .unwrap_or_default();
    let missing: Vec<(String, String)> = pairs
        .iter()
        .filter(|(key, _)| {
            !key.is_empty() && !named.contains(key.as_str()) && !named.contains(encode_component(key).as_str())
        })
        .cloned()
        .collect();
    if missing.is_empty() {
        return url.to_string();
    }
    let joined = format!("{base}{}{}", if base.contains('?') { "&" } else { "?" }, form_encoded(&missing));
    match fragment {
        Some(fragment) => format!("{joined}#{fragment}"),
        None => joined,
    }
}

/// `url` with each `:name` path segment replaced by the value given for it, or by `{{name}}` when it has none.
pub fn with_path_variables(url: &str, variables: &[(String, String)]) -> String {
    let (path, query) = match url.find(['?', '#']) {
        Some(at) => url.split_at(at),
        None => (url, ""),
    };
    let replaced: Vec<String> = path
        .split('/')
        .map(|segment| match segment.strip_prefix(':').and_then(|name| variables.iter().find(|(key, _)| key == name)) {
            Some((key, value)) if value.is_empty() => format!("{{{{{key}}}}}"),
            Some((_, value)) => value.clone(),
            None => segment.to_string(),
        })
        .collect();
    format!("{}{query}", replaced.join("/"))
}

// ── variables ──────────────────────────────────────────────────────────────────────────────────────────

/// Postman's generated values and the Bifurc generator that makes the same kind of thing.
const DYNAMIC_VARIABLES: [(&str, &str); 14] = [
    ("$guid", "random.uuid"),
    ("$randomUUID", "random.uuid"),
    ("$timestamp", "random.timestamp"),
    ("$isoTimestamp", "random.isoDateTime"),
    ("$randomInt", "random.int"),
    ("$randomEmail", "random.email"),
    ("$randomFirstName", "random.firstName"),
    ("$randomLastName", "random.lastName"),
    ("$randomFullName", "random.name"),
    ("$randomUserName", "random.username"),
    ("$randomPhoneNumber", "random.phone"),
    ("$randomCity", "random.city"),
    ("$randomCountry", "random.country"),
    ("$randomBoolean", "random.boolean"),
];

/// A variable's name as Bifurc can hold it: word characters only.
pub fn variable_name(name: &str) -> String {
    let name = name.trim();
    let name = name.strip_prefix("_.").unwrap_or(name);
    name.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

/// `text` with the variable references other tools write (`{{ name }}`, `{{ _.name }}`, `{{base-url}}`,
/// `{{$guid}}`) as Bifurc reads them. Anything that is not a plain reference is left alone.
pub fn bifurc_variables(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        let Some(close) = rest[open + 2..].find("}}") else { break };
        let inner = &rest[open + 2..open + 2 + close];
        out.push_str(&rest[..open]);
        let trimmed = inner.trim();
        let plain = !trimmed.is_empty()
            && trimmed.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '$'))
            && !trimmed.starts_with(['-', '.']);
        let dynamic = DYNAMIC_VARIABLES.iter().find(|(from, _)| *from == trimmed);
        let known = trimmed.starts_with("random.") || trimmed.starts_with("vault.");
        match dynamic {
            Some((_, to)) => out.push_str(&format!("{{{{{to}}}}}")),
            None if plain && !known && !trimmed.starts_with('$') => {
                out.push_str(&format!("{{{{{}}}}}", variable_name(trimmed)));
            }
            None => out.push_str(&rest[open..open + close + 4]),
        }
        rest = &rest[open + close + 4..];
    }
    out.push_str(rest);
    out
}

/// Every text of a request rewritten by [`bifurc_variables`]. Scripts are code, not templates, and are kept.
fn rewrite_variables(value: &mut Value) {
    match value {
        Value::String(text) if text.contains("{{") => *text = bifurc_variables(text),
        Value::Array(items) => items.iter_mut().for_each(rewrite_variables),
        Value::Object(fields) => {
            for (key, field) in fields.iter_mut() {
                if !matches!(key.as_str(), "preScript" | "postScript" | "testScript") {
                    rewrite_variables(field);
                }
            }
        }
        _ => {}
    }
}

/// Add the variables a file defines to the workspace's Global environment. One that is already there keeps
/// the value it has.
fn add_variables(cfg: &mut Value, ws_id: &str, variables: &[(String, String)]) -> Result<(), String> {
    let mut global =
        read_entity(ws_id, "environments", GLOBAL_ENV_ID).unwrap_or_else(|| default_global_environment(ws_id));
    let mut taken: HashSet<String> =
        array_of(&global, "variables").iter().map(|v| str_of(v, "key").to_string()).collect();
    let mut added = false;
    for (key, value) in variables {
        let key = variable_name(key);
        if key.is_empty() || !taken.insert(key.clone()) {
            continue;
        }
        let variable = json!({ "id": generate_id(), "key": key, "value": bifurc_variables(value) });
        crate::utils::json::array_mut(&mut global, "variables").push(variable);
        added = true;
    }
    if added {
        write_flat_entity(ws_id, "environments", GLOBAL_ENV_ID, &global).map_err(io_failure)?;
        upsert_by_id(cfg, "environments", global);
    }
    Ok(())
}

// ── storing ────────────────────────────────────────────────────────────────────────────────────────────

const COLLECTION_KEY: &str = "\0collection";

/// Write what a file brought into a new import folder of workspace `ws_id`.
pub fn store(ws_id: &str, collection: Collection) -> ImportOutcome {
    let done = |imported: usize| match collection.skipped {
        0 => Imported::all(imported),
        skipped => Imported::with_skipped(imported, skipped),
    };
    if collection.drafts.is_empty() {
        return Ok(done(0));
    }
    let mut cfg = load_config();
    let mut root = ImportRoot::new("requestFolders", "requests");
    let (root_id, _) = root.get(&mut cfg, ws_id)?;

    // A file with a name of its own gets a folder of that name, and its folders go inside it.
    let named = !collection.name.trim().is_empty();
    let known: HashSet<&str> = collection.folders.iter().map(|f| f.key.as_str()).collect();
    let mut specs: Vec<FolderSpec> = Vec::new();
    if named {
        specs.push(FolderSpec { key: COLLECTION_KEY.into(), name: collection.name.trim().to_string(), parent: None });
    }
    for folder in &collection.folders {
        let parent = match folder.parent.as_deref().filter(|p| known.contains(p)) {
            Some(parent) => Some(parent.to_string()),
            None => named.then(|| COLLECTION_KEY.to_string()),
        };
        specs.push(FolderSpec { key: folder.key.clone(), name: folder.name.clone(), parent });
    }
    let folder_ids = add_import_folders(&mut cfg, "requestFolders", "requests", ws_id, &root_id, &specs)?;
    let top = folder_ids.get(COLLECTION_KEY).cloned().unwrap_or_else(|| root_id.clone());

    let mut imported = 0;
    for draft in &collection.drafts {
        let id = generate_id();
        let folder_id =
            draft.folder.as_ref().and_then(|key| folder_ids.get(key)).cloned().unwrap_or_else(|| top.clone());
        let mut entity = draft.entity.clone();
        rewrite_variables(&mut entity);
        let fields = object_mut(&mut entity);
        fields.insert("id".into(), json!(id));
        if !fields.get("createdAt").is_some_and(Value::is_number) {
            fields.insert("createdAt".into(), json!(now_ms()));
        }
        fields.insert("folderId".into(), json!(folder_id));
        fields.insert("workspaceId".into(), json!(ws_id));
        write_entity(ws_id, draft.kind, &id, &entity, Some(&folder_id)).map_err(io_failure)?;
        imported += 1;
    }

    add_variables(&mut cfg, ws_id, &collection.variables)?;
    commit_config(&cfg)?;
    Ok(done(imported))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_references_are_rewritten_for_bifurc() {
        assert_eq!(bifurc_variables("{{ _.base_url }}/a/{{ id }}"), "{{base_url}}/a/{{id}}");
        assert_eq!(
            bifurc_variables("{{base-url}}/{{$guid}}/{{random.uuid}}"),
            "{{base_url}}/{{random.uuid}}/{{random.uuid}}"
        );
        assert_eq!(
            bifurc_variables("{{vault.key}} {{#if x}} {% tag %} {{ a | upper }}"),
            "{{vault.key}} {{#if x}} {% tag %} {{ a | upper }}"
        );
        assert_eq!(bifurc_variables("{{open"), "{{open");
    }

    #[test]
    fn addresses_take_query_and_path_values() {
        let pairs = vec![("a b".to_string(), "{{v}}&".to_string()), ("x".to_string(), "9".to_string())];
        assert_eq!(with_query("http://h/p?x=1#f", &pairs), "http://h/p?x=1&a%20b={{v}}%26#f");
        assert_eq!(with_query("http://h/p", &[]), "http://h/p");
        let vars = vec![("id".to_string(), "7".to_string()), ("org".to_string(), String::new())];
        assert_eq!(with_path_variables("http://h/:org/u/:id/:other?q=:id", &vars), "http://h/{{org}}/u/7/:other?q=:id");
    }

    #[test]
    fn authorization_becomes_headers_or_query() {
        let of = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string()).unwrap_or_default()
        };
        let bearer = auth_parts("bearer", &of(&[("token", "{{t}}")]));
        assert_eq!(bearer.headers, [("Authorization".to_string(), "Bearer {{t}}".to_string())]);
        let basic = auth_parts("basic", &of(&[("username", "u"), ("password", "p")]));
        assert_eq!(basic.headers[0].1, "Basic dTpw");
        assert!(auth_parts("basic", &of(&[("username", "{{u}}"), ("password", "p")])).headers.is_empty());
        let key = auth_parts("apikey", &of(&[("key", "k"), ("value", "v"), ("in", "query")]));
        assert_eq!((key.headers.len(), key.query.len()), (0, 1));
        assert!(auth_parts("digest", &of(&[("username", "u")])).headers.is_empty());
    }

    #[test]
    fn a_call_is_the_request_type_it_describes() {
        let call = |url: &str, headers: &[(&str, &str)], body: &str| Http {
            name: "N".into(),
            method: "post".into(),
            url: url.into(),
            headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            body: body.into(),
            ..Http::default()
        };
        assert_eq!(call("http://h/a", &[], "x").draft(None).kind, "requests");
        assert_eq!(call("wss://h/a", &[], "").draft(None).kind, "sockets");
        assert_eq!(call("http://h/a", &[("Accept", "text/event-stream")], "").draft(None).kind, "sseRequests");
        assert_eq!(call("http://h/a", &[("SOAPAction", "x")], "<a/>").draft(None).kind, "soapRequests");
        let graphql = call("http://h/g", &[("Content-Type", "application/json")], "{\"query\":\"{ a }\"}").draft(None);
        assert_eq!(
            (graphql.kind, &graphql.entity["name"], &graphql.entity["query"]),
            ("graphqlRequests", &json!("N"), &json!("{ a }"))
        );
        let rest = call("http://h/a", &[], "x").draft(None);
        assert_eq!((&rest.entity["method"], &rest.entity["name"]), (&json!("POST"), &json!("N")));
    }
}
