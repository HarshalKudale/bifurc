//! The request pipeline: resolve `{{variables}}`, pre-script, send, post-script, test script.
//!
//! The send itself is injected, so the pipeline knows no protocol. The request a caller hands in holds text
//! (`body`); what comes back off the wire is base64; the post and test scripts see the response body decoded
//! to text. A failed send fails the pipeline, a failed script does not: script errors are values.

use std::{future::Future, time::Instant};

use serde_json::{Map, Value, json};

use super::request_body::resolve_request_body;
use crate::{
    env::{apply_variable_changes, authored_variables_text, load_merged_environment, resolve_headers, resolve_vars},
    protocol::Result,
    scripting::execute_ipc_script,
    vault::{Redactor, references_in, resolve_value_allowed},
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
pub(crate) fn env_to_map(env: Option<&Value>) -> Value {
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
fn environment_after_script(base: Option<Value>, env_vars: &Value) -> Option<Value> {
    let base = base?;
    let vars = match env_vars.as_object() {
        Some(vars) if !vars.is_empty() => vars,
        _ => return Some(base),
    };
    let variables: Vec<Value> =
        vars.iter().map(|(key, value)| json!({ "id": key, "key": key, "value": value })).collect();
    Some(merged(&base, &json!({ "variables": variables })))
}

fn collect_logs(output: &Value, into: &mut Vec<Value>) {
    into.extend(output.get("logs").and_then(Value::as_array).cloned().unwrap_or_default());
}

/// Write the variables a script set or removed (against the environment the run started with) to disk.
pub(crate) fn persist_script_variables(ws_id: &str, before: &Value, after: &Value) {
    let (Some(before), Some(after)) = (before.as_object(), after.as_object()) else { return };
    let text = |v: &Value| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
    let set: Vec<(String, String)> = after
        .iter()
        .filter(|(key, value)| before.get(*key) != Some(*value))
        .map(|(key, value)| (key.clone(), text(value)))
        .collect();
    let removed: Vec<String> = before.keys().filter(|key| !after.contains_key(*key)).cloned().collect();
    apply_variable_changes(ws_id, &set, &removed);
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
    let full_env = load_merged_environment(ws_id);
    // Scripts, the echoed request and the logs hold `{{vault.secret}}` as written; the values are put in only
    // on the copy that is sent, and only for references the request's own author wrote (not ones that arrived
    // from elsewhere, such as a response a script copied from).
    let mut allowed = references_in(&request.to_string());
    for script in [&scripts.pre, &scripts.post, &scripts.test].into_iter().flatten() {
        allowed.extend(references_in(script));
    }
    allowed.extend(references_in(&full_env.as_ref().map(authored_variables_text).unwrap_or_default()));
    let env = full_env.map(|mut env| {
        env["workspaceId"] = json!("");
        env
    });

    let mut resolved = json!({
        "method": request.get("method").cloned().unwrap_or(Value::Null),
        "url": resolve_vars(str_of(request, "url"), env.as_ref()),
        "headers": resolve_headers(request.get("headers").unwrap_or(&Value::Null), env.as_ref()),
    });
    resolved = merged(&resolved, &resolve_request_body(request, env.as_ref()));
    let original_vars = env_to_map(env.as_ref());
    let mut effective_env = env;
    let mut script_logs: Vec<Value> = Vec::new();

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
            "cookies": crate::cookies::for_url(ws_id, str_of(&resolved, "url")),
        }))
        .await;
        pre_script_error = opt_str(&pre, "error").map(str::to_string);
        collect_logs(&pre, &mut script_logs);
        if let Some(changed) = pre.get("request").filter(|v| v.is_object()) {
            resolved = merged(&resolved, changed);
        }
        effective_env = environment_after_script(effective_env, pre.get("envVars").unwrap_or(&Value::Null));
        crate::cookies::apply(ws_id, str_of(&resolved, "url"), &pre["cookies"]);
    }

    let send_start = Instant::now();
    let mut wire = resolved.clone();
    resolve_value_allowed(ws_id, &mut wire, &allowed);
    let response = send(wire).await?;
    let duration_ms = send_start.elapsed().as_millis() as u64;

    let response_body = body_text(&response);
    let sent = json!({
        "method": resolved["method"],
        "url": resolved["url"],
        "headers": resolved["headers"],
        "body": resolved["body"],
    });

    let mut post_script_error: Option<String> = None;
    if let Some(script) = has_script(&scripts.post) {
        let post = execute_ipc_script(json!({
            "script": script,
            "context": "post",
            "request": sent,
            "response": {
                "status": response["status"],
                "headers": response["headers"],
                "body": response_body,
                "responseTime": duration_ms,
            },
            "envVars": env_to_map(effective_env.as_ref()),
            "cookies": crate::cookies::for_url(ws_id, str_of(&sent, "url")),
        }))
        .await;
        post_script_error = opt_str(&post, "error").map(str::to_string);
        collect_logs(&post, &mut script_logs);
        effective_env = environment_after_script(effective_env, post.get("envVars").unwrap_or(&Value::Null));
        crate::cookies::apply(ws_id, str_of(&sent, "url"), &post["cookies"]);
    }

    let mut tests = Value::Array(Vec::new());
    let mut test_logs = Value::Array(Vec::new());
    if let Some(script) = has_script(&scripts.test) {
        let test = execute_ipc_script(json!({
            "script": script,
            "context": "test",
            "request": sent,
            "response": {
                "status": response["status"],
                "headers": response["headers"],
                "body": response_body,
                "responseTime": duration_ms,
            },
            "envVars": env_to_map(effective_env.as_ref()),
            "cookies": crate::cookies::for_url(ws_id, str_of(&sent, "url")),
        }))
        .await;
        if let Some(results) = test.get("testResults").filter(|v| v.is_array()) {
            tests = results.clone();
        }
        if let Some(logs) = test.get("testLogs").filter(|v| v.is_array()) {
            test_logs = logs.clone();
        }
        effective_env = environment_after_script(effective_env, test.get("envVars").unwrap_or(&Value::Null));
        crate::cookies::apply(ws_id, str_of(&sent, "url"), &test["cookies"]);
    }
    if let Value::Array(logs) = &mut test_logs {
        script_logs.append(logs);
        *logs = script_logs;
    }
    persist_script_variables(ws_id, &original_vars, &env_to_map(effective_env.as_ref()));

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
    // What goes back to a client (and into history and reports) names a secret, never shows it.
    if let Some(redactor) = Redactor::for_workspace(ws_id) {
        let mut body = out["response"]["body"].take();
        redactor.base64_body(&mut body);
        out["response"]["body"] = body;
        for part in ["request", "tests", "testLogs", "preScriptError", "postScriptError"] {
            if let Some(value) = out.get_mut(part) {
                redactor.value(value);
            }
        }
        if let Some(headers) = out["response"].get_mut("headers") {
            redactor.value(headers);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_script_environment_keeps_the_base() {
        let base = json!({ "id": "merged", "variables": [{ "id": "a", "key": "a", "value": "1" }] });
        assert_eq!(environment_after_script(Some(base.clone()), &json!({})), Some(base.clone()));
        assert_eq!(environment_after_script(None, &json!({ "x": "1" })), None);
        let out = environment_after_script(Some(base), &json!({ "x": "2" })).unwrap();
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
