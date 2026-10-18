use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use super::*;
use crate::{
    kernel::{CommandContext, CommandRegistry, bus},
    rest::test_server,
    testing::scratch_data_root,
};

const MASKED_B64: &str = "%E2%80%A2%E2%80%A2%E2%80%A2%E2%80%A2";

fn rec(method: &str, url: &str, body: Option<&str>) -> Recorded {
    Recorded::parse(&json!({
        "request": { "method": method, "url": url, "headers": {}, "body": body },
        "response": { "status": 200, "headers": {}, "body": "ok" },
    }))
    .unwrap()
}

fn hit(recorded: &Recorded, method: &str, target: &str, content_type: &str, body: &str) -> bool {
    matches(recorded, &Incoming::new(method, target, content_type, body.as_bytes()))
}

#[test]
fn method_path_and_query_decide_the_match() {
    let r = rec("POST", "https://api.x.com/v1/a%20b?x=1&y=2", None);
    assert!(hit(&r, "post", "/v1/a%20b?y=2&x=1", "", ""));
    assert!(hit(&r, "POST", "/v1/a b?x=1&y=2", "", ""));
    assert!(!hit(&r, "GET", "/v1/a%20b?x=1&y=2", "", ""));
    assert!(!hit(&r, "POST", "/v1/a%20c?x=1&y=2", "", ""));
    assert!(!hit(&r, "POST", "/v1/a%20b/?x=1&y=2", "", ""));
    // Query: same pairs, any order, no extras, no missing; repeated keys count.
    assert!(!hit(&r, "POST", "/v1/a%20b?x=1", "", ""));
    assert!(!hit(&r, "POST", "/v1/a%20b?x=1&y=2&z=3", "", ""));
    assert!(!hit(&r, "POST", "/v1/a%20b?x=1&y=3", "", ""));
    let twice = rec("GET", "https://api.x.com/l?t=a&t=b", None);
    assert!(hit(&twice, "GET", "/l?t=b&t=a", "", ""));
    assert!(!hit(&twice, "GET", "/l?t=a&t=a", "", ""));
}

#[test]
fn a_masked_query_value_matches_any_value_even_when_percent_encoded() {
    let r = rec("GET", &format!("https://api.x.com/v1?api_key={MASKED_B64}&page=2"), None);
    assert_eq!(r.query, [("api_key".to_string(), MASK.to_string()), ("page".to_string(), "2".to_string())]);
    assert!(hit(&r, "GET", "/v1?page=2&api_key=sk_live_123", "", ""));
    assert!(!hit(&r, "GET", "/v1?page=2", "", ""));
    assert!(!hit(&r, "GET", "/v1?page=3&api_key=k", "", ""));
    // A wildcard never steals the pair an exact value needs.
    let r = rec("GET", &format!("https://x.com/v?a={MASKED_B64}&a=1"), None);
    assert!(hit(&r, "GET", "/v?a=1&a=2", "", ""));
    assert!(hit(&r, "GET", "/v?a=2&a=1", "", ""));
    assert!(!hit(&r, "GET", "/v?a=2&a=3", "", ""));
    // Volatile query keys are ignored.
    let r = rec("GET", "https://x.com/v?id=1&timestamp=5", None);
    assert!(hit(&r, "GET", "/v?id=1&Timestamp=9", "", ""));
    assert!(hit(&r, "GET", "/v?id=1", "", ""));
}

#[test]
fn json_bodies_ignore_volatile_keys_timestamps_and_masked_strings() {
    let recorded = r#"{"amount":5,"Idempotency-Key":"a","card":{"number":"••••","exp_month":1},"items":[{"created_at":"2026-10-18T10:00:00Z","n":1}],"at":"2026-01-01T00:00:00Z"}"#;
    let r = rec("POST", "https://x.com/c", Some(recorded));
    let same_but = |body: &str| hit(&r, "POST", "/c", "application/json", body);
    assert!(same_but(
        r#"{"at":"2027-05-05 12:30:00.5+05:30","items":[{"n":1,"created_at":99}],"card":{"exp_month":1,"number":"4242"},"amount":5,"idempotency_key":"zzz","request_id":"r"}"#
    ));
    assert!(!same_but(
        r#"{"at":"2026-01-01T00:00:00Z","items":[{"n":2}],"card":{"exp_month":1,"number":"4"},"amount":5}"#
    ));
    assert!(!same_but(r#"{"at":"yesterday","items":[{"n":1}],"card":{"exp_month":1,"number":"4"},"amount":5}"#));
    assert!(!same_but(
        r#"{"at":"2026-01-01T00:00:00Z","items":[{"n":1}],"card":{"exp_month":1,"number":"4"},"amount":6}"#
    ));
    // A masked string is a wildcard for strings only by being a string wildcard of any kind; extra keys differ.
    assert!(!same_but(
        r#"{"at":"2026-01-01T00:00:00Z","items":[{"n":1}],"card":{"exp_month":1,"number":"4"},"amount":5,"extra":1}"#
    ));
    assert!(!same_but("not json"));
    // Timestamps only: both sides must look like one.
    assert!(
        is_timestamp("2026-10-18T10:24:16Z")
            && is_timestamp("2026-10-18 10:24")
            && is_timestamp("2026-10-18T10:24:16.5+05:30")
    );
    assert!(!is_timestamp("2026-10-18") && !is_timestamp("hello world, 2026") && !is_timestamp("2026-10-18Tab:cd"));
}

#[test]
fn form_bodies_follow_the_query_rules_and_other_bodies_must_be_equal() {
    let form = rec("POST", "https://x.com/f", Some(&format!("b=2&a=1&token={MASKED_B64}&nonce=9")));
    let ct = "application/x-www-form-urlencoded; charset=utf-8";
    assert!(hit(&form, "POST", "/f", ct, "a=1&token=secret&b=2&nonce=10"));
    assert!(!hit(&form, "POST", "/f", ct, "a=1&b=3&token=s"));
    assert!(!hit(&form, "POST", "/f", "text/plain", "a=1&token=secret&b=2"));
    let raw = rec("PUT", "https://x.com/r", Some("plain text"));
    assert!(hit(&raw, "PUT", "/r", "text/plain", "plain text"));
    assert!(!hit(&raw, "PUT", "/r", "text/plain", "plain text "));
    // Empty equals empty (a recorded `null` body too), and nothing else.
    let empty = rec("GET", "https://x.com/e", None);
    assert!(hit(&empty, "GET", "/e", "", ""));
    assert!(!hit(&empty, "GET", "/e", "", "x"));
    assert!(!hit(&raw, "PUT", "/r", "", ""));
}

fn cassette(items: Vec<Value>) -> Value {
    json!({ "version": 1, "name": "c", "hosts": ["api.x.com"], "recordedAt": 1, "interactions": items })
}

fn item(url: &str, status: u16, body: &str) -> Value {
    json!({ "request": { "method": "GET", "url": url, "headers": {} },
            "response": { "status": status, "headers": { "set-cookie": MASK, "Content-Type": "text/plain",
                          "connection": "keep-alive" }, "body": body } })
}

fn ask(replay: &mut Replay, method: &str, target: &str) -> Option<Reply> {
    replay.decide(&Incoming::new(method, target, "", b""), target, &format!("https://api.x.com{target}"), true)
}

#[test]
fn repeated_requests_get_matches_in_order_then_the_last_one() {
    let file = cassette(vec![
        item("https://api.x.com/p", 202, "first"),
        item("https://api.x.com/q", 200, "other"),
        item("https://api.x.com/p", 200, "second"),
    ]);
    let mut replay = Replay::new("c", "ws1", &file, false);
    let answers: Vec<_> = (0..4).map(|_| ask(&mut replay, "GET", "/p").unwrap()).collect();
    let bodies: Vec<_> = answers.iter().map(|a| String::from_utf8(a.body.clone()).unwrap()).collect();
    assert_eq!(bodies, ["first", "second", "second", "second"]);
    assert_eq!(answers[0].status, 202);
    assert_eq!(replay.served, 4);
    // Hop-by-hop and masked headers are gone, names are lowercase, the length is the body's.
    let h = &answers[0].headers;
    assert_eq!(h.get("content-type").map(String::as_str), Some("text/plain"));
    assert_eq!(h.get("connection").map(String::as_str), Some("close"));
    assert_eq!(h.get("content-length").map(String::as_str), Some("5"));
    assert!(!h.contains_key("set-cookie") && !h.contains_key("Content-Type"));
    assert_eq!((answers[0].via, answers[0].target.as_deref()), ("mock", Some("cassette:c")));
}

#[test]
fn unmatched_requests_are_a_404_naming_the_closest_or_pass_through() {
    let mut file = cassette(vec![
        item("https://api.x.com/v1/charges", 200, "a"),
        item("https://api.x.com/v1/customers", 200, "b"),
    ]);
    file["interactions"][1]["request"]["method"] = json!("POST");
    let mut replay = Replay::new("c", "ws1", &file, false);
    let miss = ask(&mut replay, "GET", "/v1/chargez?x=1").unwrap();
    assert_eq!((miss.status, miss.via, miss.target), (404, "error", None));
    assert_eq!(miss.headers.get("x-bifurc-cassette").map(String::as_str), Some("unmatched"));
    let body: Value = serde_json::from_slice(&miss.body).unwrap();
    assert_eq!(body["error"], "no recorded interaction in cassette \"c\" matches GET /v1/chargez?x=1");
    // Same method first, then the longer shared path.
    assert_eq!(body["closest"], json!(["GET /v1/charges", "POST /v1/customers"]));
    ask(&mut replay, "GET", "/v1/chargez?x=1");
    assert_eq!(replay.unmatched_count, 2);
    assert_eq!(replay.unmatched.len(), 1);
    assert_eq!(replay.unmatched[0]["closest"], "GET /v1/charges");

    let mut through = Replay::new("c", "ws1", &file, true);
    assert!(ask(&mut through, "GET", "/nope").is_none());
    assert_eq!((through.passed, through.unmatched_count), (1, 0));
    // A truncated recorded body is served empty with a warning header.
    let mut cut = cassette(vec![item("https://api.x.com/big", 200, "")]);
    cut["interactions"][0]["response"]["body"] = Value::Null;
    cut["interactions"][0]["response"]["truncated"] = json!(true);
    let reply = ask(&mut Replay::new("c", "ws1", &cut, false), "GET", "/big").unwrap();
    assert!(reply.body.is_empty());
    assert_eq!(reply.headers.get("x-bifurc-cassette").map(String::as_str), Some("truncated"));
}

fn registry() -> Arc<CommandRegistry> {
    let mut registry = CommandRegistry::new();
    crate::cassettes::register(&mut registry);
    crate::mocks::register(&mut registry);
    Arc::new(registry)
}

async fn call(registry: &Arc<CommandRegistry>, name: &str, payload: Value) -> Result<Value> {
    registry.invoke(name, payload, CommandContext::internal(registry.clone())).await
}

fn write_cassette(name: &str, file: &Value) {
    let dir = ws_dir("ws1").join(DIR);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{name}.json")), serde_json::to_string_pretty(file).unwrap()).unwrap();
}

async fn send(port: u16, method: &str, path: &str, body: &str) -> String {
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), socket.read_to_end(&mut out)).await.unwrap().unwrap();
    String::from_utf8_lossy(&out).into_owned()
}

#[tokio::test]
async fn replay_commands_and_list_with_a_scratch_data_root() {
    let _root = scratch_data_root();
    let registry = registry();
    write_cassette(
        "checkout",
        &cassette(vec![item("https://api.x.com/a", 200, "a"), item("https://api.x.com/b", 200, "b")]),
    );
    write_cassette("other", &cassette(vec![]));
    std::fs::write(ws_dir("ws1").join(DIR).join("half.json.tmp"), "{").unwrap();
    let listed = call(&registry, "cassette.list", json!({ "workspaceId": "ws1" })).await.unwrap();
    let rows = listed["cassettes"].as_array().unwrap();
    assert_eq!(rows.iter().map(|r| str_of(r, "name")).collect::<Vec<_>>(), ["checkout", "other"]);
    assert_eq!(
        (&rows[0]["interactions"], &rows[0]["hosts"], &rows[0]["recordedAt"]),
        (&json!(2), &json!(["api.x.com"]), &json!(1))
    );

    assert_eq!(
        call(&registry, "cassette.replayStop", json!({})).await.unwrap_err().code,
        crate::protocol::ErrorCode::NotFound
    );
    assert_eq!(call(&registry, "cassette.replayStatus", json!({})).await.unwrap()["replaying"], Value::Null);
    let missing = call(&registry, "cassette.replayStart", json!({ "workspaceId": "ws1", "name": "nope" })).await;
    assert_eq!(missing.unwrap_err().code, crate::protocol::ErrorCode::NotFound);
    let bad = call(&registry, "cassette.replayStart", json!({ "workspaceId": "ws1", "name": "../x" })).await;
    assert_eq!(bad.unwrap_err().code, crate::protocol::ErrorCode::BadRequest);

    let started =
        call(&registry, "cassette.replayStart", json!({ "workspaceId": "ws1", "name": "checkout" })).await.unwrap();
    assert_eq!(
        (&started["name"], &started["hosts"], &started["interactions"], &started["passthrough"]),
        (&json!("checkout"), &json!(["api.x.com"]), &json!(2), &json!(false))
    );
    let twice = call(&registry, "cassette.replayStart", json!({ "workspaceId": "ws1", "name": "other" })).await;
    assert!(twice.unwrap_err().to_string().contains("already replaying"));
    // Recording while replaying is refused.
    let rec =
        call(&registry, "cassette.recordStart", json!({ "workspaceId": "ws1", "name": "r", "hosts": ["a.com"] })).await;
    assert!(rec.unwrap_err().to_string().contains("replaying \"checkout\""));
    let status = call(&registry, "cassette.replayStatus", json!({})).await.unwrap();
    assert_eq!(status["replaying"]["name"], "checkout");
    let stopped = call(&registry, "cassette.replayStop", json!({})).await.unwrap();
    assert_eq!(
        (&stopped["served"], &stopped["unmatchedCount"], &stopped["unmatched"]),
        (&json!(0), &json!(0), &json!([]))
    );

    // And replaying while recording.
    call(&registry, "cassette.recordStart", json!({ "workspaceId": "ws1", "name": "r", "hosts": ["a.com"] }))
        .await
        .unwrap();
    let busy = call(&registry, "cassette.replayStart", json!({ "workspaceId": "ws1", "name": "checkout" })).await;
    assert!(busy.unwrap_err().to_string().contains("recording \"r\""));
    call(&registry, "cassette.recordStop", json!({})).await.unwrap();
}

#[tokio::test]
async fn the_mock_server_serves_recorded_answers_and_leaves_the_rest_to_the_mocks() {
    let _root = scratch_data_root();
    let logs = Arc::new(Mutex::new(Vec::<Value>::new()));
    let sink = logs.clone();
    let listener = bus().on("log.entry", move |entry| sink.lock().push(entry.clone()));
    let mut post = item("https://api.x.com/v1/charges", 201, r#"{"id":"ch_2"}"#);
    post["request"]["method"] = json!("POST");
    post["request"]["body"] = json!(r#"{"amount":5,"idempotency_key":"a","card":{"number":"••••"}}"#);
    write_cassette(
        "checkout",
        &cassette(vec![
            item("https://api.x.com/v1/charges/ch_1", 200, "first"),
            item("https://api.x.com/v1/charges/ch_1", 200, "second"),
            item(&format!("https://api.x.com/v1/list?api_key={MASKED_B64}&page=2"), 200, "listed"),
            post,
        ]),
    );
    let registry = registry();
    call(&registry, "cassette.replayStart", json!({ "workspaceId": "ws1", "name": "checkout" })).await.unwrap();
    let port = call(&registry, "mocks.serve", json!({ "workspaceId": "ws1", "port": 0, "detach": true })).await.unwrap()
        ["port"]
        .as_u64()
        .unwrap() as u16;

    let first = send(port, "GET", "/v1/charges/ch_1", "").await;
    assert!(first.starts_with("HTTP/1.1 200 ") && first.ends_with("first"), "{first}");
    assert!(!first.to_lowercase().contains("set-cookie"), "{first}");
    assert!(send(port, "GET", "/v1/charges/ch_1", "").await.ends_with("second"));
    assert!(send(port, "GET", "/v1/charges/ch_1", "").await.ends_with("second"));
    assert!(send(port, "GET", "/v1/list?page=2&api_key=real", "").await.ends_with("listed"));
    let created =
        send(port, "POST", "/v1/charges", r#"{"card":{"number":"4242"},"idempotency_key":"zz","amount":5}"#).await;
    assert!(created.starts_with("HTTP/1.1 201 ") && created.ends_with(r#"{"id":"ch_2"}"#), "{created}");

    // The mock server cannot tell a request for the cassette's hosts from one for the workspace's own mocks: a
    // miss goes on to the mocks (none here, so the mock server's own 404) and is not the cassette's to count.
    let miss = send(port, "GET", "/v1/charges/ch_9", "").await;
    assert!(miss.starts_with("HTTP/1.1 404 ") && !miss.to_lowercase().contains("x-bifurc-cassette"), "{miss}");
    assert!(miss.contains("no mock matches GET /v1/charges/ch_9"), "{miss}");
    assert_eq!(call(&registry, "cassette.replayStatus", json!({})).await.unwrap()["replaying"]["served"], 5);

    let stopped = call(&registry, "cassette.replayStop", json!({})).await.unwrap();
    assert_eq!(
        (&stopped["served"], &stopped["passedThrough"], &stopped["unmatchedCount"]),
        (&json!(5), &json!(0), &json!(0))
    );

    // The replayed answer is a "mock" entry that names the cassette; the miss is the mock server's error entry.
    let entries = logs.lock().clone();
    let served = entries.iter().find(|e| str_of(e, "url").ends_with("/v1/list?page=2&api_key=real")).unwrap();
    assert_eq!(
        (str_of(served, "via"), str_of(served, "target"), str_of(served, "origin")),
        ("mock", "cassette:checkout", "server")
    );
    let missed = entries.iter().find(|e| str_of(e, "url").ends_with("/ch_9")).unwrap();
    assert_eq!((str_of(missed, "via"), missed["status"].as_u64()), ("error", Some(404)));

    // With a fallthrough, a miss reaches the upstream; recorded ones are still replayed.
    let canned = HashMap::from([("/real".to_string(), ("text/plain".to_string(), "from upstream".to_string()))]);
    let upstream = test_server::start(canned).await;
    call(&registry, "mocks.serveStop", json!({})).await.unwrap();
    let port = call(
        &registry,
        "mocks.serve",
        json!({ "workspaceId": "ws1", "port": 0, "detach": true, "fallthrough": upstream }),
    )
    .await
    .unwrap()["port"]
        .as_u64()
        .unwrap() as u16;
    let started = call(
        &registry,
        "cassette.replayStart",
        json!({ "workspaceId": "ws1", "name": "checkout", "passthrough": true }),
    )
    .await
    .unwrap();
    assert_eq!(started["passthrough"], true);
    assert!(send(port, "GET", "/real", "").await.ends_with("from upstream"));
    assert!(send(port, "GET", "/v1/charges/ch_1", "").await.ends_with("first"));
    let stopped = call(&registry, "cassette.replayStop", json!({})).await.unwrap();
    assert_eq!(
        (&stopped["served"], &stopped["passedThrough"], &stopped["unmatchedCount"]),
        (&json!(1), &json!(0), &json!(0))
    );
    // Nothing replays: the same request goes to the fallthrough now.
    assert!(!send(port, "GET", "/v1/charges/ch_1", "").await.ends_with("first"));
    call(&registry, "mocks.serveStop", json!({})).await.unwrap();
    bus().off(listener);
}
