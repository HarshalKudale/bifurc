//! `requests`: the saved requests of every type (REST, GraphQL, gRPC, SOAP, WebSocket, SSE, Socket.IO,
//! MCP) behind one group.
//!
//! A type is a row of [`TYPES`]: where its requests are stored, which options describe one and which engine
//! command sends it. `list`, `show`, `update` and `send` find a saved request among all the types, so `--type`
//! is only ever needed to create one or to tell two requests of the same name apart.

use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::{
    args::ParsedArgs,
    commands::{
        Context, body_from, created, find_one, header_flags, id_of, list_entities, merge_headers, need, refused,
        text_from, unknown_verb, with_field, workspace_id,
    },
    curl::quote,
    error::{CliError, Result},
    format::{aligned, describe_reflection, describe_socket, js_template, json},
    response::{Response, mask_header, present, revealed, short_type, size},
};

/// One type of request.
pub(crate) struct RequestType {
    /// What `--type` calls it.
    pub(crate) name: &'static str,
    /// The entity kind its requests are stored as.
    pub(crate) kind: &'static str,
    /// The command that sends one, saved (`{ workspaceId, id }`) or not (`{ workspaceId, request }`). A
    /// WebSocket has none: it is a connection, not one exchange.
    pub(crate) send: Option<&'static str>,
    /// The field `--url` sets, and the one `-H` adds to.
    pub(crate) url_key: &'static str,
    pub(crate) headers_key: &'static str,
    /// The options only this type takes, beyond `--url`, `--name` and `-H`.
    options: &'static [&'static str],
    /// The fields of a request that is sent without being saved.
    pub(crate) inline: &'static [&'static str],
}

pub(crate) const TYPES: [RequestType; 8] = [
    RequestType {
        name: "rest",
        kind: "requests",
        send: Some("request.send"),
        url_key: "url",
        headers_key: "headers",
        options: &["method", "data", "body-file"],
        inline: &["method", "url", "headers", "body"],
    },
    RequestType {
        name: "graphql",
        kind: "graphqlRequests",
        send: Some("graphql.send"),
        url_key: "endpointUrl",
        headers_key: "headers",
        options: &["query", "query-file", "variables", "operation"],
        inline: &["endpointUrl", "headers", "query", "variables", "operationName"],
    },
    RequestType {
        name: "grpc",
        kind: "grpcRequests",
        send: Some("grpc.send"),
        url_key: "serverAddress",
        headers_key: "metadata",
        options: &["method", "data", "body-file", "proto", "reflection"],
        inline: &[
            "serverAddress",
            "serviceName",
            "methodName",
            "requestBody",
            "metadata",
            "protoFileId",
            "useReflection",
        ],
    },
    RequestType {
        name: "soap",
        kind: "soapRequests",
        send: Some("soap.send"),
        url_key: "endpointUrl",
        headers_key: "headers",
        options: &["action", "data", "body-file"],
        inline: &["endpointUrl", "soapAction", "headers", "body"],
    },
    RequestType {
        name: "websocket",
        kind: "wsConnections",
        send: None,
        url_key: "url",
        headers_key: "headers",
        options: &[],
        inline: &["url", "headers"],
    },
    RequestType {
        name: "sse",
        kind: "sseRequests",
        send: None,
        url_key: "url",
        headers_key: "headers",
        options: &["method", "data", "body-file"],
        inline: &["url", "method", "headers", "body"],
    },
    RequestType {
        name: "socketio",
        kind: "socketioRequests",
        send: None,
        url_key: "url",
        headers_key: "headers",
        options: &["namespace", "path", "auth", "listen"],
        inline: &["url", "headers", "path", "namespace", "auth", "listeners"],
    },
    RequestType {
        name: "mcp",
        kind: "mcpRequests",
        send: None,
        url_key: "url",
        headers_key: "headers",
        options: &["transport"],
        inline: &["url", "transport", "headers"],
    },
];

/// The options every type takes to describe a request.
const COMMON_OPTIONS: [&str; 3] = ["url", "name", "header"];

/// Every option that describes a request of some type: what `create` and `update` accept before the type is
/// known.
fn describing_options() -> Vec<&'static str> {
    let mut all = COMMON_OPTIONS.to_vec();
    for ty in &TYPES {
        all.extend(ty.options.iter().filter(|option| !COMMON_OPTIONS.contains(option)));
    }
    all.sort_unstable();
    all.dedup();
    all
}

const TYPE_NAMES: &str = "rest, graphql, grpc, soap, websocket, sse, socketio, mcp";

pub(crate) fn type_named(name: &str) -> Result<&'static RequestType> {
    let wanted = match name.to_lowercase().as_str() {
        "ws" => "websocket".to_string(),
        "http" => "rest".to_string(),
        "socket.io" | "sio" => "socketio".to_string(),
        other => other.to_string(),
    };
    TYPES
        .iter()
        .find(|ty| ty.name == wanted)
        .ok_or_else(|| CliError::usage(format!("unknown request type \"{name}\": one of {TYPE_NAMES}")))
}

/// The types a command looks at: the one `--type` names, else all of them.
fn types_of(args: &ParsedArgs) -> Result<Vec<&'static RequestType>> {
    match args.flag("type") {
        Some(name) => Ok(vec![type_named(name)?]),
        None => Ok(TYPES.iter().collect()),
    }
}

/// Refuse an option that describes a request of another type. `extra` are options the verb adds for this type.
fn check_options(ty: &RequestType, args: &ParsedArgs, extra: &[&str]) -> Result<()> {
    let describing = describing_options();
    let foreign = args.names().into_iter().find(|name| {
        describing.contains(name)
            && !COMMON_OPTIONS.contains(name)
            && !ty.options.contains(name)
            && !extra.contains(name)
    });
    match foreign {
        Some(option) => Err(CliError::usage(format!(
            "--{option} is not an option of a {} request (see: bifurc requests --help)",
            ty.name
        ))),
        None => Ok(()),
    }
}

/// The request with the type it is of, which its file does not always say.
fn tagged(ty: &RequestType, entity: &Value) -> Value {
    with_field(entity, "type", json!(ty.name))
}

pub(crate) fn text(entity: &Value, key: &str) -> String {
    entity.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// Every saved request of `types`, each with its type.
pub(crate) fn saved(
    ctx: &mut Context,
    workspace: &str,
    types: &[&'static RequestType],
) -> Result<Vec<(&'static RequestType, Value)>> {
    let mut all = Vec::new();
    for ty in types {
        all.extend(list_entities(ctx, workspace, ty.kind)?.into_iter().map(|entity| (*ty, entity)));
    }
    Ok(all)
}

/// The saved request `wanted` names, by id or by name, among the types `--type` allows.
fn find(ctx: &mut Context, workspace: &str, wanted: &str) -> Result<(&'static RequestType, Value)> {
    let types = types_of(ctx.args)?;
    let all = saved(ctx, workspace, &types)?;
    let entities: Vec<Value> = all.iter().map(|(_, entity)| entity.clone()).collect();
    let hit = find_one(&entities, wanted, &["name"], "request", "requests")?;
    let index = entities.iter().position(|entity| std::ptr::eq(entity, hit)).expect("the hit is one of them");
    Ok((all[index].0, hit.clone()))
}

/// A request of the type with nothing in it yet.
fn blank(ty: &RequestType) -> Value {
    match ty.name {
        "rest" => json!({ "name": "", "protocol": "rest", "method": "GET", "url": "", "headers": {}, "body": "" }),
        "graphql" => json!({
            "name": "", "protocol": "graphql", "endpointUrl": "", "headers": {}, "query": "", "variables": "",
            "operationName": "",
        }),
        "grpc" => json!({
            "name": "", "protocol": "grpc", "serverAddress": "", "serviceName": "", "methodName": "",
            "requestBody": "{}", "metadata": {}, "protoFileId": null, "useReflection": true, "streamingType": "unary",
        }),
        "soap" => {
            json!({ "name": "", "protocol": "soap", "endpointUrl": "", "soapAction": "", "headers": {}, "body": "" })
        }
        "sse" => json!({ "name": "", "url": "", "method": "GET", "headers": {}, "body": "" }),
        "socketio" => json!({ "name": "", "url": "", "headers": {}, "path": "", "namespace": "", "auth": "" }),
        "mcp" => json!({ "name": "", "transport": "", "url": "", "headers": {} }),
        _ => json!({ "name": "", "url": "", "headers": {} }),
    }
}

/// The transport an MCP request's address implies: an `http(s)://` address is an endpoint, anything else is
/// the command that starts the server.
fn mcp_transport(address: &str) -> &'static str {
    let address = address.trim_start();
    if address.starts_with("http://") || address.starts_with("https://") { "http" } else { "stdio" }
}

/// `pkg.Service/Method` (a leading slash is fine) as the service and the method.
fn grpc_method(path: &str) -> Result<(String, String)> {
    match path.trim_start_matches('/').rsplit_once('/') {
        Some((service, method)) if !service.is_empty() && !method.is_empty() => {
            Ok((service.to_string(), method.to_string()))
        }
        _ => Err(CliError::usage("the gRPC method is \"package.Service/Method\"")),
    }
}

/// Put what the options say into a request of the type. Returns whether any option said anything.
fn apply_options(ty: &RequestType, entity: &mut Value, args: &ParsedArgs) -> Result<bool> {
    let mut changed = false;
    let mut set = |entity: &mut Value, key: &str, value: Value| {
        entity[key] = value;
        changed = true;
    };
    if let Some(name) = args.flag("name") {
        set(entity, "name", json!(name));
    }
    if let Some(url) = args.flag("url") {
        set(entity, ty.url_key, json!(url));
    }
    let headers = header_flags(args)?;
    if !headers.is_empty() {
        let mut map: Map<String, Value> =
            entity.get(ty.headers_key).and_then(Value::as_object).cloned().unwrap_or_default();
        merge_headers(&mut map, &headers);
        set(entity, ty.headers_key, Value::Object(map));
    }
    let body = if ty.options.contains(&"data") { body_from(args)? } else { None };
    match ty.name {
        "rest" | "sse" => {
            if let Some(method) = args.flag("method") {
                set(entity, "method", json!(method.to_uppercase()));
            }
            if let Some(body) = body {
                set(entity, "body", json!(body));
            }
        }
        "graphql" => {
            if let Some(query) = text_from(args, "query", "query-file")? {
                set(entity, "query", json!(query));
            }
            if let Some(variables) = args.flag("variables") {
                if !variables.trim().is_empty() && serde_json::from_str::<Value>(variables).is_err() {
                    return Err(CliError::usage(format!("--variables is not valid JSON: {variables}")));
                }
                set(entity, "variables", json!(variables));
            }
            if let Some(operation) = args.flag("operation") {
                set(entity, "operationName", json!(operation));
            }
        }
        "grpc" => {
            if let Some(path) = args.flag("method") {
                let (service, method) = grpc_method(path)?;
                set(entity, "serviceName", json!(service));
                set(entity, "methodName", json!(method));
            }
            if let Some(body) = body {
                set(entity, "requestBody", json!(body));
            }
            match (args.flag("proto").filter(|proto| !proto.is_empty()), args.has("reflection")) {
                (Some(_), true) => return Err(CliError::usage("--proto and --reflection cannot both be given")),
                (Some(proto), false) => {
                    set(entity, "protoFileId", json!(proto));
                    set(entity, "useReflection", json!(false));
                }
                (None, true) => {
                    set(entity, "protoFileId", Value::Null);
                    set(entity, "useReflection", json!(true));
                }
                (None, false) => {}
            }
        }
        "soap" => {
            if let Some(action) = args.flag("action") {
                set(entity, "soapAction", json!(action));
            }
            if let Some(body) = body {
                set(entity, "body", json!(body));
            }
        }
        "socketio" => {
            for option in ["namespace", "path", "auth"] {
                if let Some(value) = args.flag(option) {
                    set(entity, option, json!(value));
                }
            }
            // The events listened to are the ones named, replacing the ones there were.
            if args.has("listen") {
                set(entity, "listeners", json!(args.flags("listen")));
                set(entity, "listenAll", json!(false));
            }
        }
        "mcp" => match args.flag("transport") {
            Some(transport) if matches!(transport, "stdio" | "http") => set(entity, "transport", json!(transport)),
            Some(other) => return Err(CliError::usage(format!("--transport is stdio or http, not \"{other}\""))),
            // The address says which it is, when it is given or nothing has said yet.
            None if args.flag("url").is_some() || text(entity, "transport").is_empty() => {
                entity["transport"] = json!(mcp_transport(&text(entity, "url")));
            }
            None => {}
        },
        _ => {}
    }
    Ok(changed)
}

/// Refuse a request that cannot be sent: one with no address, a gRPC one with no method, a GraphQL one with no
/// query.
fn complete(ty: &RequestType, entity: &Value) -> Result<()> {
    if text(entity, ty.url_key).trim().is_empty() {
        return Err(CliError::usage("missing --url"));
    }
    match ty.name {
        "grpc" if text(entity, "methodName").is_empty() => {
            Err(CliError::usage("missing --method <package.Service/Method>"))
        }
        "graphql" if text(entity, "query").trim().is_empty() => Err(CliError::usage("missing --query or --query-file")),
        _ => Ok(()),
    }
}

/// What a request is called when no `--name` says.
fn default_name(ty: &RequestType, entity: &Value) -> String {
    let url = text(entity, ty.url_key);
    match ty.name {
        "rest" => format!("{} {url}", text(entity, "method")),
        "graphql" => match text(entity, "operationName") {
            operation if operation.is_empty() => format!("GraphQL {url}"),
            operation => operation,
        },
        "grpc" => format!("{}/{}", text(entity, "serviceName"), text(entity, "methodName")),
        "soap" => format!("SOAP {url}"),
        "sse" => format!("SSE {url}"),
        "socketio" => format!("Socket.IO {url}"),
        "mcp" => format!("MCP {url}"),
        _ => format!("WebSocket {url}"),
    }
}

/// A new request of the type from the options, which must describe one that can be sent.
fn described(ty: &RequestType, args: &ParsedArgs, extra: &[&str]) -> Result<Value> {
    check_options(ty, args, extra)?;
    let mut entity = blank(ty);
    apply_options(ty, &mut entity, args)?;
    complete(ty, &entity)?;
    Ok(entity)
}

fn response_of(ty: &RequestType, response: &Value) -> Response {
    match ty.name {
        "rest" => Response::from_rest(response),
        "grpc" => Response::from_grpc(response),
        _ => Response::from_text(response),
    }
}

/// What the engine answered a send, printed as the output options say; a failure of what was sent fails the command.
fn sent(ctx: &mut Context, ty: &RequestType, response: Value) -> Result<String> {
    present(ctx, &response_of(ty, &response))
}

/// Send the saved request `wanted` as `requests send` does (`--var`, `--env`, `--fresh-session` apply) and
/// return its response; only types that answer once can be.
pub(crate) fn send_saved(ctx: &mut Context, workspace: &str, wanted: &str) -> Result<Response> {
    let (ty, entity) = find(ctx, workspace, wanted)?;
    let Some(command) = ty.send else {
        return Err(CliError::usage(format!("a {} request does not answer once, so it cannot be compared", ty.name)));
    };
    let mut payload = json!({ "workspaceId": workspace, "id": id_of(&entity) });
    add_overrides(ctx, ty, workspace, Some(&entity), &mut payload)?;
    Ok(response_of(ty, &ctx.request(command, payload)?))
}

/// How long a WebSocket, an SSE stream or a Socket.IO connection is given to open.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// `--wait`: how many seconds a live connection is listened to, `default` when not given.
pub(crate) fn listen_seconds(args: &ParsedArgs, default: f64) -> Result<f64> {
    match args.flag("wait") {
        Some(raw) => raw
            .parse::<f64>()
            .ok()
            .filter(|seconds| seconds.is_finite() && (0.0..=3600.0).contains(seconds))
            .ok_or_else(|| CliError::usage(format!("--wait must be a number of seconds, not \"{raw}\""))),
        None => Ok(default),
    }
}

/// What a send of a request that answers once prints (see [`present`]).
const OUTPUT_OPTIONS: [&str; 11] =
    ["body", "headers", "select", "out", "max-body", "raw", "quiet", "reveal", "expect-status", "expect", "extract"];

/// The options only a send takes, each for the types [`exchange_options`] says.
const SEND_OPTIONS: [&str; 7] = ["wait", "event", "ack", "tool", "prompt", "resource", "arg"];

/// The options that lay values over a request that answers once: `--var` and `--env` for any such send, `--patch`
/// for a saved one, and `--fresh-session` (a throwaway jar) for a rest one.
const OVERRIDE_OPTIONS: [&str; 4] = ["var", "env", "patch", "fresh-session"];

/// The field of a saved request `--data` and `--patch` act on: its body, a GraphQL request's variables.
fn body_field(ty: &RequestType) -> &'static str {
    match ty.name {
        "graphql" => "variables",
        "grpc" => "requestBody",
        _ => "body",
    }
}

/// Refuse the output options and the override options on a send of a request that does not answer once.
fn refuse_overrides(ty: &RequestType, args: &ParsedArgs) -> Result<()> {
    if ty.send.is_none()
        && let Some(option) = OUTPUT_OPTIONS.iter().find(|option| args.has(option))
    {
        return Err(CliError::usage(format!("--{option} does not apply to a {} request", ty.name)));
    }
    match OVERRIDE_OPTIONS.iter().find(|option| args.has(option)) {
        Some(option) if ty.send.is_none() => Err(CliError::usage(format!(
            "--{option} applies to rest, graphql, soap and grpc sends, not to a {} request",
            ty.name
        ))),
        _ => Ok(()),
    }
}

/// Add `variables` and `environmentId` to the payload of a send, a run of a folder or a run of a flow, from
/// `--var` and `--env`.
pub(crate) fn add_scope(ctx: &mut Context, workspace: &str, payload: &mut Value) -> Result<()> {
    let args = ctx.args;
    let mut variables = Map::new();
    for pair in args.flags("var") {
        match pair.split_once('=') {
            Some((key, value)) if !key.is_empty() => variables.insert(key.to_string(), json!(value)),
            _ => return Err(CliError::usage(format!("--var must be KEY=VALUE, not \"{pair}\""))),
        };
    }
    if !variables.is_empty() {
        payload["variables"] = Value::Object(variables);
    }
    if let Some(wanted) = args.flag("env") {
        let environments = list_entities(ctx, workspace, "environments")?;
        payload["environmentId"] = json!(id_of(find_one(&environments, wanted, &["name"], "environment", "env")?));
    }
    Ok(())
}

/// Add `variables`, `environmentId` and, for a saved request (`saved`), `overrides` to the payload of a one-shot
/// send, from `--var`, `--env`, `--data` and `--patch`.
pub(crate) fn add_overrides(
    ctx: &mut Context,
    ty: &RequestType,
    workspace: &str,
    saved: Option<&Value>,
    payload: &mut Value,
) -> Result<()> {
    add_scope(ctx, workspace, payload)?;
    let args = ctx.args;
    if args.has("fresh-session") {
        if ty.name != "rest" {
            return Err(CliError::usage(format!("--fresh-session applies to rest requests, not {}", ty.name)));
        }
        payload["freshJar"] = json!(true);
    }
    let Some(saved) = saved else { return Ok(()) };
    let field = body_field(ty);
    let mut body = body_from(args)?;
    let patches = args.flags("patch");
    if !patches.is_empty() {
        let source = body.unwrap_or_else(|| text(saved, field));
        let mut parsed: Value = serde_json::from_str(&source).map_err(|_| {
            CliError::usage(format!(
                "--patch needs a JSON body, and the body of \"{}\" is not JSON",
                text(saved, "name")
            ))
        })?;
        for patch in patches {
            crate::select::patch(&mut parsed, patch).map_err(CliError::usage)?;
        }
        body = Some(parsed.to_string());
    }
    if let Some(body) = body {
        payload["overrides"] = json!({ field: body });
    }
    Ok(())
}

/// The options a send of this type takes beyond the ones that describe a request: a WebSocket's message and how
/// long to listen, an SSE stream's listening time (its body is part of the request), a Socket.IO event with its
/// arguments and whether the server is to acknowledge it, what to ask an MCP server for.
fn exchange_options(ty: &RequestType) -> &'static [&'static str] {
    match ty.name {
        "sse" => &["wait"],
        "socketio" => &["event", "data", "ack", "wait"],
        "websocket" => &["data", "wait"],
        "mcp" => &["tool", "prompt", "resource", "arg", "wait"],
        _ => &[],
    }
}

/// One exchange over a WebSocket: connect (unless the engine already holds the connection), send `--body`,
/// listen for `--wait` seconds, print what went by and disconnect what was connected here.
fn socket_exchange(ctx: &mut Context, workspace: &str, socket_id: &str, target: Option<&Value>) -> Result<String> {
    let wait = listen_seconds(ctx.args, 2.0)?;
    let key = json!({ "workspaceId": workspace, "socketId": socket_id });
    let before = ctx.request("ws.status", key.clone())?;
    let held = before.get("status").and_then(Value::as_str) == Some("connected");
    let seen: Vec<Value> = before
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|message| message.get("id").cloned())
        .collect();
    if !held {
        let mut connect = key.clone();
        if let Some(target) = target {
            connect["url"] = target.get("url").cloned().unwrap_or(Value::Null);
            connect["headers"] = target.get("headers").cloned().unwrap_or_else(|| json!({}));
        }
        refused(ctx.request("ws.connect", connect)?)?;
        let started = Instant::now();
        loop {
            let status = ctx.request("ws.status", key.clone())?;
            if status.get("status").and_then(Value::as_str) != Some("connecting") || started.elapsed() > CONNECT_TIMEOUT
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let mut failure = None;
    if let Some(message) = body_from(ctx.args)? {
        let answer =
            ctx.request("ws.send", json!({ "workspaceId": workspace, "socketId": socket_id, "data": message }))?;
        failure = refused(answer).err();
    }
    if failure.is_none() && wait > 0.0 {
        std::thread::sleep(Duration::from_secs_f64(wait));
    }
    let mut snapshot = ctx.request("ws.status", key.clone())?;
    if !held {
        ctx.request("ws.disconnect", key)?;
    }
    if let Some(messages) = snapshot.get_mut("messages").and_then(Value::as_array_mut) {
        messages.retain(|message| message.get("id").is_none_or(|id| !seen.contains(id)));
    }
    let opened = matches!(snapshot.get("status").and_then(Value::as_str), Some("connected" | "disconnected"));
    let out = ctx.print(&snapshot, || describe_socket(&snapshot));
    match failure {
        Some(error) if !ctx.json => Err(error),
        None if opened => Ok(out),
        _ => Err(CliError::Failed(out)),
    }
}

/// `requests list|show|create|update|send|reflect`.
/// What the list's second column says of a request: its method, else its type in capitals.
fn kind_label(ty: &RequestType, entity: &Value) -> String {
    match ty.name {
        "rest" | "sse" => text(entity, "method").to_uppercase(),
        "soap" => "SOAP".into(),
        "websocket" => "WS".into(),
        "socketio" => "SIO".into(),
        other => other.to_uppercase(),
    }
}

/// What the list's third column says of a request: its URL without a leading `{{variable}}`, a gRPC
/// request's `service/method`.
fn list_url(ty: &RequestType, entity: &Value) -> String {
    if ty.name == "grpc" {
        return format!("{}/{}", text(entity, "serviceName"), text(entity, "methodName"));
    }
    let url = text(entity, ty.url_key);
    match url.strip_prefix("{{").and_then(|rest| rest.split_once("}}")) {
        Some((_, rest)) if !rest.is_empty() => rest.to_string(),
        _ => url,
    }
}

/// The path (`a/b/`) of every folder of a tree (`requestFolders`, `mockFolders`) by id, from the engine's folder tree.
pub(crate) fn folder_paths(ctx: &mut Context, tree: &str) -> Result<std::collections::HashMap<String, String>> {
    let config = ctx.request("config.get", json!({}))?;
    let folders = config.get(tree).and_then(Value::as_array).cloned().unwrap_or_default();
    let by_id: std::collections::HashMap<String, &Value> = folders.iter().map(|f| (text(f, "id"), f)).collect();
    Ok(by_id
        .keys()
        .map(|id| {
            let (mut names, mut at) = (Vec::new(), Some(id.as_str()));
            // The depth cap stops a parent loop in a damaged tree.
            while let Some(folder) = at.and_then(|id| by_id.get(id)).filter(|_| names.len() < 32) {
                names.push(text(folder, "name"));
                at = folder.get("parentId").and_then(Value::as_str);
            }
            names.reverse();
            (id.clone(), format!("{}/", names.join("/")))
        })
        .collect())
}

/// `requests show` for people: the line a request sends, where it is saved, then what it holds.
fn describe(ty: &RequestType, entity: &Value, folder: &str) -> String {
    let mut first = vec![kind_label(ty, entity), text(entity, ty.url_key)];
    match ty.name {
        "grpc" => first.push(format!("{}/{}", text(entity, "serviceName"), text(entity, "methodName"))),
        "graphql" if !text(entity, "operationName").is_empty() => first.push(text(entity, "operationName")),
        _ => {}
    }
    let place = if folder.is_empty() { String::new() } else { format!(" · {folder}") };
    let mut lines = vec![first.join(" "), format!("{}{place} · id {}", ty.name, id_of(entity))];
    let headers = entity.get(ty.headers_key).and_then(Value::as_object);
    let names: Vec<&str> = headers.map(|h| h.keys().map(String::as_str).collect()).unwrap_or_default();
    if !names.is_empty() {
        lines.push(format!("headers  {}", names.join(", ")));
    }
    let body = text(entity, body_field(ty));
    if !body.is_empty() && !matches!(body.trim(), "{}" | "null") {
        let content_type = headers
            .and_then(|h| h.iter().find(|(name, _)| name.eq_ignore_ascii_case("content-type")))
            .map(|(_, value)| js_template(Some(value)))
            .unwrap_or_default();
        let kind = short_type(&content_type, body.as_bytes());
        lines.push(format!("body     {} {kind}", size(body.len())).trim_end().to_string());
    }
    let scripts: Vec<&str> = [("preScript", "pre"), ("postScript", "post"), ("testScript", "test")]
        .iter()
        .filter(|(key, _)| !text(entity, key).trim().is_empty())
        .map(|(_, name)| *name)
        .collect();
    if !scripts.is_empty() {
        lines.push(format!("scripts  {}", scripts.join(", ")));
    }
    lines.join("\n")
}

/// `requests show --curl`: a REST request as a curl command, its variables resolved by the engine and its
/// secret header values masked unless `--reveal` is given at a terminal.
fn curl_command(ctx: &mut Context, workspace: &str, ty: &RequestType, entity: &Value) -> Result<String> {
    if ty.name != "rest" {
        return Err(CliError::usage("only REST requests print as curl"));
    }
    let reveal = revealed(ctx.args);
    let mut resolve = |text: &str| -> Result<String> {
        let answer = ctx.request("env.resolveText", json!({ "workspaceId": workspace, "text": text }))?;
        Ok(answer.get("text").and_then(Value::as_str).unwrap_or(text).to_string())
    };
    let method = text(entity, "method").to_uppercase();
    let mut words = vec!["curl".to_string()];
    if !method.is_empty() && method != "GET" {
        words.extend(["-X".to_string(), method]);
    }
    words.push(quote(&resolve(&text(entity, "url"))?));
    for (name, value) in entity.get("headers").and_then(Value::as_object).into_iter().flatten() {
        let value = resolve(&js_template(Some(value)))?;
        let value = if reveal { value } else { mask_header(name, &value) };
        words.extend(["-H".to_string(), quote(&format!("{name}: {value}"))]);
    }
    let body = text(entity, "body");
    if !body.is_empty() {
        words.extend(["--data-raw".to_string(), quote(&resolve(&body)?)]);
    }
    Ok(words.join(" "))
}

pub fn requests(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    let args = ctx.args;
    let mut describing = describing_options();
    describing.push("type");
    match sub.unwrap_or("list") {
        "list" => {
            ctx.accept(&["type", "folder", "grep", "ids"], 0)?;
            let types = types_of(args)?;
            let workspace = workspace_id(ctx)?;
            let all = saved(ctx, &workspace, &types)?;
            let folders = folder_paths(ctx, "requestFolders")?;
            let folder = args.flag("folder").map(|f| f.trim_matches('/').to_lowercase());
            let grep = args.flag("grep").map(str::to_lowercase);
            let mut kept: Vec<(String, &RequestType, &Value)> = all
                .iter()
                .map(|(ty, entity)| (folders.get(&text(entity, "folderId")).cloned().unwrap_or_default(), *ty, entity))
                .filter(|(path, ty, entity)| {
                    let path = path.to_lowercase();
                    let in_folder = folder
                        .as_deref()
                        .is_none_or(|f| path.trim_end_matches('/') == f || path.starts_with(&format!("{f}/")));
                    let found = grep.as_deref().is_none_or(|g| {
                        [text(entity, "name"), text(entity, ty.url_key), kind_label(ty, entity)]
                            .iter()
                            .any(|field| field.to_lowercase().contains(g))
                    });
                    in_folder && found
                })
                .collect();
            kept.sort_by_cached_key(|(path, _, entity)| (path.to_lowercase(), text(entity, "name").to_lowercase()));
            let entities: Vec<Value> = kept
                .iter()
                .map(|(path, ty, entity)| with_field(&tagged(ty, entity), "folderPath", json!(path)))
                .collect();
            Ok(ctx.print(&Value::Array(entities), || {
                if kept.is_empty() {
                    return "(none)".to_string();
                }
                // With no folder anywhere the first column would only be blank space.
                let folderless = kept.iter().all(|(path, ..)| path.is_empty());
                let cells: Vec<Vec<String>> = kept
                    .iter()
                    .map(|(path, ty, entity)| {
                        let mut cells = vec![kind_label(ty, entity), list_url(ty, entity), text(entity, "name")];
                        if !folderless {
                            cells.insert(0, path.clone());
                        }
                        if args.has("ids") {
                            cells.insert(0, id_of(entity));
                        }
                        cells
                    })
                    .collect();
                aligned(&cells)
            }))
        }
        "show" => {
            ctx.accept(&["type", "curl", "reveal"], 1)?;
            let wanted = need(args.positional(2), "request id or name")?;
            let workspace = workspace_id(ctx)?;
            let (ty, entity) = find(ctx, &workspace, &wanted)?;
            // The listing carries the content but not the memory: this read tells the engine what was shown.
            let seen = json!({ "workspaceId": workspace, "kind": ty.kind, "id": id_of(&entity) });
            let _ = ctx.request("entity.load", seen);
            if args.has("curl") {
                return curl_command(ctx, &workspace, ty, &entity);
            }
            let entity = tagged(ty, &entity);
            if ctx.json {
                return Ok(json(&entity));
            }
            let folders = folder_paths(ctx, "requestFolders")?;
            Ok(describe(ty, &entity, folders.get(&text(&entity, "folderId")).map_or("", String::as_str)))
        }
        "create" | "add" => {
            describing.push("curl");
            ctx.accept(&describing, 1)?;
            let (ty, mut entity) = match args.flag("curl") {
                // The engine reads the command and says which type of request it is.
                Some(curl) => {
                    if args.flag("url").is_some() || args.positional(2).is_some() {
                        return Err(CliError::usage("--curl already says where the request goes; leave out the URL"));
                    }
                    let built = ctx.request("curl.buildRequest", json!({ "curl": curl }))?;
                    let kind = js_template(built.get("kind"));
                    let ty = TYPES.iter().find(|ty| ty.kind == kind).ok_or_else(|| {
                        CliError::msg(format!("the engine built a request of an unknown kind \"{kind}\""))
                    })?;
                    if args
                        .flag("type")
                        .is_some_and(|wanted| type_named(wanted).is_ok_and(|wanted| wanted.name != ty.name))
                    {
                        return Err(CliError::msg(format!("the cURL command describes a {} request", ty.name)));
                    }
                    check_options(ty, args, &[])?;
                    let mut entity = built.get("request").cloned().unwrap_or(Value::Null);
                    apply_options(ty, &mut entity, args)?;
                    (ty, entity)
                }
                None => {
                    let ty = type_named(args.flag("type").unwrap_or("rest"))?;
                    check_options(ty, args, &[])?;
                    let mut entity = blank(ty);
                    // The URL may be the first argument, as it is for curl.
                    if let (None, Some(url)) = (args.flag("url"), args.positional(2)) {
                        entity[ty.url_key] = json!(url);
                    }
                    apply_options(ty, &mut entity, args)?;
                    (ty, entity)
                }
            };
            complete(ty, &entity)?;
            if text(&entity, "name").is_empty() {
                entity["name"] = json!(default_name(ty, &entity));
            }
            let workspace = workspace_id(ctx)?;
            let response =
                ctx.request("entity.create", json!({ "kind": ty.kind, "workspaceId": workspace, "entity": entity }))?;
            let saved = tagged(ty, &created(&response));
            Ok(ctx.print(&saved, || format!("Saved {} request {}", ty.name, id_of(&saved))))
        }
        "update" => {
            describing.push("force");
            ctx.accept(&describing, 1)?;
            let wanted = need(args.positional(2), "request id or name")?;
            let workspace = workspace_id(ctx)?;
            let (ty, mut entity) = find(ctx, &workspace, &wanted)?;
            check_options(ty, args, &[])?;
            if !apply_options(ty, &mut entity, args)? {
                return Err(CliError::usage(format!(
                    "nothing to change: give at least one option of a {} request (see: bifurc requests --help)",
                    ty.name
                )));
            }
            complete(ty, &entity)?;
            if entity.get("workspaceId").is_none_or(Value::is_null) {
                entity["workspaceId"] = json!(workspace);
            }
            ctx.request("entity.update", json!({ "kind": ty.kind, "entity": entity }))?;
            let entity = tagged(ty, &entity);
            Ok(ctx.print(&entity, || format!("Updated {} request {}", ty.name, id_of(&entity))))
        }
        "send" => {
            describing.extend(SEND_OPTIONS);
            ctx.accept(&[describing.as_slice(), &OUTPUT_OPTIONS, &OVERRIDE_OPTIONS].concat(), 1)?;
            match args.positional(2) {
                // A saved request goes by id: the engine loads it, resolves it and runs its scripts.
                Some(wanted) => {
                    let workspace = workspace_id(ctx)?;
                    let (ty, entity) = find(ctx, &workspace, wanted)?;
                    let exchange = exchange_options(ty);
                    // `--data` replaces the body of what is sent once; the rest of the request stays as saved.
                    let given = args.names().into_iter().find(|name| {
                        *name != "type"
                            && describing.contains(name)
                            && !exchange.contains(name)
                            && !(*name == "data" && ty.send.is_some())
                    });
                    if let Some(option) = given {
                        return Err(CliError::usage(format!(
                            "a saved request is sent as it is saved, without --{option}; change it with: bifurc requests update"
                        )));
                    }
                    refuse_overrides(ty, args)?;
                    let id = id_of(&entity);
                    match ty.send {
                        Some(command) => {
                            let mut payload = json!({ "workspaceId": workspace, "id": id });
                            add_overrides(ctx, ty, &workspace, Some(&entity), &mut payload)?;
                            let response = ctx.request(command, payload)?;
                            sent(ctx, ty, response)
                        }
                        None if ty.name == "sse" => crate::stream::stream_exchange(ctx, &workspace, &id, None),
                        None if ty.name == "socketio" => crate::socketio::socketio_exchange(ctx, &workspace, &id, None),
                        None if ty.name == "mcp" => crate::mcp::mcp_exchange(ctx, &workspace, &id, None),
                        None => socket_exchange(ctx, &workspace, &id, None),
                    }
                }
                None => {
                    if args.flag("url").is_none() {
                        return Err(CliError::usage(
                            "missing request id or name, or --url for a request that is not saved",
                        ));
                    }
                    let ty = type_named(args.flag("type").unwrap_or("rest"))?;
                    let exchange = exchange_options(ty);
                    if let Some(option) = SEND_OPTIONS.iter().find(|o| args.has(o) && !exchange.contains(o)) {
                        return Err(CliError::usage(format!(
                            "--{option} is not an option of a {} request (see: bifurc requests --help)",
                            ty.name
                        )));
                    }
                    refuse_overrides(ty, args)?;
                    if args.has("patch") {
                        return Err(CliError::usage(
                            "--patch changes a saved request's body; give the body with --data",
                        ));
                    }
                    let entity = described(ty, args, exchange)?;
                    let workspace = workspace_id(ctx)?;
                    let request: Map<String, Value> = ty
                        .inline
                        .iter()
                        .map(|key| (key.to_string(), entity.get(*key).cloned().unwrap_or(Value::Null)))
                        .collect();
                    match ty.send {
                        Some(command) => {
                            let mut payload = json!({ "workspaceId": workspace, "request": request });
                            if ty.name == "rest" {
                                payload["scripts"] = json!({});
                            }
                            add_overrides(ctx, ty, &workspace, None, &mut payload)?;
                            let response = ctx.request(command, payload)?;
                            sent(ctx, ty, response)
                        }
                        None if ty.name == "sse" => {
                            let stream_id = format!("cli-{}", std::process::id());
                            crate::stream::stream_exchange(ctx, &workspace, &stream_id, Some(&Value::Object(request)))
                        }
                        None if ty.name == "socketio" => {
                            let connection_id = format!("cli-{}", std::process::id());
                            let target = Value::Object(request);
                            crate::socketio::socketio_exchange(ctx, &workspace, &connection_id, Some(&target))
                        }
                        None if ty.name == "mcp" => {
                            let connection_id = format!("cli-{}", std::process::id());
                            let target = Value::Object(request);
                            crate::mcp::mcp_exchange(ctx, &workspace, &connection_id, Some(&target))
                        }
                        None => {
                            let socket_id = format!("cli-{}", std::process::id());
                            socket_exchange(ctx, &workspace, &socket_id, Some(&Value::Object(request)))
                        }
                    }
                }
            }
        }
        "reflect" => {
            ctx.accept(&[], 1)?;
            let address = need(args.positional(2), "gRPC server address, e.g. localhost:50051")?;
            let workspace = workspace_id(ctx)?;
            let response =
                ctx.request("grpc.reflect", json!({ "serverAddress": address, "workspaceId": workspace }))?;
            if response.get("ok").and_then(Value::as_bool) == Some(false) {
                let error = response.get("error").and_then(Value::as_str).unwrap_or("the server did not answer");
                return Err(if ctx.json { CliError::Failed(json(&response)) } else { CliError::msg(error) });
            }
            Ok(ctx.print(&response, || describe_reflection(&response)))
        }
        other => Err(unknown_verb("requests", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run, sub};

    fn listing(entities: Value) -> Result<Value> {
        Ok(json!({ "entities": entities }))
    }

    fn rest() -> Value {
        json!({"id": "r1", "name": "Ping", "protocol": "rest", "method": "GET", "url": "http://x/ping",
               "headers": {"Accept": "a/b"}, "body": "", "preRequestScript": "// kept", "workspaceId": "w1"})
    }

    fn grpc() -> Value {
        json!({"id": "g1", "name": "Hello", "protocol": "grpc", "serverAddress": "localhost:50051",
               "serviceName": "pkg.Greeter", "methodName": "SayHello", "requestBody": "{}", "metadata": {},
               "protoFileId": null, "useReflection": true, "workspaceId": "w1"})
    }

    /// An engine with one request of each type.
    fn saved_requests() -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            ("entity.list:requests", listing(json!([rest()]))),
            (
                "entity.list:graphqlRequests",
                listing(json!([
                {"id": "q1", "name": "Viewer", "endpointUrl": "http://x/graphql", "operationName": "Viewer",
                 "query": "{ viewer { id } }", "variables": "", "headers": {}}])),
            ),
            ("entity.list:grpcRequests", listing(json!([grpc()]))),
            (
                "entity.list:soapRequests",
                listing(json!([
                {"id": "s1", "name": "Add", "endpointUrl": "http://x/soap", "soapAction": "urn:Add", "headers": {},
                 "body": "<a/>"}])),
            ),
            (
                "entity.list:wsConnections",
                listing(json!([
                {"id": "k1", "name": "Feed", "url": "ws://x/feed", "headers": {}}])),
            ),
            (
                "entity.list:sseRequests",
                listing(json!([
                {"id": "e1", "name": "Events", "url": "http://x/events", "method": "GET", "headers": {}, "body": ""}])),
            ),
            (
                "entity.list:socketioRequests",
                listing(json!([
                {"id": "i1", "name": "Chat", "url": "http://x", "namespace": "/chat", "path": "", "auth": "",
                 "headers": {}}])),
            ),
            (
                "entity.list:mcpRequests",
                listing(json!([
                {"id": "m1", "name": "Tools", "transport": "stdio", "url": "npx my-server", "headers": {}}])),
            ),
            ("entity.create", Ok(json!({"id": "n1", "entity": {"id": "n1"}}))),
            ("entity.update", Ok(json!({"ok": true}))),
        ]
    }

    #[test]
    fn list_shows_every_type_in_the_same_columns_sorted_by_name() {
        let argv = ["requests"];
        let (out, _) = run(&argv, saved_requests(), |c| requests(c, None));
        assert_eq!(
            out.unwrap(),
            "SOAP     http://x/soap         Add\n\
             SIO      http://x              Chat\n\
             GET      http://x/events       Events\n\
             WS       ws://x/feed           Feed\n\
             GRPC     pkg.Greeter/SayHello  Hello\n\
             GET      http://x/ping         Ping\n\
             MCP      npx my-server         Tools\n\
             GRAPHQL  http://x/graphql      Viewer"
        );
    }

    /// Requests in folders `auth/`, `auth/admin/`, `orders/` and the root.
    fn foldered() -> Vec<Answer> {
        let mut answers = saved_requests();
        answers[0] = (
            "config.get",
            Ok(json!({"activeWorkspaceId": "w1", "requestFolders": [
                {"id": "f1", "name": "auth", "parentId": null},
                {"id": "f2", "name": "admin", "parentId": "f1"},
                {"id": "f3", "name": "orders", "parentId": null}]})),
        );
        answers[1] = (
            "entity.list:requests",
            listing(json!([
                {"id": "a", "name": "Login", "method": "POST", "url": "{{baseUrl}}/login", "folderId": "f1"},
                {"id": "b", "name": "Create user", "method": "POST", "url": "{{baseUrl}}/users", "folderId": "f1"},
                {"id": "c", "name": "Get order", "method": "GET", "url": "{{baseUrl}}/orders/{{id}}", "folderId": "f3"},
                {"id": "d", "name": "Ban", "method": "DELETE", "url": "http://x/ban", "folderId": "f2"},
                {"id": "e", "name": "Root one", "method": "GET", "url": "{{baseUrl}}"}])),
        );
        answers
    }

    fn list_of(extra: &[&str]) -> String {
        let argv = [&["requests", "list", "--type", "rest"], extra].concat();
        run(&argv, foldered(), |c| requests(c, sub(&argv))).0.unwrap()
    }

    #[test]
    fn list_has_the_folder_method_url_without_a_leading_variable_and_name() {
        let want = [
            "             GET     {{baseUrl}}     Root one",
            "auth/        POST    /users          Create user",
            "auth/        POST    /login          Login",
            "auth/admin/  DELETE  http://x/ban    Ban",
            "orders/      GET     /orders/{{id}}  Get order",
        ];
        assert_eq!(list_of(&[]), want.join("\n"));
    }

    #[test]
    fn list_filters_by_folder_and_text_and_can_show_ids() {
        assert_eq!(list_of(&["--folder", "AUTH/"]).lines().count(), 3);
        assert_eq!(list_of(&["--folder", "auth/admin"]), "auth/admin/  DELETE  http://x/ban  Ban");
        assert_eq!(list_of(&["--folder", "adm"]), "(none)");
        assert_eq!(list_of(&["--grep", "LOGIN"]), "auth/  POST  /login  Login");
        assert_eq!(list_of(&["--grep", "delete"]), "auth/admin/  DELETE  http://x/ban  Ban");
        assert_eq!(list_of(&["--grep", "/orders", "--ids"]), "c  orders/  GET  /orders/{{id}}  Get order");
        let json: Value = serde_json::from_str(&list_of(&["--json", "--folder", "orders"])).unwrap();
        assert_eq!(json[0]["folderPath"], "orders/");
    }

    #[test]
    fn list_takes_one_type_and_json_holds_the_requests_with_their_type() {
        let argv = ["requests", "list", "--type", "grpc", "--json"];
        let (out, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        let listed: Value = serde_json::from_str(&out.unwrap()).unwrap();
        let want = with_field(&with_field(&grpc(), "type", json!("grpc")), "folderPath", json!(""));
        assert_eq!(listed, json!([want]));
        assert_eq!(payloads(&calls, "entity.list"), [json!({"workspaceId": "w1", "kind": "grpcRequests"})]);

        let argv = ["requests", "list", "-t", "ws"];
        let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "WS  ws://x/feed  Feed");

        let argv = ["requests", "list", "--type", "carrier-pigeon"];
        let (out, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::usage(
                "unknown request type \"carrier-pigeon\": one of rest, graphql, grpc, soap, websocket, sse, socketio, mcp"
            )
        );
        assert!(calls.is_empty());
    }

    #[test]
    fn show_finds_a_request_of_any_type_by_id_or_name() {
        for (wanted, ty) in [
            ("r1", "rest"),
            ("hello", "grpc"),
            ("Feed", "websocket"),
            ("events", "sse"),
            ("chat", "socketio"),
            ("tools", "mcp"),
        ] {
            let argv = ["requests", "show", wanted, "--json"];
            let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
            assert_eq!(serde_json::from_str::<Value>(&out.unwrap()).unwrap()["type"], ty, "{wanted}");
        }
        let argv = ["requests", "show", "nope"];
        let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::not_found("no request \"nope\" (see: bifurc requests list)"));
        let argv = ["requests", "show", "Hello", "--type", "rest"];
        let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::not_found("no request \"Hello\" (see: bifurc requests list)"));
        let argv = ["requests", "show"];
        let (out, _) = run(&argv, vec![], |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("missing request id or name"));
    }

    #[test]
    fn a_misspelt_name_suggests_the_close_ones() {
        let argv = ["requests", "show", "Logn"];
        let (out, _) = run(&argv, foldered(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::not_found("no request \"Logn\" — did you mean \"Login\"?"));
    }

    fn show_of(wanted: &str, extra: &[&str]) -> (Result<String>, Vec<(String, Value)>) {
        let mut answers = foldered();
        answers[1] = (
            "entity.list:requests",
            listing(json!([{"id": "mvbzxzympjvgk", "name": "Get orders", "method": "GET", "folderId": "f3",
                "url": "{{baseUrl}}/orders.json",
                "headers": {"Authorization": "Bearer abcdef", "Content-Type": "application/json"},
                "body": "{\"a\":1,\"b\":\"two\",\"c\":[1,2,3]}", "preScript": "x", "testScript": "y"}])),
        );
        answers.push(("env.resolveText", Ok(json!({"text": "resolved"}))));
        let argv = [&["requests", "show", wanted], extra].concat();
        run(&argv, answers, |c| requests(c, sub(&argv)))
    }

    #[test]
    fn show_is_a_compact_summary_of_what_applies() {
        assert_eq!(
            show_of("get orders", &[]).0.unwrap(),
            "GET {{baseUrl}}/orders.json\n\
             rest · orders/ · id mvbzxzympjvgk\n\
             headers  Authorization, Content-Type\n\
             body     29B json\n\
             scripts  pre, test"
        );
        let argv = ["requests", "show", "Feed"];
        let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "WS ws://x/feed\nwebsocket · id k1");
    }

    #[test]
    fn show_curl_resolves_variables_masks_secrets_and_is_for_rest_only() {
        let (out, calls) = show_of("Get orders", &["--curl"]);
        assert_eq!(
            out.unwrap(),
            "curl resolved -H 'Authorization: ••••(8)' -H 'Content-Type: resolved' --data-raw resolved"
        );
        assert_eq!(
            payloads(&calls, "env.resolveText")[0],
            json!({"workspaceId": "w1", "text": "{{baseUrl}}/orders.json"})
        );
        let argv = ["requests", "show", "Hello", "--curl"];
        let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("only REST requests print as curl"));
    }

    #[test]
    fn create_builds_a_rest_request_from_the_options() {
        let argv =
            ["requests", "create", "--url", "http://x/a", "-X", "post", "-H", "Accept: a/b", "-H", "X-N:1", "-d", "{}"];
        let (out, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Saved rest request n1");
        assert_eq!(
            payloads(&calls, "entity.create"),
            [json!({"kind": "requests", "workspaceId": "w1", "entity": {
                "name": "POST http://x/a", "protocol": "rest", "method": "POST", "url": "http://x/a",
                "headers": {"Accept": "a/b", "X-N": "1"}, "body": "{}"}})]
        );
    }

    #[test]
    fn create_takes_the_url_as_an_argument_a_name_and_a_body_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("body.txt");
        std::fs::write(&file, "from file").unwrap();
        let file = file.to_string_lossy().into_owned();
        let argv = ["requests", "add", "http://y", "--name", "Mine", "--body-file", &file, "--body", "ignored"];
        let (_, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        let entity = &payloads(&calls, "entity.create")[0]["entity"];
        assert_eq!(entity["name"], "Mine");
        assert_eq!(entity["url"], "http://y");
        assert_eq!(entity["body"], "from file");
        assert_eq!(entity["method"], "GET");
    }

    #[test]
    fn create_builds_each_other_type() {
        let argv = [
            "requests",
            "create",
            "--type",
            "grpc",
            "--url",
            "localhost:50051",
            "--method",
            "/pkg.Greeter/SayHello",
            "-d",
            "{\"name\":\"a\"}",
            "-H",
            "x-k: v",
        ];
        let (out, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Saved grpc request n1");
        assert_eq!(
            payloads(&calls, "entity.create"),
            [json!({"kind": "grpcRequests", "workspaceId": "w1", "entity": {
                "name": "pkg.Greeter/SayHello", "protocol": "grpc", "serverAddress": "localhost:50051",
                "serviceName": "pkg.Greeter", "methodName": "SayHello", "requestBody": "{\"name\":\"a\"}",
                "metadata": {"x-k": "v"}, "protoFileId": null, "useReflection": true, "streamingType": "unary"}})]
        );

        let argv = ["requests", "create", "-t", "grpc", "--url", "h:1", "--method", "S/M", "--proto", "p1"];
        let (_, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        let entity = &payloads(&calls, "entity.create")[0]["entity"];
        assert_eq!((&entity["protoFileId"], &entity["useReflection"]), (&json!("p1"), &json!(false)));

        let argv = [
            "requests",
            "create",
            "--type",
            "graphql",
            "--url",
            "http://x/graphql",
            "--query",
            "query Me { me { id } }",
            "--variables",
            "{\"a\":1}",
            "--operation",
            "Me",
        ];
        let (_, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(
            payloads(&calls, "entity.create")[0],
            json!({"kind": "graphqlRequests", "workspaceId": "w1", "entity": {
                "name": "Me", "protocol": "graphql", "endpointUrl": "http://x/graphql", "headers": {},
                "query": "query Me { me { id } }", "variables": "{\"a\":1}", "operationName": "Me"}})
        );

        let argv =
            ["requests", "create", "--type", "soap", "--url", "http://x/soap", "--action", "urn:Add", "-d", "<a/>"];
        let (_, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(
            payloads(&calls, "entity.create")[0],
            json!({"kind": "soapRequests", "workspaceId": "w1", "entity": {
                "name": "SOAP http://x/soap", "protocol": "soap", "endpointUrl": "http://x/soap", "soapAction": "urn:Add",
                "headers": {}, "body": "<a/>"}})
        );

        let argv = ["requests", "create", "--type", "ws", "--url", "ws://x/feed", "-H", "a: 1"];
        let (out, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Saved websocket request n1");
        assert_eq!(
            payloads(&calls, "entity.create")[0],
            json!({"kind": "wsConnections", "workspaceId": "w1", "entity": {
                "name": "WebSocket ws://x/feed", "url": "ws://x/feed", "headers": {"a": "1"}}})
        );
    }

    #[test]
    fn create_refuses_what_cannot_be_sent_and_options_of_another_type_before_asking_the_engine() {
        for (argv, message) in [
            (vec!["requests", "create"], "missing --url"),
            (vec!["requests", "create", "-t", "grpc", "--url", "h:1"], "missing --method <package.Service/Method>"),
            (
                vec!["requests", "create", "-t", "grpc", "--url", "h:1", "--method", "NoSlash"],
                "the gRPC method is \"package.Service/Method\"",
            ),
            (
                vec![
                    "requests",
                    "create",
                    "-t",
                    "grpc",
                    "--url",
                    "h:1",
                    "--method",
                    "S/M",
                    "--proto",
                    "p",
                    "--reflection",
                ],
                "--proto and --reflection cannot both be given",
            ),
            (vec!["requests", "create", "-t", "graphql", "--url", "http://x"], "missing --query or --query-file"),
            (
                vec![
                    "requests",
                    "create",
                    "-t",
                    "graphql",
                    "--url",
                    "http://x",
                    "--query",
                    "{a}",
                    "--variables",
                    "{oops",
                ],
                "--variables is not valid JSON: {oops",
            ),
            (
                vec!["requests", "create", "--url", "http://x", "--query", "{a}"],
                "--query is not an option of a rest request (see: bifurc requests --help)",
            ),
            (
                vec!["requests", "create", "-t", "ws", "--url", "ws://x", "-d", "hi"],
                "--data is not an option of a websocket request (see: bifurc requests --help)",
            ),
            (vec!["requests", "create", "--url", "u", "-H", "nocolon"], "header \"nocolon\" must be \"name: value\""),
            (
                vec!["requests", "create", "--url", "u", "--pre-script", "x"],
                "unknown option --pre-script for \"requests create\" (see: bifurc requests --help)",
            ),
        ] {
            let (out, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
            assert_eq!(out.unwrap_err(), CliError::usage(message), "{argv:?}");
            assert!(calls.is_empty(), "{argv:?}");
        }
        let argv = ["requests", "create", "--url", "u", "--body-file", "/definitely/not/here"];
        let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert!(out.unwrap_err().to_string().starts_with("cannot read /definitely/not/here"));
    }

    #[test]
    fn create_from_a_curl_command_is_of_the_type_the_engine_says() {
        let built = json!({"kind": "graphqlRequests", "request": {
            "name": "Me", "protocol": "graphql", "endpointUrl": "http://x/graphql", "operationName": "Me",
            "query": "query Me { me { id } }", "variables": "", "headers": {}}});
        let mut answers = saved_requests();
        answers.push(("curl.buildRequest", Ok(built)));
        let argv = ["requests", "create", "--curl", "curl http://x/graphql -d ...", "--name", "Mine"];
        let (out, calls) = run(&argv, answers.clone(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Saved graphql request n1");
        assert_eq!(payloads(&calls, "curl.buildRequest"), [json!({"curl": "curl http://x/graphql -d ..."})]);
        let sent = &payloads(&calls, "entity.create")[0];
        assert_eq!(sent["kind"], "graphqlRequests");
        assert_eq!(sent["entity"]["name"], "Mine");
        assert_eq!(sent["entity"]["query"], "query Me { me { id } }");

        let argv = ["requests", "create", "--curl", "curl http://x/graphql", "--type", "rest"];
        let (out, _) = run(&argv, answers.clone(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("the cURL command describes a graphql request"));

        let argv = ["requests", "create", "--curl", "curl http://x", "--url", "http://y"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("--curl already says where the request goes; leave out the URL"));
        assert!(calls.is_empty());
    }

    #[test]
    fn update_changes_what_is_given_and_keeps_scripts_and_the_rest() {
        let argv = ["requests", "update", "ping", "-X", "put", "-H", "accept:", "-H", "X-New: 1"];
        let (out, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Updated rest request r1");
        assert_eq!(
            payloads(&calls, "entity.update"),
            [json!({"kind": "requests", "entity": {
                "id": "r1", "name": "Ping", "protocol": "rest", "method": "PUT", "url": "http://x/ping",
                "headers": {"X-New": "1"}, "body": "", "preRequestScript": "// kept", "workspaceId": "w1"}})]
        );
    }

    #[test]
    fn update_takes_the_options_of_the_type_the_request_is_of() {
        let argv = ["requests", "update", "g1", "--method", "pkg.Greeter/SayBye", "--url", "localhost:6000"];
        let (out, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Updated grpc request g1");
        let sent = &payloads(&calls, "entity.update")[0];
        assert_eq!(sent["kind"], "grpcRequests");
        assert_eq!(sent["entity"]["methodName"], "SayBye");
        assert_eq!(sent["entity"]["serverAddress"], "localhost:6000");
        assert!(sent["entity"].get("type").is_none());

        let argv = ["requests", "update", "k1", "--name", "Live feed"];
        let (_, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        let sent = &payloads(&calls, "entity.update")[0];
        assert_eq!(sent["entity"]["name"], "Live feed");
        assert_eq!(sent["entity"]["workspaceId"], "w1");

        let argv = ["requests", "update", "g1", "--query", "{a}"];
        let (out, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::usage("--query is not an option of a grpc request (see: bifurc requests --help)")
        );
        assert!(payloads(&calls, "entity.update").is_empty());

        let argv = ["requests", "update", "g1"];
        let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::usage(
                "nothing to change: give at least one option of a grpc request (see: bifurc requests --help)"
            )
        );
    }

    #[test]
    fn send_by_id_sends_only_the_id_with_the_command_of_its_type() {
        let argv = ["requests", "send", "r1"];
        let response = json!({"status": 200, "durationMs": 3, "headers": {"a": "b"}, "body": "b2s="});
        let mut answers = saved_requests();
        answers.push(("request.send", Ok(response)));
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "200 OK · 3ms · 2B text");
        assert_eq!(payloads(&calls, "request.send"), [json!({"workspaceId": "w1", "id": "r1"})]);

        let argv = ["requests", "send", "Hello"];
        let mut answers = saved_requests();
        answers.push(("grpc.send", Ok(json!({"ok": true, "status": 0, "durationMs": 3, "responses": ["{}"]}))));
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "gRPC OK · 3ms · 1 message");
        assert_eq!(payloads(&calls, "grpc.send"), [json!({"workspaceId": "w1", "id": "g1"})]);

        for (wanted, command) in [("q1", "graphql.send"), ("s1", "soap.send")] {
            let argv = ["requests", "send", wanted];
            let mut answers = saved_requests();
            answers.push((command, Ok(json!({"status": 200, "durationMs": 1, "headers": {}, "body": "plain"}))));
            let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
            assert_eq!(out.unwrap(), "200 OK · 1ms · 5B text");
            assert_eq!(payloads(&calls, command), [json!({"workspaceId": "w1", "id": wanted})]);
        }
    }

    #[test]
    fn send_of_a_saved_request_takes_no_option_that_would_change_it() {
        let argv = ["requests", "send", "r1", "-X", "post"];
        let (out, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::usage(
                "a saved request is sent as it is saved, without --method; change it with: bifurc requests update"
            )
        );
        assert!(payloads(&calls, "request.send").is_empty());
    }

    /// Send `argv` with the saved requests, the engine answering a plain 200 to `command`: the result and the
    /// payload the engine got.
    fn send_with(argv: &[&str], command: &'static str) -> (Result<String>, Vec<Value>) {
        let mut answers = saved_requests();
        answers.push(("config.get", Ok(config())));
        answers.push((
            "entity.list:environments",
            listing(json!([{"id": "__global__", "name": "Global"}, {"id": "e9", "name": "Staging"}])),
        ));
        answers.push((command, Ok(json!({"status": 200, "durationMs": 1, "headers": {}, "body": ""}))));
        let (out, calls) = run(argv, answers, |c| requests(c, sub(argv)));
        (out, payloads(&calls, command))
    }

    #[test]
    fn var_and_env_ride_along_with_a_saved_or_an_inline_send() {
        let (out, sent) = send_with(
            &["requests", "send", "r1", "--var", "a=1", "--var", "b=x=y", "--env", "staging"],
            "request.send",
        );
        out.unwrap();
        assert_eq!(
            sent,
            [json!({"workspaceId": "w1", "id": "r1", "variables": {"a": "1", "b": "x=y"}, "environmentId": "e9"})]
        );
        let (out, sent) =
            send_with(&["requests", "send", "--url", "http://x", "--var", "a=", "--env", "e9"], "request.send");
        out.unwrap();
        assert_eq!(sent[0]["variables"], json!({"a": ""}));
        assert_eq!(sent[0]["environmentId"], "e9");
        assert_eq!(sent[0]["request"]["url"], "http://x");
    }

    #[test]
    fn a_bad_var_or_an_unknown_env_is_refused() {
        for bad in ["novalue", "=x"] {
            let (out, sent) = send_with(&["requests", "send", "r1", "--var", bad], "request.send");
            assert_eq!(out.unwrap_err(), CliError::usage(format!("--var must be KEY=VALUE, not \"{bad}\"")));
            assert!(sent.is_empty());
        }
        let (out, sent) = send_with(&["requests", "send", "r1", "--env", "nope"], "request.send");
        assert_eq!(out.unwrap_err(), CliError::not_found("no environment \"nope\" (see: bifurc env list)"));
        assert!(sent.is_empty());
    }

    #[test]
    fn data_replaces_the_saved_body_of_each_type() {
        for (id, command, field) in [
            ("r1", "request.send", "body"),
            ("s1", "soap.send", "body"),
            ("q1", "graphql.send", "variables"),
            ("g1", "grpc.send", "requestBody"),
        ] {
            let (out, sent) = send_with(&["requests", "send", id, "--data", "{\"n\":1}"], command);
            out.unwrap();
            assert_eq!(sent, [json!({"workspaceId": "w1", "id": id, "overrides": {field: "{\"n\":1}"}})]);
        }
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("b.json").to_string_lossy().into_owned();
        std::fs::write(&file, "from file").unwrap();
        let at_file = format!("@{file}");
        let (out, sent) = send_with(&["requests", "send", "r1", "-d", at_file.as_str()], "request.send");
        out.unwrap();
        assert_eq!(sent[0]["overrides"], json!({"body": "from file"}));
    }

    #[test]
    fn patch_changes_fields_of_the_saved_or_given_json_body() {
        // The saved body of `g1` is `{}`; two patches, the second one nested.
        let (out, sent) =
            send_with(&["requests", "send", "g1", "--patch", ".qty=2", "--patch", ".a.b=\"x\""], "grpc.send");
        out.unwrap();
        let body: Value = serde_json::from_str(sent[0]["overrides"]["requestBody"].as_str().unwrap()).unwrap();
        assert_eq!(body, json!({"qty": 2, "a": {"b": "x"}}));
        // Over the body `--data` gives, not the saved one.
        let (out, sent) = send_with(&["requests", "send", "r1", "-d", "{\"a\":1}", "--patch", ".a=3"], "request.send");
        out.unwrap();
        assert_eq!(sent[0]["overrides"], json!({"body": "{\"a\":3}"}));
        // A body that is not JSON.
        let (out, sent) = send_with(&["requests", "send", "s1", "--patch", ".a=1"], "soap.send");
        assert_eq!(out.unwrap_err(), CliError::usage("--patch needs a JSON body, and the body of \"Add\" is not JSON"));
        assert!(sent.is_empty());
    }

    #[test]
    fn output_options_are_refused_for_a_request_that_streams() {
        for flag in [["--select", ".a"], ["--out", "f"], ["--expect-status", "200"]] {
            let argv = ["requests", "send", "--type", "websocket", "--url", "ws://x", flag[0], flag[1]];
            let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
            assert_eq!(out.unwrap_err(), CliError::usage(format!("{} does not apply to a websocket request", flag[0])));
        }
        let argv = ["requests", "send", "--type", "websocket", "--url", "ws://x", "--quiet"];
        let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("--quiet does not apply to a websocket request"));
    }

    #[test]
    fn the_override_options_are_refused_where_they_do_not_apply() {
        for (id, name) in [("k1", "websocket"), ("e1", "sse"), ("i1", "socketio"), ("m1", "mcp")] {
            for option in [["--var", "a=1"], ["--env", "e9"], ["--patch", ".a=1"]] {
                let (out, _) = send_with(&["requests", "send", id, option[0], option[1]], "request.send");
                let message =
                    format!("{} applies to rest, graphql, soap and grpc sends, not to a {name} request", option[0]);
                assert_eq!(out.unwrap_err(), CliError::usage(message));
            }
        }
        let argv = ["requests", "send", "--type", "websocket", "--url", "ws://x", "--var", "a=1"];
        let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert!(out.is_err());
        let (out, _) = send_with(&["requests", "send", "--url", "http://x", "--patch", ".a=1"], "request.send");
        assert_eq!(
            out.unwrap_err(),
            CliError::usage("--patch changes a saved request's body; give the body with --data")
        );
        // Other describing options stay refused for a saved send.
        let (out, _) = send_with(&["requests", "send", "r1", "--var", "a=1", "-H", "x: y"], "request.send");
        assert_eq!(
            out.unwrap_err(),
            CliError::usage(
                "a saved request is sent as it is saved, without --header; change it with: bifurc requests update"
            )
        );
    }

    #[test]
    fn send_inline_sends_the_whole_request() {
        let argv = ["requests", "send", "--url", "http://x", "-X", "put", "-H", "a: 1", "-d", "hi", "-w", "w3"];
        let response = json!({"status": 201, "durationMs": 1, "headers": {}, "body": ""});
        let (out, calls) = run(&argv, vec![("request.send", Ok(response))], |c| requests(c, sub(&argv)));
        assert!(out.unwrap().starts_with("201 Created"));
        assert_eq!(
            calls[0].1,
            json!({"workspaceId": "w3", "scripts": {},
                   "request": {"method": "PUT", "url": "http://x", "headers": {"a": "1"}, "body": "hi"}})
        );
    }

    #[test]
    fn send_inline_of_the_other_types_sends_what_their_command_takes() {
        let argv = [
            "requests",
            "send",
            "-t",
            "grpc",
            "--url",
            "localhost:50051",
            "--method",
            "pkg.Svc/Do",
            "-d",
            "{\"a\":1}",
            "-H",
            "x-k: v",
            "-w",
            "w1",
        ];
        let reply = json!({"ok": true, "status": 0, "durationMs": 3, "metadata": {"m": "1"}, "responses": ["{}"]});
        let (out, calls) = run(&argv, vec![("grpc.send", Ok(reply))], |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "gRPC OK · 3ms · 1 message");
        assert_eq!(
            calls[0],
            (
                "grpc.send".to_string(),
                json!({"workspaceId": "w1", "request": {
                "serverAddress": "localhost:50051", "serviceName": "pkg.Svc", "methodName": "Do",
                "requestBody": "{\"a\":1}", "metadata": {"x-k": "v"}, "protoFileId": null, "useReflection": true}})
            )
        );

        let argv = ["requests", "send", "-t", "graphql", "--url", "http://x/g", "--query", "{a}", "-w", "w1"];
        let reply = json!({"status": 200, "durationMs": 1, "headers": {}, "body": "{}"});
        let (_, calls) = run(&argv, vec![("graphql.send", Ok(reply.clone()))], |c| requests(c, sub(&argv)));
        assert_eq!(
            calls[0].1,
            json!({"workspaceId": "w1", "request": {
                "endpointUrl": "http://x/g", "headers": {}, "query": "{a}", "variables": "", "operationName": ""}})
        );

        let argv =
            ["requests", "send", "-t", "soap", "--url", "http://x/s", "--action", "urn:A", "-d", "<a/>", "-w", "w1"];
        let (_, calls) = run(&argv, vec![("soap.send", Ok(reply))], |c| requests(c, sub(&argv)));
        assert_eq!(
            calls[0].1,
            json!({"workspaceId": "w1", "request": {
                "endpointUrl": "http://x/s", "soapAction": "urn:A", "headers": {}, "body": "<a/>"}})
        );
    }

    #[test]
    fn a_failed_test_or_script_fails_the_command_and_still_prints_the_response() {
        let argv = ["requests", "send", "r1"];
        let response = json!({"status": 200, "durationMs": 3, "headers": {}, "body": "",
                              "tests": [{"name": "is 201", "passed": false, "error": "got 200"}]});
        let mut answers = saved_requests();
        answers.push(("request.send", Ok(response)));
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::Failed("200 OK · 3ms · 0B · tests 0/1 ✗\n✗ test \"is 201\": got 200".into())
        );

        let response = json!({"status": 200, "durationMs": 3, "headers": {}, "body": "", "postScriptError": "boom",
                              "tests": [{"name": "ok", "passed": true}]});
        let mut answers = saved_requests();
        answers.push(("request.send", Ok(response)));
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert!(matches!(out.unwrap_err(), CliError::Failed(report) if report.contains("✗ post-script error: boom")));
    }

    #[test]
    fn a_failed_grpc_call_fails_the_command_with_its_reason() {
        let reply = json!({"ok": false, "error": "UNAVAILABLE: down", "status": 14});
        let argv = ["requests", "send", "g1"];
        let mut answers = saved_requests();
        answers.push(("grpc.send", Ok(reply.clone())));
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::Failed("gRPC 14 UNAVAILABLE · 0ms\n✗ UNAVAILABLE: down".into()));

        let argv = ["requests", "send", "g1", "--json"];
        let mut answers = saved_requests();
        answers.push(("grpc.send", Ok(reply)));
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert!(matches!(out.unwrap_err(), CliError::Failed(report) if report.contains("UNAVAILABLE")));
    }

    #[test]
    fn send_without_an_id_or_a_url_fails() {
        let argv = ["requests", "send"];
        let (out, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::usage("missing request id or name, or --url for a request that is not saved")
        );
        assert!(calls.is_empty());
        let argv = ["requests", "frob"];
        let (out, _) = run(&argv, vec![], |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("unknown: requests frob (see: bifurc requests --help)"));
    }

    /// Send the saved REST request `r1` with `extra` options, the engine answering `response`: the result and
    /// the bytes `--raw` handed out.
    fn send_r1(extra: &[&str], response: Value) -> (Result<String>, Option<Vec<u8>>) {
        let mut argv = vec!["requests", "send", "r1"];
        argv.extend(extra);
        let mut answers = saved_requests();
        answers.push(("request.send", Ok(response)));
        let raw = std::cell::RefCell::new(None);
        let (out, _) = run(&argv, answers, |c| {
            let out = requests(c, sub(&argv));
            *raw.borrow_mut() = c.raw.take();
            out
        });
        (out, raw.into_inner())
    }

    /// What `request.send` answers: the body is base64 as the engine sends it.
    fn reply(status: u16, headers: Value, body: &[u8], tests: Value) -> Value {
        use base64::Engine as _;
        json!({"status": status, "durationMs": 12, "headers": headers,
               "body": base64::engine::general_purpose::STANDARD.encode(body), "tests": tests})
    }

    fn json_reply(body: &str) -> Value {
        reply(200, json!({"Content-Type": "application/json"}), body.as_bytes(), json!([]))
    }

    /// `requests send r1` with `extra`, the engine answering `response`: the result and the payloads of
    /// `session.setVariables`.
    fn extract_with(extra: &[&str], response: Value) -> (Result<String>, Vec<Value>) {
        let mut argv = vec!["requests", "send", "r1"];
        argv.extend(extra);
        let mut answers = saved_requests();
        answers.push(("request.send", Ok(response)));
        answers.push(("session.setVariables", Ok(json!({"ok": true}))));
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        (out, payloads(&calls, "session.setVariables"))
    }

    #[test]
    fn extract_keeps_values_as_session_variables_and_says_so_masked_when_secret() {
        let body = r#"{"cart":{"id":"c_91","n":2},"token":"abcdef"}"#;
        let (out, stored) = extract_with(
            &[
                "--extract",
                "cartId=.cart.id",
                "--extract",
                "n=.cart.n",
                "--extract",
                "authToken=.token",
                "--extract",
                "code=status",
            ],
            json_reply(body),
        );
        let out = out.unwrap();
        assert_eq!(out.lines().nth(1).unwrap(), "extracted cartId=c_91 n=2 authToken=••••(6) code=200");
        assert_eq!(stored, [json!({"variables": {"cartId": "c_91", "n": "2", "authToken": "abcdef", "code": "200"}})]);
        let (out, _) = extract_with(&["--extract", "id=.id", "--json"], json_reply(r#"{"id":"x"}"#));
        assert_eq!(serde_json::from_str::<Value>(&out.unwrap()).unwrap()["extracted"], json!({"id": "x"}));
    }

    #[test]
    fn an_extract_path_that_finds_nothing_fails_with_a_reason() {
        let (out, stored) = extract_with(&["--extract", "id=.nope"], json_reply(r#"{"id":1}"#));
        let Err(CliError::Failed(text)) = out else { panic!("{out:?}") };
        assert_eq!(text.lines().nth(1).unwrap(), "✗ extract id: .nope matched nothing");
        assert!(stored.is_empty());
        let (out, _) = extract_with(&["--extract", "id"], json_reply("{}"));
        assert_eq!(out.unwrap_err().code(), 2);
    }

    #[test]
    fn fresh_session_asks_for_a_throwaway_jar_of_a_rest_send() {
        let (out, sent) = send_with(&["requests", "send", "r1", "--fresh-session"], "request.send");
        out.unwrap();
        assert_eq!(sent[0]["freshJar"], true);
    }

    #[test]
    fn the_default_output_is_one_summary_line() {
        let tests = json!([{"name": "a", "passed": true}, {"name": "b", "passed": true}]);
        let (out, _) = send_r1(&[], reply(200, json!({"content-type": "application/json"}), b"{\"id\":1}", tests));
        assert_eq!(out.unwrap(), "200 OK · 12ms · 8B json · tests 2/2 ✓");
        let (out, _) = send_r1(&[], reply(201, json!({"content-type": "application/json"}), b"{\"id\":77}", json!([])));
        assert_eq!(out.unwrap(), "201 Created · 12ms · 9B json");
        let (out, _) = send_r1(&[], reply(299, json!({}), b"", json!([])));
        assert_eq!(out.unwrap(), "299 · 12ms · 0B");
    }

    #[test]
    fn an_error_status_adds_up_to_384_bytes_of_the_body() {
        let body = format!("{{\"error\":\"{}\"}}", "x".repeat(600));
        let (out, _) =
            send_r1(&[], reply(422, json!({"content-type": "application/json"}), body.as_bytes(), json!([])));
        let out = out.unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "422 Unprocessable Entity · 12ms · 612B json");
        assert_eq!(lines[1], &body[..384]);
        assert_eq!(lines[2], "… 228B truncated — use --out or --select");
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn a_failed_test_fails_with_its_reason_and_the_body_preview() {
        let tests = json!([{"name": "is 201", "passed": false, "error": "got 200"}]);
        let (out, _) = send_r1(&[], reply(200, json!({"content-type": "text/plain"}), b"hi", tests));
        assert_eq!(
            out.unwrap_err(),
            CliError::Failed("200 OK · 12ms · 2B text · tests 0/1 ✗\n✗ test \"is 201\": got 200\nhi".into())
        );
    }

    #[test]
    fn body_prints_the_text_cut_at_max_body_and_zero_means_all() {
        let text = "a".repeat(30);
        let ok = reply(200, json!({"content-type": "text/plain"}), text.as_bytes(), json!([]));
        let (out, _) = send_r1(&["--body"], ok.clone());
        assert_eq!(out.unwrap(), format!("200 OK · 12ms · 30B text\n{text}"));
        let (out, _) = send_r1(&["--body", "--max-body", "10"], ok.clone());
        let cut = format!("200 OK · 12ms · 30B text\n{}\n… 20B truncated — use --out or --select", "a".repeat(10));
        assert_eq!(out.unwrap(), cut);
        let (out, _) = send_r1(&["--body", "--max-body", "0"], ok.clone());
        assert_eq!(out.unwrap(), format!("200 OK · 12ms · 30B text\n{text}"));
        let (out, _) = send_r1(&["--body", "--max-body", "lots"], ok);
        assert_eq!(out.unwrap_err(), CliError::usage("--max-body must be a whole number, not \"lots\""));
    }

    #[test]
    fn select_prints_only_the_values_picked() {
        let (out, _) = send_r1(&["--select", ".items[].id"], json_reply("{\"items\":[{\"id\":1},{\"id\":\"b\"}]}"));
        assert_eq!(out.unwrap(), "1\nb");
        let html = reply(200, json!({"content-type": "text/html"}), b"<p>", json!([]));
        let (out, _) = send_r1(&["--select", ".a"], html);
        assert_eq!(out.unwrap_err(), CliError::msg("--select needs a JSON body, but this response is html"));
        // An error status shows the summary: the values of an error body are not what was asked for.
        let (out, _) = send_r1(&["--select", ".a"], reply(404, json!({}), b"{\"a\":1}", json!([])));
        assert_eq!(out.unwrap(), "404 Not Found · 12ms · 7B json\n{\"a\":1}");
    }

    #[test]
    fn expect_status_and_expect_fail_with_exit_1_and_a_reason() {
        let (out, _) = send_r1(&["--expect-status", "200"], json_reply("{}"));
        assert_eq!(out.unwrap(), "200 OK · 12ms · 2B json");
        let (out, _) = send_r1(&["--expect-status", "201"], json_reply("{}"));
        assert_eq!(
            out.unwrap_err(),
            CliError::Failed("200 OK · 12ms · 2B json\n✗ expect status == 201, got 200\n{}".into())
        );
        let (out, _) = send_r1(&["--expect", "status < 300", "--expect", ".n >= 2"], json_reply("{\"n\":3}"));
        assert!(out.is_ok());
        let (out, _) = send_r1(&["--expect", "status < 300", "--expect", ".n >= 2"], json_reply("{\"n\":1}"));
        assert!(matches!(out.unwrap_err(), CliError::Failed(r) if r.contains("✗ expect .n >= 2, got 1")));
        let text = reply(200, json!({"content-type": "text/plain"}), b"x", json!([]));
        let (out, _) = send_r1(&["--expect", ".n == 1"], text.clone());
        assert!(
            matches!(out.unwrap_err(), CliError::Failed(r) if r.contains("✗ expect .n == 1, but the body is not JSON"))
        );
        let (out, _) = send_r1(&["--expect", "status == 200"], text);
        assert!(out.is_ok());
        let (out, _) = send_r1(&["--expect", "nonsense"], json_reply("{}"));
        assert!(matches!(out.unwrap_err(), CliError::Exit(2, _)));
    }

    #[test]
    fn headers_are_listed_with_secrets_masked() {
        let headers =
            json!({"Content-Type": "text/plain", "Authorization": "Bearer abc", "Set-Cookie": "sid=1; Path=/"});
        let (out, _) = send_r1(&["--headers"], reply(200, headers.clone(), b"", json!([])));
        assert_eq!(
            out.unwrap(),
            "200 OK · 12ms · 0B text\nContent-Type: text/plain\nAuthorization: ••••(10)\nSet-Cookie: ••••(13)"
        );
        // Not a terminal here, so --reveal is ignored.
        let (out, _) = send_r1(&["--headers", "--reveal"], reply(200, headers, b"", json!([])));
        assert!(out.unwrap().contains("Authorization: ••••(10)"));
    }

    #[test]
    fn json_is_the_contract_object() {
        let tests = json!([{"name": "a", "passed": false, "error": "no"}]);
        let ok = reply(200, json!({"Content-Type": "application/json", "X-Token": "t"}), b"{\"a\":[1,2]}", tests);
        let (out, _) = send_r1(&["--json", "--headers", "--select", ".a[]", "--expect", ".a | length == 2"], ok);
        let Err(CliError::Failed(out)) = out else { panic!("a failed test fails the command") };
        let out: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            out,
            json!({"schemaVersion": 1, "status": 200, "statusText": "OK", "durationMs": 12, "size": 11,
                   "contentType": "application/json",
                   "tests": [{"name": "a", "passed": false, "error": "no"}],
                   "body": "{\"a\":[1,2]}", "bodyTruncated": false, "error": "test \"a\": no",
                   "headers": {"Content-Type": "application/json", "X-Token": "••••(1)"}, "selected": [1, 2],
                   "expectations": [{"text": ".a | length == 2", "passed": true, "error": null}]})
        );
        let (out, _) = send_r1(&["--json"], json_reply("{}"));
        let out: Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!((&out["body"], &out["error"], out.get("headers")), (&Value::Null, &Value::Null, None));
    }

    #[test]
    fn raw_hands_out_exactly_the_body_bytes_and_quiet_prints_nothing() {
        let bytes: &[u8] = &[0, 159, 146, 150];
        let (out, raw) = send_r1(&["--raw"], reply(200, json!({}), bytes, json!([])));
        assert_eq!((out.unwrap(), raw.as_deref()), (String::new(), Some(bytes)));
        // A failure is reported on stderr, so that stdout stays the body.
        let failing = reply(200, json!({}), b"x", json!([{"name": "t", "passed": false}]));
        let (out, raw) = send_r1(&["--raw"], failing.clone());
        assert_eq!((out.unwrap_err(), raw), (CliError::msg("✗ test \"t\""), Some(b"x".to_vec())));
        let (out, raw) = send_r1(&["--quiet"], json_reply("{}"));
        assert_eq!((out.unwrap(), raw), (String::new(), None));
        let (out, _) = send_r1(&["--quiet"], failing);
        assert_eq!(out.unwrap_err(), CliError::Failed(String::new()));
    }

    #[test]
    fn out_saves_the_whole_body_and_a_binary_body_is_never_printed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("body.bin").to_string_lossy().into_owned();
        let bytes: Vec<u8> = (0..=255u8).cycle().take(4300).collect();
        let binary = reply(200, json!({"content-type": "application/octet-stream"}), &bytes, json!([]));
        let (out, _) = send_r1(&["--out", &file, "--body"], binary);
        assert_eq!(
            out.unwrap(),
            format!("200 OK · 12ms · 4.2KB binary\nbinary body not shown (4.2KB) — use --out\nsaved {file} (4.2KB)")
        );
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        // Not even a failure shows it.
        let (out, _) = send_r1(&[], reply(500, json!({}), &[0xff, 0xfe, 0xfd], json!([])));
        assert_eq!(out.unwrap(), "500 Internal Server Error · 12ms · 3B binary");
    }

    #[test]
    fn graphql_and_soap_answer_in_the_same_format() {
        for (id, command) in [("q1", "graphql.send"), ("s1", "soap.send")] {
            let argv = ["requests", "send", id, "--expect-status", "200"];
            let mut answers = saved_requests();
            let headers = json!({"content-type": "application/json"});
            answers.push((command, Ok(json!({"status": 200, "durationMs": 4, "headers": headers, "body": "{}"}))));
            let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
            assert_eq!(out.unwrap(), "200 OK · 4ms · 2B json", "{command}");
        }
    }

    #[test]
    fn grpc_summarises_messages_and_names_a_failed_status() {
        let two = json!({"ok": true, "status": 0, "durationMs": 12, "responses": ["{\"a\":1}", "{}"]});
        let argv = ["requests", "send", "g1", "--json"];
        let mut answers = saved_requests();
        answers.push(("grpc.send", Ok(two.clone())));
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        let out: Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!((&out["statusText"], &out["status"], &out["size"]), (&json!("OK"), &json!(0), &json!(12)));

        let argv = ["requests", "send", "g1", "--body"];
        let mut answers = saved_requests();
        answers.push(("grpc.send", Ok(two)));
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "gRPC OK · 12ms · 2 messages\n[{\"a\":1},{}]");

        let argv = ["requests", "send", "g1"];
        let mut answers = saved_requests();
        let failed = json!({"ok": false, "status": 5, "statusMessage": "no such user", "durationMs": 9});
        answers.push(("grpc.send", Ok(failed)));
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::Failed("gRPC 5 NOT_FOUND · 9ms\n✗ no such user".into()));
    }

    #[test]
    fn data_is_the_request_body_and_at_file_reads_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("body.json").to_string_lossy().into_owned();
        std::fs::write(&file, "{\"a\":1}").unwrap();
        let at_file = format!("@{file}");
        let response = json!({"status": 200, "durationMs": 1, "headers": {}, "body": ""});
        for (flag, value, expected) in [
            ("--data", "hi", "hi"),
            ("-d", "hi", "hi"),
            ("--body-file", file.as_str(), "{\"a\":1}"),
            ("--data", at_file.as_str(), "{\"a\":1}"),
        ] {
            let argv = ["requests", "send", "--url", "http://x", flag, value, "-w", "w3"];
            let (_, calls) = run(&argv, vec![("request.send", Ok(response.clone()))], |c| requests(c, sub(&argv)));
            assert_eq!(calls[0].1["request"]["body"], expected, "{flag}");
        }
        // `--body` is still the request body outside of a send.
        let argv = ["requests", "create", "--url", "http://x", "--body", "b"];
        let (_, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(payloads(&calls, "entity.create")[0]["entity"]["body"], "b");
    }

    fn socket(status: &str, messages: Value) -> Result<Value> {
        Ok(json!({"workspaceId": "w1", "socketId": "k1", "url": "ws://x/feed", "status": status, "error": null,
                  "messages": messages}))
    }

    #[test]
    fn sending_a_saved_websocket_connects_sends_listens_and_disconnects() {
        // Every status the engine reports here is the same: the exchange as it stands once the message went.
        let exchanged = json!([{"id": "k1-1", "direction": "sent", "data": "ping"},
                               {"id": "k1-2", "direction": "received", "data": "pong"}]);
        let mut answers = saved_requests();
        answers.extend([
            ("ws.status", socket("disconnected", exchanged)),
            ("ws.connect", Ok(json!({"ok": true}))),
            ("ws.send", Ok(json!({"ok": true}))),
            ("ws.disconnect", Ok(json!({"ok": true}))),
        ]);
        let argv = ["requests", "send", "Feed", "-d", "ping", "--wait", "0"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        // The messages already there before this exchange are not this exchange's.
        assert_eq!(out.unwrap(), "WebSocket disconnected  ws://x/feed");
        let key = json!({"workspaceId": "w1", "socketId": "k1"});
        assert_eq!(payloads(&calls, "ws.connect")[0], key);
        assert_eq!(payloads(&calls, "ws.send"), [json!({"workspaceId": "w1", "socketId": "k1", "data": "ping"})]);
        assert_eq!(payloads(&calls, "ws.disconnect"), [key]);
    }

    #[test]
    fn a_websocket_the_engine_already_holds_is_used_and_left_open() {
        let mut answers = saved_requests();
        answers.extend([("ws.status", socket("connected", json!([]))), ("ws.send", Ok(json!({"ok": true})))]);
        let argv = ["requests", "send", "k1", "-d", "ping", "--wait", "0"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "WebSocket connected  ws://x/feed");
        assert!(payloads(&calls, "ws.connect").is_empty());
        assert!(payloads(&calls, "ws.disconnect").is_empty());
    }

    #[test]
    fn a_websocket_that_does_not_open_fails_the_command() {
        let refused = json!({"workspaceId": "w1", "socketId": "k1", "url": "ws://x/feed", "status": "error",
                             "error": "connection refused", "messages": []});
        let mut answers = saved_requests();
        answers.extend([
            ("ws.status", Ok(refused)),
            ("ws.connect", Ok(json!({"ok": true}))),
            ("ws.disconnect", Ok(json!({"ok": true}))),
        ]);
        let argv = ["requests", "send", "k1", "--wait", "0"];
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::Failed("WebSocket error  ws://x/feed\nerror: connection refused".into())
        );

        let argv = ["requests", "send", "k1", "--wait", "soon"];
        let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("--wait must be a number of seconds, not \"soon\""));
    }

    #[test]
    fn an_unsaved_websocket_connects_to_the_url_given() {
        let answers = vec![
            ("ws.status", socket("connected", json!([]))),
            ("ws.connect", Ok(json!({"ok": true}))),
            ("ws.disconnect", Ok(json!({"ok": true}))),
        ];
        // The first status is asked before connecting; the fake says "connected", so nothing is connected here.
        let argv = ["requests", "send", "-t", "ws", "--url", "ws://x/feed", "-H", "a: 1", "--wait", "0", "-w", "w1"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "WebSocket connected  ws://x/feed");
        assert_eq!(payloads(&calls, "ws.status")[0]["socketId"], format!("cli-{}", std::process::id()));
    }

    #[test]
    fn reflect_lists_methods_with_their_kind() {
        let argv = ["requests", "reflect", "h:1"];
        let reply = json!({"ok": true, "services": [{"name": "p.S", "methods": [
            {"name": "A", "inputType": "p.I", "outputType": "p.O", "clientStreaming": false, "serverStreaming": true}]}]});
        let answers = vec![("config.get", Ok(config())), ("grpc.reflect", Ok(reply))];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "p.S\n  A(p.I) -> p.O  [server streaming]");
        assert_eq!(payloads(&calls, "grpc.reflect"), [json!({"serverAddress": "h:1", "workspaceId": "w1"})]);

        let answers = vec![("config.get", Ok(config())), ("grpc.reflect", Ok(json!({"ok": false, "error": "down"})))];
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("down"));
    }
}
