//! The request pipeline: resolve `{{variables}}`, pre-script, send, post-script, test script.
//!
//! The send itself is injected, so the pipeline knows no protocol. The request a caller hands in holds text
//! (`body`); what comes back off the wire is base64; the post and test scripts see the response body decoded
//! to text. A failed send fails the pipeline, a failed script does not: script errors are values.

use std::{future::Future, time::Instant};

use serde_json::{Map, Value, json};

use super::request_body::resolve_request_body;
use crate::{
    env::{load_merged_environment, resolve_headers, resolve_vars},
    protocol::Result,
    scripting::execute_ipc_script,
    utils::{
        json::{array_of, merged, opt_str, str_of},
        replay::decode_base64_lenient,
    },
};

/// The three scripts a request may carry. Absent or whitespace-only means "do not run it".
#[derive(Debug, Clone, Default)]
pub struct PipelineScripts {
    pub pre: Option<String>,
    pub post: Option<String>,
    pub test: Option<String>,
}

fn has_script(script: &Option<String>) -> Option<&str> {
    script.as_deref().filter(|script| !script.trim().is_empty())
}

/// The flat map the sandbox's `lp.environment` reads.
fn env_to_map(env: Option<&Value>) -> Value {
    let mut map = Map::new();
    if let Some(env) = env {
        for variable in array_of(env, "variables") {
            map.insert(str_of(variable, "key").to_string(), variable.get("value").cloned().unwrap_or(Value::Null));
        }
    }
    Value::Object(map)
}

/// The effective environment after a pre-script ran: its variables as it left them. The base is kept as it
/// is when there is none, or when the script left no variables at all.
fn environment_after_pre_script(base: Option<Value>, env_vars: &Value) -> Option<Value> {
    let base = base?;
    let vars = match env_vars.as_object() {
        Some(vars) if !vars.is_empty() => vars,
        _ => return Some(base),
    };
    let variables: Vec<Value> =
        vars.iter().map(|(key, value)| json!({ "id": key, "key": key, "value": value })).collect();
    Some(merged(&base, &json!({ "variables": variables })))
}

fn body_text(response: &Value) -> String {
    String::from_utf8_lossy(&decode_base64_lenient(str_of(response, "body"))).into_owned()
}

/// Run the pipeline for `request` (`method`, `url`, `headers`, `body`, `bodyEncoding?`, `bodyParts?`, all
/// unresolved). `send` receives the resolved request and returns `{status, headers, body}` with a base64
/// body. The result holds `request` (what was sent), `response`, `tests`, `testLogs`, `durationMs` and, only
/// when a script failed, `preScriptError` / `postScriptError`.
pub async fn run_request_pipeline<F, Fut>(
    ws_id: &str,
    request: &Value,
    scripts: &PipelineScripts,
    send: F,
) -> Result<Value>
where
    F: FnOnce(Value) -> Fut + Send,
    Fut: Future<Output = Result<Value>> + Send,
{
    let env = load_merged_environment(ws_id);

    let mut resolved = json!({
        "method": request.get("method").cloned().unwrap_or(Value::Null),
        "url": resolve_vars(str_of(request, "url"), env.as_ref()),
        "headers": resolve_headers(request.get("headers").unwrap_or(&Value::Null), env.as_ref()),
    });
    resolved = merged(&resolved, &resolve_request_body(request, env.as_ref()));
    let mut effective_env = env;

    let mut pre_script_error: Option<String> = None;
    if let Some(script) = has_script(&scripts.pre) {
        let pre = execute_ipc_script(json!({
            "script": script,
            "context": "pre",
            "request": {
                "method": resolved["method"],
                "url": resolved["url"],
                "headers": resolved["headers"],
                "body": resolved["body"],
            },
            "envVars": env_to_map(effective_env.as_ref()),
        }))
        .await;
        pre_script_error = opt_str(&pre, "error").map(str::to_string);
        if let Some(changed) = pre.get("request").filter(|v| v.is_object()) {
            resolved = merged(&resolved, changed);
        }
        effective_env = environment_after_pre_script(effective_env, pre.get("envVars").unwrap_or(&Value::Null));
    }

    let send_start = Instant::now();
    let response = send(resolved.clone()).await?;
    let duration_ms = send_start.elapsed().as_millis() as u64;

    let response_body = body_text(&response);

    let mut post_script_error: Option<String> = None;
    if let Some(script) = has_script(&scripts.post) {
        let post = execute_ipc_script(json!({
            "script": script,
            "context": "post",
            "response": { "status": response["status"], "headers": response["headers"], "body": response_body },
            "envVars": env_to_map(effective_env.as_ref()),
        }))
        .await;
        post_script_error = opt_str(&post, "error").map(str::to_string);
    }

    let mut tests = Value::Array(Vec::new());
    let mut test_logs = Value::Array(Vec::new());
    if let Some(script) = has_script(&scripts.test) {
        let test = execute_ipc_script(json!({
            "script": script,
            "context": "test",
            "response": {
                "status": response["status"],
                "headers": response["headers"],
                "body": response_body,
                "responseTime": duration_ms,
            },
            "envVars": env_to_map(effective_env.as_ref()),
        }))
        .await;
        if let Some(results) = test.get("testResults").filter(|v| v.is_array()) {
            tests = results.clone();
        }
        if let Some(logs) = test.get("testLogs").filter(|v| v.is_array()) {
            test_logs = logs.clone();
        }
    }

    let mut out = json!({
        "request": resolved,
        "response": response,
        "tests": tests,
        "testLogs": test_logs,
        "durationMs": duration_ms,
    });
    if let Some(error) = pre_script_error {
        out["preScriptError"] = json!(error);
    }
    if let Some(error) = post_script_error {
        out["postScriptError"] = json!(error);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_script_environment_keeps_the_base() {
        let base = json!({ "id": "merged", "variables": [{ "id": "a", "key": "a", "value": "1" }] });
        assert_eq!(environment_after_pre_script(Some(base.clone()), &json!({})), Some(base.clone()));
        assert_eq!(environment_after_pre_script(None, &json!({ "x": "1" })), None);
        let out = environment_after_pre_script(Some(base), &json!({ "x": "2" })).unwrap();
        assert_eq!(out["variables"], json!([{ "id": "x", "key": "x", "value": "2" }]));
        assert_eq!(out["id"], "merged");
    }

    #[test]
    fn blank_scripts_do_not_run() {
        assert!(has_script(&None).is_none());
        assert!(has_script(&Some("  \n".into())).is_none());
        assert_eq!(has_script(&Some("x".into())), Some("x"));
    }
}
