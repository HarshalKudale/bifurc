//! `flows`: saved graphs of requests, listed, shown and run. A flow is made in the desktop app, so there is no
//! create, update or delete here.
//!
//! A run is the engine's: `flow.run` starts it and the engine reports its progress as events, which this
//! command prints until the run is finished.

use std::{collections::HashMap, io::Write};

use serde_json::{Value, json};

use crate::{
    commands::{Context, find_one, list_entities, need, unknown_verb, workspace_id},
    error::{CliError, Result},
    format::{js_template, json, table},
};

/// The events a run reports through.
const EVENTS: [&str; 2] = ["event.flow.progress", "event.flow.finished"];

/// The events a load test reports through.
const LOAD_EVENTS: [&str; 2] = ["event.flow.loadtest.progress", "event.flow.loadtest.finished"];

/// `flows list|show|run`. A run writes a line to `out` as each node finishes: a flow can take minutes, and
/// what it has done so far should not wait for its end.
pub fn flows(ctx: &mut Context, sub: Option<&str>, out: &mut dyn Write) -> Result<String> {
    let args = ctx.args;
    match sub.unwrap_or("list") {
        "list" => {
            ctx.accept(&[], 0)?;
            let workspace = workspace_id(ctx)?;
            let all = list_entities(ctx, &workspace, "flows")?;
            Ok(ctx.print(&Value::Array(all.clone()), || table(&all, &["id", "name"])))
        }
        "show" => {
            ctx.accept(&[], 1)?;
            let wanted = need(args.positional(2), "flow id or name")?;
            let workspace = workspace_id(ctx)?;
            let id = find_flow(ctx, &workspace, &wanted)?;
            let loaded = ctx.request("entity.load", json!({ "workspaceId": workspace, "kind": "flows", "id": id }))?;
            let flow = loaded.get("entity").cloned().unwrap_or(Value::Null);
            if ctx.json {
                return Ok(json(&flow));
            }
            let checked = ctx.request("flow.validate", json!({ "workspaceId": workspace, "id": id }))?;
            Ok(describe_flow(&flow, &checked))
        }
        verb @ ("run" | "loadtest") => {
            let load = verb == "loadtest";
            ctx.accept(&["input", "timeout", "max-events", "instances"][..if load { 4 } else { 3 }], 1)?;
            let wanted = need(args.positional(2), "flow id or name")?;
            let mut payload = json!({});
            if load {
                let instances = need(args.flag("instances"), "--instances")?;
                let instances: u64 = instances.parse().ok().filter(|n| (1..=10_000).contains(n)).ok_or_else(|| {
                    CliError::msg(format!("--instances must be a whole number from 1 to 10000, not \"{instances}\""))
                })?;
                payload["instances"] = json!(instances);
            }
            let mut inputs = serde_json::Map::new();
            for raw in args.flags("input") {
                let Some((name, value)) = raw.split_once('=').filter(|(name, _)| !name.is_empty()) else {
                    return Err(CliError::msg(format!("--input must be name=value, not \"{raw}\"")));
                };
                inputs.insert(name.to_string(), json!(value));
            }
            if !inputs.is_empty() {
                payload["inputs"] = Value::Object(inputs);
            }
            if let Some(seconds) = args.flag("timeout") {
                let seconds: f64 =
                    seconds.parse().ok().filter(|s: &f64| s.is_finite() && *s >= 0.0).ok_or_else(|| {
                        CliError::msg(format!("--timeout must be a number of seconds, not \"{seconds}\""))
                    })?;
                payload["timeoutMs"] = json!((seconds * 1000.0).round() as u64);
            }
            if let Some(events) = args.flag("max-events") {
                let events: u64 = events.parse().ok().filter(|n| *n >= 1).ok_or_else(|| {
                    CliError::msg(format!("--max-events must be a whole number of at least 1, not \"{events}\""))
                })?;
                payload["maxEvents"] = json!(events);
            }
            let workspace = workspace_id(ctx)?;
            let id = find_flow(ctx, &workspace, &wanted)?;
            payload["workspaceId"] = json!(workspace);
            payload["id"] = json!(id);
            if load { load_test(ctx, payload, out) } else { run_flow(ctx, payload, out) }
        }
        other => Err(unknown_verb("flows", other)),
    }
}

/// The id of the flow `wanted` names, by id or by name.
fn find_flow(ctx: &mut Context, workspace: &str, wanted: &str) -> Result<String> {
    let all = list_entities(ctx, workspace, "flows")?;
    Ok(js_template(find_one(&all, wanted, &["name"], "flow", "flows")?.get("id")))
}

/// What `flows show` prints: the inputs, the nodes with the request each runs, and what the engine finds wrong.
fn describe_flow(flow: &Value, checked: &Value) -> String {
    let mut lines = vec![format!("{}  ({})", js_template(flow.get("name")), js_template(flow.get("id")))];
    let inputs = flow.get("inputs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    if !inputs.is_empty() {
        lines.push(String::new());
        lines.push("Inputs".to_string());
        lines.extend(
            inputs.iter().map(|i| format!("  {} = {}", js_template(i.get("name")), js_template(i.get("value")))),
        );
    }
    lines.push(String::new());
    lines.push("Nodes".to_string());
    let nodes = flow.get("nodes").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let rows: Vec<Value> = nodes
        .iter()
        .map(|node| {
            let id = js_template(node.get("id"));
            let request = checked.pointer(&format!("/nodes/{id}/request")).filter(|r| r.is_object());
            let runs = request.map_or(String::new(), |r| {
                format!(
                    "{} {} {}",
                    js_template(r.get("name")),
                    js_template(r.get("method")),
                    js_template(r.get("address"))
                )
                .trim_end()
                .to_string()
            });
            json!({"id": id, "type": node.get("type"), "name": node.get("name"), "request": runs})
        })
        .collect();
    lines.push(table(&rows, &["id", "type", "name", "request"]));
    let problems = checked.get("problems").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    if !problems.is_empty() {
        lines.push(String::new());
        lines.push("Problems".to_string());
        lines.extend(problems.iter().map(problem_line));
    }
    lines.join("\n")
}

/// `  error n3: message`; the id is the node's or else the edge's.
fn problem_line(problem: &Value) -> String {
    let at = ["nodeId", "edgeId"]
        .iter()
        .filter_map(|key| problem.get(*key).and_then(Value::as_str).filter(|id| !id.is_empty()))
        .next()
        .unwrap_or("flow");
    format!("  {} {at}: {}", js_template(problem.get("severity")), js_template(problem.get("message")))
}

/// One line for a node that finished, `None` for one that has not.
fn node_line(node: &Value) -> Option<String> {
    let name = js_template(node.get("name"));
    let runs = node.get("runs").and_then(Value::as_u64).unwrap_or(0);
    let again = if runs > 1 { format!(" [run {runs}]") } else { String::new() };
    match node.get("status").and_then(Value::as_str)? {
        "succeeded" => {
            let took = node.get("durationMs").and_then(Value::as_u64).map(|ms| format!(" ({ms} ms)"));
            Some(format!("  ok   {name}{again}{}", took.unwrap_or_default()))
        }
        "failed" => {
            let error = node.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()).unwrap_or("failed");
            Some(format!("  FAIL {name}{again}: {error}"))
        }
        _ => None,
    }
}

/// The engine's answer to a start it refused, as the command's failure: the error and the flow's problems.
fn refused(started: &Value) -> Result<()> {
    if started.get("ok").and_then(Value::as_bool) != Some(false) {
        return Ok(());
    }
    let mut lines = vec![js_template(started.get("error"))];
    let problems = started.get("problems").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    lines.extend(problems.iter().map(|p| {
        let at = p.get("nodeId").and_then(Value::as_str).filter(|id| !id.is_empty()).unwrap_or("flow");
        format!("  {at}: {}", js_template(p.get("message")))
    }));
    Err(CliError::Failed(lines.join("\n")))
}

/// Start the load test `payload` asks for and follow it to its end; the summary is the answer.
fn load_test(ctx: &mut Context, payload: Value, out: &mut dyn Write) -> Result<String> {
    ctx.request("subscribe", json!({ "events": LOAD_EVENTS }))?;
    let started = ctx.request("flow.loadtest.start", payload.clone())?;
    refused(&started)?;
    let load_id = started.get("loadId").cloned().unwrap_or(Value::Null);
    let count = |from: &Value, key: &str| from.get(key).and_then(Value::as_u64).unwrap_or(0);

    let mut tenths = 0;
    let report = loop {
        let event = match ctx.engine()?.next_event() {
            Ok(Some(event)) => event,
            // The connection ended before the finish was seen: the engine still holds the report.
            Ok(None) | Err(_) => {
                let target = json!({ "workspaceId": payload["workspaceId"], "id": payload["id"] });
                let latest = ctx.request("flow.loadtest.status", target)?;
                if latest.get("loadId") == Some(&load_id) && latest.get("finishedAt").is_some_and(|at| !at.is_null()) {
                    break latest;
                }
                return Err(CliError::msg("the connection to the engine ended before the load test finished"));
            }
        };
        let told = event.get("payload").unwrap_or(&Value::Null);
        if told.get("loadId") != Some(&load_id) {
            continue;
        }
        match event.get("event").and_then(Value::as_str) {
            Some("event.flow.loadtest.progress") => {
                let (all, running) = (count(told, "instances").max(1), count(told, "running"));
                // A line for each tenth of the instances that is done, not one for each report.
                let done = (all - running.min(all)) * 10 / all;
                if done > tenths && !ctx.json {
                    let _ = writeln!(out, "  {} of {all} done, {} failed", all - running, count(told, "failed"));
                }
                tenths = tenths.max(done);
            }
            Some("event.flow.loadtest.finished") => break told.get("report").cloned().unwrap_or(Value::Null),
            _ => {}
        }
    };
    let clean = report.get("status").and_then(Value::as_str) == Some("finished") && count(&report, "failed") == 0;
    let summary = ctx.print(&report, || summarize_load(&report));
    if clean { Ok(summary) } else { Err(CliError::Failed(summary)) }
}

/// The end of a load test: the counts, how long an instance took, each node's times, and what failed how often.
fn summarize_load(report: &Value) -> String {
    let ms = |key: &str| js_template(report.get(key));
    let mut lines = vec![
        format!(
            "{}: {}  ({} instances, {} passed, {} failed, {:.1} per second)",
            js_template(report.get("flowName")),
            js_template(report.get("status")),
            ms("instances"),
            ms("passed"),
            ms("failed"),
            report.get("perSecond").and_then(Value::as_f64).unwrap_or(0.0),
        ),
        format!(
            "ms per instance: min {}, avg {}, p50 {}, p95 {}, p99 {}, max {}",
            ms("minMs"),
            ms("avgMs"),
            ms("p50Ms"),
            ms("p95Ms"),
            ms("p99Ms"),
            ms("maxMs")
        ),
    ];
    let list = |key: &str| report.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    lines.push(table(list("nodes"), &["name", "runs", "failed", "avgMs", "maxMs"]));
    // The same failure of many instances is one line.
    let mut failures: Vec<(String, u64)> = Vec::new();
    for row in list("rows") {
        let status = js_template(row.get("status"));
        let what = match row.get("error").and_then(Value::as_str) {
            Some(error) => error.to_string(),
            None if status != "succeeded" => format!("{status} at {}", js_template(row.get("at"))),
            None => continue,
        };
        match failures.iter_mut().find(|(seen, _)| *seen == what) {
            Some((_, times)) => *times += 1,
            None => failures.push((what, 1)),
        }
    }
    lines.extend(failures.iter().map(|(what, times)| format!("  FAIL x{times}: {what}")));
    lines.join("\n")
}

/// Start the run `payload` asks for and follow it to its end; the summary is the answer.
fn run_flow(ctx: &mut Context, payload: Value, out: &mut dyn Write) -> Result<String> {
    // Subscribe first: the run's events are not replayed to a session that comes late.
    ctx.request("subscribe", json!({ "events": EVENTS }))?;
    let started = ctx.request("flow.run", payload.clone())?;
    refused(&started)?;
    let run_id = js_template(started.get("runId"));
    let mine = |event: &Value| {
        let p = event.get("payload").unwrap_or(&Value::Null);
        ["workspaceId", "flowId", "runId"]
            .iter()
            .zip([&payload["workspaceId"], &payload["id"], &json!(run_id)])
            .all(|(key, want)| p.get(*key) == Some(want))
    };

    // A node that keeps running (after a stream) is reported again; it is printed when its runs or status changed.
    let mut printed: HashMap<String, (u64, String)> = HashMap::new();
    let mut show = |ctx: &Context, nodes: Option<&Value>, out: &mut dyn Write| {
        for node in nodes.and_then(Value::as_array).into_iter().flatten() {
            let seen = (
                node.get("runs").and_then(Value::as_u64).unwrap_or(0),
                node.get("status").and_then(Value::as_str).unwrap_or_default().to_string(),
            );
            let id = js_template(node.get("id"));
            let line = node_line(node).filter(|_| printed.get(&id) != Some(&seen));
            if let Some(line) = line {
                if !ctx.json {
                    let _ = writeln!(out, "{line}");
                }
                printed.insert(id, seen);
            }
        }
    };

    let report = loop {
        let event = match ctx.engine()?.next_event() {
            Ok(Some(event)) => event,
            // The connection ended before the finish was seen: the engine still holds the run's report.
            Ok(None) | Err(_) => {
                let latest =
                    ctx.request("flow.status", json!({ "workspaceId": payload["workspaceId"], "id": payload["id"] }))?;
                if latest.get("runId").and_then(Value::as_str) == Some(run_id.as_str()) {
                    break latest;
                }
                return Err(CliError::msg("the connection to the engine ended before the flow finished"));
            }
        };
        if !mine(&event) {
            continue;
        }
        match event.get("event").and_then(Value::as_str) {
            Some("event.flow.progress") => show(ctx, event.pointer("/payload/nodes"), out),
            Some("event.flow.finished") => break event.pointer("/payload/report").cloned().unwrap_or(Value::Null),
            _ => {}
        }
    };
    // The last progress report may have been left out to keep to the rate; the report has every node.
    show(ctx, report.get("nodes"), out);

    let succeeded = report.get("status").and_then(Value::as_str) == Some("succeeded");
    let failed_tests = report.get("failedTests").and_then(Value::as_u64).unwrap_or(0);
    let summary = ctx.print(&report, || summarize(&report));
    if succeeded && failed_tests == 0 { Ok(summary) } else { Err(CliError::Failed(summary)) }
}

/// The end of a run: its status, how much ran, the failed tests and the error.
fn summarize(report: &Value) -> String {
    let count = |key: &str| report.get(key).and_then(Value::as_u64).unwrap_or(0);
    let mut lines = vec![format!(
        "{}: {}  ({} node runs, {} tests passed, {} failed)",
        js_template(report.get("flowName")),
        js_template(report.get("status")),
        count("nodeRuns"),
        count("passedTests"),
        count("failedTests"),
    )];
    for node in report.get("nodes").and_then(Value::as_array).into_iter().flatten() {
        for test in node.get("tests").and_then(Value::as_array).into_iter().flatten() {
            if test.get("passed").and_then(Value::as_bool) == Some(false) {
                let error = test.get("error").and_then(Value::as_str).map(|e| format!(": {e}")).unwrap_or_default();
                lines.push(format!(
                    "  FAIL {} / {}{error}",
                    js_template(node.get("name")),
                    js_template(test.get("name"))
                ));
            }
        }
    }
    if let Some(error) = report.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()) {
        lines.push(format!("error: {error}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run, run_with_events, sub};

    fn listing(entities: Value) -> Result<Value> {
        Ok(json!({ "entities": entities }))
    }

    fn engine() -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            (
                "entity.list:flows",
                listing(json!([{"id": "f1", "name": "Checkout", "workspaceId": "w1"}, {"id": "f2", "name": "Other"}])),
            ),
            (
                "entity.load:flows",
                Ok(json!({"ok": true, "entity": {"id": "f1", "name": "Checkout",
                    "inputs": [{"name": "user", "value": "alice"}],
                    "nodes": [{"id": "n1", "name": "Start", "type": "start"},
                              {"id": "n2", "name": "Login", "type": "request", "requestKind": "requests", "requestId": "r1"}],
                    "edges": [{"id": "e1", "from": "n1", "port": "out", "to": "n2"}], "settings": {}}})),
            ),
            (
                "flow.validate",
                Ok(json!({"ok": true,
                    "problems": [{"nodeId": "n2", "edgeId": "", "severity": "warning", "message": "no tests"}],
                    "nodes": {"n1": {"ports": [], "request": null, "variables": []},
                              "n2": {"ports": [], "request": {"id": "r1", "kind": "requests", "name": "Login",
                                     "method": "POST", "address": "http://x/login"}, "variables": []}}})),
            ),
            ("subscribe", Ok(json!({"events": EVENTS}))),
            ("flow.run", Ok(json!({"ok": true, "runId": "run1"}))),
        ]
    }

    fn event(name: &str, run_id: &str, payload: Value) -> Value {
        let mut body = json!({"workspaceId": "w1", "flowId": "f1", "runId": run_id});
        body.as_object_mut().unwrap().extend(payload.as_object().unwrap().clone());
        json!({"event": name, "seq": 1, "payload": body})
    }

    fn progress(run_id: &str, nodes: Value) -> Value {
        event("event.flow.progress", run_id, json!({"status": "running", "nodes": nodes, "edges": []}))
    }

    fn finished(status: &str, error: &str, failed_tests: u64, runs: u64) -> Value {
        let report = json!({"flowName": "Checkout", "runId": "run1", "status": status, "error": error,
            "nodeRuns": 2, "totalTests": 2, "passedTests": 2 - failed_tests, "failedTests": failed_tests,
            "nodes": [
                {"id": "n2", "name": "Login", "status": "succeeded", "runs": runs, "durationMs": 35,
                 "tests": [{"name": "is 200", "passed": failed_tests == 0, "error": "got 503"}]}]});
        event("event.flow.finished", "run1", json!({"status": status, "error": error, "report": report}))
    }

    fn go(
        argv: &[&str],
        answers: Vec<Answer>,
        events: Vec<Value>,
    ) -> (Result<String>, Vec<String>, Vec<(String, Value)>) {
        // What a run writes as it goes is read back from the writer it was given, never from the process's own
        // output: the MCP server runs this in its own process, where that output is the protocol's.
        let mut written = Vec::new();
        let (out, calls) = run_with_events(argv, answers, events, |c| flows(c, sub(argv), &mut written));
        let lines = String::from_utf8(written).unwrap().lines().map(str::to_string).collect();
        (out, lines, calls)
    }

    #[test]
    fn list_is_a_table_and_the_entities_as_json() {
        let argv = ["flows", "list"];
        let (out, _) = run(&argv, engine(), |c| flows(c, sub(&argv), &mut std::io::sink()));
        assert_eq!(out.unwrap(), "ID  NAME\nf1  Checkout\nf2  Other");
        let argv = ["flows", "list", "--json"];
        let (out, _) = run(&argv, engine(), |c| flows(c, sub(&argv), &mut std::io::sink()));
        assert_eq!(serde_json::from_str::<Value>(&out.unwrap()).unwrap()[0]["id"], "f1");
    }

    #[test]
    fn show_names_the_request_each_node_runs_and_the_problems() {
        let argv = ["flows", "show", "checkout"];
        let (out, calls) = run(&argv, engine(), |c| flows(c, sub(&argv), &mut std::io::sink()));
        assert_eq!(
            out.unwrap(),
            "Checkout  (f1)\n\nInputs\n  user = alice\n\nNodes\n\
             ID  TYPE     NAME   REQUEST\n\
             n1  start    Start\n\
             n2  request  Login  Login POST http://x/login\n\n\
             Problems\n  warning n2: no tests"
        );
        assert_eq!(payloads(&calls, "flow.validate"), [json!({"workspaceId": "w1", "id": "f1"})]);

        let argv = ["flows", "show", "f1", "--json"];
        let (out, _) = run(&argv, engine(), |c| flows(c, sub(&argv), &mut std::io::sink()));
        assert_eq!(serde_json::from_str::<Value>(&out.unwrap()).unwrap()["nodes"][1]["requestId"], "r1");
    }

    #[test]
    fn run_prints_each_node_once_and_a_summary() {
        let events = vec![
            progress("other", json!([{"id": "n9", "name": "Not mine", "status": "failed", "runs": 1}])),
            progress("run1", json!([{"id": "n2", "name": "Login", "status": "running", "runs": 1}])),
            progress(
                "run1",
                json!([{"id": "n2", "name": "Login", "status": "succeeded", "runs": 1, "durationMs": 35}]),
            ),
            progress(
                "run1",
                json!([{"id": "n2", "name": "Login", "status": "succeeded", "runs": 1, "durationMs": 35}]),
            ),
            progress("run1", json!([{"id": "n2", "name": "Login", "status": "succeeded", "runs": 2, "durationMs": 9}])),
            finished("succeeded", "", 0, 2),
        ];
        let argv = [
            "flows",
            "run",
            "Checkout",
            "--input",
            "user=bob=1",
            "--input",
            "x=",
            "--timeout",
            "1.5",
            "--max-events",
            "10",
        ];
        let (out, lines, calls) = go(&argv, engine(), events);
        assert_eq!(lines, ["  ok   Login (35 ms)", "  ok   Login [run 2] (9 ms)"]);
        assert_eq!(out.unwrap(), "Checkout: succeeded  (2 node runs, 2 tests passed, 0 failed)");
        assert_eq!(
            payloads(&calls, "flow.run"),
            [json!({"workspaceId": "w1", "id": "f1", "inputs": {"user": "bob=1", "x": ""}, "timeoutMs": 1500,
                    "maxEvents": 10})]
        );
        assert_eq!(payloads(&calls, "subscribe"), [json!({"events": EVENTS})]);
    }

    #[test]
    fn a_failed_run_or_failed_test_fails_the_command_with_the_report() {
        let events = vec![
            progress(
                "run1",
                json!([{"id": "n2", "name": "Get user", "status": "failed", "runs": 1, "error": "HTTP 503"}]),
            ),
            finished("failed", "Get user: HTTP 503", 1, 1),
        ];
        let argv = ["flows", "run", "f1"];
        let (out, lines, _) = go(&argv, engine(), events);
        assert_eq!(lines, ["  FAIL Get user: HTTP 503", "  ok   Login (35 ms)"]);
        let CliError::Failed(report) = out.unwrap_err() else { panic!("a failed run is a failed command") };
        assert_eq!(
            report,
            "Checkout: failed  (2 node runs, 1 tests passed, 1 failed)\n  FAIL Login / is 200: got 503\nerror: Get user: HTTP 503"
        );

        // A passed run with a failed test still exits 1.
        let (out, _, _) = go(&argv, engine(), vec![finished("succeeded", "", 1, 1)]);
        assert!(matches!(out.unwrap_err(), CliError::Failed(_)));
    }

    #[test]
    fn run_with_json_prints_nothing_while_running_and_the_report_at_the_end() {
        let argv = ["flows", "run", "f1", "--json"];
        let events = vec![
            progress("run1", json!([{"id": "n2", "name": "Login", "status": "succeeded", "runs": 1}])),
            finished("succeeded", "", 0, 1),
        ];
        let (out, lines, _) = go(&argv, engine(), events);
        assert!(lines.is_empty());
        assert_eq!(serde_json::from_str::<Value>(&out.unwrap()).unwrap()["runId"], "run1");
    }

    #[test]
    fn a_refused_run_prints_the_error_and_the_problems() {
        // The first answer for a command wins, so the refusal replaces the engine's.
        let mut answers = engine();
        answers.retain(|(name, _)| *name != "flow.run");
        answers.push((
            "flow.run",
            Ok(json!({"ok": false, "error": "The flow has errors.",
                "problems": [{"nodeId": "n2", "edgeId": "", "severity": "error", "message": "no request"}]})),
        ));
        let argv = ["flows", "run", "f1"];
        let (out, _, _) = go(&argv, answers, vec![]);
        assert_eq!(out.unwrap_err(), CliError::Failed("The flow has errors.\n  n2: no request".into()));
    }

    #[test]
    fn a_lost_connection_falls_back_to_the_latest_report() {
        let mut answers = engine();
        answers.push((
            "flow.status",
            Ok(json!({"flowName": "Checkout", "runId": "run1", "status": "succeeded", "nodeRuns": 1, "passedTests": 0,
                      "failedTests": 0, "nodes": [{"id": "n2", "name": "Login", "status": "succeeded", "runs": 1}]})),
        ));
        let argv = ["flows", "run", "f1"];
        let (out, lines, _) = go(&argv, answers, vec![]);
        assert_eq!(lines, ["  ok   Login"]);
        assert_eq!(out.unwrap(), "Checkout: succeeded  (1 node runs, 0 tests passed, 0 failed)");

        // The latest report of another run is no answer.
        let mut answers = engine();
        answers.push(("flow.status", Ok(Value::Null)));
        let (out, _, _) = go(&argv, answers, vec![]);
        assert_eq!(out.unwrap_err(), CliError::msg("the connection to the engine ended before the flow finished"));
    }

    #[test]
    fn bad_command_lines_are_refused_before_the_engine_is_asked() {
        for (argv, message) in [
            (vec!["flows", "run", "f1", "--input", "oops"], "--input must be name=value, not \"oops\""),
            (vec!["flows", "run", "f1", "--timeout", "soon"], "--timeout must be a number of seconds, not \"soon\""),
            (
                vec!["flows", "run", "f1", "--max-events", "0"],
                "--max-events must be a whole number of at least 1, not \"0\"",
            ),
            (vec!["flows", "run"], "missing flow id or name"),
            (
                vec!["flows", "list", "--wait", "1"],
                "unknown option --wait for \"flows list\" (see: bifurc flows --help)",
            ),
            (
                vec!["flows", "show", "a", "b"],
                "unexpected argument \"b\" for \"flows show\" (see: bifurc flows --help)",
            ),
        ] {
            let (out, lines, calls) = go(&argv, engine(), vec![]);
            assert_eq!(out.unwrap_err(), CliError::msg(message), "{argv:?}");
            assert!(lines.is_empty() && calls.is_empty(), "{argv:?}");
        }
    }

    #[test]
    fn a_load_test_prints_how_far_it_is_and_what_failed_how_often() {
        let told = |name: &str, load_id: &str, payload: Value| {
            let mut body = json!({"workspaceId": "w1", "flowId": "f1", "loadId": load_id});
            body.as_object_mut().unwrap().extend(payload.as_object().unwrap().clone());
            json!({"event": name, "seq": 1, "payload": body})
        };
        let progress = |load_id: &str, running: u64, failed: u64| {
            told("event.flow.loadtest.progress", load_id, json!({"instances": 20, "running": running, "failed": failed}))
        };
        let finished = |failed: u64| {
            let row = |status: &str, error: Value| json!({"status": status, "at": "Login", "error": error, "failedTests": 0});
            let mut rows = vec![row("succeeded", Value::Null); (20 - failed) as usize];
            rows.extend(vec![row("failed", json!("Login: HTTP 503")); failed as usize]);
            let report = json!({"flowName": "Checkout", "loadId": "l1", "status": "finished", "instances": 20,
                "passed": 20 - failed, "failed": failed, "perSecond": 12.34, "minMs": 5, "avgMs": 8, "p50Ms": 7,
                "p95Ms": 11, "p99Ms": 12, "maxMs": 12, "rows": rows,
                "nodes": [{"id": "n2", "name": "Login", "runs": 20, "failed": failed, "avgMs": 6, "maxMs": 9}]});
            told("event.flow.loadtest.finished", "l1", json!({"report": report}))
        };
        let mut answers = engine();
        answers.push(("flow.loadtest.start", Ok(json!({"ok": true, "loadId": "l1", "instances": 20}))));

        let events = vec![progress("other", 0, 20), progress("l1", 19, 0), progress("l1", 10, 0), progress("l1", 9, 1), finished(0)];
        let argv = ["flows", "loadtest", "Checkout", "--instances", "20", "--input", "user=bob", "--timeout", "2"];
        let (out, lines, calls) = go(&argv, answers.clone(), events);
        assert_eq!(lines, ["  10 of 20 done, 0 failed"]);
        assert_eq!(
            out.unwrap(),
            "Checkout: finished  (20 instances, 20 passed, 0 failed, 12.3 per second)\n\
             ms per instance: min 5, avg 8, p50 7, p95 11, p99 12, max 12\n\
             NAME   RUNS  FAILED  AVGMS  MAXMS\n\
             Login  20    0       6      9"
        );
        assert_eq!(
            payloads(&calls, "flow.loadtest.start"),
            [json!({"instances": 20, "inputs": {"user": "bob"}, "timeoutMs": 2000, "workspaceId": "w1", "id": "f1"})]
        );

        // An instance that failed fails the command, and the same failure is told once.
        let argv = ["flows", "loadtest", "f1", "--instances", "20"];
        let (out, _, _) = go(&argv, answers.clone(), vec![finished(3)]);
        let CliError::Failed(report) = out.unwrap_err() else { panic!("a failed instance is a failed command") };
        assert!(report.ends_with("\n  FAIL x3: Login: HTTP 503"), "{report}");

        for (argv, message) in [
            (vec!["flows", "loadtest", "f1"], "missing --instances"),
            (
                vec!["flows", "loadtest", "f1", "--instances", "0"],
                "--instances must be a whole number from 1 to 10000, not \"0\"",
            ),
            (
                vec!["flows", "run", "f1", "--instances", "2"],
                "unknown option --instances for \"flows run\" (see: bifurc flows --help)",
            ),
        ] {
            let (out, _, calls) = go(&argv, answers.clone(), vec![]);
            assert_eq!(out.unwrap_err(), CliError::msg(message), "{argv:?}");
            assert!(calls.is_empty(), "{argv:?}");
        }
    }

    #[test]
    fn a_flow_that_is_not_there_is_named() {
        let argv = ["flows", "run", "nope"];
        let (out, _, calls) = go(&argv, engine(), vec![]);
        assert_eq!(out.unwrap_err(), CliError::msg("no flow \"nope\" (see: bifurc flows list)"));
        assert!(payloads(&calls, "flow.run").is_empty());
    }
}
