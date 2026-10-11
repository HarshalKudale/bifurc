//! What a request node and an MCP node do: run their saved request through the command a client would use,
//! and turn the answer into the node's output and the port it leaves by.

use std::{sync::Arc, time::Duration};

use serde_json::{Map, Value, json};

use super::{
    model::{Node, PORT_ERROR, PORT_SUCCESS},
    reference::render,
    run::{Live, Run, Token},
    streams::Subscription,
};
use crate::{
    env::load_merged_environment,
    rest::pipeline::{env_to_map, persist_script_variables},
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

/// The node's variables with their references filled in.
pub fn bindings(run: &Run, node: &Node, token: &Token) -> Result<Vec<(String, String)>, String> {
    let scope = run.scope(token);
    node.variables.iter().map(|pair| Ok((pair.name.clone(), render(&pair.value, &scope)?))).collect()
}

/// Run the node's after-execute script over what it produced, before that is handed on. The script reads and
/// replaces `lp.output`, and the variables it sets are the run's. It does not run for a node that failed, nor
/// when the saved request's own post script is the one chosen; a script that fails sends the node out by its
/// error port.
pub async fn after_execute(run: &Run, node: &Node, token: &Token, produced: Outcome) -> Outcome {
    let (port, output) = produced;
    if port == PORT_ERROR || node.runs_post_script() || node.after_execute.trim().is_empty() {
        return (port, output);
    }
    let nodes: Map<String, Value> = token.nodes.iter().map(|(id, value)| (id.clone(), (**value).clone())).collect();
    let data = json!({ "output": *output, "input": *token.input, "nodes": nodes, "flow": run.inputs });
    let result = run
        .scoped(Vec::new(), async {
            let before = env_to_map(load_merged_environment(&run.ws_id).as_ref());
            let result = execute_flow_script(&node.after_execute, data, before.clone()).await;
            persist_script_variables(&run.ws_id, &before, &result["envVars"]);
            result
        })
        .await;
    match opt_str(&result, "error") {
        Some(error) => {
            let kept = if output.is_object() { (*output).clone() } else { json!({ "output": *output }) };
            outcome(kept, Some(format!("The after-execute script failed: {error}")))
        }
        None => (port, Arc::new(result["output"].clone())),
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
    } else if node.fail_on_tests && failing > 0 {
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

async fn send_once(run: &Run, node: &Node, token: &Token) -> Outcome {
    let variables = match bindings(run, node, token) {
        Ok(variables) => variables,
        Err(error) => return failed(error),
    };
    let mut payload = json!({ "workspaceId": run.ws_id, "id": node.request_id });
    let command = match node.request_kind.as_str() {
        "requests" => {
            // A pre script is never run for a node, and the tests always are.
            payload["skipScripts"] = if node.runs_post_script() { json!(["pre"]) } else { json!(["pre", "post"]) };
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
pub async fn request(run: &Arc<Run>, node: &Node, token: &Token) -> Outcome {
    with_retries(node, || send_once(run, node, token)).await
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

async fn call_once(run: &Run, node: &Node, token: &Token) -> Outcome {
    let scope = run.scope(token);
    let prepared = bindings(run, node, token).and_then(|variables| {
        let mut arguments = Map::new();
        for pair in &node.arguments {
            arguments.insert(pair.name.clone(), json!(render(&pair.value, &scope)?));
        }
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
pub async fn mcp(run: &Arc<Run>, node: &Node, token: &Token) -> Outcome {
    with_retries(node, || call_once(run, node, token)).await
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
        assert_eq!(http_output(&node(json!({})), &answer(200, tests.clone())).0, PORT_SUCCESS);
        let (port, output) = http_output(&node(json!({ "failOnTests": true })), &answer(200, tests));
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
