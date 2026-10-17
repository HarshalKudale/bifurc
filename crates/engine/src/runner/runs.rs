//! Collection runs: run a folder's requests in order through `request.send`, and keep the state of each run
//! so a client that attaches mid-run (or after it) can read it back with `runner.status`.
//!
//! A run's snapshot is the `runner.status` result itself, kept as JSON and updated in place.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
    time::Duration,
};

use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::{
    cookies,
    env::send_scoped,
    kernel::{CommandContext, bus},
    protocol::{EngineError, Result},
    runtime::failed,
    utils::{
        id::{generate_id, now_ms},
        json::{array_of, str_of},
        replay::decode_base64_lenient,
    },
    workspace::tree::{self, Scope},
};

type Run = Arc<Mutex<Value>>;

static RUNS: LazyLock<Mutex<HashMap<String, Run>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn key_of(ws_id: &str, folder_id: &str) -> String {
    format!("{ws_id}\u{0}{folder_id}")
}

fn find_run(ws_id: &str, folder_id: &str) -> Option<Run> {
    RUNS.lock().get(&key_of(ws_id, folder_id)).cloned()
}

/// The run's current state, or `None` if it never started.
pub fn get_run(ws_id: &str, folder_id: &str) -> Option<Value> {
    find_run(ws_id, folder_id).map(|run| run.lock().clone())
}

/// Ask the run to stop at its next iteration boundary.
pub fn cancel_run(ws_id: &str, folder_id: &str) -> Value {
    match find_run(ws_id, folder_id) {
        Some(run) => {
            run.lock()["cancelled"] = json!(true);
            json!({ "ok": true })
        }
        None => failed(
            &EngineError::not_found(format!("No run of folder \"{folder_id}\" in workspace \"{ws_id}\"."))
                .keyed("runner.no_run", json!({ "id": folder_id, "workspace": ws_id })),
        ),
    }
}

/// How much of a response body a result row keeps.
const BODY_LIMIT: usize = 16 * 1024;

fn truncated(body: &Value) -> Value {
    match body.as_str() {
        Some(text) if text.len() > BODY_LIMIT => {
            let mut end = BODY_LIMIT;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            json!(format!(
                "{}
... truncated, {} bytes in all",
                &text[..end],
                text.len()
            ))
        }
        _ => body.clone(),
    }
}

/// A request to run: `{ id, name, method, url, scripts }`.
fn request_ref(entity: &Value) -> Value {
    let text = |key: &str, default: &str| match entity.get(key) {
        None | Some(Value::Null) => default.to_string(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    };
    let script = |key: &str| entity.get(key).and_then(Value::as_str).unwrap_or_default();
    json!({
        "id": text("id", ""), "name": text("name", ""), "method": text("method", "GET"), "url": text("url", ""),
        "scripts": { "pre": script("preScript"), "post": script("postScript"), "test": script("testScript") },
    })
}

async fn load_runner_config(ctx: &CommandContext, ws_id: &str, folder_id: &str) -> Option<Value> {
    let payload = json!({ "workspaceId": ws_id, "folderId": folder_id });
    ctx.registry.invoke("runner.loadConfig", payload, ctx.clone()).await.ok().filter(Value::is_object)
}

/// The folder's requests in the order its runner config gives; requests the config does not list come last.
async fn load_ordered_requests(ctx: &CommandContext, ws_id: &str, folder_id: &str) -> Result<Vec<Value>> {
    let listed =
        ctx.registry.invoke("entity.list", json!({ "workspaceId": ws_id, "kind": "requests" }), ctx.clone()).await?;
    let mut in_folder: Vec<&Value> = array_of(&listed, "entities")
        .iter()
        .filter(|entity| match entity.get("folderId") {
            None | Some(Value::Null) => false,
            Some(id) => id.as_str() == Some(folder_id),
        })
        .collect();
    let config = load_runner_config(ctx, ws_id, folder_id).await;
    let order: Vec<String> = config
        .as_ref()
        .map(|config| {
            array_of(config, "requestOrder")
                .iter()
                .map(|id| id.as_str().map(str::to_string).unwrap_or_else(|| id.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let rank = |entity: &Value| {
        let id = request_ref(entity)["id"].as_str().unwrap_or_default().to_string();
        order.iter().rposition(|candidate| *candidate == id).unwrap_or(9999)
    };
    in_folder.sort_by_key(|entity| rank(entity));
    Ok(in_folder.into_iter().map(request_ref).collect())
}

/// The outcome of one request: the result row the report and the progress event carry.
async fn run_one(ctx: &CommandContext, ws_id: &str, request: &Value) -> Value {
    let sent = ctx
        .registry
        .invoke("request.send", json!({ "workspaceId": ws_id, "id": str_of(request, "id") }), ctx.clone())
        .await;
    match sent {
        Ok(sent) => {
            let mut result = json!({
                "requestId": request["id"],
                "requestName": request["name"],
                "method": sent["request"]["method"],
                "url": sent["request"]["url"],
                "status": sent["status"],
                "responseTime": sent["durationMs"],
                "tests": sent.get("tests").filter(|v| v.is_array()).cloned().unwrap_or_else(|| json!([])),
                "testLogs": sent.get("testLogs").filter(|v| v.is_array()).cloned().unwrap_or_else(|| json!([])),
                "scripts": request["scripts"],
                "requestHeaders": sent["request"]["headers"],
                "requestBody": truncated(&sent["request"]["body"]),
                "responseHeaders": sent["headers"],
                "responseBody": truncated(&json!(String::from_utf8_lossy(&decode_base64_lenient(str_of(&sent, "body"))))),
            });
            for key in ["preScriptError", "postScriptError"] {
                if let Some(error) = sent.get(key) {
                    result[key] = error.clone();
                }
            }
            result
        }
        Err(error) => {
            let mut result = json!({
            "requestId": request["id"],
            "requestName": request["name"],
            "method": request["method"],
            "url": request["url"],
            "status": null,
            "responseTime": 0,
            "error": error.message,
            "tests": [],
            "testLogs": [],
            "scripts": request["scripts"],
            });
            // Beside the English `error`, so a client can show it in the user's language.
            if let (Some(key), Some(params)) = (error.key(), error.params()) {
                result["errorKey"] = json!(key);
                result["errorParams"] = params.clone();
            }
            result
        }
    }
}

/// Whether a request of the run failed: it could not be sent, a script threw, or a test did not pass.
fn is_failed(result: &Value) -> bool {
    let present = |key: &str| result.get(key).is_some_and(|value| !value.is_null());
    present("error")
        || present("preScriptError")
        || present("postScriptError")
        || array_of(result, "tests").iter().any(|test| test["passed"] != true)
}

/// The run's report from its results. The folder's name titles an exported report.
fn build_report(ws_id: &str, folder_id: &str, started_at: i64, results: Vec<Value>) -> Value {
    let tests: Vec<&Value> = results.iter().flat_map(|result| array_of(result, "tests")).collect();
    let passed = tests.iter().filter(|test| test.get("passed").and_then(Value::as_bool) == Some(true)).count();
    json!({
        "folderId": folder_id,
        "folderName": tree::folder(ws_id, Scope::Requests, folder_id).map(|folder| folder.name),
        "startedAt": started_at,
        "completedAt": now_ms(),
        "totalRequests": results.len(),
        "totalTests": tests.len(),
        "passedTests": passed,
        "failedTests": tests.len() - passed,
        "results": results,
    })
}

async fn run_collection(
    ctx: CommandContext,
    run: Run,
    ws_id: String,
    folder_id: String,
    requests: Vec<Value>,
    delay_ms: Option<u64>,
    bail: bool,
) -> Value {
    let started_at = now_ms();
    let mut results: Vec<Value> = Vec::new();
    for (index, request) in requests.iter().enumerate() {
        if run.lock()["cancelled"] == json!(true) {
            break;
        }
        if let (true, Some(delay)) = (index > 0, delay_ms) {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        let result = run_one(&ctx, &ws_id, request).await;
        results.push(result.clone());
        let total = {
            let mut snapshot = run.lock();
            snapshot["results"].as_array_mut().expect("results is an array").push(result.clone());
            snapshot["completed"] = json!(results.len());
            snapshot["total"].clone()
        };
        bus().emit(
            "runner.progress",
            json!({ "workspaceId": ws_id, "folderId": folder_id, "index": index, "total": total, "result": result }),
        );
        if bail && is_failed(&result) {
            break;
        }
    }
    build_report(&ws_id, &folder_id, started_at, results)
}

/// Start a run in the background. `{ ok: true, total }`, or `{ ok: false, error }` if the folder is already
/// running. Progress and the finished report go out as `runner.progress` / `runner.finished` events.
/// `params` may also hold `variables` / `environmentId` (laid over each request's environment), `freshJar`
/// (the run's cookies live in a jar of their own that is dropped at its end) and `bail` (stop after the first
/// request that failed).
pub async fn start_run(ctx: &CommandContext, params: &Value) -> Result<Value> {
    let (ws_id, folder_id) = (str_of(params, "workspaceId"), str_of(params, "folderId"));
    if let Some(existing) = get_run(ws_id, folder_id)
        && existing["running"] == json!(true)
    {
        return Ok(failed(
            &EngineError::conflict(format!(
                "A run of folder \"{folder_id}\" is already in flight in workspace \"{ws_id}\". \
                 Cancel it with runner.cancel, or wait for it to finish."
            ))
            .keyed("runner.already_running", json!({ "id": folder_id, "workspace": ws_id })),
        ));
    }

    let requests = load_ordered_requests(ctx, ws_id, folder_id).await?;
    let delay_ms = load_runner_config(ctx, ws_id, folder_id)
        .await
        .and_then(|config| config.get("delayMs").and_then(Value::as_f64))
        .filter(|delay| *delay > 0.0)
        .map(|delay| delay as u64);

    let total = requests.len();
    let run: Run = Arc::new(Mutex::new(json!({
        "workspaceId": ws_id,
        "folderId": folder_id,
        "startedAt": now_ms(),
        "total": total,
        "completed": 0,
        "running": true,
        "cancelled": false,
        "results": [],
    })));
    RUNS.lock().insert(key_of(ws_id, folder_id), run.clone());

    // The scope and the jar are task-locals: they are entered inside the spawned task, where the requests are sent.
    let work = {
        let (ctx, run, ws_id, folder_id) = (ctx.clone(), run.clone(), ws_id.to_string(), folder_id.to_string());
        let (params, bail) = (params.clone(), params["bail"] == json!(true));
        async move {
            let sending = send_scoped(&params, run_collection(ctx, run, ws_id, folder_id, requests, delay_ms, bail));
            if params["freshJar"] == json!(true) {
                cookies::private(&generate_id(), sending).await
            } else {
                sending.await
            }
        }
    };
    let task = tokio::spawn(crate::env::session::carry(work));
    let (ws_id, folder_id) = (ws_id.to_string(), folder_id.to_string());
    tokio::spawn(async move {
        match task.await {
            Ok(report) => {
                {
                    let mut snapshot = run.lock();
                    snapshot["running"] = json!(false);
                    snapshot["completedAt"] = report["completedAt"].clone();
                    snapshot["report"] = report.clone();
                }
                bus().emit("runner.finished", json!({ "workspaceId": ws_id, "folderId": folder_id, "report": report }));
            }
            Err(error) => {
                let mut snapshot = run.lock();
                snapshot["running"] = json!(false);
                let error = EngineError::engine(error.to_string())
                    .keyed("runner.failed", json!({ "detail": error.to_string() }));
                crate::runtime::set_failure(&mut snapshot, &error);
            }
        }
    });

    Ok(json!({ "ok": true, "total": total }))
}

/// Forget every run. Tests only.
#[cfg(test)]
pub(crate) fn reset_runs() {
    RUNS.lock().clear();
}
