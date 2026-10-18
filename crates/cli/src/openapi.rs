//! `openapi infer`: an OpenAPI 3.0.3 document built from the workspace's saved REST requests, their recorded
//! and captured answers, and the enabled mocks. `contract check --openapi` reads it back and pairs it with the
//! same requests: paths are relative to `servers[0]`, only whole-segment variables are templated, and every
//! object lists its `required` properties (the engine's importer holds a mock to those).

use std::collections::{BTreeMap, HashSet};

use serde_json::{Map, Value, json};

use crate::{
    capture::mock_side,
    commands::{Context, workspace_id},
    contract::{method_matches, path_of, paths_match},
    error::{CliError, Result},
    infer::{Ty, infer, pascal},
    requests::{TYPES, folder_paths, saved, text},
    response::reason,
    shape::Side,
    types::{capture_sides, history_sides, recent_captures},
};

pub fn openapi(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    match sub {
        Some("infer") => infer_command(ctx),
        other => Err(CliError::usage(format!(
            "openapi needs a verb: infer{} (see: bifurc openapi --help)",
            other.map(|o| format!(", not \"{o}\"")).unwrap_or_default()
        ))),
    }
}

/// A JSON Schema (OpenAPI 3.0 flavour) for an inferred type.
fn schema(ty: &Ty) -> Value {
    match ty {
        Ty::Unknown => json!({}),
        Ty::Null => json!({ "nullable": true }),
        Ty::Bool => json!({ "type": "boolean" }),
        Ty::Int => json!({ "type": "integer" }),
        Ty::Num => json!({ "type": "number" }),
        Ty::Str => json!({ "type": "string" }),
        Ty::Array(element) => json!({ "type": "array", "items": schema(element) }),
        Ty::Object(fields) => {
            let mut out = json!({ "type": "object" });
            let properties: Map<String, Value> = fields.iter().map(|f| (f.name.clone(), schema(&f.ty))).collect();
            out["properties"] = Value::Object(properties);
            // shortcut: an empty `required` is left out (3.0 forbids it), and the importer then holds a mock to
            // every property; upgrade by letting the importer read an absent list as "none" for these files.
            let required: Vec<&str> = fields.iter().filter(|f| !f.optional).map(|f| f.name.as_str()).collect();
            if !required.is_empty() {
                out["required"] = json!(required);
            }
            out
        }
        Ty::Union(members) => {
            let nullable = members.contains(&Ty::Null);
            let mut rest: Vec<Value> = members.iter().filter(|m| **m != Ty::Null).map(schema).collect();
            let mut out = if rest.len() == 1 { rest.remove(0) } else { json!({ "oneOf": rest }) };
            if nullable {
                out["nullable"] = json!(true);
            }
            out
        }
    }
}

/// `{{id}}` and `:id` become `{id}`; a name is cut to letters, digits and `_`, and made unique in the path.
fn templated(path: &str) -> String {
    let mut seen: HashSet<String> = HashSet::new();
    let mut unique = |raw: &str| {
        let clean: String = raw.trim().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        let base = match clean.trim_matches('_') {
            "" => "param".to_string(),
            name => name.to_string(),
        };
        let (mut candidate, mut n) = (base.clone(), 2);
        while !seen.insert(candidate.clone()) {
            candidate = format!("{base}{n}");
            n += 1;
        }
        candidate
    };
    let segments: Vec<String> = path
        .split('/')
        .map(|segment| {
            if let Some(var) = segment.strip_prefix(':').filter(|v| !v.is_empty()) {
                return format!("{{{}}}", unique(var));
            }
            let (mut out, mut rest) = (String::new(), segment);
            while let Some(start) = rest.find("{{") {
                let Some(end) = rest[start..].find("}}") else { break };
                out.push_str(&rest[..start]);
                out.push_str(&format!("{{{}}}", unique(&rest[start + 2..start + end])));
                rest = &rest[start + end + 2..];
            }
            out + rest
        })
        .collect();
    segments.join("/")
}

/// The path with every variable segment as `{}`: two spellings of one path share it.
fn key_of(path: &str) -> String {
    path.split('/').map(|s| if s.contains('{') { "{}" } else { s }).collect::<Vec<_>>().join("/")
}

/// The scheme and host of a URL, when it has them (a `{{baseUrl}}` variable is no server).
// shortcut: a leading variable base gives no `servers` entry (the importer reads such a url as a base path);
// upgrade by emitting it as a server variable once the importer resolves workspace variables.
fn server_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    (!host.is_empty() && !scheme.contains(['{', '/'])).then(|| format!("{scheme}://{host}"))
}

fn query_names(url: &str) -> Vec<String> {
    let query = url.split('#').next().unwrap_or_default().split_once('?').map_or("", |(_, q)| q);
    let mut names: Vec<String> = Vec::new();
    for pair in query.split('&') {
        let name = pair.split('=').next().unwrap_or_default();
        if !name.is_empty() && !name.contains("{{") && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    names
}

fn camel(name: &str) -> String {
    let p = pascal(name);
    let mut chars = p.chars();
    chars.next().map(|first| first.to_ascii_lowercase().to_string() + chars.as_str()).unwrap_or_default()
}

/// The media type that a short type (`json`, `html`, ...) stands for.
fn media_of(short: &str) -> Option<String> {
    Some(
        match short {
            "" => return None,
            "json" => "application/json",
            "html" => "text/html",
            "text" => "text/plain",
            "xml" => "application/xml",
            "js" => "application/javascript",
            "css" => "text/css",
            "form" => "application/x-www-form-urlencoded",
            "binary" => "application/octet-stream",
            other => return Some(format!("application/{other}")),
        }
        .to_string(),
    )
}

struct Op {
    method: String,
    /// `/orders/{id}`, as the document writes it.
    path: String,
    /// The path as a request spells it, for pairing with mocks.
    raw: String,
    name: String,
    query: Vec<String>,
    /// The request body: its media type and the JSON it parsed to.
    body: Option<(String, Option<Value>)>,
    samples: Vec<Side>,
}

fn header(entity: &Value, name: &str) -> String {
    entity
        .get("headers")
        .and_then(Value::as_object)
        .and_then(|h| h.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)))
        .and_then(|(_, v)| v.as_str())
        .unwrap_or_default()
        .to_string()
}

fn request_body(request: &Value) -> Option<(String, Option<Value>)> {
    let body = text(request, "body");
    if body.trim().is_empty() {
        return None;
    }
    let parsed: Option<Value> = serde_json::from_str(&body).ok();
    let declared = header(request, "content-type");
    let media = match (&parsed, declared.split(';').next().unwrap_or_default().trim()) {
        (_, d) if d.contains("json") => d.to_string(),
        (Some(_), "") => "application/json".to_string(),
        (_, "") => return None,
        (_, d) => d.to_string(),
    };
    let parsed = parsed.filter(|_| media.contains("json"));
    Some((media, parsed))
}

/// The operation object of one `Op`; adds the responses it lists to `listed`.
fn operation(op: &Op, ids: &mut HashSet<String>, listed: &mut usize) -> Value {
    let mut id = camel(&op.name);
    if id.is_empty() {
        id = camel(&format!("{} {}", op.method, op.path));
    }
    let (base, mut n) = (id.clone(), 2);
    while !ids.insert(id.clone()) {
        id = format!("{base}{n}");
        n += 1;
    }
    let mut out = json!({ "operationId": id, "summary": op.name });
    let mut parameters: Vec<Value> = op
        .path
        .split('/')
        .filter_map(|s| s.strip_prefix('{').and_then(|r| r.strip_suffix('}')))
        .map(|name| json!({ "name": name, "in": "path", "required": true, "schema": { "type": "string" } }))
        .collect();
    parameters.extend(
        op.query
            .iter()
            .map(|name| json!({ "name": name, "in": "query", "required": false, "schema": { "type": "string" } })),
    );
    if !parameters.is_empty() {
        out["parameters"] = json!(parameters);
    }
    if let Some((media, parsed)) = &op.body {
        let content = match parsed {
            Some(value) => json!({ "schema": schema(&infer(std::slice::from_ref(value))) }),
            None => json!({}),
        };
        out["requestBody"] = json!({ "content": { media.as_str(): content } });
    }

    let mut by_status: BTreeMap<u16, Vec<&Side>> = BTreeMap::new();
    for side in op.samples.iter().filter(|s| s.status >= 100) {
        by_status.entry(side.status).or_default().push(side);
    }
    let mut responses = Map::new();
    for (status, sides) in &by_status {
        let description = match reason(*status) {
            "" => format!("status {status}"),
            known => known.to_string(),
        };
        let mut response = json!({ "description": description });
        let bodies: Vec<Value> = sides.iter().filter_map(|s| s.body.clone()).collect();
        if !bodies.is_empty() {
            response["content"] = json!({ "application/json": { "schema": schema(&infer(&bodies)) } });
        } else if let Some(media) = sides.iter().find_map(|s| media_of(&s.content_type)) {
            response["content"] = json!({ media: {} });
        }
        responses.insert(status.to_string(), response);
    }
    *listed += responses.len();
    if responses.is_empty() {
        responses.insert("default".into(), json!({ "description": "no recorded response" }));
    }
    out["responses"] = Value::Object(responses);
    out
}

fn add_server(url: &str, servers: &mut Vec<String>) {
    if let Some(server) = server_of(url).filter(|s| !servers.contains(s)) {
        servers.push(server);
    }
}

/// The saved REST requests, in `folder` and below when given.
fn requests_in(ctx: &mut Context, workspace: &str, folder: Option<&str>) -> Result<Vec<Value>> {
    let folders = folder_paths(ctx, "requestFolders")?;
    let wanted = folder.map(|f| f.trim_matches('/').to_lowercase());
    Ok(saved(ctx, workspace, &[&TYPES[0]])?
        .into_iter()
        .map(|(_, r)| r)
        .filter(|r| {
            let path = folders.get(&text(r, "folderId")).map(|p| p.to_lowercase()).unwrap_or_default();
            wanted.as_deref().is_none_or(|f| path.trim_end_matches('/') == f || path.starts_with(&format!("{f}/")))
        })
        .collect())
}

fn infer_command(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["folder", "out", "title"], 0)?;
    let args = ctx.args;
    let workspace = workspace_id(ctx)?;
    let folder = args.flag("folder");
    let requests = requests_in(ctx, &workspace, folder)?;

    let (mut ops, mut servers): (Vec<Op>, Vec<String>) = (Vec::new(), Vec::new());
    let captures = if requests.is_empty() { Value::Null } else { recent_captures(ctx, &workspace)? };
    for request in &requests {
        let (method, url) = (text(request, "method").to_uppercase(), text(request, "url"));
        add_server(&url, &mut servers);
        let raw = path_of(&url);
        let path = templated(&raw);
        let at = match ops.iter().position(|o| o.method == method && key_of(&o.path) == key_of(&path)) {
            Some(at) => at,
            None => {
                let (name, body) = (text(request, "name"), request_body(request));
                ops.push(Op { method, path, raw, name, query: Vec::new(), body, samples: Vec::new() });
                ops.len() - 1
            }
        };
        let mut samples = history_sides(ctx, &workspace, &text(request, "id"))?;
        samples.extend(capture_sides(ctx, &workspace, &captures, request, |_| true)?);
        ops[at].samples.extend(samples);
        for name in query_names(&url) {
            if !ops[at].query.contains(&name) {
                ops[at].query.push(name);
            }
        }
    }

    // shortcut: a regex mock is skipped (its pattern is JavaScript's and the CLI has no regex engine), and so is
    // a mock for any method (`*`) that matches no saved request; with --folder only the mocks of the selected
    // requests count. Upgrade the first with a regex dependency.
    let mocks =
        if requests.is_empty() && folder.is_some() { Vec::new() } else { crate::entities::all_mocks(ctx, &workspace)? };
    for mock in mocks.iter().filter(|m| m["enabled"] == json!(true) && m["useRegex"] != json!(true)) {
        let (method, pattern) = (text(mock, "method").to_uppercase(), text(mock, "urlPattern"));
        let raw = path_of(&pattern);
        let mut hits: Vec<usize> = (0..ops.len())
            .filter(|&i| method_matches(&method, &ops[i].method) && paths_match(&raw, &ops[i].raw))
            .collect();
        if hits.is_empty() && folder.is_none() && method != "*" {
            add_server(&pattern, &mut servers);
            let path = templated(&raw);
            ops.push(Op {
                method,
                path,
                raw,
                name: text(mock, "name"),
                query: Vec::new(),
                body: None,
                samples: Vec::new(),
            });
            hits.push(ops.len() - 1);
        }
        for i in hits {
            ops[i].samples.push(mock_side(mock));
        }
    }
    if ops.is_empty() {
        return Err(CliError::not_found(match folder {
            Some(f) => format!("no saved requests in folder \"{f}\" (see: bifurc requests list)"),
            None => "no saved requests or mocks yet; save one: bifurc curl <url> --save \"Name\"".into(),
        }));
    }

    ops.sort_by(|a, b| (&a.path, &a.method).cmp(&(&b.path, &b.method)));
    let (mut ids, mut responses) = (HashSet::new(), 0);
    let samples: usize = ops.iter().map(|o| o.samples.iter().filter(|s| s.status >= 100).count()).sum();
    let mut paths = Map::new();
    for op in &ops {
        let item = paths.entry(op.path.clone()).or_insert_with(|| json!({}));
        item[op.method.to_lowercase()] = operation(op, &mut ids, &mut responses);
    }
    let mut document = json!({ "openapi": "3.0.3", "info": { "title": args.flag("title").unwrap_or("Bifurc API"), "version": "1.0.0" } });
    if !servers.is_empty() {
        document["servers"] = json!(servers.iter().map(|url| json!({ "url": url })).collect::<Vec<_>>());
    }
    document["paths"] = Value::Object(paths);
    let rendered = serde_json::to_string_pretty(&document).unwrap_or_default();

    let saved_to = match args.flag("out") {
        Some(path) => {
            if let Some(dir) = std::path::Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir).map_err(|e| CliError::msg(format!("cannot write {path}: {e}")))?;
            }
            std::fs::write(path, &rendered).map_err(|e| CliError::msg(format!("cannot write {path}: {e}")))?;
            Some(path)
        }
        None => None,
    };
    let count = ops.len();
    let result = json!({ "schemaVersion": 1, "operations": count, "responses": responses, "samples": samples, "document": document });
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    Ok(ctx.print(&result, || match saved_to {
        Some(path) => format!(
            "saved {path} · {} · {} from {}",
            plural(count, "operation"),
            plural(responses, "response"),
            plural(samples, "sample")
        ),
        None => rendered.clone(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, run};

    fn go(argv: &[&str], answers: Vec<Answer>) -> (Result<String>, Vec<(String, Value)>) {
        run(argv, answers, |ctx| {
            let sub = ctx.args.positional(1).map(str::to_string);
            openapi(ctx, sub.as_deref())
        })
    }

    fn mock(id: &'static str, method: &str, url: &str, status: u16, body: &str, extra: Value) -> Answer {
        let key: &'static str = Box::leak(format!("entity.load:mocks:{id}").into_boxed_str());
        let mut m = json!({"id": id, "name": format!("Mock {method}"), "method": method, "urlPattern": url, "enabled": true,
            "responseStatus": status, "responseBody": body, "responseHeaders": {"Content-Type": "application/json"}});
        m.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        (key, Ok(json!({"ok": true, "entity": m})))
    }

    fn engine() -> Vec<Answer> {
        let history = |entries: Value| Ok(json!({ "entries": entries }));
        let json_h = json!({"content-type": "application/json"});
        vec![
            (
                "config.get",
                Ok(
                    json!({"activeWorkspaceId": "w1", "workspaces": [{"id": "w1", "name": "Default"}], "requestFolders": [
                    {"id": "f1", "name": "shop", "parentId": null}, {"id": "f2", "name": "ops", "parentId": null}]}),
                ),
            ),
            (
                "entity.list:requests",
                Ok(json!({"entities": [
                    {"id": "r1", "name": "Get orders", "method": "GET", "url": "{{baseUrl}}/orders/{{id}}?expand=1", "folderId": "f1"},
                    {"id": "r2", "name": "Make order", "method": "POST", "url": "http://127.0.0.1:18296/orders", "folderId": "f1",
                     "headers": {"Content-Type": "application/json"}, "body": "{\"item\":\"a\",\"qty\":2}"},
                    {"id": "r3", "name": "Ping", "method": "GET", "url": "{{baseUrl}}/ping", "folderId": "f2"},
                    {"id": "r4", "name": "Get order again", "method": "GET", "url": "{{baseUrl}}/orders/:orderId", "folderId": "f1"}]})),
            ),
            (
                "response.history::r1",
                history(json!([
                    {"status": 200, "headers": json_h, "body": r#"{"id":1,"note":null,"n":1}"#},
                    {"status": 200, "headers": json_h, "body": r#"{"id":2,"total":1.5,"n":2.5}"#},
                    {"status": 404, "headers": json_h, "body": r#"{"error":"x"}"#}])),
            ),
            (
                "response.history::r2",
                history(json!([{"status": 201, "headers": {"content-type": "text/html"}, "body": "<p>ok</p>"}])),
            ),
            ("response.history", history(json!([]))),
            ("capture.list", Ok(json!({"entries": []}))),
            ("entity.list:mocks", Ok(json!({"entities": [{"id": "m1"}, {"id": "m2"}, {"id": "m3"}, {"id": "m4"}]}))),
            mock("m1", "GET", "http://h/orders/{{id}}", 200, r#"{"id":3,"tags":["a"]}"#, json!({})),
            mock("m2", "DELETE", "/orders/{{id}}", 204, "", json!({"responseHeaders": {}})),
            mock("m3", "GET", "/re.*", 200, "{}", json!({"useRegex": true})),
            mock("m4", "GET", "/off", 200, "{}", json!({"enabled": false})),
        ]
    }

    fn document(argv: &[&str]) -> Value {
        serde_json::from_str(&go(argv, engine()).0.unwrap()).unwrap()
    }

    #[test]
    fn json_schema_has_required_nullable_integer_union_array_and_unknown() {
        let samples = [
            json!({"id": 1, "n": 1, "note": null, "tags": ["a"], "mix": 1, "u": [], "x": 1}),
            json!({"id": 2, "n": 2.5, "note": "t", "tags": [], "mix": "s", "u": []}),
        ];
        let got = schema(&infer(&samples));
        assert_eq!(got["required"], json!(["id", "n", "note", "tags", "mix", "u"]));
        let p = &got["properties"];
        assert_eq!(p["id"], json!({"type": "integer"}));
        assert_eq!(p["n"], json!({"type": "number"}));
        assert_eq!(p["note"], json!({"type": "string", "nullable": true}));
        assert_eq!(p["tags"], json!({"type": "array", "items": {"type": "string"}}));
        assert_eq!(p["mix"], json!({"oneOf": [{"type": "string"}, {"type": "integer"}]}));
        assert_eq!(p["u"], json!({"type": "array", "items": {}}));
        assert_eq!(p["x"], json!({"type": "integer"}));
        assert_eq!(schema(&infer(&[json!(null)])), json!({"nullable": true}));
        assert_eq!(schema(&infer(&[])), json!({}));
    }

    #[test]
    fn paths_are_templated_and_two_spellings_share_a_key() {
        assert_eq!(templated("/orders/{{id}}/items/:item"), "/orders/{id}/items/{item}");
        assert_eq!(templated("/a/{{id}}/b/{{id}}"), "/a/{id}/b/{id2}");
        assert_eq!(templated("/v{{ver}}/x/{{$guid}}"), "/v{ver}/x/{guid}");
        assert_eq!(key_of("/orders/{id}"), key_of("/orders/{orderId}"));
        assert_eq!(server_of("http://127.0.0.1:18296/orders?x=1").as_deref(), Some("http://127.0.0.1:18296"));
        assert_eq!(server_of("{{baseUrl}}/orders"), None);
        assert_eq!(server_of("/orders"), None);
    }

    #[test]
    fn requests_and_mocks_make_one_operation_each_with_responses_by_status() {
        let doc = document(&["openapi", "infer"]);
        assert_eq!(doc["openapi"], "3.0.3");
        assert_eq!(doc["servers"], json!([{"url": "http://127.0.0.1:18296"}]));
        let paths = doc["paths"].as_object().unwrap();
        assert_eq!(paths.keys().collect::<Vec<_>>(), ["/orders", "/orders/{id}", "/ping"]);
        // Get orders + Get order again + the GET mock are one operation; the DELETE mock has no request.
        let get = &doc["paths"]["/orders/{id}"]["get"];
        assert_eq!(get["operationId"], "getOrders");
        assert_eq!(get["summary"], "Get orders");
        assert_eq!(
            get["parameters"],
            json!([
                {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}},
                {"name": "expand", "in": "query", "required": false, "schema": {"type": "string"}}])
        );
        let ok = &get["responses"]["200"]["content"]["application/json"]["schema"];
        assert_eq!(ok["required"], json!(["id"]));
        assert_eq!(ok["properties"]["note"], json!({"nullable": true}));
        assert_eq!(ok["properties"]["n"], json!({"type": "number"}));
        assert_eq!(ok["properties"]["tags"], json!({"type": "array", "items": {"type": "string"}}));
        assert_eq!(get["responses"]["404"]["description"], "Not Found");
        assert_eq!(doc["paths"]["/orders/{id}"]["delete"]["responses"]["204"], json!({"description": "No Content"}));
        let post = &doc["paths"]["/orders"]["post"];
        assert_eq!(post["requestBody"]["content"]["application/json"]["schema"]["required"], json!(["item", "qty"]));
        assert_eq!(post["responses"]["201"], json!({"description": "Created", "content": {"text/html": {}}}));
        assert_eq!(
            doc["paths"]["/ping"]["get"]["responses"],
            json!({"default": {"description": "no recorded response"}})
        );
        assert!(get["responses"].get("default").is_none());
        // The regex and the disabled mock are left out.
        assert!(paths.keys().all(|p| !p.contains("re.") && p != "/off"));
    }

    #[test]
    fn a_folder_keeps_its_requests_and_the_mocks_of_their_paths_only() {
        let doc = document(&["openapi", "infer", "--folder", "shop", "--title", "Shop"]);
        assert_eq!(doc["info"]["title"], "Shop");
        let paths = doc["paths"].as_object().unwrap();
        assert_eq!(paths.keys().collect::<Vec<_>>(), ["/orders", "/orders/{id}"]);
        assert!(doc["paths"]["/orders/{id}"].get("delete").is_none());
        assert_eq!(document(&["openapi", "infer", "--folder", "ops"])["paths"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn out_writes_the_file_and_json_wraps_the_document() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("spec/openapi.json");
        let out = go(&["openapi", "infer", "--out", file.to_str().unwrap()], engine()).0.unwrap();
        assert_eq!(out, format!("saved {} · 4 operations · 4 responses from 6 samples", file.display()));
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(written["openapi"], "3.0.3");
        let wrapped: Value = serde_json::from_str(&go(&["openapi", "infer", "--json"], engine()).0.unwrap()).unwrap();
        assert_eq!(
            (wrapped["schemaVersion"].clone(), wrapped["operations"].clone(), wrapped["samples"].clone()),
            (json!(1), json!(4), json!(6))
        );
        assert_eq!(wrapped["document"], written);
    }

    #[test]
    fn nothing_to_describe_is_exit_3_and_a_bad_verb_or_option_is_exit_2() {
        let mut empty: Vec<Answer> = engine().into_iter().filter(|(name, _)| *name != "entity.list:requests").collect();
        empty.push(("entity.list:requests", Ok(json!({"entities": []}))));
        let err = go(&["openapi", "infer", "--folder", "shop"], empty).0.unwrap_err();
        assert_eq!(err.code(), 3);
        assert!(err.to_string().contains("no saved requests in folder \"shop\""), "{err}");
        assert_eq!(go(&["openapi", "infer", "--folder", "nope"], engine()).0.unwrap_err().code(), 3);
        assert_eq!(go(&["openapi"], engine()).0.unwrap_err().code(), 2);
        assert_eq!(go(&["openapi", "infer", "--bogus=1"], engine()).0.unwrap_err().code(), 2);
    }
}
