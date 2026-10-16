//! gRPC against a real server: a small echo service and a reflection service, both served by tonic on a
//! loopback port, driven through the commands exactly as a client would.

use std::{pin::Pin, sync::Arc, task::{Context, Poll}};

use futures::{Stream, StreamExt as _};
use prost_reflect::{DescriptorPool, DynamicMessage, Value as Field};
use serde_json::{Value, json};
use tonic::{
    Code, Request, Response, Status, Streaming,
    body::Body,
    server::{Grpc, NamedService, StreamingService},
    transport::{Server, server::TcpIncoming},
};

use super::{call::DynamicCodec, proto, reflection::server::Reflection, *};
use crate::{kernel::CommandContext, testing::scratch_data_root, workspace::fs::write_entity};

pub const ECHO_PROTO: &str = r#"
syntax = "proto3";
package test.echo;
message Req { string text = 1; int32 n = 2; }
message Resp { string text = 1; int32 n = 2; }
service Echo {
  rpc Say(Req) returns (Resp);
  rpc Count(Req) returns (stream Resp);
  rpc Collect(stream Req) returns (Resp);
  rpc Chat(stream Req) returns (stream Resp);
  rpc Fail(Req) returns (Resp);
}
"#;

#[derive(Clone)]
struct Echo {
    pool: DescriptorPool,
    method: String,
}

impl NamedService for Echo {
    const NAME: &'static str = "test.echo.Echo";
}

type Replies = Pin<Box<dyn Stream<Item = Result<DynamicMessage, Status>> + Send>>;

impl Echo {
    fn reply(&self, text: &str, n: i32) -> DynamicMessage {
        let desc = self.pool.get_message_by_name("test.echo.Resp").unwrap();
        let mut message = DynamicMessage::new(desc);
        message.set_field_by_name("text", Field::String(text.to_owned()));
        message.set_field_by_name("n", Field::I32(n));
        message
    }
}

impl StreamingService<DynamicMessage> for Echo {
    type Response = DynamicMessage;
    type ResponseStream = Replies;
    type Future = Pin<Box<dyn std::future::Future<Output = Result<Response<Replies>, Status>> + Send>>;

    fn call(&mut self, request: Request<Streaming<DynamicMessage>>) -> Self::Future {
        let this = self.clone();
        Box::pin(async move {
            let token = request.metadata().get("x-token").and_then(|v| v.to_str().ok()).unwrap_or("none").to_owned();
            let mut inbound = request.into_inner();
            if this.method == "Chat" {
                let live = inbound.map(move |message| {
                    let message = message?;
                    let text = message.get_field_by_name("text").unwrap().as_str().unwrap().to_owned();
                    let n = message.get_field_by_name("n").unwrap().as_i32().unwrap();
                    Ok(this.reply(&format!("chat {text}"), n))
                });
                return Ok(Response::new(Box::pin(live) as Replies));
            }
            let mut received = Vec::new();
            while let Some(message) = inbound.message().await? {
                let text = message.get_field_by_name("text").unwrap().as_str().unwrap().to_owned();
                let n = message.get_field_by_name("n").unwrap().as_i32().unwrap();
                received.push((text, n));
            }
            let replies: Vec<Result<DynamicMessage, Status>> = match this.method.as_str() {
                "Say" => vec![Ok(this.reply(&format!("echo: {} (token {token})", received[0].0), received[0].1))],
                "Count" => (1..=received[0].1).map(|i| Ok(this.reply(&received[0].0, i))).collect(),
                "Collect" => {
                    let joined = received.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join("+");
                    vec![Ok(this.reply(&joined, received.len() as i32))]
                }
                "Fail" => vec![Err(Status::not_found("nothing here"))],
                other => vec![Err(Status::unimplemented(other.to_owned()))],
            };
            let mut response = Response::new(Box::pin(futures::stream::iter(replies)) as Replies);
            response.metadata_mut().insert("x-served-by", "echo".parse().unwrap());
            Ok(response)
        })
    }
}

impl tower_service::Service<http::Request<Body>> for Echo {
    type Response = http::Response<Body>;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        let mut this = self.clone();
        Box::pin(async move {
            this.method = request.uri().path().rsplit('/').next().unwrap_or_default().to_owned();
            let input = this.pool.get_message_by_name("test.echo.Req").unwrap();
            Ok(Grpc::new(DynamicCodec(input)).streaming(this, request).await)
        })
    }
}

/// Serves the echo service, with reflection when asked, and returns its `host:port`.
pub(crate) async fn serve(with_reflection: bool) -> String {
    let pool = proto::compile("echo.proto", vec![("echo.proto".into(), ECHO_PROTO.into())]).unwrap();
    let incoming = TcpIncoming::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = incoming.local_addr().unwrap().to_string();
    let echo = Echo { pool: pool.clone(), method: String::new() };
    let mut builder = Server::builder();
    let router = builder.add_service(echo);
    tokio::spawn(async move {
        if with_reflection {
            let reflection = Reflection { pool, services: vec!["test.echo.Echo".into()] };
            router.add_service(reflection).serve_with_incoming(incoming).await.unwrap();
        } else {
            router.serve_with_incoming(incoming).await.unwrap();
        }
    });
    address
}

async fn call(action: &str, payload: Value) -> crate::protocol::Result<Value> {
    let mut registry = CommandRegistry::new();
    register(&mut registry);
    let registry = Arc::new(registry);
    let ctx = CommandContext::internal(registry.clone());
    registry.invoke(action, payload, ctx).await
}

fn execute(address: &str, method: &str, body: Value) -> Value {
    json!({
        "serverAddress": address, "serviceName": "test.echo.Echo", "methodName": method,
        "requestBody": body.to_string(), "metadata": { "X-Token": "t0k" }, "protoFileId": null, "useReflection": true,
    })
}

fn texts(result: &Value) -> Vec<String> {
    result["responses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| serde_json::from_str::<Value>(r.as_str().unwrap()).unwrap()["text"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn reflection_lists_services_and_methods() {
    let address = serve(true).await;
    let reflected = call("grpc.reflect", json!({ "serverAddress": address })).await.unwrap();
    assert_eq!(reflected["ok"], true, "{reflected}");
    let services = reflected["services"].as_array().unwrap();
    assert_eq!(services.len(), 1);
    assert_eq!(services[0]["name"], "test.echo.Echo");
    let methods = services[0]["methods"].as_array().unwrap();
    let find = |name: &str| methods.iter().find(|m| m["name"] == name).unwrap().clone();
    assert_eq!(find("Say")["inputType"], "test.echo.Req");
    assert_eq!(find("Say")["clientStreaming"], false);
    assert_eq!(find("Count")["serverStreaming"], true);
    assert_eq!(find("Collect")["clientStreaming"], true);
    assert_eq!(find("Chat")["clientStreaming"], true);
    assert_eq!(find("Chat")["serverStreaming"], true);
}

#[tokio::test]
async fn reflect_says_so_when_the_server_has_no_reflection_or_is_down() {
    let address = serve(false).await;
    let reflected = call("grpc.reflect", json!({ "serverAddress": address })).await.unwrap();
    assert_eq!(reflected["ok"], false);
    assert!(reflected["error"].as_str().unwrap().contains("does not support reflection"), "{reflected}");

    let down = call("grpc.reflect", json!({ "serverAddress": "127.0.0.1:1" })).await.unwrap();
    assert_eq!(down["ok"], false);
    assert!(down["error"].as_str().unwrap().contains("Could not connect"), "{down}");
}

#[tokio::test]
async fn every_streaming_kind_runs() {
    let address = serve(true).await;

    let unary = call("grpc.execute", execute(&address, "Say", json!({ "text": "hi", "n": 3 }))).await.unwrap();
    assert_eq!(unary["ok"], true, "{unary}");
    assert_eq!(texts(&unary), ["echo: hi (token t0k)"]);
    assert_eq!(unary["status"], 0);
    assert_eq!(unary["statusMessage"], "OK");
    assert_eq!(unary["metadata"]["x-served-by"], "echo");
    assert!(unary["durationMs"].is_u64());

    let server = call("grpc.execute", execute(&address, "Count", json!({ "text": "c", "n": 3 }))).await.unwrap();
    assert_eq!(server["responses"].as_array().unwrap().len(), 3);
    assert_eq!(serde_json::from_str::<Value>(server["responses"][2].as_str().unwrap()).unwrap()["n"], 3);

    let client = call("grpc.execute", execute(&address, "Collect", json!([{ "text": "a" }, { "text": "b" }, { "text": "c" }])))
        .await
        .unwrap();
    assert_eq!(texts(&client), ["a+b+c"]);

    let bidi = call("grpc.execute", execute(&address, "Chat", json!([{ "text": "x" }, { "text": "y" }]))).await.unwrap();
    assert_eq!(texts(&bidi), ["chat x", "chat y"]);

    let single = call("grpc.execute", execute(&address, "Chat", json!({ "text": "solo" }))).await.unwrap();
    assert_eq!(texts(&single), ["chat solo"]);
}

#[tokio::test]
async fn a_failing_call_reports_its_status() {
    let address = serve(true).await;
    let failed = call("grpc.execute", execute(&address, "Fail", json!({}))).await.unwrap();
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["status"], Code::NotFound as i32);
    assert_eq!(failed["statusMessage"], "nothing here");
    assert_eq!(failed["error"], "NOT_FOUND: nothing here");
}

#[tokio::test]
async fn mistakes_are_reported_rather_than_sent() {
    let address = serve(true).await;
    let cases = [
        (execute(&address, "Nope", json!({})), "has no method"),
        (
            json!({ "serverAddress": address, "serviceName": "no.Such", "methodName": "M", "requestBody": "{}",
                    "metadata": {}, "protoFileId": null, "useReflection": true }),
            "has no service",
        ),
        (execute(&address, "Say", json!({ "text": 5 })), "does not match test.echo.Req"),
        (
            json!({ "serverAddress": address, "serviceName": "test.echo.Echo", "methodName": "Say",
                    "requestBody": "{nope", "metadata": {}, "protoFileId": null, "useReflection": true }),
            "not valid JSON",
        ),
        (
            json!({ "serverAddress": "", "serviceName": "s", "methodName": "m", "requestBody": "{}",
                    "metadata": {}, "protoFileId": null, "useReflection": true }),
            "Enter the server address",
        ),
    ];
    for (payload, expected) in cases {
        let result = call("grpc.execute", payload).await.unwrap();
        assert_eq!(result["ok"], false, "{result}");
        assert!(result["error"].as_str().unwrap().contains(expected), "{expected}: {result}");
    }
}

#[tokio::test]
async fn a_saved_proto_file_stands_in_for_reflection() {
    let address = serve(false).await;
    let root = scratch_data_root();
    write_entity(
        root.ws_id,
        "protoFiles",
        "p1",
        &json!({ "id": "p1", "name": "echo.proto", "content": ECHO_PROTO, "createdAt": 1, "workspaceId": root.ws_id }),
        None,
    )
    .unwrap();
    let mut payload = execute(&address, "Say", json!({ "text": "proto" }));
    payload["protoFileId"] = json!("p1");
    payload["useReflection"] = json!(false);
    payload["workspaceId"] = json!(root.ws_id);
    let result = call("grpc.execute", payload.clone()).await.unwrap();
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(texts(&result), ["echo: proto (token t0k)"]);

    payload["protoFileId"] = json!("missing");
    let missing = call("grpc.execute", payload).await.unwrap();
    assert_eq!(missing["ok"], false);
    assert!(missing["error"].as_str().unwrap().contains("is not saved"), "{missing}");
}

#[tokio::test]
async fn send_runs_a_saved_request_with_its_variables_resolved() {
    let address = serve(true).await;
    let root = scratch_data_root();
    write_entity(
        root.ws_id,
        "grpcRequests",
        "r1",
        &json!({
            "id": "r1", "name": "r", "serverAddress": address, "serviceName": "test.echo.Echo", "methodName": "Say",
            "requestBody": "{\"text\": \"saved\"}", "metadata": { "x-token": "abc" }, "useReflection": true,
            "streamingType": "unary",
        }),
        None,
    )
    .unwrap();
    let sent = call("grpc.send", json!({ "workspaceId": root.ws_id, "id": "r1" })).await.unwrap();
    assert_eq!(sent["ok"], true, "{sent}");
    assert_eq!(texts(&sent), ["echo: saved (token abc)"]);
    let missing = call("grpc.send", json!({ "workspaceId": root.ws_id, "id": "nope" })).await.unwrap_err();
    assert_eq!(missing.code, crate::protocol::ErrorCode::NotFound);
}

/// Collects the live-call events of one stream.
struct StreamEvents {
    messages: Arc<std::sync::Mutex<Vec<(String, String)>>>,
    statuses: Arc<std::sync::Mutex<Vec<Value>>>,
    ids: Vec<crate::kernel::bus::ListenerId>,
}

impl StreamEvents {
    fn watch(stream_id: &'static str) -> Self {
        let messages = Arc::new(std::sync::Mutex::new(Vec::new()));
        let statuses = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (m, st) = (messages.clone(), statuses.clone());
        let ids = vec![
            crate::kernel::bus().on("grpc.stream.message", move |p| {
                if p["streamId"] == stream_id {
                    let message = &p["message"];
                    let text = serde_json::from_str::<Value>(message["data"].as_str().unwrap()).unwrap();
                    m.lock().unwrap().push((
                        message["direction"].as_str().unwrap().to_owned(),
                        text["text"].as_str().unwrap_or_default().to_owned(),
                    ));
                }
            }),
            crate::kernel::bus().on("grpc.stream.status", move |p| {
                if p["streamId"] == stream_id {
                    st.lock().unwrap().push(p.clone());
                }
            }),
        ];
        Self { messages, statuses, ids }
    }

    fn messages(&self) -> Vec<(String, String)> {
        self.messages.lock().unwrap().clone()
    }

    fn closed(&self) -> Option<Value> {
        self.statuses.lock().unwrap().iter().find(|s| s["status"] == "closed").cloned()
    }

    async fn until(&self, what: &str, mut condition: impl FnMut(&Self) -> bool) {
        for _ in 0..200 {
            if condition(self) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for {what}: {:?} {:?}", self.messages(), self.statuses.lock().unwrap());
    }
}

impl Drop for StreamEvents {
    fn drop(&mut self) {
        for id in &self.ids {
            crate::kernel::bus().off(*id);
        }
    }
}

fn save_request(ws_id: &str, id: &str, address: &str, method: &str, body: &str) {
    write_entity(
        ws_id,
        "grpcRequests",
        id,
        &json!({
            "id": id, "name": id, "serverAddress": address, "serviceName": "test.echo.Echo", "methodName": method,
            "requestBody": body, "metadata": {}, "useReflection": true, "streamingType": "unary",
        }),
        None,
    )
    .unwrap();
}

#[tokio::test]
async fn a_bidirectional_call_stays_open_and_answers_each_message() {
    let address = serve(true).await;
    let root = scratch_data_root();
    save_request(root.ws_id, "chat", &address, "Chat", "{}");
    let events = StreamEvents::watch("s-chat");
    let open = call("grpc.stream.open", json!({ "workspaceId": root.ws_id, "streamId": "s-chat", "id": "chat" })).await.unwrap();
    assert_eq!(open, json!({ "ok": true, "kind": "bidi" }));

    let send = |text: &'static str| {
        call("grpc.stream.send", json!({ "workspaceId": root.ws_id, "streamId": "s-chat", "data": format!("{{\"text\":\"{text}\"}}") }))
    };
    assert_eq!(send("one").await.unwrap(), json!({ "ok": true }));
    // The reply arrives while the call is still open: the second message has not been sent yet.
    events.until("the first reply", |e| e.messages().iter().any(|(d, t)| d == "received" && t == "chat one")).await;
    assert!(events.closed().is_none());
    assert_eq!(send("two").await.unwrap(), json!({ "ok": true }));
    events.until("the second reply", |e| e.messages().iter().any(|(_, t)| t == "chat two")).await;

    let bad = call("grpc.stream.send", json!({ "workspaceId": root.ws_id, "streamId": "s-chat", "data": "{nope" })).await.unwrap();
    assert_eq!(bad["ok"], false);

    call("grpc.stream.end", json!({ "workspaceId": root.ws_id, "streamId": "s-chat" })).await.unwrap();
    events.until("the call to close", |e| e.closed().is_some()).await;
    assert_eq!(events.closed().unwrap()["grpcStatus"], 0);
    let sent: Vec<_> = events.messages().into_iter().filter(|(d, _)| d == "sent").map(|(_, t)| t).collect();
    assert_eq!(sent, ["one", "two"]);
    let gone = call("grpc.stream.send", json!({ "workspaceId": root.ws_id, "streamId": "s-chat", "data": "{}" })).await.unwrap();
    assert_eq!(gone["ok"], false);
}

#[tokio::test]
async fn a_client_streaming_call_answers_when_it_is_ended() {
    let address = serve(true).await;
    let root = scratch_data_root();
    save_request(root.ws_id, "collect", &address, "Collect", "{}");
    let events = StreamEvents::watch("s-collect");
    let open =
        call("grpc.stream.open", json!({ "workspaceId": root.ws_id, "streamId": "s-collect", "id": "collect" })).await.unwrap();
    assert_eq!(open["kind"], "client");
    for text in ["a", "b", "c"] {
        let data = format!("{{\"text\":\"{text}\"}}");
        call("grpc.stream.send", json!({ "workspaceId": root.ws_id, "streamId": "s-collect", "data": data })).await.unwrap();
    }
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(events.messages().iter().all(|(d, _)| d == "sent"), "no reply before the end");
    call("grpc.stream.end", json!({ "workspaceId": root.ws_id, "streamId": "s-collect" })).await.unwrap();
    events.until("the reply", |e| e.closed().is_some()).await;
    assert!(events.messages().contains(&("received".to_owned(), "a+b+c".to_owned())));
}

#[tokio::test]
async fn a_server_streaming_call_sends_its_body_and_ends_on_its_own() {
    let address = serve(true).await;
    let root = scratch_data_root();
    save_request(root.ws_id, "count", &address, "Count", "{\"text\": \"c\", \"n\": 3}");
    let events = StreamEvents::watch("s-count");
    let open = call("grpc.stream.open", json!({ "workspaceId": root.ws_id, "streamId": "s-count", "id": "count" })).await.unwrap();
    assert_eq!(open["kind"], "server");
    events.until("the call to end", |e| e.closed().is_some()).await;
    assert_eq!(events.messages().iter().filter(|(d, _)| d == "received").count(), 3);
    let refused = call("grpc.stream.send", json!({ "workspaceId": root.ws_id, "streamId": "s-count", "data": "{}" })).await.unwrap();
    assert_eq!(refused["ok"], false);
}

#[tokio::test]
async fn a_live_call_can_be_cancelled_and_a_bad_open_is_refused() {
    let address = serve(true).await;
    let root = scratch_data_root();
    save_request(root.ws_id, "chat2", &address, "Chat", "{}");
    let events = StreamEvents::watch("s-cancel");
    call("grpc.stream.open", json!({ "workspaceId": root.ws_id, "streamId": "s-cancel", "id": "chat2" })).await.unwrap();
    call("grpc.stream.cancel", json!({ "workspaceId": root.ws_id, "streamId": "s-cancel" })).await.unwrap();
    events.until("the call to close", |e| e.closed().is_some()).await;
    assert_eq!(events.closed().unwrap()["grpcStatus"], Code::Cancelled as i32);

    save_request(root.ws_id, "dead", "127.0.0.1:1", "Chat", "{}");
    let refused = call("grpc.stream.open", json!({ "workspaceId": root.ws_id, "streamId": "s-dead", "id": "dead" })).await.unwrap();
    assert_eq!(refused["ok"], false);
    let missing = call("grpc.stream.open", json!({ "workspaceId": root.ws_id, "streamId": "s-x", "id": "nope" })).await.unwrap_err();
    assert_eq!(missing.code, crate::protocol::ErrorCode::NotFound);
}

#[test]
fn the_reflection_service_is_not_listed() {
    let reflection = r#"
        syntax = "proto3";
        package grpc.reflection.v1alpha;
        message Q { string s = 1; }
        service ServerReflection { rpc ServerReflectionInfo(stream Q) returns (stream Q); }
    "#;
    let pool = proto::compile(
        "echo.proto",
        vec![("echo.proto".into(), ECHO_PROTO.into()), ("reflection.proto".into(), reflection.into())],
    )
    .unwrap();
    let mut both = proto::compile("reflection.proto", vec![("reflection.proto".into(), reflection.into())]).unwrap();
    for file in pool.files() {
        let _ = both.add_file_descriptor_proto(file.file_descriptor_proto().clone());
    }
    assert!(both.get_service_by_name("grpc.reflection.v1alpha.ServerReflection").is_some());
    let services = super::services_json(&both, None);
    let names: Vec<_> = services.as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap().to_owned()).collect();
    assert_eq!(names, ["test.echo.Echo"]);
}
