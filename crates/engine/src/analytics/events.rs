//! What a command becomes in analytics: an event name, and properties that say what kind of thing the user did.
//!
//! Everything that can leave this machine about a command is decided in this file. A property is a boolean, a
//! count, or a word from a fixed list ([`label`], [`http_method`]): never a name, a URL, a header, a body, a
//! script or an id. Only commands a client sends become events, so a runner's requests, an import's entities
//! and the traffic the proxies handle are never counted one by one.

use serde_json::{Map, Value, json};

use crate::{
    kernel::Scope,
    protocol::ErrorCode,
    transport::scopes::required_scope_for,
    utils::json::{bool_of, opt_str, str_of, truthy},
};

pub struct Event {
    pub name: String,
    pub properties: Map<String, Value>,
    /// While they wait to be sent, a later event replaces an earlier one with the same key: saving one entity
    /// ten times is one `saved` event. The key stays on this machine.
    pub replaces: Option<String>,
}

fn event(name: impl Into<String>, properties: Value) -> Option<Event> {
    let properties = properties.as_object().cloned().unwrap_or_default();
    Some(Event { name: name.into(), properties, replaces: None })
}

/// A short machine word (`postman`, `graphqlRequests`), or `other`: free text never passes.
fn label(value: &str) -> &str {
    let plain = !value.is_empty()
        && value.len() <= 40
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
    if plain { value } else { "other" }
}

fn http_method(value: &str) -> &'static str {
    const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
    METHODS.into_iter().find(|method| method.eq_ignore_ascii_case(value.trim())).unwrap_or("OTHER")
}

fn has_text(value: &Value, key: &str) -> bool {
    opt_str(value, key).is_some_and(|text| !text.trim().is_empty())
}

/// How many entries `value[key]` holds, as a map or a list.
fn count(value: &Value, key: &str) -> usize {
    match value.get(key) {
        Some(Value::Object(map)) => map.len(),
        Some(Value::Array(list)) => list.len(),
        _ => 0,
    }
}

/// The event noun of an entity kind, and the request type when the kind is a request.
fn noun(kind: &str) -> Option<(&'static str, Option<&'static str>)> {
    Some(match kind {
        "mocks" => ("mock", None),
        "proxyRules" | "rules" => ("proxy rule", None),
        "mappings" => ("mapping", None),
        "environments" => ("environment", None),
        "flows" => ("flow", None),
        "requests" => ("request", Some("rest")),
        "graphqlRequests" => ("request", Some("graphql")),
        "soapRequests" => ("request", Some("soap")),
        "grpcRequests" => ("request", Some("grpc")),
        "sseRequests" => ("request", Some("sse")),
        "socketioRequests" => ("request", Some("socketio")),
        "mcpRequests" => ("request", Some("mcp")),
        "webhookRequests" => ("request", Some("webhook")),
        "wsConnections" => ("request", Some("websocket")),
        "graphqlSchemas" => ("definition file", Some("graphql")),
        "wsdls" => ("definition file", Some("wsdl")),
        "protoFiles" => ("definition file", Some("proto")),
        _ => return None,
    })
}

fn kind_properties(kind: &str) -> Option<(&'static str, Map<String, Value>)> {
    let (noun, subtype) = noun(kind)?;
    let mut properties = Map::new();
    if let Some(subtype) = subtype {
        let key = if noun == "request" { "request_type" } else { "file_type" };
        properties.insert(key.into(), json!(subtype));
    }
    Some((noun, properties))
}

/// Which parts of the response a mock replaces.
fn mock_shape(mock: &Value) -> Value {
    let streaming = match opt_str(mock, "streamingMode") {
        Some("sse") => "sse",
        Some("chunked") => "chunked",
        _ => "none",
    };
    json!({
        "method": http_method(str_of(mock, "method")),
        "uses_regex": truthy(mock, "useRegex"),
        "standalone": truthy(mock, "standalone"),
        "streaming": streaming,
        "mocks_status": truthy(mock, "responseStatusMocked"),
        "mocks_body": truthy(mock, "responseBodyMocked"),
        "mocks_delay": truthy(mock, "responseDelayMocked"),
        "mocked_header_count": count(mock, "mockedResponseHeaders"),
        "enabled": bool_of(mock, "enabled", false),
    })
}

fn rule_shape(rule: &Value) -> Value {
    let target = match opt_str(rule, "targetType") {
        Some("mapping") => "mapping",
        Some("external") => "external",
        _ => "other",
    };
    json!({
        "uses_regex": truthy(rule, "useRegex"),
        "target": target,
        "has_request_script": has_text(rule, "requestScript"),
        "has_response_script": has_text(rule, "responseScript"),
        "enabled": bool_of(rule, "enabled", false),
    })
}

fn request_shape(request_type: &str, request: &Value) -> Value {
    let mut shape = json!({
        "header_count": count(request, "headers") + count(request, "metadata"),
        "has_pre_script": has_text(request, "preScript"),
        "has_post_script": has_text(request, "postScript"),
        "has_tests": has_text(request, "testScript"),
    });
    match request_type {
        "rest" => {
            let body = if count(request, "bodyParts") > 0 {
                "multipart"
            } else if !has_text(request, "body") {
                "none"
            } else if opt_str(request, "bodyEncoding") == Some("base64") {
                "binary"
            } else {
                "text"
            };
            shape["method"] = json!(http_method(str_of(request, "method")));
            shape["body"] = json!(body);
        }
        "graphql" => shape["has_variables"] = json!(has_text(request, "variables")),
        "grpc" => {
            shape["uses_reflection"] = json!(truthy(request, "useReflection"));
            shape["has_proto_file"] = json!(has_text(request, "protoFileId"));
        }
        _ => {}
    }
    shape
}

fn entity_event(verb: &str, kind: &str, entity: Option<&Value>) -> Option<Event> {
    let (noun, mut properties) = kind_properties(kind)?;
    let shape = entity.map(|entity| match noun {
        "mock" => mock_shape(entity),
        "proxy rule" => rule_shape(entity),
        "request" => request_shape(properties["request_type"].as_str().unwrap_or(""), entity),
        _ => json!({}),
    });
    if let Some(Value::Object(shape)) = shape {
        properties.extend(shape);
    }
    let replaces = (verb == "saved").then(|| format!("{kind}/{}", str_of(entity.unwrap_or(&Value::Null), "id")));
    Some(Event { name: format!("{noun} {verb}"), properties, replaces })
}

/// A request going out: its type, and whether it was a saved one or one typed in and sent.
fn sent(request_type: &str, params: &Value, streaming: bool) -> Option<Event> {
    let mut properties = json!({ "request_type": request_type, "saved": has_text(params, "id") });
    if streaming {
        properties["streaming"] = json!(true);
    }
    event("request sent", properties)
}

/// The workspace a command is about, by its id on this device: what its payload names as `workspaceId`, and
/// for the commands on a workspace itself the workspace they name. `None` for a command that belongs to no
/// workspace, and for one that only makes a workspace (there is none yet to be anyone's).
pub fn workspace_of<'a>(action: &str, params: &'a Value) -> Option<&'a str> {
    let named = |key: &str| opt_str(params, key).filter(|id| !id.is_empty());
    match action {
        "workspace.rename" | "workspace.delete" | "workspace.setActive" => named("id"),
        "workspace.add" | "workspace.import" => None,
        _ => named("workspaceId"),
    }
}

/// The id of the workspace a command made, which only its result knows: `workspace.add` answers the workspace
/// itself, `workspace.import` `{ workspace }`. `None` for any other command, and for one that made nothing.
pub fn made_workspace<'a>(action: &str, result: &'a Value) -> Option<&'a str> {
    let id = match action {
        "workspace.add" => result.get("id"),
        "workspace.import" => result.pointer("/workspace/id"),
        _ => None,
    };
    id.and_then(Value::as_str).filter(|id| !id.is_empty())
}

/// Whether `action` is something a user does. A read is what an interface does to draw itself (status polls,
/// lists, variable resolution), apart from the few a user asks for by hand; switching an entity on or off and
/// the steps of a transfer are not counted either.
pub fn is_user_action(action: &str) -> bool {
    const ASKED_FOR_READS: [&str; 5] =
        ["request.parseCurl", "curl.buildRequest", "curl.buildMock", "graphql.introspect", "soap.fetchWsdl"];
    const NOT_COUNTED: [&str; 5] =
        ["entity.setEnabled", "blob.put", "blob.release", "import.preflight", "runner.loadConfig"];
    match required_scope_for(action) {
        None => false,
        Some(Scope::Read) => ASKED_FOR_READS.contains(&action),
        // The browser companion's commands are a page's own traffic, one call per request.
        Some(_) => !NOT_COUNTED.contains(&action) && !action.starts_with("browser."),
    }
}

/// The event `action` becomes if it succeeds; `None` for a command that goes out under its own name
/// ([`finished`]).
pub fn describe(action: &str, params: &Value) -> Option<Event> {
    let kind = str_of(params, "kind");
    match action {
        "entity.create" => {
            let verb = if noun(kind)?.0 == "definition file" { "added" } else { "created" };
            entity_event(verb, kind, params.get("entity"))
        }
        "entity.update" => entity_event("saved", kind, params.get("entity")),
        "entity.delete" => entity_event("deleted", kind, None),
        "entity.duplicate" => entity_event("duplicated", kind, None),
        "folder.add" => event("folder created", json!({ "kind": label(kind) })),

        "request.send" => sent("rest", params, false),
        "graphql.send" | "graphql.execute" => sent("graphql", params, false),
        "soap.send" | "soap.execute" => sent("soap", params, false),
        "webhook.send" => sent("webhook", params, false),
        "grpc.send" | "grpc.execute" => sent("grpc", params, false),
        "grpc.stream.open" => sent("grpc", params, true),
        "sse.connect" => event("request sent", json!({ "request_type": "sse", "saved": !has_text(params, "url") })),
        "mcp.connect" => event("request sent", json!({ "request_type": "mcp", "saved": !has_text(params, "url") })),
        "socketio.connect" => {
            event("request sent", json!({ "request_type": "socketio", "saved": !has_text(params, "url") }))
        }
        "ws.connect" => event("request sent", json!({ "request_type": "websocket", "saved": true })),
        "request.replay" => event("request replayed", json!({ "method": http_method(str_of(params, "method")) })),
        "runner.start" => event("runner started", json!({})),
        "flow.run" => event("flow started", json!({})),

        "workspace.add" => event("workspace created", json!({ "with_account": has_text(params, "accountId") })),
        "workspace.import" => event("workspace imported", json!({})),
        "workspace.delete" => event("workspace deleted", json!({})),
        "sync.publish" => event(
            "workspace published",
            json!({ "private": bool_of(params, "private", true), "sync_all": truthy(params, "syncAll") }),
        ),
        "sync.all" => event("workspace synced", json!({ "how": "all" })),
        "sync.push" => event("workspace synced", json!({ "how": "push" })),
        "sync.pull" => event("workspace synced", json!({ "how": "pull" })),
        "git.sync" | "entity.publish" | "folder.publish" => {
            event("changes synced", json!({ "path_count": count(params, "paths") }))
        }
        "account.login.start" => event("account sign-in started", json!({})),
        "account.logout" => event("account signed out", json!({})),

        "import.commit" => event(
            "import committed",
            json!({
                "kind": label(kind),
                "format": label(str_of(params, "format")),
                "collisions": label(opt_str(params, "collisionStrategy").unwrap_or("default")),
            }),
        ),
        "export.create" => {
            event("export created", json!({ "kind": label(kind), "format": label(str_of(params, "format")) }))
        }
        "capture.buildMock" => event("mock built from capture", json!({})),
        "capture.buildRequest" => event("request built from capture", json!({})),
        "capture.import" => event(
            "requests imported from capture",
            json!({ "secrets": label(opt_str(params, "secrets").unwrap_or("keep")), "cookiesToJar": truthy(params, "cookiesToJar"), "flow": truthy(params, "flow") }),
        ),
        "curl.buildMock" => event("mock built from curl", json!({})),
        "curl.buildRequest" => event("request built from curl", json!({})),

        "vault.create" => event("vault created", json!({})),
        "vault.setSecret" => event("vault secret set", json!({})),
        // Saving the list is an event only when it creates a check, which the result says ([`complete`]).
        "healthbar.saveServices" => event("health check created", json!({})),
        // Only a check the user asked for: the ones an interface runs on a timer are not sent. Checking a
        // whole list at once is one event.
        "healthbar.checkUrl" if bool_of(params, "manual", false) => Some(Event {
            name: "health check run".to_string(),
            properties: Map::new(),
            replaces: Some("health check run".to_string()),
        }),
        "tls.generate" => event("tls certificate generated", json!({})),
        "tls.importCert" => event("tls certificate imported", json!({})),
        _ => None,
    }
}

/// Add what only the command's result says. `false` when the result reports that nothing was done.
pub fn complete(event: &mut Event, action: &str, result: &Value) -> bool {
    // The one refusal that is an event: someone wanted a workspace that is another account's.
    if action == "workspace.import" && opt_str(result, "reason") == Some(crate::gitops::NOT_OWNER) {
        event.name = "workspace import blocked".to_string();
        event.properties.insert("reason".into(), json!("not_owner"));
        return true;
    }
    if declined(result) {
        return false;
    }
    if action == "healthbar.saveServices" {
        return result.get("added").and_then(Value::as_u64).unwrap_or(0) > 0;
    }
    if action == "request.send" {
        let request = result.get("request").unwrap_or(&Value::Null);
        event.properties.insert("method".into(), json!(http_method(str_of(request, "method"))));
        event.properties.insert("has_tests".into(), json!(count(result, "tests") > 0));
        let class = match result.get("status").and_then(Value::as_u64) {
            Some(status @ 100..=599) => format!("{}xx", status / 100),
            _ => "none".to_string(),
        };
        event.properties.insert("status_class".into(), json!(class));
    }
    true
}

/// The code of a command that answered, and answered that it could not do it.
pub const NOT_OK: &str = "NOT_OK";

/// Whether `result` is an answer that says the command did not do what was asked.
fn declined(result: &Value) -> bool {
    result.get("ok") == Some(&json!(false)) || opt_str(result, "status") == Some("failed")
}

fn failed(action: &str, code: &str) -> Option<Event> {
    event("command failed", json!({ "command": action, "code": code }))
}

/// The one event of a finished command, or `None` when it has none. `described` is what [`describe`] made of
/// it before it ran; `result` is its data, or its error code. A command that succeeded is the event that
/// describes it, or else its own name. One that failed is `command failed`, whether it answered with an error
/// or with a result that says it could not ([`NOT_OK`], and the machine word the result gives as its
/// `reason`). Nothing but the name, the code and that word leaves with a failure.
///
/// What an interface does on its own is an event only when the engine itself failed at it, and then one per
/// send, however often the interface asked.
pub fn finished(described: Option<Event>, action: &str, result: Result<&Value, &str>) -> Option<Event> {
    let not_ok = |result: &Value| {
        let mut event = failed(action, NOT_OK)?;
        if let Some(reason) = opt_str(result, "reason") {
            event.properties.insert("reason".into(), json!(label(reason)));
        }
        Some(event)
    };
    match (result, described) {
        (Ok(result), Some(mut described)) => {
            if complete(&mut described, action, result) {
                Some(described)
            } else if declined(result) {
                not_ok(result)
            } else {
                None
            }
        }
        (Ok(_), None) if !is_user_action(action) => None,
        (Ok(result), None) if declined(result) => not_ok(result),
        (Ok(_), None) => event(action, json!({})),
        (Err(code), _) if is_user_action(action) => failed(action, code),
        (Err(code), _) if code == ErrorCode::EngineError.as_str() => {
            let mut event = failed(action, code)?;
            event.replaces = Some(format!("failed/{action}"));
            Some(event)
        }
        (Err(_), _) => None,
    }
}

/// Something that failed in the engine with no command to answer for it: `what` is a fixed word of the code
/// that noticed (`listener_bind`). One per send, however often it happened.
pub fn failure(what: &'static str) -> Event {
    Event {
        name: "engine failure".to_string(),
        properties: Map::from_iter([("what".to_string(), json!(what))]),
        replaces: Some(format!("failure/{what}")),
    }
}

/// Where in Bifurc's code a panic was, as a file of a crate (`engine/src/main.rs`). A packaged build
/// already names its files that way; a build made anywhere else names them under the home directory of
/// whoever built it, and that part is cut off.
fn place(file: &str) -> String {
    let path = file.replace('\\', "/");
    let absolute = path.starts_with('/') || path.as_bytes().get(1) == Some(&b':');
    let kept = match path.rfind("/src/") {
        Some(at) if absolute => &path[path[..at].rfind('/').map_or(0, |slash| slash + 1)..],
        _ if absolute => path.rsplit('/').next().unwrap_or(""),
        _ => path.as_str(),
    };
    let plain = !kept.is_empty()
        && kept.len() <= 200
        && kept.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/'));
    if plain { kept.to_string() } else { "unknown".to_string() }
}

/// What a crash report (`bifurc_protocol::crash`) becomes. A panic is a PostHog `$exception`, so that it is
/// grouped and tracked as an error there: which process, which version, and the file and line of Bifurc's
/// code. The message and the backtrace of the report can hold what a user typed, and stay in the file.
pub fn reported(report: &Value) -> Option<Event> {
    let process = label(str_of(report, "process"));
    let version = label(str_of(report, "version"));
    match opt_str(report, "kind") {
        Some("failure") => event(
            "app failure",
            json!({ "process": process, "app_version": version, "what": label(str_of(report, "what")) }),
        ),
        Some("panic") => {
            let mut parts = str_of(report, "location").rsplitn(3, ':');
            let column = parts.next().and_then(|n| n.parse::<u32>().ok());
            let line = parts.next().and_then(|n| n.parse::<u32>().ok());
            let file = place(parts.next().unwrap_or(""));
            event(
                "$exception",
                json!({
                    "process": process,
                    "app_version": version,
                    "$exception_list": [{
                        "type": format!("{process} panic"),
                        "value": format!("panicked at {file}:{}", line.unwrap_or(0)),
                        "mechanism": { "handled": false, "synthetic": false },
                        "stacktrace": { "type": "raw", "frames": [{
                            "platform": "custom", "lang": "rust", "function": "panic", "filename": file,
                            "lineno": line, "colno": column, "in_app": true, "resolved": true,
                        }] },
                    }],
                }),
            )
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn properties(event: &Event) -> Value {
        Value::Object(event.properties.clone())
    }

    #[test]
    fn a_mock_is_described_by_what_it_replaces() {
        let mock = json!({
            "id": "m1", "name": "Login", "method": "post", "urlPattern": "https://api.example.com/login",
            "responseStatusMocked": true, "responseStatus": 503, "responseBodyMocked": false,
            "responseBody": "{\"token\":\"secret\"}", "mockedResponseHeaders": ["x-one", "x-two"],
            "streamingMode": "sse", "enabled": true,
        });
        let event = describe("entity.update", &json!({ "kind": "mocks", "entity": mock })).unwrap();
        assert_eq!(event.name, "mock saved");
        assert_eq!(event.replaces.as_deref(), Some("mocks/m1"));
        assert_eq!(
            properties(&event),
            json!({
                "method": "POST", "uses_regex": false, "standalone": false, "streaming": "sse",
                "mocks_status": true, "mocks_body": false, "mocks_delay": false, "mocked_header_count": 2,
                "enabled": true,
            })
        );
    }

    #[test]
    fn a_request_is_described_by_its_type_and_parts() {
        let request = json!({
            "name": "Create user", "method": "POST", "url": "https://api.example.com/users",
            "headers": { "authorization": "Bearer secret", "accept": "*/*" }, "body": "{\"email\":\"a@b.c\"}",
            "testScript": "bifurc.test('ok', () => {})",
        });
        let event = describe("entity.create", &json!({ "kind": "requests", "entity": request })).unwrap();
        assert_eq!(event.name, "request created");
        assert_eq!(
            properties(&event),
            json!({
                "request_type": "rest", "header_count": 2, "has_pre_script": false, "has_post_script": false,
                "has_tests": true, "method": "POST", "body": "text",
            })
        );

        let rule = json!({ "pattern": "^https://internal/", "useRegex": true, "targetType": "external",
            "targetExternal": "http://10.0.0.4:8080", "requestScript": "req.headers.x = 'y'" });
        let event = describe("entity.create", &json!({ "kind": "proxyRules", "entity": rule })).unwrap();
        assert_eq!(event.name, "proxy rule created");
        assert_eq!(
            properties(&event),
            json!({ "uses_regex": true, "target": "external", "has_request_script": true,
                "has_response_script": false, "enabled": false })
        );
    }

    #[test]
    fn nothing_a_user_typed_is_in_an_event() {
        let secret = "s3cret-value";
        let entity = json!({
            "id": secret, "name": secret, "method": secret, "url": secret, "urlPattern": secret, "pattern": secret,
            "headers": { secret: secret }, "body": secret, "responseBody": secret, "preScript": secret,
            "requestScript": secret, "targetType": secret, "targetExternal": secret, "streamingMode": secret,
            "variables": secret, "protoFileId": secret, "mockedResponseHeaders": [secret],
        });
        let kinds = [
            "mocks",
            "proxyRules",
            "mappings",
            "environments",
            "requests",
            "graphqlRequests",
            "soapRequests",
            "grpcRequests",
            "sseRequests",
            "socketioRequests",
            "mcpRequests",
            "webhookRequests",
            "wsConnections",
            "graphqlSchemas",
            "wsdls",
            "protoFiles",
        ];
        let mut events: Vec<Event> = Vec::new();
        for kind in kinds {
            for action in ["entity.create", "entity.update", "entity.delete", "entity.duplicate"] {
                events.extend(describe(action, &json!({ "kind": kind, "entity": entity, "id": secret })));
            }
        }
        let params = json!({
            "workspaceId": secret, "id": secret, "name": secret, "accountId": secret, "repo": secret,
            "repoName": secret, "kind": "free text!", "format": "also free text", "paths": [secret],
            "method": secret, "url": secret, "passphrase": secret, "value": secret, "blobId": secret,
            "manual": true, "services": [{ "id": secret, "name": secret, "url": secret }],
        });
        for action in crate::protocol::schema::command_names() {
            if let Some(mut event) = describe(action, &params) {
                let result = json!({
                    "request": { "method": secret, "url": secret }, "status": 200, "body": secret, "added": 1,
                });
                assert!(complete(&mut event, action, &result));
                events.push(event);
            }
        }
        assert!(events.len() > 60);
        for event in events {
            let sent = format!("{} {}", event.name, properties(&event));
            assert!(!sent.contains(secret) && !sent.contains("free text"), "{sent}");
        }
    }

    #[test]
    fn what_an_interface_does_on_its_own_is_not_counted() {
        for action in [
            "server.status",
            "env.resolveText",
            "healthbar.checkUrl",
            "setup.getState",
            "config.get",
            "entity.setEnabled",
        ] {
            assert!(!is_user_action(action), "{action}");
        }
        assert!(describe("env.resolveText", &json!({ "workspaceId": "w", "text": "{{HOST}}" })).is_none());
        for action in ["request.send", "entity.create", "workspace.add", "request.parseCurl", "runner.start"] {
            assert!(is_user_action(action), "{action}");
        }
        assert!(describe("entity.setEnabled", &json!({ "kind": "mocks", "id": "m", "enabled": true })).is_none());
    }

    #[test]
    fn a_command_is_about_the_workspace_it_names() {
        assert_eq!(workspace_of("request.send", &json!({ "workspaceId": "w", "id": "r" })), Some("w"));
        assert_eq!(workspace_of("entity.create", &json!({ "workspaceId": "w", "kind": "mocks" })), Some("w"));
        assert_eq!(workspace_of("workspace.delete", &json!({ "id": "w" })), Some("w"));
        assert_eq!(workspace_of("workspace.rename", &json!({ "id": "w", "name": "n" })), Some("w"));
        // Nothing to be anyone's yet, and nothing to do with a workspace.
        assert_eq!(workspace_of("workspace.add", &json!({ "name": "n", "accountId": "a" })), None);
        assert_eq!(workspace_of("workspace.import", &json!({ "accountId": "a", "repo": "octo/team" })), None);
        assert_eq!(workspace_of("account.logout", &json!({ "id": "github.com:7" })), None);
        assert_eq!(workspace_of("request.send", &json!({ "workspaceId": "" })), None);
        assert_eq!(workspace_of("request.send", &json!({ "workspaceId": 7 })), None);
    }

    #[test]
    fn a_finished_command_is_one_event() {
        let create = json!({ "kind": "requests", "entity": { "id": "r1", "method": "GET" } });
        let created =
            finished(describe("entity.create", &create), "entity.create", Ok(&json!({ "ok": true }))).unwrap();
        assert_eq!(created.name, "request created");

        let parsed = finished(describe("request.parseCurl", &json!({})), "request.parseCurl", Ok(&json!({}))).unwrap();
        assert_eq!((parsed.name.as_str(), properties(&parsed)), ("request.parseCurl", json!({})));

        let failed = finished(describe("entity.create", &create), "entity.create", Err("invalid_params")).unwrap();
        assert_eq!(
            (failed.name.as_str(), properties(&failed)),
            ("command failed", json!({ "command": "entity.create", "code": "invalid_params" }))
        );

        // What an interface does on its own is no event, whether it worked or not.
        assert!(finished(None, "server.status", Ok(&json!({}))).is_none());
        assert!(finished(None, "server.status", Ok(&json!({ "ok": false }))).is_none());
        assert!(finished(None, "server.status", Err("NOT_FOUND")).is_none());
        // Unless the engine itself failed at it: once per send.
        let broken = finished(None, "server.status", Err("ENGINE_ERROR")).unwrap();
        assert_eq!(properties(&broken), json!({ "command": "server.status", "code": "ENGINE_ERROR" }));
        assert_eq!(broken.replaces.as_deref(), Some("failed/server.status"));
    }

    #[test]
    fn a_command_that_answers_that_it_could_not_is_a_failure() {
        let deleted = describe("workspace.delete", &json!({ "id": "w" }));
        let failed = finished(deleted, "workspace.delete", Ok(&json!({ "ok": false, "error": "C:\\secret" }))).unwrap();
        assert_eq!(
            (failed.name.as_str(), properties(&failed)),
            ("command failed", json!({ "command": "workspace.delete", "code": "NOT_OK" }))
        );
        // The result's reason goes along when it is a machine word, and only then.
        let reasoned = json!({ "ok": false, "reason": "notOwner" });
        let failed = finished(None, "request.parseCurl", Ok(&reasoned)).unwrap();
        assert_eq!(failed.properties["reason"], "notOwner");
        let failed =
            finished(None, "request.parseCurl", Ok(&json!({ "ok": false, "reason": "no such file" }))).unwrap();
        assert_eq!(failed.properties["reason"], "other");
        // A result that only says nothing was added is neither an event nor a failure.
        let saved = describe("healthbar.saveServices", &json!({ "workspaceId": "w" }));
        assert!(finished(saved, "healthbar.saveServices", Ok(&json!({ "ok": true, "added": 0 }))).is_none());
    }

    #[test]
    fn a_crash_report_is_sent_as_where_it_happened_and_nothing_else() {
        let secret = "s3cret-value";
        let report = json!({
            "kind": "panic", "process": "engine", "version": "0.5.0", "thread": secret,
            "location": "bifurc\\crates\\engine\\src\\main.rs:53:9", "message": secret, "backtrace": secret,
        });
        let event = reported(&report).unwrap();
        assert_eq!(event.name, "$exception");
        let exception = &event.properties["$exception_list"][0];
        assert_eq!(exception["type"], "engine panic");
        assert_eq!(exception["value"], "panicked at bifurc/crates/engine/src/main.rs:53");
        let frame = &exception["stacktrace"]["frames"][0];
        assert_eq!((&frame["lineno"], &frame["colno"]), (&json!(53), &json!(9)));
        assert!(!properties(&event).to_string().contains(secret));

        // A build made in someone's home directory names its files there: only the crate's part is kept.
        assert_eq!(place("C:\\Users\\eve\\Bifurc\\crates\\engine\\src\\rest\\send.rs"), "engine/src/rest/send.rs");
        assert_eq!(place("/home/eve/.cargo/registry/src/index/tokio-1.0.0/src/task.rs"), "tokio-1.0.0/src/task.rs");
        assert_eq!(place("/home/eve/build.rs"), "build.rs");
        assert_eq!(place("packages/my secret/src/x.rs"), "unknown");

        let failure = json!({ "kind": "failure", "process": "desktop", "version": "0.5.0", "what": "engine_start" });
        let event = reported(&failure).unwrap();
        assert_eq!(event.name, "app failure");
        assert_eq!(properties(&event), json!({ "process": "desktop", "app_version": "0.5.0", "what": "engine_start" }));
        assert!(reported(&json!({ "kind": secret })).is_none());
    }

    #[test]
    fn a_health_check_is_an_event_when_it_is_created_or_run_by_hand() {
        let url = "https://internal.example.com/health";
        assert!(describe("healthbar.checkUrl", &json!({ "url": url })).is_none());
        assert!(describe("healthbar.checkUrl", &json!({ "url": url, "manual": false })).is_none());
        let run = describe("healthbar.checkUrl", &json!({ "url": url, "manual": true })).unwrap();
        assert_eq!((run.name.as_str(), properties(&run)), ("health check run", json!({})));
        assert!(run.replaces.is_some());

        let save = json!({ "workspaceId": "w", "services": [{ "id": "s1", "name": "Billing", "url": url }] });
        let mut created = describe("healthbar.saveServices", &save).unwrap();
        assert!(!complete(&mut created, "healthbar.saveServices", &json!({ "ok": true, "added": 0 })));
        assert!(complete(&mut created, "healthbar.saveServices", &json!({ "ok": true, "added": 1 })));
        assert_eq!((created.name.as_str(), properties(&created)), ("health check created", json!({})));
    }

    #[test]
    fn a_command_that_did_nothing_is_not_an_event() {
        let mut event = describe("workspace.delete", &json!({ "id": "w" })).unwrap();
        assert!(!complete(&mut event, "workspace.delete", &json!({ "ok": false })));
        assert!(complete(&mut event, "workspace.delete", &json!({ "ok": true })));

        let import = json!({ "accountId": "github.com:7", "repo": "octo/team" });
        let mut refused = describe("workspace.import", &import).unwrap();
        let not_a_workspace = json!({ "ok": false, "error": "octo/team is not a Bifurc workspace" });
        assert!(!complete(&mut refused, "workspace.import", &not_a_workspace));
        let not_owner = json!({ "ok": false, "error": "octo/team belongs to @octo, not @eve.", "reason": "notOwner" });
        assert!(complete(&mut refused, "workspace.import", &not_owner));
        assert_eq!(
            (refused.name.as_str(), properties(&refused)),
            ("workspace import blocked", json!({ "reason": "not_owner" }))
        );

        let mut sent = describe("request.send", &json!({ "workspaceId": "w", "id": "r" })).unwrap();
        let result = json!({ "request": { "method": "get" }, "status": 404, "tests": [{}] });
        assert!(complete(&mut sent, "request.send", &result));
        assert_eq!(
            properties(&sent),
            json!({ "request_type": "rest", "saved": true, "method": "GET", "has_tests": true, "status_class": "4xx" })
        );
    }
}
