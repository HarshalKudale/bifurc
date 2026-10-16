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
    error::{CliError, Result},
    format::{
        describe_grpc, describe_reflection, describe_send, describe_socket, describe_text_response, js_template, json,
        table, truthy,
    },
};

/// One type of request.
struct RequestType {
    /// What `--type` calls it.
    name: &'static str,
    /// The entity kind its requests are stored as.
    kind: &'static str,
    /// The command that sends one, saved (`{ workspaceId, id }`) or not (`{ workspaceId, request }`). A
    /// WebSocket has none: it is a connection, not one exchange.
    send: Option<&'static str>,
    /// The field `--url` sets, and the one `-H` adds to.
    url_key: &'static str,
    headers_key: &'static str,
    /// The options only this type takes, beyond `--url`, `--name` and `-H`.
    options: &'static [&'static str],
    /// The fields of a request that is sent without being saved.
    inline: &'static [&'static str],
}

const TYPES: [RequestType; 8] = [
    RequestType {
        name: "rest",
        kind: "requests",
        send: Some("request.send"),
        url_key: "url",
        headers_key: "headers",
        options: &["method", "body", "body-file"],
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
        options: &["method", "body", "body-file", "proto", "reflection"],
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
        options: &["action", "body", "body-file"],
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
        options: &["method", "body", "body-file"],
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

fn type_named(name: &str) -> Result<&'static RequestType> {
    let wanted = match name.to_lowercase().as_str() {
        "ws" => "websocket".to_string(),
        "http" => "rest".to_string(),
        "socket.io" | "sio" => "socketio".to_string(),
        other => other.to_string(),
    };
    TYPES
        .iter()
        .find(|ty| ty.name == wanted)
        .ok_or_else(|| CliError::msg(format!("unknown request type \"{name}\": one of {TYPE_NAMES}")))
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
        describing.contains(name) && !COMMON_OPTIONS.contains(name) && !ty.options.contains(name) && !extra.contains(name)
    });
    match foreign {
        Some(option) => Err(CliError::msg(format!(
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

fn text(entity: &Value, key: &str) -> String {
    entity.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// What the table shows of a request: the same columns whatever its type.
fn row(ty: &RequestType, entity: &Value) -> Value {
    let method = match ty.name {
        "rest" => text(entity, "method"),
        "graphql" => text(entity, "operationName"),
        "grpc" => format!("{}/{}", text(entity, "serviceName"), text(entity, "methodName")),
        "soap" => text(entity, "soapAction"),
        "sse" => text(entity, "method"),
        "socketio" => text(entity, "namespace"),
        "mcp" => text(entity, "transport"),
        _ => String::new(),
    };
    json!({
        "id": entity.get("id"), "type": ty.name, "name": entity.get("name"), "method": method,
        "url": text(entity, ty.url_key),
    })
}

/// Every saved request of `types`, each with its type.
fn saved(ctx: &mut Context, workspace: &str, types: &[&'static RequestType]) -> Result<Vec<(&'static RequestType, Value)>> {
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
        "soap" => json!({ "name": "", "protocol": "soap", "endpointUrl": "", "soapAction": "", "headers": {}, "body": "" }),
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
        _ => Err(CliError::msg("the gRPC method is \"package.Service/Method\"")),
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
        let mut map: Map<String, Value> = entity.get(ty.headers_key).and_then(Value::as_object).cloned().unwrap_or_default();
        merge_headers(&mut map, &headers);
        set(entity, ty.headers_key, Value::Object(map));
    }
    let body = if ty.options.contains(&"body") { body_from(args)? } else { None };
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
                    return Err(CliError::msg(format!("--variables is not valid JSON: {variables}")));
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
                (Some(_), true) => return Err(CliError::msg("--proto and --reflection cannot both be given")),
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
            Some(other) => return Err(CliError::msg(format!("--transport is stdio or http, not \"{other}\""))),
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
        return Err(CliError::msg("missing --url"));
    }
    match ty.name {
        "grpc" if text(entity, "methodName").is_empty() => {
            Err(CliError::msg("missing --method <package.Service/Method>"))
        }
        "graphql" if text(entity, "query").trim().is_empty() => Err(CliError::msg("missing --query or --query-file")),
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

/// What the engine answered a send, printed; a failure of what was sent fails the command.
fn sent(ctx: &Context, ty: &RequestType, response: Value) -> Result<String> {
    let said = |key: &str| response.get(key).and_then(Value::as_str).is_some_and(|message| !message.is_empty());
    let (human, failed) = match ty.name {
        "rest" => {
            let tests = response.get("tests").and_then(Value::as_array);
            let a_test_failed = tests.into_iter().flatten().any(|test| !test.get("passed").is_some_and(truthy));
            (describe_send(&response), a_test_failed || said("preScriptError") || said("postScriptError"))
        }
        "grpc" => {
            if response.get("ok").and_then(Value::as_bool) == Some(false) && !ctx.json {
                let error = response.get("error").and_then(Value::as_str).unwrap_or("the gRPC call failed");
                return Err(CliError::msg(error));
            }
            (describe_grpc(&response), response.get("ok").and_then(Value::as_bool) == Some(false))
        }
        _ => (describe_text_response(&response), false),
    };
    let out = ctx.print(&response, || human);
    if failed { Err(CliError::Failed(out)) } else { Ok(out) }
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
            .ok_or_else(|| CliError::msg(format!("--wait must be a number of seconds, not \"{raw}\""))),
        None => Ok(default),
    }
}

/// The options only a send takes, each for the types [`exchange_options`] says.
const SEND_OPTIONS: [&str; 7] = ["wait", "event", "ack", "tool", "prompt", "resource", "arg"];

/// The options a send of this type takes beyond the ones that describe a request: a WebSocket's message and how
/// long to listen, an SSE stream's listening time (its body is part of the request), a Socket.IO event with its
/// arguments and whether the server is to acknowledge it, what to ask an MCP server for.
fn exchange_options(ty: &RequestType) -> &'static [&'static str] {
    match ty.name {
        "sse" => &["wait"],
        "socketio" => &["event", "body", "ack", "wait"],
        "websocket" => &["body", "wait"],
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
    let seen: Vec<Value> = before.get("messages").and_then(Value::as_array).into_iter().flatten()
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
            if status.get("status").and_then(Value::as_str) != Some("connecting") || started.elapsed() > CONNECT_TIMEOUT {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let mut failure = None;
    if let Some(message) = ctx.args.flag("body") {
        let answer = ctx.request("ws.send", json!({ "workspaceId": workspace, "socketId": socket_id, "data": message }))?;
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
pub fn requests(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    let args = ctx.args;
    let mut describing = describing_options();
    describing.push("type");
    match sub.unwrap_or("list") {
        "list" => {
            ctx.accept(&["type"], 0)?;
            let types = types_of(args)?;
            let workspace = workspace_id(ctx)?;
            let all = saved(ctx, &workspace, &types)?;
            let entities: Vec<Value> = all.iter().map(|(ty, entity)| tagged(ty, entity)).collect();
            let rows: Vec<Value> = all.iter().map(|(ty, entity)| row(ty, entity)).collect();
            Ok(ctx.print(&Value::Array(entities), || table(&rows, &["id", "type", "name", "method", "url"])))
        }
        "show" => {
            ctx.accept(&["type"], 1)?;
            let wanted = need(args.positional(2), "request id or name")?;
            let workspace = workspace_id(ctx)?;
            let (ty, entity) = find(ctx, &workspace, &wanted)?;
            let entity = tagged(ty, &entity);
            Ok(ctx.print(&entity, || json(&entity)))
        }
        "create" | "add" => {
            describing.push("curl");
            ctx.accept(&describing, 1)?;
            let (ty, mut entity) = match args.flag("curl") {
                // The engine reads the command and says which type of request it is.
                Some(curl) => {
                    if args.flag("url").is_some() || args.positional(2).is_some() {
                        return Err(CliError::msg("--curl already says where the request goes; leave out the URL"));
                    }
                    let built = ctx.request("curl.buildRequest", json!({ "curl": curl }))?;
                    let kind = js_template(built.get("kind"));
                    let ty = TYPES
                        .iter()
                        .find(|ty| ty.kind == kind)
                        .ok_or_else(|| CliError::msg(format!("the engine built a request of an unknown kind \"{kind}\"")))?;
                    if args.flag("type").is_some_and(|wanted| type_named(wanted).is_ok_and(|wanted| wanted.name != ty.name)) {
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
            ctx.accept(&describing, 1)?;
            let wanted = need(args.positional(2), "request id or name")?;
            let workspace = workspace_id(ctx)?;
            let (ty, mut entity) = find(ctx, &workspace, &wanted)?;
            check_options(ty, args, &[])?;
            if !apply_options(ty, &mut entity, args)? {
                return Err(CliError::msg(format!(
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
            ctx.accept(&describing, 1)?;
            match args.positional(2) {
                // A saved request goes by id: the engine loads it, resolves it and runs its scripts.
                Some(wanted) => {
                    let workspace = workspace_id(ctx)?;
                    let (ty, entity) = find(ctx, &workspace, wanted)?;
                    let exchange = exchange_options(ty);
                    let given = args.names().into_iter().find(|name| {
                        *name != "type" && describing.contains(name) && !exchange.contains(name)
                    });
                    if let Some(option) = given {
                        return Err(CliError::msg(format!(
                            "a saved request is sent as it is saved, without --{option}; change it with: bifurc requests update"
                        )));
                    }
                    let id = id_of(&entity);
                    match ty.send {
                        Some(command) => {
                            let response = ctx.request(command, json!({ "workspaceId": workspace, "id": id }))?;
                            sent(ctx, ty, response)
                        }
                        None if ty.name == "sse" => crate::stream::stream_exchange(ctx, &workspace, &id, None),
                        None if ty.name == "socketio" => {
                            crate::socketio::socketio_exchange(ctx, &workspace, &id, None)
                        }
                        None if ty.name == "mcp" => crate::mcp::mcp_exchange(ctx, &workspace, &id, None),
                        None => socket_exchange(ctx, &workspace, &id, None),
                    }
                }
                None => {
                    if args.flag("url").is_none() {
                        return Err(CliError::msg("missing request id or name, or --url for a request that is not saved"));
                    }
                    let ty = type_named(args.flag("type").unwrap_or("rest"))?;
                    let exchange = exchange_options(ty);
                    if let Some(option) = SEND_OPTIONS.iter().find(|o| args.has(o) && !exchange.contains(o)) {
                        return Err(CliError::msg(format!(
                            "--{option} is not an option of a {} request (see: bifurc requests --help)",
                            ty.name
                        )));
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
            let response = ctx.request("grpc.reflect", json!({ "serverAddress": address, "workspaceId": workspace }))?;
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
            ("entity.list:graphqlRequests", listing(json!([
                {"id": "q1", "name": "Viewer", "endpointUrl": "http://x/graphql", "operationName": "Viewer",
                 "query": "{ viewer { id } }", "variables": "", "headers": {}}]))),
            ("entity.list:grpcRequests", listing(json!([grpc()]))),
            ("entity.list:soapRequests", listing(json!([
                {"id": "s1", "name": "Add", "endpointUrl": "http://x/soap", "soapAction": "urn:Add", "headers": {},
                 "body": "<a/>"}]))),
            ("entity.list:wsConnections", listing(json!([
                {"id": "k1", "name": "Feed", "url": "ws://x/feed", "headers": {}}]))),
            ("entity.list:sseRequests", listing(json!([
                {"id": "e1", "name": "Events", "url": "http://x/events", "method": "GET", "headers": {}, "body": ""}]))),
            ("entity.list:socketioRequests", listing(json!([
                {"id": "i1", "name": "Chat", "url": "http://x", "namespace": "/chat", "path": "", "auth": "",
                 "headers": {}}]))),
            ("entity.list:mcpRequests", listing(json!([
                {"id": "m1", "name": "Tools", "transport": "stdio", "url": "npx my-server", "headers": {}}]))),
            ("entity.create", Ok(json!({"id": "n1", "entity": {"id": "n1"}}))),
            ("entity.update", Ok(json!({"ok": true}))),
        ]
    }

    #[test]
    fn list_shows_every_type_in_the_same_columns() {
        let argv = ["requests"];
        let (out, _) = run(&argv, saved_requests(), |c| requests(c, None));
        assert_eq!(
            out.unwrap(),
            "ID  TYPE       NAME    METHOD                URL\n\
             r1  rest       Ping    GET                   http://x/ping\n\
             q1  graphql    Viewer  Viewer                http://x/graphql\n\
             g1  grpc       Hello   pkg.Greeter/SayHello  localhost:50051\n\
             s1  soap       Add     urn:Add               http://x/soap\n\
             k1  websocket  Feed                          ws://x/feed\n\
             e1  sse        Events  GET                   http://x/events\n\
             i1  socketio   Chat    /chat                 http://x\n\
             m1  mcp        Tools   stdio                 npx my-server"
        );
    }

    #[test]
    fn list_takes_one_type_and_json_holds_the_requests_with_their_type() {
        let argv = ["requests", "list", "--type", "grpc", "--json"];
        let (out, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        let listed: Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!(listed, json!([with_field(&grpc(), "type", json!("grpc"))]));
        assert_eq!(payloads(&calls, "entity.list"), [json!({"workspaceId": "w1", "kind": "grpcRequests"})]);

        let argv = ["requests", "list", "-t", "ws"];
        let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "ID  TYPE       NAME  METHOD  URL\nk1  websocket  Feed          ws://x/feed");

        let argv = ["requests", "list", "--type", "carrier-pigeon"];
        let (out, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::msg("unknown request type \"carrier-pigeon\": one of rest, graphql, grpc, soap, websocket, sse, socketio, mcp")
        );
        assert!(calls.is_empty());
    }

    #[test]
    fn show_finds_a_request_of_any_type_by_id_or_name() {
        for (wanted, ty) in [("r1", "rest"), ("hello", "grpc"), ("Feed", "websocket"), ("events", "sse"), ("chat", "socketio"), ("tools", "mcp")] {
            let argv = ["requests", "show", wanted];
            let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
            assert_eq!(serde_json::from_str::<Value>(&out.unwrap()).unwrap()["type"], ty, "{wanted}");
        }
        let argv = ["requests", "show", "nope"];
        let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("no request \"nope\" (see: bifurc requests list)"));
        let argv = ["requests", "show", "Hello", "--type", "rest"];
        let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("no request \"Hello\" (see: bifurc requests list)"));
        let argv = ["requests", "show"];
        let (out, _) = run(&argv, vec![], |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("missing request id or name"));
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
            "requests", "create", "--type", "grpc", "--url", "localhost:50051", "--method", "/pkg.Greeter/SayHello",
            "-d", "{\"name\":\"a\"}", "-H", "x-k: v",
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
            "requests", "create", "--type", "graphql", "--url", "http://x/graphql", "--query", "query Me { me { id } }",
            "--variables", "{\"a\":1}", "--operation", "Me",
        ];
        let (_, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(
            payloads(&calls, "entity.create")[0],
            json!({"kind": "graphqlRequests", "workspaceId": "w1", "entity": {
                "name": "Me", "protocol": "graphql", "endpointUrl": "http://x/graphql", "headers": {},
                "query": "query Me { me { id } }", "variables": "{\"a\":1}", "operationName": "Me"}})
        );

        let argv = ["requests", "create", "--type", "soap", "--url", "http://x/soap", "--action", "urn:Add", "-d", "<a/>"];
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
            (vec!["requests", "create", "-t", "grpc", "--url", "h:1", "--method", "NoSlash"],
             "the gRPC method is \"package.Service/Method\""),
            (vec!["requests", "create", "-t", "grpc", "--url", "h:1", "--method", "S/M", "--proto", "p", "--reflection"],
             "--proto and --reflection cannot both be given"),
            (vec!["requests", "create", "-t", "graphql", "--url", "http://x"], "missing --query or --query-file"),
            (vec!["requests", "create", "-t", "graphql", "--url", "http://x", "--query", "{a}", "--variables", "{oops"],
             "--variables is not valid JSON: {oops"),
            (vec!["requests", "create", "--url", "http://x", "--query", "{a}"],
             "--query is not an option of a rest request (see: bifurc requests --help)"),
            (vec!["requests", "create", "-t", "ws", "--url", "ws://x", "-d", "hi"],
             "--body is not an option of a websocket request (see: bifurc requests --help)"),
            (vec!["requests", "create", "--url", "u", "-H", "nocolon"], "header \"nocolon\" must be \"name: value\""),
            (vec!["requests", "create", "--url", "u", "--pre-script", "x"],
             "unknown option --pre-script for \"requests create\" (see: bifurc requests --help)"),
        ] {
            let (out, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
            assert_eq!(out.unwrap_err(), CliError::msg(message), "{argv:?}");
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
        assert_eq!(out.unwrap_err(), CliError::msg("--curl already says where the request goes; leave out the URL"));
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
            CliError::msg("--query is not an option of a grpc request (see: bifurc requests --help)")
        );
        assert!(payloads(&calls, "entity.update").is_empty());

        let argv = ["requests", "update", "g1"];
        let (out, _) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::msg("nothing to change: give at least one option of a grpc request (see: bifurc requests --help)")
        );
    }

    #[test]
    fn send_by_id_sends_only_the_id_with_the_command_of_its_type() {
        let argv = ["requests", "send", "r1"];
        let response = json!({"status": 200, "durationMs": 3, "headers": {"a": "b"}, "body": "b2s="});
        let mut answers = saved_requests();
        answers.push(("request.send", Ok(response)));
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "HTTP 200  (3 ms)\na: b\n\nok");
        assert_eq!(payloads(&calls, "request.send"), [json!({"workspaceId": "w1", "id": "r1"})]);

        let argv = ["requests", "send", "Hello"];
        let mut answers = saved_requests();
        answers.push(("grpc.send", Ok(json!({"ok": true, "status": 0, "durationMs": 3, "responses": ["{}"]}))));
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "gRPC OK  (3 ms)\n\n{}");
        assert_eq!(payloads(&calls, "grpc.send"), [json!({"workspaceId": "w1", "id": "g1"})]);

        for (wanted, command) in [("q1", "graphql.send"), ("s1", "soap.send")] {
            let argv = ["requests", "send", wanted];
            let mut answers = saved_requests();
            answers.push((command, Ok(json!({"status": 200, "durationMs": 1, "headers": {}, "body": "plain"}))));
            let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
            assert_eq!(out.unwrap(), "HTTP 200  (1 ms)\n\nplain");
            assert_eq!(payloads(&calls, command), [json!({"workspaceId": "w1", "id": wanted})]);
        }
    }

    #[test]
    fn send_of_a_saved_request_takes_no_option_that_would_change_it() {
        let argv = ["requests", "send", "r1", "-X", "post"];
        let (out, calls) = run(&argv, saved_requests(), |c| requests(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::msg("a saved request is sent as it is saved, without --method; change it with: bifurc requests update")
        );
        assert!(payloads(&calls, "request.send").is_empty());
    }

    #[test]
    fn send_inline_sends_the_whole_request() {
        let argv = ["requests", "send", "--url", "http://x", "-X", "put", "-H", "a: 1", "-d", "hi", "-w", "w3"];
        let response = json!({"status": 201, "durationMs": 1, "headers": {}, "body": ""});
        let (out, calls) = run(&argv, vec![("request.send", Ok(response))], |c| requests(c, sub(&argv)));
        assert!(out.unwrap().starts_with("HTTP 201"));
        assert_eq!(
            calls[0].1,
            json!({"workspaceId": "w3", "scripts": {},
                   "request": {"method": "PUT", "url": "http://x", "headers": {"a": "1"}, "body": "hi"}})
        );
    }

    #[test]
    fn send_inline_of_the_other_types_sends_what_their_command_takes() {
        let argv = [
            "requests", "send", "-t", "grpc", "--url", "localhost:50051", "--method", "pkg.Svc/Do", "-d", "{\"a\":1}",
            "-H", "x-k: v", "-w", "w1",
        ];
        let reply = json!({"ok": true, "status": 0, "durationMs": 3, "metadata": {"m": "1"}, "responses": ["{}"]});
        let (out, calls) = run(&argv, vec![("grpc.send", Ok(reply))], |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "gRPC OK  (3 ms)\nm: 1\n\n{}");
        assert_eq!(
            calls[0],
            ("grpc.send".to_string(), json!({"workspaceId": "w1", "request": {
                "serverAddress": "localhost:50051", "serviceName": "pkg.Svc", "methodName": "Do",
                "requestBody": "{\"a\":1}", "metadata": {"x-k": "v"}, "protoFileId": null, "useReflection": true}}))
        );

        let argv = ["requests", "send", "-t", "graphql", "--url", "http://x/g", "--query", "{a}", "-w", "w1"];
        let reply = json!({"status": 200, "durationMs": 1, "headers": {}, "body": "{}"});
        let (_, calls) = run(&argv, vec![("graphql.send", Ok(reply.clone()))], |c| requests(c, sub(&argv)));
        assert_eq!(
            calls[0].1,
            json!({"workspaceId": "w1", "request": {
                "endpointUrl": "http://x/g", "headers": {}, "query": "{a}", "variables": "", "operationName": ""}})
        );

        let argv = ["requests", "send", "-t", "soap", "--url", "http://x/s", "--action", "urn:A", "-d", "<a/>", "-w", "w1"];
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
        assert_eq!(out.unwrap_err(), CliError::Failed("HTTP 200  (3 ms)\n\n\nFAIL  is 201 — got 200".into()));

        let response = json!({"status": 200, "durationMs": 3, "headers": {}, "body": "", "postScriptError": "boom",
                              "tests": [{"name": "ok", "passed": true}]});
        let mut answers = saved_requests();
        answers.push(("request.send", Ok(response)));
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert!(matches!(out.unwrap_err(), CliError::Failed(report) if report.contains("post-script error: boom")));
    }

    #[test]
    fn a_failed_grpc_call_is_an_error_and_with_json_a_failed_report() {
        let reply = json!({"ok": false, "error": "UNAVAILABLE: down", "status": 14});
        let argv = ["requests", "send", "g1"];
        let mut answers = saved_requests();
        answers.push(("grpc.send", Ok(reply.clone())));
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("UNAVAILABLE: down"));

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
            CliError::msg("missing request id or name, or --url for a request that is not saved")
        );
        assert!(calls.is_empty());
        let argv = ["requests", "frob"];
        let (out, _) = run(&argv, vec![], |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("unknown: requests frob (see: bifurc requests --help)"));
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
        assert_eq!(out.unwrap_err(), CliError::msg("--wait must be a number of seconds, not \"soon\""));
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
