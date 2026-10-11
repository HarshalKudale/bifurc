//! Flows run against loopback servers, through the commands a client uses.

use std::{collections::HashMap, sync::Arc, time::Duration};

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use super::run::reset_runs;
use crate::{
    kernel::{CommandContext, CommandRegistry, bus},
    rest::test_server,
    testing::{ScratchRoot, scratch_data_root},
    utils::json::{array_of, str_of},
    workspace::fs::{create_folder, read_entity, write_entity, write_flat_entity},
};

const WS: &str = "ws1";

fn context() -> (Arc<CommandRegistry>, CommandContext) {
    let mut registry = CommandRegistry::new();
    super::register(&mut registry);
    crate::rest::register(&mut registry);
    crate::scripting::register(&mut registry);
    crate::sse::register(&mut registry);
    crate::grpc::register(&mut registry);
    crate::websocket::register(&mut registry);
    crate::socketio::register(&mut registry);
    let registry = Arc::new(registry);
    let ctx = CommandContext::internal(registry.clone());
    (registry, ctx)
}

async fn call(action: &str, payload: Value) -> Value {
    let (registry, ctx) = context();
    registry.invoke(action, payload, ctx).await.unwrap()
}

/// A scratch store with a folder for the requests a test saves.
struct Store {
    _root: ScratchRoot,
    folder: String,
}

fn store() -> Store {
    let root = scratch_data_root();
    reset_runs();
    let folder = create_folder(WS, "requests", "F", None, false).unwrap().0;
    Store { _root: root, folder }
}

impl Store {
    fn request(&self, id: &str, url: String, extra: Value) {
        let mut entity = json!({
            "id": id, "name": id, "method": "GET", "url": url, "headers": {}, "body": "", "workspaceId": WS, "createdAt": 0,
        });
        entity.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        write_entity(WS, "requests", id, &entity, Some(self.folder.as_str())).unwrap();
    }

    fn stream(&self, id: &str, url: String) {
        let entity = json!({ "id": id, "name": id, "method": "GET", "url": url, "headers": {}, "workspaceId": WS, "createdAt": 0 });
        write_entity(WS, "sseRequests", id, &entity, Some(self.folder.as_str())).unwrap();
    }

    /// A saved request of another type, beside the REST ones.
    fn saved(&self, kind: &str, id: &str, mut entity: Value) {
        entity["id"] = json!(id);
        entity["name"] = json!(id);
        write_entity(WS, kind, id, &entity, Some(self.folder.as_str())).unwrap();
    }

    fn flow(&self, id: &str, mut flow: Value) {
        flow["id"] = json!(id);
        flow["name"] = json!(id);
        write_flat_entity(WS, "flows", id, &flow).unwrap();
    }
}

fn start() -> Value {
    json!({ "id": "start", "type": "start" })
}

fn request(id: &str, request_id: &str, extra: Value) -> Value {
    let mut node = json!({ "id": id, "type": "request", "requestKind": "requests", "requestId": request_id });
    node.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    node
}

fn edge(from: &str, port: &str, to: &str) -> Value {
    json!({ "id": format!("{from}.{port}-{to}"), "from": from, "port": port, "to": to })
}

/// Start a run and wait for its `flow.finished` report.
async fn run_with(id: &str, extra: Value) -> Value {
    let finished = Arc::new(Mutex::new(None::<Value>));
    let (sink, flow_id) = (finished.clone(), id.to_string());
    let listener = bus().on("flow.finished", move |payload| {
        if str_of(payload, "flowId") == flow_id {
            *sink.lock() = Some(payload["report"].clone());
        }
    });
    let mut params = json!({ "workspaceId": WS, "id": id });
    params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    let started = call("flow.run", params).await;
    assert_eq!(started["ok"], true, "{started}");
    for _ in 0..400 {
        if finished.lock().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    bus().off(listener);
    finished.lock().take().expect("the run finishes")
}

async fn run(id: &str) -> Value {
    run_with(id, json!({})).await
}

fn node<'a>(report: &'a Value, id: &str) -> &'a Value {
    array_of(report, "nodes").iter().find(|node| node["id"] == id).expect("a node of the flow")
}

fn taken(report: &Value, edge_id: &str) -> u64 {
    array_of(report, "edges").iter().find(|edge| edge["id"] == edge_id).expect("an edge of the flow")["count"]
        .as_u64()
        .unwrap()
}

/// What the echo server says it received, read from a request node's output.
fn echoed(report: &Value, id: &str) -> Value {
    serde_json::from_str(str_of(&node(report, id)["output"], "body")).expect("the echo is JSON")
}

/// A server that answers every request with an event stream of `events`, then ends it.
async fn event_server(events: &'static [&'static str]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut chunk = [0u8; 1024];
                while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&chunk[..n]),
                    }
                }
                let _ = socket
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n")
                    .await;
                for event in events {
                    let _ = socket.write_all(format!("data: {event}\n\n").as_bytes()).await;
                    let _ = socket.flush().await;
                    tokio::time::sleep(Duration::from_millis(15)).await;
                }
                let _ = socket.shutdown().await;
            });
        }
    });
    format!("http://127.0.0.1:{port}")
}

#[tokio::test]
async fn a_flow_is_saved_listed_and_deleted_like_any_entity() {
    let _store = store();
    let mut registry = CommandRegistry::new();
    crate::workspace::entities::register(&mut registry);
    let registry = Arc::new(registry);
    let ctx = CommandContext::internal(registry.clone());
    let entity =
        json!({ "name": "Checkout", "nodes": [start()], "edges": [], "inputs": [{ "name": "user", "value": "1" }] });

    let created =
        registry.invoke("entity.create", json!({ "kind": "flows", "entity": entity }), ctx.clone()).await.unwrap();
    let id = str_of(&created, "id").to_string();
    assert!(crate::workspace::fs::entity_dir(WS, "flows").join(format!("{id}.json")).exists());
    let listed = crate::workspace::config::load_config()["flows"].clone();
    assert_eq!(
        (listed[0]["id"].as_str(), &listed[0]["name"], listed[0].get("nodes")),
        (Some(id.as_str()), &json!("Checkout"), None)
    );
    let loaded = registry
        .invoke("entity.load", json!({ "workspaceId": WS, "kind": "flows", "id": id }), ctx.clone())
        .await
        .unwrap();
    assert_eq!(loaded["entity"]["nodes"][0]["type"], "start");
    let mut renamed = loaded["entity"].clone();
    renamed["name"] = json!("Checkout v2");
    registry.invoke("entity.update", json!({ "kind": "flows", "entity": renamed }), ctx.clone()).await.unwrap();
    assert_eq!(read_entity(WS, "flows", &id).unwrap()["name"], "Checkout v2");

    for unreadable in [json!({ "name": "" }), json!({ "name": "F", "nodes": [{ "id": "a", "type": "nope" }] })] {
        let refused =
            registry.invoke("entity.create", json!({ "kind": "flows", "entity": unreadable }), ctx.clone()).await;
        assert!(refused.is_err(), "{refused:?}");
    }

    registry.invoke("entity.delete", json!({ "kind": "flows", "id": id }), ctx).await.unwrap();
    assert!(!crate::workspace::fs::entity_dir(WS, "flows").join(format!("{id}.json")).exists());
}

#[tokio::test]
async fn a_response_feeds_the_next_request_through_a_condition() {
    let store = store();
    let canned = HashMap::from([(
        "/login".to_string(),
        ("application/json".to_string(), r#"{"token":"t-1","user":{"id":7}}"#.to_string()),
    )]);
    let base = test_server::start(canned).await;
    store.request("r-login", format!("{base}/login"), json!({}));
    store.request(
        "r-user",
        format!("{base}/users/{{{{id}}}}"),
        json!({ "headers": { "Authorization": "Bearer {{token}}" } }),
    );
    store.flow(
        "chain",
        json!({
            "inputs": [{ "name": "user", "value": "1" }],
            "nodes": [
                start(),
                request("login", "r-login", json!({})),
                { "id": "check", "type": "condition", "branches": [{ "id": "ok", "match": "all", "rules": [
                    { "path": "input.status", "op": "equals", "value": "200" },
                    { "path": "input.body.user.id", "op": "greaterThan", "value": "5" },
                ] }] },
                request("user", "r-user", json!({ "variables": [
                    { "name": "token", "value": "{{nodes.login.body.token}}" },
                    { "name": "id", "value": "{{flow.user}}" },
                ] })),
                { "id": "other", "type": "delay", "delayMs": 1 },
            ],
            "edges": [
                edge("start", "out", "login"), edge("login", "success", "check"),
                edge("check", "ok", "user"), edge("check", "else", "other"),
            ],
        }),
    );

    let report = run_with("chain", json!({ "inputs": { "user": "42" } })).await;
    assert_eq!(report["status"], "succeeded", "{report}");
    assert_eq!(report["inputs"], json!({ "user": "42" }));
    let received = echoed(&report, "user");
    assert_eq!(received["path"], "/users/42");
    assert_eq!(received["headers"]["authorization"], "Bearer t-1");
    assert_eq!(node(&report, "check")["port"], "ok");
    assert_eq!(node(&report, "check")["output"]["status"], 200, "a condition passes on what it was given");
    assert_eq!(node(&report, "other")["status"], "idle");
    assert_eq!((taken(&report, "check.ok-user"), taken(&report, "check.else-other")), (1, 0));
    assert_eq!(call("flow.status", json!({ "workspaceId": WS, "id": "chain" })).await["status"], "succeeded");

    // The flow's default input is used when the caller gives none, and an input it does not declare is refused.
    assert_eq!(echoed(&run("chain").await, "user")["path"], "/users/1");
    let refused = call("flow.run", json!({ "workspaceId": WS, "id": "chain", "inputs": { "nope": "1" } })).await;
    assert_eq!(refused["ok"], false);
    assert!(str_of(&refused, "error").contains("no input named \"nope\""), "{refused}");
}

#[tokio::test]
async fn a_failure_ends_the_run_unless_its_error_port_is_connected() {
    let store = store();
    let base = test_server::start(HashMap::new()).await;
    store.request("r-down", format!("{base}/status/503"), json!({}));
    store.request("r-fine", format!("{base}/fine"), json!({}));
    let flow = |down: Value, edges: Vec<Value>| json!({ "nodes": [start(), down, request("after", "r-fine", json!({}))], "edges": edges });

    store.flow(
        "stops",
        flow(
            request("down", "r-down", json!({})),
            vec![edge("start", "out", "down"), edge("down", "success", "after")],
        ),
    );
    let report = run("stops").await;
    assert_eq!((&report["status"], &report["error"]), (&json!("failed"), &json!("down: HTTP 503")));
    assert_eq!(node(&report, "down")["status"], "failed");
    assert_eq!(node(&report, "after")["status"], "idle");

    store.flow(
        "handled",
        flow(request("down", "r-down", json!({})), vec![edge("start", "out", "down"), edge("down", "error", "after")]),
    );
    let report = run("handled").await;
    assert_eq!(report["status"], "succeeded", "{report}");
    assert_eq!(node(&report, "after")["input"]["status"], 503, "the failed answer is what the error port hands on");

    let lenient = request("down", "r-down", json!({ "failOnStatus": false }));
    store.flow("lenient", flow(lenient, vec![edge("start", "out", "down"), edge("down", "success", "after")]));
    let report = run("lenient").await;
    assert_eq!((&report["status"], &node(&report, "down")["port"]), (&json!("succeeded"), &json!("success")));

    store.request("r-refused", "http://127.0.0.1:1/".to_string(), json!({}));
    store.flow(
        "refused",
        json!({ "nodes": [start(), request("gone", "r-refused", json!({}))], "edges": [edge("start", "out", "gone")] }),
    );
    let report = run("refused").await;
    assert_eq!(report["status"], "failed");
    assert!(str_of(&report, "error").starts_with("gone: "), "{report}");
}

#[tokio::test]
async fn a_flow_that_cannot_run_says_what_is_wrong() {
    let store = store();
    store.flow(
        "broken",
        json!({ "nodes": [start(), request("a", "missing", json!({}))], "edges": [edge("start", "out", "a")] }),
    );
    let refused = call("flow.run", json!({ "workspaceId": WS, "id": "broken" })).await;
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["problems"][0]["nodeId"], "a");
    assert_eq!(call("flow.status", json!({ "workspaceId": WS, "id": "broken" })).await, Value::Null);

    let described = call("flow.validate", json!({ "workspaceId": WS, "id": "broken" })).await;
    assert_eq!(described["ok"], false);
    let inline = call("flow.validate", json!({ "workspaceId": WS, "flow": { "nodes": [start()], "edges": [] } })).await;
    assert_eq!(inline["ok"], true);
    assert_eq!(inline["nodes"]["start"]["ports"], json!([{ "id": "out", "name": "Next" }]));

    let (registry, ctx) = context();
    let missing = registry.invoke("flow.run", json!({ "workspaceId": WS, "id": "nope" }), ctx).await.unwrap_err();
    assert_eq!(missing.code, crate::protocol::ErrorCode::NotFound);
    assert!(super::check_shape(&json!({ "name": " ", "nodes": [] })).is_err());
    assert!(super::check_shape(&json!({ "name": "F", "nodes": [{ "id": "a", "type": "nope" }] })).is_err());
    assert!(super::check_shape(&json!({ "name": "F" })).is_ok());
}

#[tokio::test]
async fn what_a_script_sets_stays_in_the_run_unless_the_flow_keeps_it() {
    let store = store();
    let canned =
        HashMap::from([("/login".to_string(), ("application/json".to_string(), r#"{"token":"t-9"}"#.to_string()))]);
    let base = test_server::start(canned).await;
    let script = "lp.environment.set('token', lp.response.json().token);";
    store.request("r-login", format!("{base}/login"), json!({ "postScript": script }));
    store.request("r-user", format!("{base}/me"), json!({ "headers": { "Authorization": "Bearer {{token}}" } }));
    let flow = |settings: Value, login: Value| {
        json!({
            "settings": settings,
            "nodes": [start(), login, request("user", "r-user", json!({}))],
            "edges": [edge("start", "out", "login"), edge("login", "success", "user")],
        })
    };
    let on_disk = || {
        let global = read_entity(WS, "environments", "__global__").unwrap();
        array_of(&global, "variables").iter().any(|variable| variable["key"] == "token")
    };

    store.flow("scoped", flow(json!({}), request("login", "r-login", json!({}))));
    let report = run("scoped").await;
    assert_eq!(echoed(&report, "user")["headers"]["authorization"], "Bearer t-9");
    assert!(!on_disk(), "a run keeps what its scripts set to itself");

    store.flow("silent", flow(json!({}), request("login", "r-login", json!({ "runPostScript": false }))));
    let report = run("silent").await;
    assert_eq!(echoed(&report, "user")["headers"]["authorization"], "Bearer {{token}}", "the script did not run");

    store.flow("kept", flow(json!({ "persistVariables": true }), request("login", "r-login", json!({}))));
    run("kept").await;
    assert!(on_disk(), "a flow that keeps variables writes them as a request sent by hand does");
}

#[tokio::test]
async fn a_node_runs_its_own_script_or_the_requests_post_script_never_both() {
    let store = store();
    let canned =
        HashMap::from([("/login".to_string(), ("application/json".to_string(), r#"{"token":"t-9"}"#.to_string()))]);
    let base = test_server::start(canned).await;
    store.request(
        "r-login",
        format!("{base}/login"),
        json!({
            "preScript": "throw new Error('a pre script ran');",
            "postScript": "lp.environment.set('token', 'from-post');",
            "testScript": "test('answered', () => expect(lp.response.status).to.equal(200));",
        }),
    );
    store.request("r-user", format!("{base}/me"), json!({ "headers": { "Authorization": "Bearer {{token}}" } }));
    let flow = |login: Value| {
        json!({
            "inputs": [{ "name": "suffix", "value": "!" }],
            "nodes": [start(), request("login", "r-login", login), request("user", "r-user", json!({}))],
            "edges": [edge("start", "out", "login"), edge("login", "success", "user")],
        })
    };
    let own = "lp.environment.set('token', JSON.parse(lp.output.body).token + lp.flow.suffix); lp.output = { status: lp.output.status, from: lp.input.suffix };";

    store.flow("post", flow(json!({ "afterExecute": own })));
    let report = run("post").await;
    assert_eq!(report["status"], "succeeded", "the pre script is not run for a node: {report}");
    assert_eq!(echoed(&report, "user")["headers"]["authorization"], "Bearer from-post");
    assert_eq!(node(&report, "login")["output"]["body"], r#"{"token":"t-9"}"#, "the node's own script did not run");

    store.flow("own", flow(json!({ "runPostScript": false, "afterExecute": own })));
    let report = run("own").await;
    assert_eq!(echoed(&report, "user")["headers"]["authorization"], "Bearer t-9!");
    assert_eq!(node(&report, "login")["output"], json!({ "status": 200, "from": "!" }), "the next node is handed this");
    assert_eq!(node(&report, "user")["input"], json!({ "status": 200, "from": "!" }));
    assert_eq!((&report["totalTests"], &report["failedTests"]), (&json!(1), &json!(0)), "the tests always run");

    store.flow("thrown", flow(json!({ "runPostScript": false, "afterExecute": "throw new Error('nope');" })));
    let report = run("thrown").await;
    assert_eq!(report["status"], "failed");
    assert_eq!(report["error"], "login: The after-execute script failed: Error: nope");
    assert_eq!(node(&report, "login")["output"]["status"], 200, "what the request answered is kept");
}

#[tokio::test]
async fn what_a_response_holds_is_never_read_as_a_variable() {
    let store = store();
    let canned = HashMap::from([(
        "/login".to_string(),
        ("application/json".to_string(), r#"{"token":"{{host}} {{keys.secret}}"}"#.to_string()),
    )]);
    let base = test_server::start(canned).await;
    write_flat_entity(
        WS,
        "environments",
        "__global__",
        &json!({ "id": "__global__", "name": "Global", "variables": [{ "id": "host", "key": "host", "value": "internal.test" }] }),
    )
    .unwrap();
    store.request("r-login", format!("{base}/login"), json!({}));
    store.request(
        "r-user",
        format!("{base}/me"),
        json!({ "headers": { "X-Token": "{{token}}", "X-Host": "{{host}}" } }),
    );
    store.flow(
        "inert",
        json!({
            "nodes": [
                start(),
                request("login", "r-login", json!({})),
                request("user", "r-user", json!({ "variables": [{ "name": "token", "value": "{{input.body.token}}" }] })),
            ],
            "edges": [edge("start", "out", "login"), edge("login", "success", "user")],
        }),
    );
    let received = echoed(&run("inert").await, "user");
    assert_eq!(received["headers"]["x-host"], "internal.test", "what the request's author wrote is resolved");
    assert_eq!(received["headers"]["x-token"], "{{host}} {{keys.secret}}", "what a response said is sent as it is");
}

#[tokio::test]
async fn a_stream_hands_on_every_message_until_it_ends() {
    let store = store();
    let base = test_server::start(HashMap::new()).await;
    let events = event_server(&[r#"{"n":1}"#, r#"{"n":2}"#, r#"{"n":3}"#]).await;
    store.stream("s-events", format!("{events}/events"));
    store.request("r-item", format!("{base}/items/{{{{n}}}}"), json!({}));
    store.request("r-done", format!("{base}/done"), json!({}));
    let flow = |stream: Value| {
        json!({
            "nodes": [
                start(),
                stream,
                request("item", "r-item", json!({ "variables": [{ "name": "n", "value": "{{input.data.n}}" }] })),
                request("done", "r-done", json!({})),
            ],
            "edges": [edge("start", "out", "events"), edge("events", "message", "item"), edge("events", "closed", "done")],
        })
    };
    let stream = |extra: Value| {
        let mut node =
            json!({ "id": "events", "type": "stream", "requestKind": "sseRequests", "requestId": "s-events" });
        node.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        node
    };

    store.flow("stream", flow(stream(json!({}))));
    let report = run("stream").await;
    assert_eq!(report["status"], "succeeded", "{report}");
    assert_eq!(taken(&report, "events.message-item"), 3);
    assert_eq!((&node(&report, "item")["runs"], &node(&report, "events")["runs"]), (&json!(3), &json!(1)));
    assert_eq!(echoed(&report, "item")["path"], "/items/3", "messages are handed on in the order they arrived");
    assert_eq!(
        (&node(&report, "events")["port"], &node(&report, "events")["output"]["reason"]),
        (&json!("closed"), &json!("closed"))
    );
    assert_eq!(node(&report, "done")["input"]["messages"], 3);

    store.flow("limited", flow(stream(json!({ "maxMessages": 2 }))));
    let report = run("limited").await;
    assert_eq!(
        (taken(&report, "events.message-item"), &node(&report, "events")["output"]["reason"]),
        (2, &json!("limit"))
    );

    let report = run_with("stream", json!({ "maxEvents": 1 })).await;
    assert_eq!((taken(&report, "events.message-item"), &report["status"]), (1, &json!("succeeded")));

    let tenfold = "lp.output = { data: { n: JSON.parse(lp.output.data).n * 10 } };";
    store.flow("scripted", flow(stream(json!({ "afterExecute": tenfold, "maxMessages": 1 }))));
    assert_eq!(echoed(&run("scripted").await, "item")["path"], "/items/10", "each message goes through the script");

    store.stream("s-refused", "http://127.0.0.1:1/events".to_string());
    store.flow(
        "unreachable",
        json!({
            "nodes": [start(), { "id": "events", "type": "stream", "requestKind": "sseRequests", "requestId": "s-refused" }],
            "edges": [edge("start", "out", "events")],
        }),
    );
    let report = run("unreachable").await;
    assert_eq!(report["status"], "failed", "{report}");
    assert_eq!(node(&report, "events")["port"], "error");
}

#[tokio::test]
async fn a_run_can_be_cancelled_times_out_and_is_the_only_one_of_its_flow() {
    let store = store();
    store.flow("slow", json!({ "nodes": [start(), { "id": "wait", "type": "delay", "delayMs": 60_000 }], "edges": [edge("start", "out", "wait")] }));

    let report = run_with("slow", json!({ "timeoutMs": 60 })).await;
    assert_eq!(report["status"], "failed");
    assert!(str_of(&report, "error").contains("took longer than 0.06 s"), "{report}");
    assert_eq!(node(&report, "wait")["status"], "stopped");

    let progress = Arc::new(Mutex::new(Vec::<Value>::new()));
    let sink = progress.clone();
    let listener = bus().on("flow.progress", move |payload| sink.lock().push(payload.clone()));
    let stopper = tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let again = call("flow.run", json!({ "workspaceId": WS, "id": "slow" })).await;
        assert!(str_of(&again, "error").contains("already running"), "{again}");
        assert_eq!(call("flow.status", json!({ "workspaceId": WS, "id": "slow" })).await["status"], "running");
        assert_eq!(call("flow.cancel", json!({ "workspaceId": WS, "id": "slow" })).await, json!({ "ok": true }));
    });
    let report = run("slow").await;
    stopper.await.unwrap();
    bus().off(listener);
    assert_eq!((&report["status"], &report["error"]), (&json!("cancelled"), &Value::Null));
    let running = progress
        .lock()
        .iter()
        .any(|event| array_of(event, "nodes").iter().any(|n| n["id"] == "wait" && n["status"] == "running"));
    assert!(running, "progress reports the node that is running");
    assert_eq!(call("flow.cancel", json!({ "workspaceId": WS, "id": "never" })).await["ok"], false);
}

#[tokio::test]
async fn branches_of_one_port_all_run_and_a_retry_tries_again() {
    let store = store();
    let base = test_server::start(HashMap::new()).await;
    store.request("r-a", format!("{base}/a"), json!({}));
    store.request("r-down", format!("{base}/status/500"), json!({}));
    store.flow(
        "fan",
        json!({
            "nodes": [
                start(),
                request("left", "r-a", json!({})),
                request("right", "r-a", json!({})),
                request("flaky", "r-down", json!({ "retries": 2, "retryDelayMs": 1 })),
                request("join", "r-a", json!({})),
            ],
            "edges": [
                edge("start", "out", "left"), edge("start", "out", "right"), edge("start", "out", "flaky"),
                edge("left", "success", "join"), edge("right", "success", "join"), edge("flaky", "error", "join"),
            ],
        }),
    );
    let report = run("fan").await;
    assert_eq!(report["status"], "succeeded", "{report}");
    assert_eq!(node(&report, "join")["runs"], 3, "a node runs once for each arrival");
    assert_eq!(node(&report, "flaky")["status"], "failed");
    assert_eq!(report["nodeRuns"], 7);
}

/// A WebSocket server that sends `greeting` to whoever connects, then answers each text frame with the
/// frames `reply` gives for it. Returns its `host:port`.
async fn socket_server(greeting: &'static str, reply: fn(&str) -> Vec<String>) -> String {
    use futures::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Ok(mut socket) = accept_async(tcp).await else { return };
                if socket.send(Message::text(greeting)).await.is_err() {
                    return;
                }
                while let Some(Ok(message)) = socket.next().await {
                    let Message::Text(text) = message else { continue };
                    for frame in reply(text.as_str()) {
                        if socket.send(Message::text(frame)).await.is_err() {
                            return;
                        }
                    }
                }
            });
        }
    });
    address
}

fn stream(id: &str, kind: &str, request_id: &str, extra: Value) -> Value {
    let mut node = json!({ "id": id, "type": "stream", "requestKind": kind, "requestId": request_id });
    node.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    node
}

/// A saved gRPC request for the echo service of `grpc::tests`.
fn grpc_request(address: &str, method: &str, body: &str) -> Value {
    json!({
        "serverAddress": address, "serviceName": "test.echo.Echo", "methodName": method, "requestBody": body,
        "metadata": { "x-token": "{{token}}" }, "useReflection": true, "streamingType": "unary",
    })
}

#[tokio::test]
async fn a_grpc_call_is_a_request_node_that_answers_once() {
    let store = store();
    let base = test_server::start(HashMap::new()).await;
    let address = crate::grpc::tests::serve(true).await;
    store.saved("grpcRequests", "g-say", grpc_request(&address, "Say", r#"{"text": "{{who}}", "n": 7}"#));
    store.saved("grpcRequests", "g-fail", grpc_request(&address, "Fail", "{}"));
    store.request("r-item", format!("{base}/items/{{{{n}}}}"), json!({}));
    let call = |id: &str, request_id: &str, extra: Value| {
        let mut node = json!({ "id": id, "type": "request", "requestKind": "grpcRequests", "requestId": request_id });
        node.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        node
    };

    store.flow(
        "grpc",
        json!({
            "inputs": [{ "name": "user", "value": "Ada" }],
            "nodes": [
                start(),
                call("say", "g-say", json!({ "variables": [
                    { "name": "who", "value": "{{flow.user}}" }, { "name": "token", "value": "abc" },
                ] })),
                request("item", "r-item", json!({ "variables": [{ "name": "n", "value": "{{input.message.n}}" }] })),
            ],
            "edges": [edge("start", "out", "say"), edge("say", "success", "item")],
        }),
    );
    let report = run("grpc").await;
    assert_eq!(report["status"], "succeeded", "{report}");
    let said = &node(&report, "say")["output"];
    assert_eq!((&said["ok"], &said["status"]), (&json!(true), &json!(0)));
    let message: Value = serde_json::from_str(str_of(said, "message")).unwrap();
    assert_eq!(message["text"], "echo: Ada (token abc)", "the node's variables reach the body and the metadata");
    assert_eq!(echoed(&report, "item")["path"], "/items/7", "a later node reads a field of the answer");

    let failing = |extra: Value| json!({ "nodes": [start(), call("fail", "g-fail", extra)], "edges": [edge("start", "out", "fail")] });
    store.flow("grpc-fail", failing(json!({})));
    let report = run("grpc-fail").await;
    assert_eq!(report["status"], "failed");
    assert!(str_of(&report, "error").starts_with("fail: NOT_FOUND"), "{report}");
    assert_eq!(node(&report, "fail")["output"]["status"], 5);

    store.flow("grpc-lenient", failing(json!({ "failOnStatus": false })));
    let report = run("grpc-lenient").await;
    assert_eq!((&report["status"], &node(&report, "fail")["port"]), (&json!("succeeded"), &json!("success")));
}

#[tokio::test]
async fn a_live_grpc_call_is_a_stream_node_that_hands_on_each_message() {
    let store = store();
    let base = test_server::start(HashMap::new()).await;
    let address = crate::grpc::tests::serve(true).await;
    store.saved("grpcRequests", "g-count", grpc_request(&address, "Count", r#"{"text": "{{word}}", "n": 3}"#));
    store.saved("grpcRequests", "g-chat", grpc_request(&address, "Chat", r#"{"text": "first", "n": 1}"#));
    store.saved("grpcRequests", "g-fail", grpc_request(&address, "Fail", "{}"));
    store.request("r-item", format!("{base}/items/{{{{n}}}}"), json!({}));
    store.request("r-done", format!("{base}/done"), json!({}));
    let flow = |call: Value, binding: &str| {
        json!({
            "nodes": [
                start(),
                call,
                request("item", "r-item", json!({ "variables": [{ "name": "n", "value": binding }] })),
                request("done", "r-done", json!({})),
            ],
            "edges": [edge("start", "out", "call"), edge("call", "message", "item"), edge("call", "closed", "done")],
        })
    };

    // A server-streaming call: the request's body is sent when it opens, and the call ends on its own.
    let word = json!({ "variables": [{ "name": "word", "value": "tick" }] });
    store.flow(
        "grpc-count",
        flow(stream("call", "grpcRequests", "g-count", word), "{{input.data.text}}-{{input.data.n}}"),
    );
    let report = run("grpc-count").await;
    assert_eq!(report["status"], "succeeded", "{report}");
    assert_eq!((taken(&report, "call.message-item"), &node(&report, "item")["runs"]), (3, &json!(3)));
    assert_eq!(echoed(&report, "item")["path"], "/items/tick-3", "in order, with the node's variables in the body");
    let call = &node(&report, "call")["output"];
    assert_eq!(
        (&node(&report, "call")["port"], &call["reason"], &call["messages"]),
        (&json!("closed"), &json!("closed"), &json!(3))
    );
    assert_eq!(node(&report, "done")["status"], "succeeded");

    // A two-way call starts empty and stays open: the saved body is not sent, the node's message is, and the
    // node's limit ends the call.
    let again = json!({ "message": r#"{"text": "again", "n": 2}"#, "maxMessages": 1 });
    store.flow("grpc-chat", flow(stream("call", "grpcRequests", "g-chat", again), "{{input.data.n}}"));
    let report = run("grpc-chat").await;
    assert_eq!(report["status"], "succeeded", "{report}");
    assert_eq!((taken(&report, "call.message-item"), &node(&report, "call")["output"]["reason"]), (1, &json!("limit")));
    let last: Value = serde_json::from_str(str_of(&node(&report, "item")["input"], "data")).unwrap();
    assert_eq!(last, json!({ "text": "chat again", "n": 2 }));

    // A call that ends with a status other than OK leaves by the error port.
    store.flow(
        "grpc-broken",
        json!({
            "nodes": [start(), stream("call", "grpcRequests", "g-fail", json!({}))],
            "edges": [edge("start", "out", "call")],
        }),
    );
    let report = run("grpc-broken").await;
    assert_eq!((&report["status"], &node(&report, "call")["port"]), (&json!("failed"), &json!("error")));
    assert!(str_of(&report, "error").contains("status 5: nothing here"), "{report}");
}

#[tokio::test]
async fn a_websocket_hands_on_what_it_receives_and_sends_when_it_is_reached_again() {
    let store = store();
    let base = test_server::start(HashMap::new()).await;
    let address = socket_server("hello", |text| vec![text.to_string()]).await;
    store.saved("sockets", "w-echo", json!({ "url": format!("ws://{address}/{{{{room}}}}"), "headers": {} }));
    store.request("r-item", format!("{base}/items/{{{{n}}}}"), json!({}));
    let socket = |extra: Value| {
        let room = json!({ "variables": [{ "name": "room", "value": "lobby" }] });
        let mut node = stream("socket", "wsConnections", "w-echo", room);
        node.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        node
    };
    let item = request("item", "r-item", json!({ "variables": [{ "name": "n", "value": "{{input.data}}" }] }));

    // The greeting, then the echo of the message the node sends once it is connected.
    store.flow(
        "ws",
        json!({
            "inputs": [{ "name": "word", "value": "ping" }],
            "nodes": [start(), socket(json!({ "message": "say-{{flow.word}}", "maxMessages": 2 })), item.clone()],
            "edges": [edge("start", "out", "socket"), edge("socket", "message", "item")],
        }),
    );
    let report = run("ws").await;
    assert_eq!(report["status"], "succeeded", "{report}");
    assert_eq!(
        (taken(&report, "socket.message-item"), &node(&report, "socket")["output"]["reason"]),
        (2, &json!("limit"))
    );
    assert_eq!(echoed(&report, "item")["path"], "/items/say-ping");
    assert_eq!(node(&report, "item")["input"]["direction"], "received", "what the flow sent is not handed on");

    // A second path that reaches the node while it listens sends its message over the open connection.
    store.flow(
        "ws-again",
        json!({
            "nodes": [
                start(),
                socket(json!({ "message": "again", "maxMessages": 3 })),
                { "id": "later", "type": "delay", "delayMs": 150 },
                item,
            ],
            "edges": [
                edge("start", "out", "socket"), edge("start", "out", "later"), edge("later", "out", "socket"),
                edge("socket", "message", "item"),
            ],
        }),
    );
    let report = run("ws-again").await;
    assert_eq!(report["status"], "succeeded", "{report}");
    assert_eq!(taken(&report, "socket.message-item"), 3, "the greeting and two echoes: {report}");
    assert_eq!(node(&report, "socket")["runs"], 2);

    // A server that is not there is a failure of the node.
    store.saved("sockets", "w-gone", json!({ "url": "ws://127.0.0.1:1/", "headers": {} }));
    store.flow(
        "ws-gone",
        json!({
            "nodes": [start(), stream("socket", "wsConnections", "w-gone", json!({}))],
            "edges": [edge("start", "out", "socket")],
        }),
    );
    let report = run("ws-gone").await;
    assert_eq!((&report["status"], &node(&report, "socket")["port"]), (&json!("failed"), &json!("error")));
}

#[tokio::test]
async fn a_socketio_connection_hands_on_each_event_and_emits_the_nodes_message() {
    let store = store();
    let base = test_server::start(HashMap::new()).await;
    // Engine.IO v4 over a WebSocket: the open packet, then a namespace connect answered with a sid and a
    // `welcome` event, and `echo` events sent back as they came.
    let open = r#"0{"sid":"eio","upgrades":[],"pingInterval":25000,"pingTimeout":20000,"maxPayload":1000000}"#;
    let address = socket_server(open, |frame| match frame {
        frame if frame.starts_with("40") => vec![r#"40{"sid":"s1"}"#.to_string(), r#"42["welcome","hi"]"#.to_string()],
        frame if frame.starts_with(r#"42["echo""#) => vec![frame.to_string()],
        _ => Vec::new(),
    })
    .await;
    let saved = json!({ "url": format!("http://{address}"), "headers": {}, "listenAll": true });
    store.saved("socketioRequests", "s-echo", saved);
    store.request("r-item", format!("{base}/items/{{{{n}}}}"), json!({}));
    let emits = json!({ "event": "echo", "message": r#"["{{flow.word}}", 2]"#, "maxMessages": 2 });
    let reads = json!({ "variables": [{ "name": "n", "value": "{{input.event}}-{{input.data[0]}}" }] });
    store.flow(
        "sio",
        json!({
            "inputs": [{ "name": "word", "value": "ping" }],
            "nodes": [start(), stream("events", "socketioRequests", "s-echo", emits), request("item", "r-item", reads)],
            "edges": [edge("start", "out", "events"), edge("events", "message", "item")],
        }),
    );
    let report = run("sio").await;
    assert_eq!(report["status"], "succeeded", "{report}");
    assert_eq!(
        (taken(&report, "events.message-item"), &node(&report, "events")["output"]["reason"]),
        (2, &json!("limit"))
    );
    assert_eq!(echoed(&report, "item")["path"], "/items/echo-ping", "the event's name and its first argument");
}
