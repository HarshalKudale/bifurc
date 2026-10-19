//! Token budgets of the default output (requirement 13): representative commands against the scripted engine, each
//! output counted and held to its budget. A regression here is a bug in the output, not in the budget.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::{
    capture, commands,
    error::{CliError, Result},
    extras, flows, requests,
    testing::{Answer, config, run, run_with_events, sub},
    usage::OVERVIEW,
};

/// ceil(chars / 3.5): a tokenizer-free estimate that errs high for JSON and prose alike.
fn tokens(text: &str) -> usize {
    (text.chars().count() * 2).div_ceil(7)
}

fn within(budget: usize, output: &str) {
    let used = tokens(output);
    println!("budget: {used}/{budget}");
    assert!(used <= budget, "{used} tokens, the budget is {budget}:\n{output}");
}

/// The output of a command that may have failed: a failure's text is what the caller reads.
fn text(result: Result<String>) -> String {
    match result {
        Ok(out) | Err(CliError::Failed(out)) => out,
        Err(other) => panic!("{other:?}"),
    }
}

fn listing(entities: Value) -> Result<Value> {
    Ok(json!({ "entities": entities }))
}

/// A JSON body of about `size` bytes.
fn json_body(size: usize) -> String {
    let items: Vec<Value> = (0..size / 40)
        .map(|i| json!({"id": i, "name": format!("item {i}"), "price": 9.5, "tags": ["a", "b"]}))
        .collect();
    serde_json::to_string(&json!({ "items": items })).unwrap()
}

fn send(status: u16, body: &str, tests: Value) -> String {
    let reply = json!({"status": status, "durationMs": 42, "headers": {"content-type": "application/json"},
                       "body": STANDARD.encode(body), "tests": tests});
    let answers: Vec<Answer> = vec![
        ("config.get", Ok(config())),
        (
            "entity.list:requests",
            listing(json!([{"id": "r1", "name": "Orders", "method": "GET", "url": "http://x/o"}])),
        ),
        ("entity.list", listing(json!([]))),
        ("request.send", Ok(reply)),
    ];
    let argv = ["requests", "send", "r1"];
    text(run(&argv, answers, |c| requests::requests(c, sub(&argv))).0)
}

#[test]
fn a_passing_send_is_one_line() {
    let tests = json!([{"name": "a", "passed": true}, {"name": "b", "passed": true}, {"name": "c", "passed": true}]);
    within(40, &send(200, &json_body(4000), tests));
}

#[test]
fn a_failing_send_is_a_summary_a_reason_and_a_short_body() {
    let tests = json!([{"name": "is 201", "passed": false, "error": "got 422"}, {"name": "b", "passed": true}]);
    within(150, &send(422, &json_body(3000), tests));
}

#[test]
fn fifty_saved_requests_list_in_a_few_folders() {
    let rows: Vec<Value> = (0..50)
        .map(|i| {
            let method = ["GET", "POST", "PUT"][i % 3];
            let mut row = json!({"id": format!("r{i}"), "name": format!("Request number {i}"),
                "method": method, "url": format!("{{{{baseUrl}}}}/api/v1/things/{i}")});
            if i % 4 != 0 {
                row["folderId"] = json!(format!("f{}", i % 4));
            }
            row
        })
        .collect();
    let mut snapshot = config();
    snapshot["requestFolders"] = json!([
        {"id": "f1", "name": "auth", "parentId": null},
        {"id": "f2", "name": "orders", "parentId": null},
        {"id": "f3", "name": "admin", "parentId": "f1"},
    ]);
    let answers: Vec<Answer> = vec![("config.get", Ok(snapshot)), ("entity.list:requests", listing(json!(rows)))];
    let argv = ["requests", "list", "--type", "rest"];
    within(800, &text(run(&argv, answers, |c| requests::requests(c, sub(&argv))).0));
}

#[test]
fn twenty_captured_entries_list_in_a_page() {
    let entries: Vec<Value> = (0..20)
        .map(|i| {
            json!({"id": format!("mgv{i:02}k3f{i:02}"), "timestamp": 1_792_332_131_000_i64 - i * 1000, "method": "GET",
                "url": format!("http://api.localhost/orders/{i}?debug=1"), "status": 200, "durationMs": 12 + i,
                "via": "proxy"})
        })
        .collect();
    let answers: Vec<Answer> =
        vec![("config.get", Ok(config())), ("capture.list", Ok(json!({"entries": entries, "matched": 20})))];
    let argv = ["capture", "list"];
    within(500, &text(run_with_events(&argv, answers, vec![], capture::list).0));
}

#[test]
fn a_flow_of_ten_passing_requests_prints_a_line_each() {
    let ids: Vec<String> = (1..=10).map(|i| format!("n{i}")).collect();
    let described: serde_json::Map<String, Value> = ids
        .iter()
        .map(|id| (id.clone(), json!({"request": {"name": id, "method": "GET", "address": format!("http://x/{id}")}})))
        .collect();
    let nodes: Vec<Value> = ids
        .iter()
        .map(|id| {
            json!({"id": id, "name": id, "status": "succeeded", "runs": 1, "durationMs": 12, "error": null,
                   "output": {"status": 200, "body": "{\"ok\":true}"}, "tests": []})
        })
        .collect();
    let mut events: Vec<Value> = nodes
        .iter()
        .map(|node| {
            json!({"event": "event.flow.progress", "seq": 1, "payload": {"workspaceId": "w1", "flowId": "f1",
                "runId": "run1", "status": "running", "nodes": [node], "edges": []}})
        })
        .collect();
    let report = json!({"flowName": "Checkout", "runId": "run1", "status": "succeeded", "error": "",
        "startedAt": 1000, "finishedAt": 1166, "nodeRuns": 10, "totalTests": 0, "passedTests": 0, "failedTests": 0,
        "nodes": nodes});
    events.push(json!({"event": "event.flow.finished", "seq": 2, "payload": {"workspaceId": "w1", "flowId": "f1",
        "runId": "run1", "status": "succeeded", "error": "", "report": report}}));
    let answers: Vec<Answer> = vec![
        ("config.get", Ok(config())),
        ("entity.list:flows", listing(json!([{"id": "f1", "name": "Checkout", "workspaceId": "w1"}]))),
        ("flow.validate", Ok(json!({"ok": true, "problems": [], "nodes": described}))),
        ("entity.list:environments", listing(json!([]))),
        ("subscribe", Ok(json!({}))),
        ("flow.run", Ok(json!({"ok": true, "runId": "run1"}))),
    ];
    let argv = ["flows", "run", "f1"];
    let out = run_with_events(&argv, answers, events, |c| flows::flows(c, sub(&argv), &mut std::io::sink()));
    within(250, &text(out.0));
}

#[test]
fn a_folder_run_of_ten_passing_requests_prints_a_line_each() {
    let results: Vec<Value> = (0..10)
        .map(|i| {
            json!({"requestName": format!("Request {i}"), "method": "GET", "url": format!("http://x/things/{i}"),
                "status": 200, "responseTime": 20 + i, "error": null, "tests": [{"name": "ok", "passed": true}],
                "responseBody": "{\"ok\":true}"})
        })
        .collect();
    let mut snapshot = config();
    snapshot["requestFolders"] = json!([{"id": "f1", "name": "auth", "parentId": null}]);
    let answers: Vec<Answer> = vec![
        ("config.get", Ok(snapshot)),
        ("entity.list:environments", listing(json!([]))),
        ("runner.start", Ok(json!({"ok": true, "total": 10}))),
        (
            "runner.status",
            Ok(json!({"running": false, "total": 10, "report": {"startedAt": 1000, "completedAt": 1812,
                "totalRequests": 10, "results": results}})),
        ),
    ];
    let out = run(&["runner", "run", "f1"], answers, |c| extras::run(c, "runner", "run").expect("a verb of extras"));
    within(250, &text(out.0));
}

#[test]
fn status_is_two_short_lines() {
    let mut snapshot = config();
    snapshot["forwardProxyPort"] = json!(8080);
    snapshot["environments"] = json!([{"id": "e2", "name": "Prod"}]);
    snapshot["mocks"] = json!([{"enabled": true}]);
    let answers: Vec<Answer> = vec![
        ("server.status", Ok(json!({"running": true, "port": 1}))),
        ("config.get", Ok(snapshot)),
        ("capture.list", Ok(json!({"matched": 143, "paused": false}))),
        ("cookies.list", Ok(json!({"domains": [{"domain": "a"}]}))),
    ];
    within(60, &text(run(&["status"], answers, commands::status).0));
}

#[test]
fn the_short_help_fits() {
    within(600, OVERVIEW);
}
