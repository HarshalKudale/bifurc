//! `contract.check` and `types.generate` against a registry whose other commands are canned.

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::{
    kernel::{CommandContext, CommandRegistry, Scope, SessionIdentity},
    protocol::{ErrorCode, Result},
};

type Calls = Arc<Mutex<Vec<(String, Value)>>>;

struct Fake {
    registry: CommandRegistry,
    calls: Calls,
}

impl Fake {
    fn new() -> Self {
        let mut registry = CommandRegistry::new();
        super::register(&mut registry);
        Self { registry, calls: Arc::default() }
    }

    fn on(mut self, name: &'static str, answer: impl Fn(&Value) -> Result<Value> + Send + Sync + 'static) -> Self {
        let calls = self.calls.clone();
        self.registry.register_sync(name, move |params, _| {
            calls.lock().push((name.to_string(), params.clone()));
            answer(&params)
        });
        self
    }

    fn fixed(self, name: &'static str, answer: Value) -> Self {
        self.on(name, move |_| Ok(answer.clone()))
    }

    async fn call(self, command: &str, params: Value) -> (Result<Value>, Calls) {
        self.call_as(command, params, None).await
    }

    async fn call_as(self, command: &str, params: Value, session: Option<SessionIdentity>) -> (Result<Value>, Calls) {
        let registry = Arc::new(self.registry);
        let ctx = CommandContext { registry: registry.clone(), session };
        (registry.invoke(command, params, ctx).await, self.calls)
    }
}

fn sent(calls: &Calls, command: &str) -> Vec<Value> {
    calls.lock().iter().filter(|(name, _)| name == command).map(|(_, p)| p.clone()).collect()
}

fn b64(text: &str) -> String {
    STANDARD.encode(text)
}

fn mock(id: &str, name: &str, method: &str, url: &str, status: u16, body: &str) -> Value {
    json!({"id": id, "name": name, "method": method, "urlPattern": url, "enabled": true,
           "responseStatus": status, "responseBody": body,
           "responseHeaders": {"Content-Type": "application/json"}})
}

fn real(status: u16, body: &str) -> Value {
    json!({"status": status, "durationMs": 3, "headers": {"content-type": "application/json"}, "body": b64(body)})
}

/// Three mocks (two paired with saved requests, one lonely) and the two saved requests.
fn workspace(fake: Fake, answer: Value) -> Fake {
    let mocks = [
        mock("m1", "Get orders", "GET", "http://h/orders", 200, r#"{"items":[{"id":1}]}"#),
        mock("m2", "Create order", "POST", "http://h/orders", 201, "{}"),
        mock("m3", "Lonely", "GET", "http://h/none", 200, "{}"),
    ];
    let rows: Vec<Value> = mocks.iter().map(|m| json!({"id": m["id"], "name": m["name"]})).collect();
    fake.on("entity.list", move |p| {
        Ok(match p["kind"].as_str() {
            Some("mocks") => json!({"entities": rows}),
            Some("requests") => json!({"entities": [
                {"id": "r1", "name": "Get orders", "method": "GET", "url": "{{baseUrl}}/orders"},
                {"id": "r2", "name": "Make order", "method": "POST", "url": "{{baseUrl}}/orders"}]}),
            _ => json!({"entities": []}),
        })
    })
    .on("entity.load", move |p| {
        let found = mocks.iter().find(|m| m["id"] == p["id"]).cloned();
        Ok(json!({"ok": found.is_some(), "entity": found}))
    })
    .fixed("request.send", answer)
}

const WS: &str = "w1";

#[tokio::test]
async fn a_check_reports_the_drift_and_counts_the_rest() {
    let body = r#"{"items":[{"id":1,"discount":2.5,"noise":1}]}"#;
    let fake = workspace(Fake::new(), real(200, body));
    let params = json!({"workspaceId": WS, "ignore": [".items[].noise"]});
    let (out, calls) = fake.call("contract.check", params).await;
    let report = out.unwrap();
    assert_eq!(
        (report["drifted"].clone(), report["ok"].clone(), report["unpairedCount"].clone()),
        (json!(2), json!(0), json!(1))
    );
    let texts = |i: usize| -> Vec<String> {
        report["checks"][i]["differences"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["text"].as_str().unwrap().into())
            .collect()
    };
    assert_eq!(texts(0), ["+ .items[].discount (number)"]);
    assert_eq!(texts(1), ["~ status 201 → 200", "+ .items (array)"]);
    assert_eq!(report["checks"][0]["source"], "real");
    assert_eq!(report["unpaired"][0]["reason"], "no saved request");
    assert_eq!(report["unpaired"][0]["method"], "GET");
    // Each request is sent once, with its own id.
    let ids: Vec<String> = sent(&calls, "request.send").iter().map(|p| p["id"].as_str().unwrap().into()).collect();
    assert_eq!(ids, ["r1", "r2"]);
}

#[tokio::test]
async fn one_mock_that_matches_is_ok_and_the_send_options_pass_through() {
    let fake = workspace(Fake::new(), real(200, r#"{"items":[{"id":2}]}"#));
    let params = json!({"workspaceId": WS, "mock": "get orders", "variables": {"k": "v"}, "environmentId": "e1", "freshJar": true});
    let (out, calls) = fake.call("contract.check", params).await;
    let report = out.unwrap();
    assert_eq!((report["ok"].clone(), report["drifted"].clone()), (json!(1), json!(0)));
    assert_eq!(report["checks"][0]["ok"], true);
    let send = &sent(&calls, "request.send")[0];
    assert_eq!(
        (send["variables"]["k"].clone(), send["environmentId"].clone(), send["freshJar"].clone()),
        (json!("v"), json!("e1"), json!(true))
    );
}

#[tokio::test]
async fn a_failed_send_falls_back_to_the_snapshot_and_from_snapshot_sends_nothing() {
    let stored = json!({"snapshots": [{"requestId": "r1", "status": 200, "contentType": "json",
                                       "body": {"items": [{"id": 0, "extra": ""}]}}]});
    let fake = Fake::new()
        .on("entity.list", |p| {
            Ok(if p["kind"] == "mocks" {
                json!({"entities": [{"id": "m1", "name": "Get orders"}, {"id": "m2", "name": "Create order"}]})
            } else {
                json!({"entities": [
                    {"id": "r1", "name": "Get orders", "method": "GET", "url": "{{baseUrl}}/orders"},
                    {"id": "r2", "name": "Make order", "method": "POST", "url": "{{baseUrl}}/orders"}]})
            })
        })
        .on("entity.load", |p| {
            Ok(json!({"ok": true, "entity": if p["id"] == "m1" {
                mock("m1", "Get orders", "GET", "http://h/orders", 200, r#"{"items":[{"id":1}]}"#)
            } else {
                mock("m2", "Create order", "POST", "http://h/orders", 201, "{}")
            }}))
        })
        .on("request.send", |_| Err(crate::protocol::EngineError::upstream("connection refused")))
        .fixed("contract.listSnapshots", stored);
    let calls = fake.calls.clone();
    let registry = Arc::new(fake.registry);
    let ask = |params: Value| {
        let (registry, ctx) = (registry.clone(), CommandContext { registry: registry.clone(), session: None });
        async move { registry.invoke("contract.check", params, ctx).await }
    };
    let report = ask(json!({"workspaceId": WS, "mock": "Get orders"})).await.unwrap();
    assert_eq!(report["checks"][0]["source"], "snapshot");
    assert_eq!(report["checks"][0]["differences"][0]["text"], "+ .items[].extra (string)");
    // Without a snapshot for the request the send failure stays a failure.
    let report = ask(json!({"workspaceId": WS, "mock": "Create order"})).await.unwrap();
    assert_eq!(report["failed"], 1);
    assert_eq!(report["checks"][0]["error"], "UPSTREAM_FAILED: connection refused");
    let before = sent(&calls, "request.send").len();
    let report = ask(json!({"workspaceId": WS, "fromSnapshot": true, "all": true})).await.unwrap();
    assert_eq!(report["drifted"], 1);
    assert_eq!(report["unpaired"][0]["reason"], "no snapshot");
    assert_eq!(sent(&calls, "request.send").len(), before);
}

#[tokio::test]
async fn from_capture_reads_the_newest_real_capture_of_the_call() {
    let fake = workspace(Fake::new(), json!({}))
        .fixed(
            "capture.list",
            json!({"entries": [
                {"id": "c0", "method": "GET", "url": "http://h/orders", "status": 200, "via": "mock"},
                {"id": "c1", "method": "GET", "url": "http://h/orders?x=1", "status": 200}]}),
        )
        .fixed(
            "capture.get",
            json!({"entry": {"resStatus": 200, "resHeaders": {"content-type": "application/json"}, "resBody": b64(r#"{"items":[{"id":1,"extra":true}]}"#)}}),
        );
    let (out, calls) = fake.call("contract.check", json!({"workspaceId": WS, "fromCapture": true})).await;
    let report = out.unwrap();
    assert_eq!(report["checks"][0]["differences"][0]["text"], "+ .items[].extra (boolean)");
    assert_eq!(sent(&calls, "capture.get")[0]["id"], "c1");
    assert!(sent(&calls, "request.send").is_empty());
    // POST /orders has no capture.
    assert!(report["unpaired"].as_array().unwrap().iter().any(|u| u["reason"] == "no real capture"));
}

fn spec_fake(mocks: Vec<Value>, operations: Value) -> Fake {
    let rows: Vec<Value> = mocks.iter().map(|m| json!({"id": m["id"], "name": m["name"]})).collect();
    Fake::new()
        .fixed("contract.fromOpenApi", operations)
        .on("entity.list", move |p| Ok(json!({"entities": if p["kind"] == "mocks" { rows.clone() } else { vec![] }})))
        .on("entity.load", move |p| {
            Ok(json!({"ok": true, "entity": mocks.iter().find(|m| m["id"] == p["id"]).cloned()}))
        })
}

#[tokio::test]
async fn a_spec_pairs_by_method_and_path_with_or_without_the_base_path_and_sends_nothing() {
    let operations = json!({"basePath": "/v1", "operations": [
        {"method": "GET", "path": "/orders", "status": 200, "contentType": "application/json", "body": {"items": [{"id": 0, "discount": 0}]}},
        {"method": "GET", "path": "/orders/{id}", "status": 200, "contentType": "application/json", "body": {"id": 0}},
        {"method": "POST", "path": "/orders", "status": 201, "contentType": "", "body": null}]});
    let mocks = vec![
        mock("m1", "Get orders", "GET", "http://h/v1/orders", 200, r#"{"items":[{"id":1}]}"#),
        mock("m3", "Lonely", "GET", "http://h/none", 200, "{}"),
        mock("m4", "One", "GET", "http://h/orders/9", 200, r#"{"id":3}"#),
    ];
    let (out, calls) =
        spec_fake(mocks, operations).call("contract.check", json!({"workspaceId": WS, "openapi": "spec text"})).await;
    let report = out.unwrap();
    assert_eq!(sent(&calls, "contract.fromOpenApi")[0]["text"], "spec text");
    assert_eq!(
        (report["drifted"].clone(), report["ok"].clone(), report["unpairedCount"].clone()),
        (json!(1), json!(1), json!(1))
    );
    assert_eq!(report["checks"][0]["path"], "/v1/orders");
    assert_eq!(report["checks"][0]["source"], "openapi");
    assert_eq!(report["checks"][0]["differences"][0]["text"], "+ .items[].discount (number)");
    assert_eq!(report["unpaired"][0]["reason"], "not in the spec");
    assert!(sent(&calls, "request.send").is_empty());
    let (out, _) =
        Fake::new().call("contract.check", json!({"workspaceId": WS, "openapi": "x", "fromCapture": true})).await;
    assert_eq!(out.unwrap_err().code, ErrorCode::BadRequest);
}

#[tokio::test]
async fn a_mock_may_carry_an_optional_property_but_not_an_undeclared_one() {
    let operations = json!({"basePath": "", "operations": [
        {"method": "GET", "path": "/a", "status": 200, "contentType": "application/json", "body": {"id": 0}, "optional": [".note"]}]});
    let mocks = vec![
        mock("m1", "Has note", "GET", "http://h/a", 200, r#"{"id":1,"note":"x"}"#),
        mock("m2", "Has extra", "GET", "http://h/a", 200, r#"{"id":1,"zzz":"x"}"#),
    ];
    let (out, _) =
        spec_fake(mocks, operations).call("contract.check", json!({"workspaceId": WS, "openapi": "x"})).await;
    let report = out.unwrap();
    assert_eq!((report["drifted"].clone(), report["ok"].clone()), (json!(1), json!(1)));
    assert_eq!(report["checks"][1]["differences"][0]["text"], "- .zzz (string)");
}

fn types_fake(history: Vec<Value>, captures: Vec<Value>) -> Fake {
    Fake::new()
        .fixed(
            "entity.list",
            json!({"entities": [
                {"id": "r1", "name": "Get orders", "method": "GET", "url": "{{baseUrl}}/orders/{{id}}"},
                {"id": "r2", "name": "Make order", "method": "POST", "url": "{{baseUrl}}/orders"}]}),
        )
        .fixed("response.history", json!({"entries": history}))
        .fixed("capture.list", json!({"entries": captures}))
        .fixed(
            "capture.get",
            json!({"entry": {"resStatus": 200, "resHeaders": {"content-type": "application/json"}, "resBody": b64(r#"{"id":3,"extra":true}"#)}}),
        )
}

fn history() -> Vec<Value> {
    vec![
        json!({"status": 200, "headers": {"Content-Type": "application/json"}, "body": r#"{"id":1,"note":null}"#}),
        json!({"status": 500, "headers": {"content-type": "application/json"}, "body": r#"{"error":"x"}"#}),
        json!({"status": 200, "headers": {"content-type": "application/json"}, "body": b64(r#"{"id":2,"total":1.5}"#), "encoding": "base64"}),
        json!({"status": 200, "headers": {"content-type": "text/html"}, "body": "<p>hi</p>"}),
    ]
}

fn captures() -> Vec<Value> {
    vec![
        json!({"id": "c1", "method": "GET", "url": "http://h/orders/7?x=1", "status": 200}),
        json!({"id": "c2", "method": "GET", "url": "http://h/users/7", "status": 200}),
        json!({"id": "c3", "method": "POST", "url": "http://h/orders/7", "status": 200}),
        json!({"id": "c4", "method": "GET", "url": "http://h/orders/8", "status": 200, "via": "mock"}),
        json!({"id": "c5", "method": "GET", "url": "http://h/orders/9", "status": 404}),
    ]
}

#[tokio::test]
async fn samples_come_from_history_and_matching_captures_only() {
    let (out, calls) = types_fake(history(), captures())
        .call("types.generate", json!({"workspaceId": WS, "request": "get ord"}))
        .await;
    let got = out.unwrap();
    assert_eq!(got["samples"], 3);
    assert_eq!((got["lang"].clone(), got["name"].clone()), (json!("ts"), json!("GetOrdersResponse")));
    assert_eq!(
        got["code"],
        "// Inferred by bifurc from 3 responses of \"Get orders\"\nexport type GetOrdersResponse = {\n  id: number;\n  note?: null;\n  total?: number;\n  extra?: boolean;\n};\n"
    );
    assert_eq!(sent(&calls, "capture.get").len(), 1);
    assert_eq!(sent(&calls, "capture.get")[0]["id"], "c1");
    assert!(sent(&calls, "request.send").is_empty());
}

#[tokio::test]
async fn no_sample_is_not_found_and_lang_and_name_shape_the_output() {
    let (out, _) =
        types_fake(vec![], vec![]).call("types.generate", json!({"workspaceId": WS, "request": "Get orders"})).await;
    let err = out.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert_eq!(err.message, "no JSON response for \"Get orders\" yet");
    assert_eq!(err.details.unwrap()["key"], "types.no_samples");

    let params = json!({"workspaceId": WS, "request": "r1", "lang": "zod", "name": "Order"});
    let got = types_fake(history(), captures()).call("types.generate", params).await.0.unwrap();
    assert_eq!((got["name"].clone(), got["lang"].clone()), (json!("Order"), json!("zod")));
    assert!(got["code"].as_str().unwrap().contains("export const Order = z.object({"));

    for bad in [json!({"name": "1 bad"}), json!({"lang": "ts", "name": "a b"})] {
        let mut params = json!({"workspaceId": WS, "request": "r1"});
        params.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
        let err = types_fake(history(), captures()).call("types.generate", params).await.0.unwrap_err();
        assert_eq!(err.code, ErrorCode::BadRequest);
    }
    let err = types_fake(history(), captures())
        .call("types.generate", json!({"workspaceId": WS, "request": "nope"}))
        .await
        .0
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn send_adds_the_sent_response_and_needs_the_execute_scope() {
    let fake = || {
        types_fake(history(), captures()).fixed(
            "request.send",
            json!({"status": 200, "headers": {"content-type": "application/json"}, "body": b64(r#"{"id":9}"#)}),
        )
    };
    let params = json!({"workspaceId": WS, "request": "Get orders", "send": true, "variables": {"a": "b"}});
    let (out, calls) = fake().call("types.generate", params.clone()).await;
    assert_eq!(out.unwrap()["samples"], 4);
    assert_eq!(sent(&calls, "request.send")[0]["id"], "r1");
    assert_eq!(sent(&calls, "request.send")[0]["variables"]["a"], "b");

    let reader = SessionIdentity { scopes: Some([Scope::Read].into_iter().collect()), ..Default::default() };
    let (out, calls) = fake().call_as("types.generate", params.clone(), Some(reader.clone())).await;
    assert_eq!(out.unwrap_err().code, ErrorCode::Forbidden);
    assert!(sent(&calls, "request.send").is_empty());
    // Without send a reader may generate.
    let mut no_send = params;
    no_send.as_object_mut().unwrap().remove("send");
    assert!(fake().call_as("types.generate", no_send, Some(reader)).await.0.is_ok());

    let down = types_fake(history(), captures()).fixed("request.send", json!({"status": 0, "headers": {}, "body": ""}));
    let err =
        down.call("types.generate", json!({"workspaceId": WS, "request": "r1", "send": true})).await.0.unwrap_err();
    assert_eq!((err.code, err.message.as_str()), (ErrorCode::UpstreamFailed, "no response"));
}
