use base64::{Engine as _, engine::general_purpose::STANDARD};

use super::*;
use crate::{
    commands::with_field,
    entities::mocks,
    testing::{Answer, config, payloads, run, sub},
};

fn mock(id: &str, name: &str, pattern: &str, status: u64, body: &str, delay: u64, created: i64) -> Value {
    json!({"id": id, "name": name, "method": "GET", "urlPattern": pattern, "useRegex": false, "standalone": true,
           "responseStatus": status, "responseBody": body, "responseDelay": delay, "createdAt": created,
           "responseHeaders": {"Content-Type": "text/html"}, "workspaceId": "w1"})
}

/// Two calls: "Get orders" with four mocks and "Get user" with one; `on` is the id of the enabled one.
fn engine(on: &str) -> Vec<Answer> {
    let orders = "http://api.localhost/orders";
    let all = [
        ("entity.load:mocks:o1", mock("o1", "Get orders", orders, 200, "[1,2,3]", 0, 1)),
        ("entity.load:mocks:o2", mock("o2", "Get orders · empty", orders, 200, "[]", 0, 2)),
        ("entity.load:mocks:o3", mock("o3", "Get orders · error", orders, 500, "", 0, 3)),
        ("entity.load:mocks:o4", mock("o4", "Get orders · slow", orders, 200, "[1,2,3]", 3000, 4)),
        ("entity.load:mocks:u1", mock("u1", "Get user", "http://api.localhost/user", 200, "{\"id\":1}", 0, 5)),
    ];
    let listed: Vec<Value> = all.iter().map(|(_, m)| m.clone()).collect();
    let mut answers: Vec<Answer> = vec![
        ("config.get", Ok(config())),
        ("entity.list:mocks", Ok(json!({ "entities": listed }))),
        ("entity.setEnabled", Ok(json!({"ok": true}))),
        ("entity.create", Ok(json!({"id": "o9", "entity": {"id": "o9"}}))),
    ];
    for (key, mock) in all {
        let enabled = mock["id"] == on;
        answers.push((key, Ok(json!({"ok": true, "entity": with_field(&mock, "enabled", json!(enabled))}))));
    }
    answers
}

fn go(argv: &[&str], answers: Vec<Answer>) -> (Result<String>, Vec<(String, Value)>) {
    run(argv, answers, |c| mocks(c, sub(argv)))
}

#[test]
fn the_scenarios_of_a_call_are_listed_with_the_active_one_starred() {
    for argv in [&["mocks", "scenario", "Get orders"][..], &["mocks", "scenario", "get orders · slow", "--list"]] {
        let (out, _) = go(argv, engine("o2"));
        assert_eq!(
            out.unwrap(),
            "  default  200  3 items\n* empty    200  []\n  error    500\n  slow     200  3 items  delay 3000ms"
        );
    }
    let (out, _) = go(&["mocks", "scenario", "Get orders"], engine("none"));
    assert!(out.unwrap().lines().all(|line| line.ends_with("(off)")));
}

#[test]
fn switching_enables_the_mock_of_the_scenario_and_default_goes_back() {
    let (out, calls) = go(&["mocks", "scenario", "Get orders", "error"], engine("o2"));
    assert_eq!(out.unwrap(), "Get orders → error (500)");
    assert_eq!(
        payloads(&calls, "entity.setEnabled"),
        [json!({"workspaceId": "w1", "kind": "mocks", "id": "o3", "enabled": true})]
    );
    let (out, calls) = go(&["mocks", "scenario", "Get orders · empty", "default"], engine("o2"));
    assert_eq!(out.unwrap(), "Get orders → default (200 3 items)");
    assert_eq!(payloads(&calls, "entity.setEnabled")[0]["id"], "o1");
    assert!(payloads(&calls, "entity.create").is_empty());
}

#[test]
fn a_preset_is_created_as_a_copy_of_the_default() {
    let (out, calls) = go(&["mocks", "scenario", "Get orders", "error500"], engine("o1"));
    assert_eq!(out.unwrap(), "Get orders → error500 (500 1 key)");
    let created = &payloads(&calls, "entity.create")[0];
    assert_eq!(created["kind"], "mocks");
    let entity = &created["entity"];
    assert_eq!(entity["name"], "Get orders · error500");
    assert_eq!(entity["method"], "GET");
    assert_eq!(entity["urlPattern"], "http://api.localhost/orders");
    assert_eq!(entity["responseStatus"], 500);
    assert_eq!(entity["responseBody"], "{\"error\":\"Internal Server Error\"}");
    assert_eq!(entity["responseHeaders"], json!({"Content-Type": "application/json"}));
    assert_eq!((entity["enabled"].clone(), entity["standalone"].clone()), (json!(true), json!(true)));
    assert!(entity.get("id").is_none() && entity.get("createdAt").is_none());
    assert!(payloads(&calls, "entity.setEnabled").is_empty());

    for (preset, status, body) in
        [("error401", 401, "{\"error\":\"Unauthorized\"}"), ("notfound", 404, "{\"error\":\"Not Found\"}")]
    {
        let (_, calls) = go(&["mocks", "scenario", "Get orders", preset], engine("o1"));
        let entity = &payloads(&calls, "entity.create")[0]["entity"];
        assert_eq!((entity["responseStatus"].clone(), entity["responseBody"].clone()), (json!(status), json!(body)));
    }
    let (out, calls) = go(&["mocks", "scenario", "Get user", "EMPTY"], engine("u1"));
    assert_eq!(out.unwrap(), "Get user → empty (200 {})");
    assert_eq!(payloads(&calls, "entity.create")[0]["entity"]["responseBody"], "{}");
}

#[test]
fn slow_and_timeout_keep_the_default_answer_and_add_a_delay() {
    let (_, calls) = go(&["mocks", "scenario", "Get user", "slow"], engine("u1"));
    let entity = &payloads(&calls, "entity.create")[0]["entity"];
    assert_eq!((entity["responseDelay"].clone(), entity["responseBody"].clone()), (json!(3000), json!("{\"id\":1}")));
    let (out, calls) = go(&["mocks", "scenario", "Get user", "slow", "--delay", "500"], engine("u1"));
    assert_eq!(out.unwrap(), "Get user → slow (200 1 key delay 500ms)");
    assert_eq!(payloads(&calls, "entity.create")[0]["entity"]["responseDelay"], 500);
    let (_, calls) = go(&["mocks", "scenario", "Get user", "timeout"], engine("u1"));
    assert_eq!(payloads(&calls, "entity.create")[0]["entity"]["responseDelay"], 60_000);
    // --delay is for a scenario that is created.
    let (out, _) = go(&["mocks", "scenario", "Get orders", "slow", "--delay", "1"], engine("o1"));
    assert_eq!(out.unwrap_err().code(), 2);
}

#[test]
fn a_scenario_that_is_neither_there_nor_a_preset_is_not_found() {
    let (out, calls) = go(&["mocks", "scenario", "Get orders", "flaky"], engine("o1"));
    let err = out.unwrap_err();
    assert_eq!(err.code(), 3);
    assert!(err.line().contains("default, empty, error, slow"), "{}", err.line());
    assert!(payloads(&calls, "entity.create").is_empty() && payloads(&calls, "entity.setEnabled").is_empty());
}

fn buffer() -> Value {
    let entry = |id: &str, mock: Option<&str>| {
        json!({"id": id, "timestamp": 1_792_332_131_000_i64, "method": "GET", "url": "http://api.localhost/orders",
               "status": 200, "durationMs": 3, "via": "mock", "mockId": mock})
    };
    json!({"entries": [entry("aaaa1111", Some("o2")), entry("aaaa2222", Some("o2")), entry("aaaa3333", Some("o1")),
                       entry("aaaa4444", Some("u1")), entry("aaaa5555", None)], "matched": 5})
}

#[test]
fn status_is_a_line_per_call_with_the_hits_of_all_its_mocks() {
    let mut answers = engine("o2");
    answers.push(("capture.list", Ok(buffer())));
    let (out, calls) = go(&["mocks", "status"], answers);
    assert_eq!(
        out.unwrap(),
        "Get orders  GET  /orders  empty  on   3 hits\nGet user    GET  /user    -      off  1 hit"
    );
    assert_eq!(payloads(&calls, "capture.list"), [json!({"workspaceId": "w1", "summary": true})]);
}

#[test]
fn status_json_has_the_mocks_of_each_group() {
    let mut answers = engine("o2");
    answers.push(("capture.list", Ok(buffer())));
    let (out, _) = go(&["mocks", "status", "--json"], answers);
    let groups: Vec<Value> = serde_json::from_str(&out.unwrap()).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!((groups[0]["scenario"].clone(), groups[0]["hits"].clone()), (json!("empty"), json!(3)));
    assert_eq!(groups[0]["mocks"].as_array().unwrap().len(), 4);
    assert_eq!(groups[1]["enabled"], false);
}

#[test]
fn hits_asks_for_the_entries_of_every_mock_of_the_call() {
    let mut answers = engine("o2");
    answers.push(("capture.list", Ok(json!({"entries": [buffer()["entries"][0].clone()], "matched": 3}))));
    let (out, calls) = go(&["mocks", "hits", "Get orders · error", "--last", "1"], answers);
    assert_eq!(
        payloads(&calls, "capture.list"),
        [json!({"workspaceId": "w1", "summary": true, "mockId": "o1,o2,o3,o4", "last": 1})]
    );
    assert_eq!(
        out.unwrap(),
        "#1111  14:02:11Z  GET  api.localhost/orders  200  3ms  mock\n… 2 more — use --last or a filter"
    );
    let mut answers = engine("u1");
    answers.push(("capture.list", Ok(json!({"entries": []}))));
    let (out, calls) = go(&["mocks", "hits", "Get user"], answers);
    assert_eq!(out.unwrap(), "(none)");
    assert_eq!(payloads(&calls, "capture.list")[0]["last"], 5);
}

#[test]
fn a_mock_is_made_from_the_answer_of_a_sent_request() {
    let sent = json!({"status": 200, "request": {"method": "GET", "url": "http://api.localhost/orders"},
        "headers": {"Content-Type": "application/json", "Content-Length": "7", "Set-Cookie": "a=1", "X-Total": "3"},
        "body": STANDARD.encode("[1,2,3]")});
    let built = json!({"mock": {"name": "", "method": "GET", "urlPattern": "http://api.localhost/orders",
        "responseStatus": 200, "responseHeaders": {"Content-Type": "application/json", "X-Total": "3"},
        "responseBody": "[1,2,3]"}});
    let answers: Vec<Answer> = vec![
        ("config.get", Ok(config())),
        ("entity.list:requests", Ok(json!({"entities": [{"id": "r1", "name": "Get orders"}]}))),
        ("request.send", Ok(sent)),
        ("capture.buildMock", Ok(built)),
        ("entity.create", Ok(json!({"id": "o9", "entity": {"id": "o9"}}))),
    ];
    let argv = ["mocks", "create", "--from-request", "Get orders", "--var", "a=1"];
    let (out, calls) = go(&argv, answers);
    assert_eq!(out.unwrap(), "created mock \"Get orders\" (GET http://api.localhost/orders, 200, 7B json)");
    assert_eq!(payloads(&calls, "request.send"), [json!({"workspaceId": "w1", "id": "r1", "variables": {"a": "1"}})]);
    let entry = &payloads(&calls, "capture.buildMock")[0]["entry"];
    assert_eq!(entry["url"], "http://api.localhost/orders");
    assert_eq!(entry["resHeaders"], json!({"Content-Type": "application/json", "X-Total": "3"}));
    let created = &payloads(&calls, "entity.create")[0]["entity"];
    assert_eq!((created["name"].clone(), created["enabled"].clone()), (json!("Get orders"), json!(true)));
    assert_eq!(created["responseBody"], "[1,2,3]");
}

#[test]
fn a_mock_is_made_from_a_captured_entry_under_a_name() {
    let built = json!({"mock": {"name": "", "method": "POST", "urlPattern": "http://a/b", "responseStatus": 201,
                                "responseHeaders": {}, "responseBody": "{}"}});
    let answers: Vec<Answer> = vec![
        ("config.get", Ok(config())),
        ("capture.list", Ok(json!({"entries": [{"id": "xxxxc1"}]}))),
        ("capture.buildMock", Ok(built)),
        ("entity.create", Ok(json!({"id": "o9", "entity": {"id": "o9"}}))),
    ];
    let (out, calls) = go(&["mocks", "create", "--from-capture", "#c1", "--name", "Made", "--disabled"], answers);
    assert!(out.unwrap().starts_with("created mock \"Made\" (POST http://a/b, 201, 2B"));
    assert_eq!(payloads(&calls, "capture.buildMock"), [json!({"workspaceId": "w1", "id": "xxxxc1"})]);
    let created = &payloads(&calls, "entity.create")[0]["entity"];
    assert_eq!((created["name"].clone(), created["enabled"].clone()), (json!("Made"), json!(false)));
}
