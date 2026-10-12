//! What a request node and an MCP node do: run their saved request through the command a client would use,
//! and turn the answer into the node's output and the port it leaves by.

use std::{sync::Arc, time::Duration};

use serde_json::{Map, Value, json};

use super::{
    model::{Node, PORT_ERROR, PORT_OUT, PORT_SUCCESS},
    reference::{render, text_of},
    run::{Live, Run, Token, Wired},
    streams::Subscription,
};
use crate::{
    scripting::execute_flow_script,
    utils::{
        json::{array_of, opt_str, str_of},
        replay::decode_base64_lenient,
    },
};

/// How long an MCP server may take to accept a connection.
const MCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// The port a node leaves by, and what it hands on.
pub type Outcome = (String, Arc<Value>);

fn failed(error: impl Into<String>) -> Outcome {
    (PORT_ERROR.to_string(), Arc::new(json!({ "ok": false, "error": error.into() })))
}

/// `output` leaving by the success port, or by the error port when `error` says why.
fn outcome(mut output: Value, error: Option<String>) -> Outcome {
    output["ok"] = json!(error.is_none());
    match error {
        Some(error) => {
            output["error"] = json!(error);
            (PORT_ERROR.to_string(), Arc::new(output))
        }
        None => (PORT_SUCCESS.to_string(), Arc::new(output)),
    }
}

/// What is wired into the inputs whose names start with `prefix` (`var:`, `arg:`), by the rest of the name.
fn named<'a>(wired: &'a Wired, prefix: &'a str) -> impl Iterator<Item = (String, Value)> + 'a {
    wired.iter().filter_map(move |(input, value)| Some((input.strip_prefix(prefix)?.to_string(), value.clone())))
}

/// The values for the request's `{{variables}}`: what was typed in for the node, its references filled in,
/// then what is wired, which wins.
pub fn bindings(run: &Run, node: &Node, token: &Token, wired: &Wired) -> Result<Vec<(String, String)>, String> {
    let scope = run.scope(token);
    let mut variables: Vec<(String, String)> = node
        .variables
        .iter()
        .map(|pair| Ok((pair.name.clone(), render(&pair.value, &scope)?)))
        .collect::<Result<_, String>>()?;
    variables.extend(named(wired, "var:").map(|(name, value)| (name, text_of(&value))));
    Ok(variables)
}

/// Work out an evaluate node's expression: JavaScript over `input`, what the node was handed, and `vars`,
/// its variables by name (what is wired into each, else what was typed for it). What the expression comes to
/// is what the node hands on; one that throws sends it out by its error port.
pub async fn evaluate(run: &Run, node: &Node, token: &Token, wired: &Wired) -> Outcome {
    let scope = run.scope(token);
    let mut vars = Map::new();
    for pair in &node.variables {
        match render(&pair.value, &scope) {
            Ok(text) => vars.insert(pair.name.clone(), json!(text)),
            Err(error) => return failed(error),
        };
    }
    vars.extend(named(wired, "var:"));
    let nodes: Map<String, Value> = token.nodes.iter().map(|(id, value)| (id.clone(), (**value).clone())).collect();
    let data = json!({ "input": *token.input, "vars": vars, "nodes": nodes, "flow": run.inputs });
    let script =
        format!("lp.output = (function (input, vars) {{ return (\n{}\n); }})(lp.input, lp.vars);", node.expression);
    let result = execute_flow_script(&script, data, json!({})).await;
    match opt_str(&result, "error") {
        Some(error) => failed(format!("The expression failed: {error}")),
        None => (PORT_OUT.to_string(), Arc::new(result["output"].clone())),
    }
}

/// Try `once` until it succeeds or the node's retries are used up.
async fn with_retries<F, Fut>(node: &Node, once: F) -> Outcome
where
    F: Fn() -> Fut,
    Fut: Future<Output = Outcome>,
{
    let mut tried = 0;
    loop {
        let result = once().await;
        if result.0 != PORT_ERROR || tried >= node.retries {
            return result;
        }
        tried += 1;
        tokio::time::sleep(Duration::from_millis(node.retry_delay_ms)).await;
    }
}

/// The answer of an HTTP exchange as a node's output. A REST answer's body arrives as base64.
fn http_output(node: &Node, answer: &Value) -> Outcome {
    let status = answer["status"].as_u64().unwrap_or(0);
    let body = match node.request_kind.as_str() {
        "requests" => String::from_utf8_lossy(&decode_base64_lenient(str_of(answer, "body"))).into_owned(),
        _ => str_of(answer, "body").to_string(),
    };
    let tests = array_of(answer, "tests");
    let failing = tests.iter().filter(|test| test["passed"] != true).count();
    let script_error = ["preScriptError", "postScriptError"].iter().find_map(|key| opt_str(answer, key));
    let error = if let Some(error) = script_error {
        Some(format!("A script of the request failed: {error}"))
    } else if node.fail_on_status && !(200..300).contains(&status) {
        Some(format!("HTTP {status}"))
    } else if !node.ignore_test_errors && failing > 0 {
        Some(format!("{failing} of {} tests failed", tests.len()))
    } else {
        None
    };
    let output = json!({
        "status": status, "headers": answer["headers"], "body": body, "durationMs": answer["durationMs"], "tests": tests,
    });
    outcome(output, error)
}

fn grpc_output(node: &Node, answer: &Value) -> Outcome {
    let error = (answer["ok"] != true && node.fail_on_status)
        .then(|| opt_str(answer, "error").unwrap_or("The call failed").to_string());
    let output = json!({
        "status": answer["status"], "statusMessage": answer["statusMessage"], "message": answer["responses"][0],
        "responses": answer["responses"], "metadata": answer["metadata"], "durationMs": answer["durationMs"],
    });
    outcome(output, error)
}

async fn send_once(run: &Run, node: &Node, token: &Token, wired: &Wired) -> Outcome {
    let variables = match bindings(run, node, token, wired) {
        Ok(variables) => variables,
        Err(error) => return failed(error),
    };
    let mut payload = json!({ "workspaceId": run.ws_id, "id": node.request_id });
    let command = match node.request_kind.as_str() {
        "requests" => {
            let scripts: &[&str] = if node.execute_scripts { &[] } else { &["pre", "post"] };
            let tests: &[&str] = if node.execute_tests { &[] } else { &["test"] };
            payload["skipScripts"] = json!([scripts, tests].concat());
            "request.send"
        }
        "graphqlRequests" => "graphql.send",
        "soapRequests" => "soap.send",
        "grpcRequests" => "grpc.send",
        other => return failed(format!("A request of the kind \"{other}\" cannot be sent by a flow.")),
    };
    match run.invoke(command, payload, variables).await {
        Err(error) => failed(error.message),
        Ok(answer) if command == "grpc.send" => grpc_output(node, &answer),
        Ok(answer) => http_output(node, &answer),
    }
}

/// Send the node's saved request.
pub async fn request(run: &Arc<Run>, node: &Node, token: &Token, wired: &Wired) -> Outcome {
    with_retries(node, || send_once(run, node, token, wired)).await
}

/// The id of the connection a run keeps to the MCP server of one saved request.
fn mcp_connection_id(run: &Run, node: &Node) -> String {
    format!("flow:{}:{}", run.flow.id, node.request_id)
}

/// Open the run's connection to the node's server, unless an earlier node already did.
async fn mcp_connect(run: &Run, node: &Node, variables: &[(String, String)]) -> Result<(), String> {
    let connection_id = mcp_connection_id(run, node);
    // Held across the connect, so two nodes that reach the same server together open it once.
    let mut connected = run.mcp.lock().await;
    if connected.contains(&connection_id) {
        return Ok(());
    }
    let mut events = Subscription::new(&run.ws_id, "connectionId", &connection_id, &["mcp.status"]);
    let payload = json!({ "workspaceId": run.ws_id, "connectionId": connection_id, "id": node.request_id });
    let answer = run.invoke("mcp.connect", payload, variables.to_vec()).await.map_err(|error| error.message)?;
    if answer["ok"] != true {
        return Err(opt_str(&answer, "error").unwrap_or("The connection failed").to_string());
    }
    run.keep(Live { disconnect: "mcp.disconnect", id_field: "connectionId", id: connection_id.clone() });
    let opened = async {
        while let Some((_, payload)) = events.next().await {
            match str_of(&payload, "status") {
                "connected" => return Ok(()),
                "error" | "disconnected" => {
                    return Err(opt_str(&payload, "error").unwrap_or("The server closed the connection").to_string());
                }
                _ => {}
            }
        }
        Err("The connection ended".to_string())
    };
    tokio::time::timeout(MCP_CONNECT_TIMEOUT, opened)
        .await
        .unwrap_or_else(|_| Err("The MCP server did not answer in time".to_string()))?;
    connected.insert(connection_id);
    Ok(())
}

async fn call_once(run: &Run, node: &Node, token: &Token, wired: &Wired) -> Outcome {
    let scope = run.scope(token);
    let prepared = bindings(run, node, token, wired).and_then(|variables| {
        let mut arguments = Map::new();
        // An argument nothing was typed for is left to the server's default.
        for pair in node.arguments.iter().filter(|pair| !pair.value.is_empty()) {
            arguments.insert(pair.name.clone(), json!(render(&pair.value, &scope)?));
        }
        // What is wired wins over what was typed, and keeps its type.
        arguments.extend(named(wired, "arg:"));
        Ok((variables, render(&node.target, &scope)?, arguments))
    });
    let (variables, target, arguments) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => return failed(error),
    };
    if let Err(error) = mcp_connect(run, node, &variables).await {
        return failed(error);
    }

    let connection_id = mcp_connection_id(run, node);
    let mut events = Subscription::new(&run.ws_id, "connectionId", &connection_id, &["mcp.status", "mcp.message"]);
    let payload = json!({
        "workspaceId": run.ws_id, "connectionId": connection_id, "type": node.call, "name": target, "arguments": arguments,
    });
    let call_id = match run.invoke("mcp.call", payload, variables).await {
        Err(error) => return failed(error.message),
        Ok(answer) if answer["ok"] != true => return failed(opt_str(&answer, "error").unwrap_or("The call failed")),
        Ok(answer) => str_of(&answer, "callId").to_string(),
    };
    while let Some((event, payload)) = events.next().await {
        if event == "mcp.status" {
            if matches!(str_of(&payload, "status"), "error" | "disconnected") {
                run.mcp.lock().await.remove(&connection_id);
                return failed(opt_str(&payload, "error").unwrap_or("The server closed the connection"));
            }
            continue;
        }
        let message = &payload["message"];
        if str_of(message, "id") != call_id || str_of(message, "status") == "pending" {
            continue;
        }
        let raw = str_of(message, "data");
        let output = json!({
            "text": message["text"], "data": serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!(raw)),
            "durationMs": message["durationMs"],
        });
        let error = (message["status"] != "ok").then(|| str_of(message, "text").to_string());
        return outcome(output, error);
    }
    failed("The connection ended before the server answered")
}

/// Call the node's tool, prompt or resource on its MCP server.
pub async fn mcp(run: &Arc<Run>, node: &Node, token: &Token, wired: &Wired) -> Outcome {
    with_retries(node, || call_once(run, node, token, wired)).await
}

#[cfg(test)]
mod tests {
    use base64::Engine;

    use super::*;
    use crate::flows::model::Flow;

    fn node(extra: Value) -> Node {
        let mut node = json!({ "id": "n", "type": "request", "requestKind": "requests", "requestId": "r" });
        node.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        Flow::parse(&json!({ "nodes": [node] })).unwrap().nodes.remove(0)
    }

    fn answer(status: u64, extra: Value) -> Value {
        let body = base64::engine::general_purpose::STANDARD.encode(r#"{"id":7}"#);
        let mut answer = json!({ "status": status, "headers": { "x": "1" }, "body": body, "durationMs": 3 });
        answer.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        answer
    }

    #[test]
    fn an_answer_leaves_by_the_port_its_node_asks_for() {
        let (port, output) = http_output(&node(json!({})), &answer(200, json!({})));
        assert_eq!(port, PORT_SUCCESS);
        assert_eq!(
            *output,
            json!({
                "status": 200, "headers": { "x": "1" }, "body": r#"{"id":7}"#, "durationMs": 3, "tests": [], "ok": true,
            })
        );

        let (port, output) = http_output(&node(json!({})), &answer(404, json!({})));
        assert_eq!((port.as_str(), &output["error"], &output["ok"]), (PORT_ERROR, &json!("HTTP 404"), &json!(false)));
        assert_eq!(http_output(&node(json!({ "failOnStatus": false })), &answer(404, json!({}))).0, PORT_SUCCESS);

        let tests = json!({ "tests": [{ "name": "a", "passed": true }, { "name": "b", "passed": false }] });
        let lenient = node(json!({ "ignoreTestErrors": true }));
        assert_eq!(http_output(&lenient, &answer(200, tests.clone())).0, PORT_SUCCESS);
        let (port, output) = http_output(&node(json!({})), &answer(200, tests));
        assert_eq!((port.as_str(), &output["error"]), (PORT_ERROR, &json!("1 of 2 tests failed")));

        let broken = http_output(&node(json!({})), &answer(200, json!({ "preScriptError": "boom" })));
        assert_eq!(broken.0, PORT_ERROR);
    }

    #[test]
    fn a_grpc_answer_fails_by_its_status() {
        let failed = json!({ "ok": false, "status": 14, "statusMessage": "down", "error": "UNAVAILABLE: down", "responses": [] });
        let (port, output) = grpc_output(&node(json!({})), &failed);
        assert_eq!((port.as_str(), &output["error"]), (PORT_ERROR, &json!("UNAVAILABLE: down")));
        assert_eq!(grpc_output(&node(json!({ "failOnStatus": false })), &failed).0, PORT_SUCCESS);
        let fine = json!({ "ok": true, "status": 0, "responses": [{ "name": "Ada" }] });
        let (port, output) = grpc_output(&node(json!({})), &fine);
        assert_eq!((port.as_str(), &output["message"]["name"]), (PORT_SUCCESS, &json!("Ada")));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failing_node_is_tried_again_as_often_as_it_allows() {
        let tried = std::sync::atomic::AtomicU32::new(0);
        let attempt = || async {
            let count = tried.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if count < 2 { failed("down") } else { outcome(json!({}), None) }
        };
        let patient = node(json!({ "retries": 3, "retryDelayMs": 1000 }));
        assert_eq!(with_retries(&patient, attempt).await.0, PORT_SUCCESS);
        assert_eq!(tried.load(std::sync::atomic::Ordering::Relaxed), 3);

        tried.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(with_retries(&node(json!({ "retries": 1 })), attempt).await.0, PORT_ERROR);
        assert_eq!(tried.load(std::sync::atomic::Ordering::Relaxed), 2);
    }
}
