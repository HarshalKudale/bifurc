//! Insomnia exports read into Bifurc requests: the v4 resource list (JSON or YAML) and the v5 YAML tree, with
//! their HTTP, GraphQL, gRPC, WebSocket, Socket.IO and MCP requests.

use serde_json::{Value, json};

use super::postman::to_bifurc_script;
use crate::{
    importexport::{
        convert::{
            Collection, Draft, Http, auth_parts, form_encoded, grpc_draft, header_record, with_path_variables,
            with_query,
        },
        support::{FolderSpec, js_str, parse_any, present, text_or},
    },
    utils::{
        curl::CurlPart,
        json::{array_of, is_truthy, opt_str},
    },
};

pub const NOT_INSOMNIA: &str = "Not a valid Insomnia export file";

/// Folders nest this deep at most.
const MAX_DEPTH: usize = 64;

/// The enabled `{ name, value }` entries of a list, as pairs.
fn pairs(list: Option<&Value>) -> Vec<(String, String)> {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| !is_truthy(item.get("disabled").unwrap_or(&Value::Null)))
        .filter(|item| is_truthy(item.get("name").unwrap_or(&Value::Null)))
        .map(|item| (js_str(&item["name"]), text_or(item, "value", "")))
        .collect()
}

/// The headers and query parameters of an `authentication` object.
fn apply_auth(call: &mut Http, auth: Option<&Value>) {
    let Some(auth) = auth.filter(|a| a.is_object() && !is_truthy(a.get("disabled").unwrap_or(&Value::Null))) else {
        return;
    };
    let parts = auth_parts(&text_or(auth, "type", ""), &|name| text_or(auth, name, ""));
    for (name, value) in parts.headers {
        call.default_header(&name, &value);
    }
    call.url = with_query(&call.url, &parts.query);
}

fn apply_body(call: &mut Http, body: Option<&Value>) {
    let Some(body) = body.filter(|b| b.is_object()) else { return };
    let mime = text_or(body, "mimeType", "");
    let text = text_or(body, "text", "");
    let params = body.get("params");
    match mime.as_str() {
        "application/graphql" => {
            // The text is the JSON a GraphQL call posts.
            let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let variables = match parsed.get("variables") {
                Some(Value::String(text)) => text.clone(),
                Some(other) if !other.is_null() => serde_json::to_string_pretty(other).unwrap_or_default(),
                _ => String::new(),
            };
            let query = if parsed.is_object() { text_or(&parsed, "query", "") } else { text };
            call.graphql = Some((query, variables, text_or(&parsed, "operationName", "")));
        }
        "multipart/form-data" => {
            call.parts = array_of(body, "params")
                .iter()
                .filter(|p| {
                    !is_truthy(p.get("disabled").unwrap_or(&Value::Null))
                        && is_truthy(p.get("name").unwrap_or(&Value::Null))
                })
                .map(|p| {
                    let file = opt_str(p, "type") == Some("file");
                    let path = text_or(p, "fileName", "");
                    CurlPart {
                        name: js_str(&p["name"]),
                        value: if file { String::new() } else { text_or(p, "value", "") },
                        file,
                        file_name: path.rsplit(['/', '\\']).next().unwrap_or("").to_string(),
                        content_type: String::new(),
                    }
                })
                .collect();
        }
        "application/x-www-form-urlencoded" => {
            call.body = form_encoded(&pairs(params));
            call.default_header("Content-Type", &mime);
        }
        _ => {
            call.body = text;
            if !mime.is_empty() && !call.body.is_empty() {
                call.default_header("Content-Type", &mime);
            }
        }
    }
}

/// The type of a request: v4 names it in `_type`, v5 only in the prefix of its id.
#[derive(PartialEq)]
enum Kind {
    Http,
    Grpc,
    WebSocket,
    SocketIo,
}

fn kind_of(item: &Value, id: &str) -> Kind {
    match opt_str(item, "_type") {
        Some("grpc_request") => Kind::Grpc,
        Some("websocket_request") => Kind::WebSocket,
        Some("socketio_request") => Kind::SocketIo,
        Some(_) => Kind::Http,
        None if id.starts_with("greq_") || item.get("protoMethodName").is_some() => Kind::Grpc,
        None if id.starts_with("ws-req_") => Kind::WebSocket,
        None if id.starts_with("socketio-req_") => Kind::SocketIo,
        None => Kind::Http,
    }
}

/// One request of either version as a draft. `created` and the scripts sit in different places in the two.
fn request_draft(item: &Value, id: &str, folder: Option<String>) -> Draft {
    let meta = item.get("meta").unwrap_or(&Value::Null);
    let created = present(item, "created").or_else(|| present(meta, "created")).cloned();
    let mut call = Http {
        name: text_or(item, "name", ""),
        method: text_or(item, "method", "GET"),
        url: with_path_variables(
            &with_query(&text_or(item, "url", ""), &pairs(item.get("parameters"))),
            &pairs(item.get("pathParameters")),
        ),
        headers: pairs(item.get("headers")),
        created_at: created.clone(),
        ..Http::default()
    };
    apply_auth(&mut call, item.get("authentication"));
    let stamped = |mut entity: Value| {
        if let Some(created) = created.clone().filter(Value::is_number) {
            entity["createdAt"] = created;
        }
        entity
    };
    match kind_of(item, id) {
        Kind::Grpc => {
            let mut metadata = pairs(item.get("metadata"));
            metadata.extend(call.headers.clone());
            let message = item.get("body").map(|b| text_or(b, "text", "")).unwrap_or_default();
            let method = text_or(item, "protoMethodName", "");
            let entity = grpc_draft(&call.name, &call.url, &method, &message, header_record(&metadata));
            Draft { kind: "grpcRequests", entity: stamped(entity), folder }
        }
        Kind::WebSocket => {
            let entity = json!({ "name": call.name, "url": call.url, "headers": header_record(&call.headers) });
            Draft { kind: "sockets", entity: stamped(entity), folder }
        }
        Kind::SocketIo => {
            let listeners: Vec<Value> = array_of(item, "eventListeners")
                .iter()
                .map(|l| json!({ "event": text_or(l, "eventName", ""), "description": text_or(l, "desc", "") }))
                .collect();
            let path = item.get("settings").map(|s| text_or(s, "path", "")).unwrap_or_default();
            let entity = json!({
                "name": call.name, "url": call.url, "headers": header_record(&call.headers), "path": path,
                "namespace": "", "auth": "", "listeners": listeners,
            });
            Draft { kind: "socketioRequests", entity: stamped(entity), folder }
        }
        Kind::Http => {
            apply_body(&mut call, item.get("body"));
            let scripts = item.get("scripts").unwrap_or(&Value::Null);
            let script = |v5: &str, v4: &str| {
                to_bifurc_script(&present(scripts, v5).or_else(|| present(item, v4)).map(js_str).unwrap_or_default())
            };
            call.pre_script = script("preRequest", "preRequestScript");
            call.test_script = script("afterResponse", "afterResponseScript");
            call.draft(folder)
        }
    }
}

/// The scalar values of an environment's data, as variables.
fn variables_of(data: Option<&Value>) -> Vec<(String, String)> {
    data.and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, value)| value.is_string() || value.is_number() || value.is_boolean())
        .map(|(key, value)| (key.clone(), js_str(value)))
        .collect()
}

fn parse_v4(data: &Value) -> Collection {
    let resources = array_of(data, "resources");
    let of_type = |ty: &'static str| resources.iter().filter(move |r| opt_str(r, "_type") == Some(ty));
    let mut out = Collection {
        name: of_type("workspace").next().map(|w| text_or(w, "name", "")).unwrap_or_default(),
        ..Collection::default()
    };
    for group in of_type("request_group") {
        out.folders.push(FolderSpec {
            key: text_or(group, "_id", ""),
            name: text_or(group, "name", "Folder"),
            parent: opt_str(group, "parentId").filter(|p| !p.is_empty()).map(str::to_string),
        });
    }
    // The base environment first, so a sub-environment's value does not take its place.
    let mut environments: Vec<&Value> = of_type("environment").collect();
    environments.sort_by_key(|env| opt_str(env, "parentId").is_some_and(|p| p.starts_with("env_")));
    for environment in environments {
        out.variables.extend(variables_of(environment.get("data")));
    }
    for resource in resources {
        let ty = opt_str(resource, "_type").unwrap_or("");
        if !matches!(ty, "request" | "grpc_request" | "websocket_request" | "socketio_request") {
            continue;
        }
        let folder = opt_str(resource, "parentId").filter(|p| !p.is_empty()).map(str::to_string);
        out.drafts.push(request_draft(resource, &text_or(resource, "_id", ""), folder));
    }
    out
}

fn walk_v5(items: &[Value], parent: Option<&str>, depth: usize, out: &mut Collection) {
    if depth > MAX_DEPTH {
        return;
    }
    for (index, item) in items.iter().enumerate() {
        let id = item.get("meta").map(|meta| text_or(meta, "id", "")).unwrap_or_default();
        if let Some(children) = item.get("children").and_then(Value::as_array) {
            let key = format!("{}/{index}", parent.unwrap_or(""));
            out.folders.push(FolderSpec {
                key: key.clone(),
                name: text_or(item, "name", "Folder"),
                parent: parent.map(str::to_string),
            });
            out.variables.extend(variables_of(item.get("environment")));
            walk_v5(children, Some(&key), depth + 1, out);
        } else {
            out.drafts.push(request_draft(item, &id, parent.map(str::to_string)));
        }
    }
}

/// The MCP client of a `mcpClient.insomnia` file.
fn mcp_draft(file: &Value) -> Option<Draft> {
    let request = file.get("mcpRequest").filter(|r| r.is_object())?;
    let url = text_or(request, "url", "").trim().to_string();
    let http = url.starts_with("http://") || url.starts_with("https://");
    let name =
        Some(text_or(request, "name", "")).filter(|n| !n.is_empty()).unwrap_or_else(|| text_or(file, "name", ""));
    let record = header_record(&pairs(request.get(if http { "headers" } else { "env" })));
    let entity =
        json!({ "name": name, "transport": if http { "http" } else { "stdio" }, "url": url, "headers": record });
    Some(Draft { kind: "mcpRequests", entity, folder: None })
}

/// The requests, folders and variables of an Insomnia export.
pub fn parse_insomnia(text: &str) -> Result<Collection, String> {
    let file = parse_any(text)?;
    let format = opt_str(&file, "type").unwrap_or("");
    if format.starts_with("mcpClient.insomnia") {
        return Ok(Collection { drafts: mcp_draft(&file).into_iter().collect(), ..Collection::default() });
    }
    if format.contains(".insomnia.rest/5") {
        let mut out = Collection { name: text_or(&file, "name", ""), ..Collection::default() };
        let environments = file.get("environments").unwrap_or(&Value::Null);
        out.variables = variables_of(environments.get("data"));
        walk_v5(array_of(&file, "collection"), None, 0, &mut out);
        return Ok(out);
    }
    if file.get("resources").is_some_and(Value::is_array) {
        return Ok(parse_v4(&file));
    }
    Err(NOT_INSOMNIA.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_v4_export_brings_its_groups_requests_and_variables() {
        let export = json!({ "__export_format": 4, "resources": [
            { "_id": "wrk_1", "_type": "workspace", "name": "W" },
            { "_id": "env_1", "_type": "environment", "parentId": "wrk_1", "data": { "base_url": "http://h", "n": 2, "o": {} } },
            { "_id": "fld_1", "_type": "request_group", "parentId": "wrk_1", "name": "Group" },
            { "_id": "req_1", "_type": "request", "parentId": "fld_1", "name": "One", "method": "put", "url": "{{ _.base_url }}/1/:id",
              "headers": [{ "name": "A", "value": "b" }, { "name": "", "value": "z" }, { "name": "Off", "value": "1", "disabled": true }],
              "parameters": [{ "name": "q", "value": "1" }], "pathParameters": [{ "name": "id", "value": "5" }],
              "authentication": { "type": "bearer", "token": "t" },
              "body": { "mimeType": "application/json", "text": "{}" }, "created": 5 },
            { "_id": "req_2", "_type": "request", "parentId": "wrk_1", "url": "http://h/2",
              "body": { "mimeType": "application/graphql", "text": "{\"query\":\"{ a }\",\"variables\":{\"x\":1}}" } },
            { "_id": "greq_1", "_type": "grpc_request", "parentId": "wrk_1", "name": "Rpc", "url": "h:50051",
              "protoMethodName": "/pkg.Svc/Do", "body": { "text": "{\"a\":1}" }, "metadata": [{ "name": "m", "value": "v" }] },
            { "_id": "ws-req_1", "_type": "websocket_request", "parentId": "wrk_1", "name": "Sock", "url": "ws://h" },
        ] })
        .to_string();
        let out = parse_insomnia(&export).unwrap();
        assert_eq!(out.name, "W");
        assert_eq!(
            out.variables,
            [("base_url".to_string(), "http://h".to_string()), ("n".to_string(), "2".to_string())]
        );
        assert_eq!(out.folders[0].key, "fld_1");
        let one = &out.drafts[0];
        assert_eq!((one.kind, one.folder.as_deref()), ("requests", Some("fld_1")));
        assert_eq!(one.entity["method"], "PUT");
        assert_eq!(one.entity["url"], "{{ _.base_url }}/1/5?q=1");
        assert_eq!(
            one.entity["headers"],
            json!({ "A": "b", "Authorization": "Bearer t", "Content-Type": "application/json" })
        );
        assert_eq!(one.entity["createdAt"], 5);
        assert_eq!(
            (out.drafts[1].kind, &out.drafts[1].entity["variables"]),
            ("graphqlRequests", &json!("{\n  \"x\": 1\n}"))
        );
        let rpc = &out.drafts[2];
        assert_eq!(
            (rpc.kind, &rpc.entity["serviceName"], &rpc.entity["methodName"]),
            ("grpcRequests", &json!("pkg.Svc"), &json!("Do"))
        );
        assert_eq!(rpc.entity["metadata"], json!({ "m": "v" }));
        assert_eq!(out.drafts[3].kind, "sockets");
        assert_eq!(parse_insomnia("{}").err().unwrap(), NOT_INSOMNIA);
    }

    #[test]
    fn a_v5_tree_is_read_by_the_ids_of_its_requests() {
        let yaml = "type: collection.insomnia.rest/5.0\nname: Five\ncollection:\n  - name: Dir\n    meta:\n      id: fld_1\n    children:\n      - url: localhost:50051\n        name: Unary\n        meta:\n          id: greq_1\n          created: 9\n        body:\n          text: '{}'\n        protoMethodName: /routeguide.RouteGuide/GetFeature\n      - url: http://h/io\n        name: Chat\n        meta:\n          id: socketio-req_1\n        settings:\n          path: /custom\n        eventListeners:\n          - eventName: tick\n            desc: every second\n  - url: http://h/a\n    name: Plain\n    meta:\n      id: req_1\n    method: POST\n    scripts:\n      preRequest: insomnia.environment.set('a', 1);\n      afterResponse: insomnia.test('ok', () => {});\n    authentication:\n      type: basic\n      username: user\n      password: password\nenvironments:\n  name: Base\n  data:\n    host: h\n";
        let out = parse_insomnia(yaml).unwrap();
        assert_eq!((out.name.as_str(), out.folders.len()), ("Five", 1));
        assert_eq!(out.variables, [("host".to_string(), "h".to_string())]);
        let unary = &out.drafts[0];
        assert_eq!((unary.kind, unary.folder.as_deref()), ("grpcRequests", Some("/0")));
        assert_eq!(unary.entity["serviceName"], "routeguide.RouteGuide");
        assert_eq!(unary.entity["createdAt"], 9);
        let chat = &out.drafts[1].entity;
        assert_eq!((out.drafts[1].kind, &chat["path"]), ("socketioRequests", &json!("/custom")));
        assert_eq!(chat["listeners"], json!([{ "event": "tick", "description": "every second" }]));
        let plain = &out.drafts[2].entity;
        assert_eq!(plain["preScript"], "bc.environment.set('a', 1);");
        assert_eq!(plain["testScript"], "bc.test('ok', () => {});");
        assert_eq!(plain["headers"], json!({ "Authorization": "Basic dXNlcjpwYXNzd29yZA==" }));
    }

    #[test]
    fn an_mcp_client_file_is_an_mcp_request() {
        let yaml = "type: mcpClient.insomnia/5.0\nname: Airlines\nmcpRequest:\n  name: ''\n  url: '  https://h/mcp'\n  transportType: streamable-http\n  headers:\n    - name: X\n      value: y\n";
        let out = parse_insomnia(yaml).unwrap();
        assert_eq!(out.drafts[0].kind, "mcpRequests");
        assert_eq!(
            out.drafts[0].entity,
            json!({ "name": "Airlines", "transport": "http", "url": "https://h/mcp", "headers": { "X": "y" } })
        );
    }
}
