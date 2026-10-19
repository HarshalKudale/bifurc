//! `request.send`: the request pipeline on the wire, for an inline request or a saved one.

use base64::{Engine, engine::general_purpose::STANDARD};
use std::{collections::HashSet, sync::LazyLock};

use parking_lot::Mutex;
use serde_json::{Value, json};

use super::{
    pipeline::{PipelineScripts, run_request_pipeline},
    request_body::encode_request_body,
};
use crate::{
    env::{VariableScope, send_scoped, with_overrides, with_variable_scope},
    kernel::{CommandContext, CommandRegistry},
    protocol::{EngineError, Result},
    utils::json::{array_of, is_truthy, opt_str, str_of},
    workspace::{config::load_entity, response_history},
};

/// The stored entity's fields the pipeline consumes.
fn request_from_entity(entity: &Value) -> Value {
    let mut request = json!({
        "method": entity.get("method").cloned().unwrap_or(Value::Null),
        "url": entity.get("url").cloned().unwrap_or(Value::Null),
        "headers": entity.get("headers").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!({})),
        "body": entity.get("body").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!("")),
    });
    if let Some(encoding) = entity.get("bodyEncoding").filter(|v| is_truthy(v)) {
        request["bodyEncoding"] = encoding.clone();
    }
    if let Some(parts) = entity.get("bodyParts").filter(|v| is_truthy(v)) {
        request["bodyParts"] = parts.clone();
    }
    if let Some(settings) = entity.get("settings").filter(|v| v.is_object()) {
        request["settings"] = settings.clone();
    }
    request
}

fn scripts_of(source: &Value, pre: &str, post: &str, test: &str) -> PipelineScripts {
    let get = |key: &str| opt_str(source, key).map(str::to_string);
    PipelineScripts { pre: get(pre), post: get(post), test: get(test) }
}

/// One pass of the pipeline: the send, its scripts, its history entry.
async fn send_once(params: &Value, ctx: &CommandContext) -> Result<Value> {
    let ws_id = str_of(params, "workspaceId").to_string();
    let (request, scripts) = if let Some(id) = opt_str(params, "id") {
        let entity = load_entity(&ws_id, "requests", id).ok_or_else(|| {
            EngineError::not_found(format!("No saved requests entity \"{id}\" in workspace \"{ws_id}\"."))
                .keyed("request.saved_not_found", json!({ "kind": "requests", "id": id, "workspace": ws_id }))
        })?;
        let mut scripts = scripts_of(&entity, "preScript", "postScript", "testScript");
        for skipped in array_of(params, "skipScripts") {
            match skipped.as_str() {
                Some("pre") => scripts.pre = None,
                Some("post") => scripts.post = None,
                Some("test") => scripts.test = None,
                _ => {}
            }
        }
        (request_from_entity(&with_overrides(entity, params)), scripts)
    } else {
        let inline = params.get("request").cloned().unwrap_or(Value::Null);
        let scripts = scripts_of(params.get("scripts").unwrap_or(&Value::Null), "pre", "post", "test");
        (inline, scripts)
    };

    let jar = ws_id.clone();
    // `jar: false` (`bifurc curl --no-jar`) sends without the workspace's cookie jar.
    let keep_jar = params.get("jar").and_then(Value::as_bool) != Some(false);
    let stream_id = params.get("streamId").filter(|id| id.is_string()).cloned();
    // Resolved by `request.replay` against the app's defaults; the pipeline does not need them.
    let settings = request.get("settings").filter(|v| v.is_object()).cloned();
    let result = run_request_pipeline(&ws_id, &request, &scripts, |resolved| async move {
        let encoded = encode_request_body(&resolved);
        let mut replay = json!({
            "method": resolved["method"],
            "url": resolved["url"],
            "headers": encoded.headers,
            "body": encoded.body_base64,
        });
        if let Some(id) = stream_id {
            replay["streamId"] = id;
        }
        if let Some(settings) = settings {
            replay["settings"] = settings;
        }
        let sent = ctx.registry.invoke("request.replay", replay, ctx.clone());
        if keep_jar { crate::cookies::scoped(&jar, sent).await } else { sent.await }
    })
    .await;
    // A send stopped by `request.cancel` is an outcome the caller can tell apart from a failure.
    let result = match result {
        Err(error) if error.details.as_ref().is_some_and(|d| d["cancelled"] == true) => {
            let mut cancelled = json!({ "ok": false, "cancelled": true });
            crate::runtime::set_failure(&mut cancelled, &error);
            return Ok(cancelled);
        }
        result => result,
    };
    if let Some(id) = response_history::entity_id(params) {
        let sent = result.as_ref().ok().map_or(&request, |done| &done["request"]);
        let (method, url) = (str_of(sent, "method").to_string(), str_of(sent, "url").to_string());
        let capture = match &result {
            Ok(done) => response_history::Capture {
                status: done["response"]["status"].as_i64().unwrap_or(0),
                status_text: None,
                headers: done["response"]["headers"].clone(),
                // `request.send` returns the bytes the upstream sent, base64-encoded.
                body: STANDARD.decode(str_of(&done["response"], "body").trim()).unwrap_or_default(),
                duration_ms: done["durationMs"].as_u64().unwrap_or(0),
                error: None,
                error_key: None,
                method,
                url,
            },
            Err(error) => response_history::failed(error, method, url),
        };
        response_history::record(&ws_id, id, capture);
    }
    let result = result?;

    let mut out = json!({
        "status": result["response"]["status"],
        "headers": result["response"]["headers"],
        "body": result["response"]["body"],
        "durationMs": result["durationMs"],
        "request": result["request"],
        "tests": result["tests"],
        "testLogs": result["testLogs"],
    });
    for key in ["preScriptError", "postScriptError", "warnings"] {
        if let Some(error) = result.get(key) {
            out[key] = error.clone();
        }
    }
    Ok(out)
}

/// Logins in progress, by workspace and session: a login that itself meets a 401 does not start another.
static LOGGING_IN: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

struct LoginGuard(String);

impl LoginGuard {
    fn take(ws_id: &str) -> Option<Self> {
        let key = format!("{ws_id}/{}", crate::env::session::named().unwrap_or_default());
        LOGGING_IN.lock().insert(key.clone()).then_some(LoginGuard(key))
    }
}

impl Drop for LoginGuard {
    fn drop(&mut self) {
        LOGGING_IN.lock().remove(&self.0);
    }
}

/// The environment's login, run once. `Err` is why it failed.
async fn run_login(ws_id: &str, login: &Value, ctx: &CommandContext) -> std::result::Result<(), String> {
    let id = str_of(login, "id");
    if str_of(login, "kind") == "flow" {
        return crate::flows::run_to_end(ctx, &json!({ "workspaceId": ws_id, "id": id })).await;
    }
    let sent = ctx.registry.invoke("request.send", json!({ "workspaceId": ws_id, "id": id }), ctx.clone()).await;
    let done = sent.map_err(|error| error.message)?;
    match done["status"].as_i64() {
        Some(200..=299) => Ok(()),
        Some(status) => Err(format!("the login request answered {status}")),
        None => Err(done["error"].as_str().unwrap_or("the login request did not complete").to_string()),
    }
}

/// A 401 under an environment with `auth.retryOn401`: run its login once, then send again once.
async fn reauthenticate(params: &Value, ctx: &CommandContext, first: Value) -> Result<Value> {
    if first["status"] != 401 {
        return Ok(first);
    }
    let ws_id = str_of(params, "workspaceId");
    let Some(auth) = crate::env::selected_environment(ws_id).and_then(|env| env.get("auth").cloned()) else {
        return Ok(first);
    };
    let login = &auth["login"];
    // The login request itself is never retried through itself.
    if !is_truthy(&auth["retryOn401"]) || !login.is_object() || opt_str(params, "id") == opt_str(login, "id") {
        return Ok(first);
    }
    let Some(_guard) = LoginGuard::take(ws_id) else { return Ok(first) };
    if let Err(error) = run_login(ws_id, login, ctx).await {
        let mut out = first;
        out["reauthentication"] = json!({ "ok": false, "error": error });
        return Ok(out);
    }
    drop(_guard);
    // Variables the login set in a named session are collected, not written: lay them over the retry.
    let scope = VariableScope { variables: crate::env::collected_variables(), ..Default::default() };
    let mut out = with_variable_scope(scope, send_once(params, ctx)).await?;
    out["reauthenticated"] = json!(true);
    Ok(out)
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register("request.send", |params: Value, ctx| async move {
        // `freshJar` (`--fresh-session`): the send has a jar of its own, empty and dropped with it.
        let fresh = params.get("freshJar").and_then(Value::as_bool) == Some(true);
        let scope_params = params.clone();
        let work = send_scoped(&scope_params, async move {
            let first = send_once(&params, &ctx).await?;
            reauthenticate(&params, &ctx, first).await
        });
        if fresh { crate::cookies::private(&crate::utils::id::generate_id(), work).await } else { work.await }
    });
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use super::*;
    use crate::{
        kernel::CommandContext, protocol::ErrorCode, rest::test_server, testing::scratch_data_root,
        workspace::fs::write_entity,
    };

    async fn call(action: &str, payload: Value) -> crate::protocol::Result<Value> {
        let mut registry = CommandRegistry::new();
        crate::rest::register(&mut registry);
        let registry = Arc::new(registry);
        let ctx = CommandContext::internal(registry.clone());
        registry.invoke(action, payload, ctx).await
    }

    #[tokio::test]
    async fn an_inline_request_runs_its_scripts_around_the_send() {
        let _root = scratch_data_root();
        let base = test_server::start(HashMap::new()).await;
        let result = call(
            "request.send",
            json!({
                "workspaceId": "ws1",
                "request": { "method": "POST", "url": format!("{base}/p"), "headers": { "X-A": "1" }, "body": "hi" },
                "scripts": {
                    "pre": "bc.request.headers.set('X-B', '2');",
                    "test": "bc.test('status', () => { bc.expect(bc.response.status).to.equal(200); });",
                },
            }),
        )
        .await
        .unwrap();
        assert_eq!(result["status"], 200);
        assert_eq!(result["request"]["headers"]["X-B"], "2");
        assert_eq!(result["tests"][0]["name"], "status");
        assert_eq!(result["tests"][0]["passed"], true);
        assert!(result.get("preScriptError").is_none());
        let body = crate::utils::replay::decode_base64_lenient(result["body"].as_str().unwrap());
        let echoed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(echoed["method"], "POST");
        assert_eq!(echoed["headers"]["x-b"], "2");
        assert_eq!(echoed["body"], "hi");
    }

    #[tokio::test]
    async fn a_saved_request_uses_its_own_scripts_and_a_script_error_is_a_value() {
        let root = scratch_data_root();
        let base = test_server::start(HashMap::new()).await;
        write_entity(
            root.ws_id,
            "requests",
            "r1",
            &json!({
                "id": "r1", "name": "r", "method": "GET", "url": format!("{base}/x"), "headers": {}, "body": "",
                "preScript": "throw new Error('nope');",
            }),
            None,
        )
        .unwrap();
        let result = call("request.send", json!({ "workspaceId": "ws1", "id": "r1" })).await.unwrap();
        assert_eq!(result["status"], 200);
        assert!(result["preScriptError"].as_str().unwrap().contains("nope"));
    }

    #[tokio::test]
    async fn a_missing_saved_request_is_not_found_and_a_failed_send_fails() {
        let _root = scratch_data_root();
        let missing = call("request.send", json!({ "workspaceId": "ws1", "id": "zzz" })).await.unwrap_err();
        assert_eq!(missing.code, ErrorCode::NotFound);
        let failed = call(
            "request.send",
            json!({ "workspaceId": "ws1", "request": { "method": "GET", "url": "nope", "headers": {}, "body": "" },
                    "scripts": {} }),
        )
        .await;
        assert!(failed.is_err());
    }

    #[tokio::test]
    async fn scripts_share_variables_through_the_run_and_persist_them() {
        let root = scratch_data_root();
        let base = test_server::start(HashMap::new()).await;
        crate::workspace::fs::write_flat_entity(
            root.ws_id,
            "environments",
            "__global__",
            &json!({ "id": "__global__", "name": "Global", "createdAt": 0, "workspaceId": root.ws_id,
                     "variables": [{ "id": "k", "key": "k", "value": "old" }, { "id": "d", "key": "d", "value": "drop" }] }),
        )
        .unwrap();
        let result = call(
            "request.send",
            json!({
                "workspaceId": "ws1",
                "request": { "method": "POST", "url": format!("{base}/p"), "headers": {}, "body": "{\"n\":1}" },
                "scripts": {
                    "pre": "bc.environment.set('stamp', 'S1'); bc.request.headers.set('X-Stamp', bc.environment.get('stamp')); bc.environment.unset('d'); console.log('pre', bc.request.method);",
                    "post": "const j = bc.response.json(); bc.environment.set('echoed', j.method + ':' + j.body); bc.environment.set('k', bc.response.status); console.log('post', bc.response.headers.get('content-type'));",
                    "test": "test('status 200', () => expect(bc.response.status).to.equal(200));                             test('post var visible', () => expect(bc.environment.get('echoed')).to.equal('POST:{\"n\":1}'));                             test('time + headers', () => { expect(bc.response.responseTime).to.be.a('number'); expect(bc.response.headers.has('content-type')).to.be.true; expect(bc.request.headers.get('x-stamp')).to.equal('S1'); });                             test('json body', () => expect(bc.response.json()).to.have.property('headers'));                             test('expected failure', () => expect(bc.response.status).to.equal(500));",
                },
            }),
        )
        .await
        .unwrap();
        assert!(result.get("preScriptError").is_none() && result.get("postScriptError").is_none(), "{result}");
        let outcomes: Vec<(&str, bool)> = result["tests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| (t["name"].as_str().unwrap(), t["passed"].as_bool().unwrap()))
            .collect();
        assert_eq!(
            outcomes,
            [
                ("status 200", true),
                ("post var visible", true),
                ("time + headers", true),
                ("json body", true),
                ("expected failure", false)
            ]
        );
        assert_eq!(result["testLogs"], json!(["pre POST", "post application/json"]));
        let env = crate::workspace::fs::read_all_entities(root.ws_id, "environments");
        let vars: Vec<(String, String)> = env[0]["variables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| (v["key"].as_str().unwrap().into(), v["value"].as_str().unwrap().into()))
            .collect();
        assert_eq!(
            vars,
            [("k".into(), "200".into()), ("stamp".into(), "S1".into()), ("echoed".into(), "POST:{\"n\":1}".into())]
        );
    }

    fn echoed(result: &Value) -> Value {
        serde_json::from_slice(&crate::utils::replay::decode_base64_lenient(result["body"].as_str().unwrap())).unwrap()
    }

    #[tokio::test]
    async fn a_send_takes_variables_an_environment_and_field_overrides() {
        let root = scratch_data_root();
        let base = test_server::start(HashMap::new()).await;
        write_entity(
            root.ws_id,
            "requests",
            "r1",
            &json!({ "id": "r1", "name": "r", "method": "POST", "url": "{{base}}/x", "headers": {}, "body": "saved {{x}}" }),
            None,
        )
        .unwrap();
        for (id, value) in [("__global__", "g"), ("staging", "s"), ("other", "o")] {
            crate::workspace::fs::write_flat_entity(
                root.ws_id,
                "environments",
                id,
                &json!({ "id": id, "name": id, "createdAt": 0, "workspaceId": root.ws_id,
                         "variables": [{ "id": "x", "key": "x", "value": value }, { "id": "b", "key": "base", "value": base }] }),
            )
            .unwrap();
        }
        let send = |extra: Value| {
            let mut params = json!({ "workspaceId": "ws1", "id": "r1" });
            params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            async move { echoed(&call("request.send", params).await.unwrap()) }
        };
        assert_eq!(send(json!({})).await["body"], "saved g");
        assert_eq!(send(json!({ "variables": { "x": "v" } })).await["body"], "saved v");
        assert_eq!(send(json!({ "environmentId": "staging" })).await["body"], "saved s");
        assert_eq!(send(json!({ "environmentId": "staging", "variables": { "x": "v" } })).await["body"], "saved v");
        assert_eq!(send(json!({ "overrides": { "body": "new {{x}}" } })).await["body"], "new g");
    }

    #[tokio::test]
    async fn a_send_with_the_jar_off_neither_sends_nor_keeps_cookies() {
        let _root = scratch_data_root();
        let base = test_server::start(HashMap::new()).await;
        let send = |path: &str, jar: bool| {
            let request = json!({ "method": "GET", "url": format!("{base}{path}"), "headers": {}, "body": "" });
            let params = json!({ "workspaceId": "ws1", "request": request, "scripts": {}, "jar": jar });
            async move { echoed(&call("request.send", params).await.unwrap()) }
        };
        send("/set-cookie/sid=s1", false).await;
        assert!(send("/x", true).await["headers"]["cookie"].is_null(), "a send with the jar off keeps nothing");
        send("/set-cookie/sid=s1", true).await;
        assert_eq!(send("/x", true).await["headers"]["cookie"], "sid=s1");
        assert!(send("/x", false).await["headers"]["cookie"].is_null());
    }

    /// A server that accepts connections and never answers.
    async fn silent_server() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        format!("http://{addr}/")
    }

    #[tokio::test]
    async fn a_send_that_hangs_can_be_cancelled_and_says_so() {
        let _root = scratch_data_root();
        let url = silent_server().await;
        let sending = tokio::spawn(async move {
            call(
                "request.send",
                json!({
                    "workspaceId": "ws1", "streamId": "s-hang", "scripts": {},
                    "request": { "method": "GET", "url": url, "headers": {}, "body": "" },
                }),
            )
            .await
        });
        while call("request.cancel", json!({ "streamId": "s-hang" })).await.unwrap()["ok"] != true {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let outcome = sending.await.unwrap().unwrap();
        assert_eq!(
            outcome,
            json!({
                "ok": false, "cancelled": true, "error": "Cancelled",
                "errorKey": "request.send.cancelled", "errorParams": {},
            })
        );
        let nothing = call("request.cancel", json!({ "streamId": "s-hang" })).await.unwrap();
        assert_eq!(nothing["ok"], false);
    }

    #[tokio::test]
    async fn a_saved_requests_time_limit_applies_and_one_without_settings_has_none() {
        let root = scratch_data_root();
        let url = silent_server().await;
        write_entity(
            root.ws_id,
            "requests",
            "slow",
            &json!({ "id": "slow", "name": "s", "method": "GET", "url": url, "headers": {}, "body": "",
                     "settings": { "timeoutMs": 200 } }),
            None,
        )
        .unwrap();
        let err = call("request.send", json!({ "workspaceId": "ws1", "id": "slow" })).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Timeout);
        assert_eq!(err.message, "Timed out after 200 ms");
    }

    #[tokio::test]
    async fn a_send_with_an_entity_id_is_recorded_and_the_rest_are_not() {
        let root = scratch_data_root();
        let base = test_server::start(HashMap::new()).await;
        let history = |id: &str| response_history::entries(root.ws_id, id);
        write_entity(
            root.ws_id,
            "requests",
            "r1",
            &json!({ "id": "r1", "name": "r", "method": "GET", "url": format!("{base}/x"), "headers": {}, "body": "" }),
            None,
        )
        .unwrap();
        call("request.send", json!({ "workspaceId": "ws1", "id": "r1" })).await.unwrap();
        let inline = json!({ "method": "POST", "url": format!("{base}/p"), "headers": {}, "body": "hi" });
        call("request.send", json!({ "workspaceId": "ws1", "entityId": "r1", "request": inline, "scripts": {} }))
            .await
            .unwrap();
        // A draft has no entity to keep a history for.
        call("request.send", json!({ "workspaceId": "ws1", "request": inline, "scripts": {} })).await.unwrap();
        let kept = history("r1");
        assert_eq!(kept.len(), 2);
        assert_eq!(
            (kept[0]["request"]["method"].clone(), kept[1]["request"]["method"].clone()),
            (json!("POST"), json!("GET"))
        );
        assert_eq!(kept[0]["status"], 200);
        assert!(kept[0]["body"].as_str().unwrap().contains("\"body\":\"hi\""));

        // A transport error is kept with its message; a cancelled send is not.
        let refused = call(
            "request.send",
            json!({ "workspaceId": "ws1", "entityId": "r1", "scripts": {}, "request": { "method": "GET", "url": "http://127.0.0.1:1/", "headers": {}, "body": "" } }),
        )
        .await;
        assert!(refused.is_err());
        assert_eq!(history("r1").len(), 3);
        assert!(history("r1")[0]["error"].is_string());

        let url = silent_server().await;
        let sending = tokio::spawn(async move {
            call(
                "request.send",
                json!({
                    "workspaceId": "ws1", "streamId": "s-hist", "entityId": "r2", "scripts": {},
                    "request": { "method": "GET", "url": url, "headers": {}, "body": "" },
                }),
            )
            .await
        });
        while call("request.cancel", json!({ "streamId": "s-hist" })).await.unwrap()["ok"] != true {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(sending.await.unwrap().unwrap()["cancelled"], true);
        assert!(history("r2").is_empty());
    }

    /// An environment `e1` with `base` (the test server) and `code`, whose login is the saved request `login`.
    fn login_setup(root: &crate::testing::ScratchRoot, base: &str, login_url: &str, cookie: &str, auth: Value) {
        crate::workspace::fs::write_flat_entity(
            root.ws_id,
            "environments",
            "e1",
            &json!({ "id": "e1", "name": "e1", "createdAt": 0, "workspaceId": root.ws_id, "auth": auth,
                     "variables": [{ "id": "b", "key": "base", "value": base }, { "id": "c", "key": "code", "value": "401" }] }),
        )
        .unwrap();
        for (id, url, post) in [
            ("login", login_url, "bc.environment.set('code', '200');"),
            ("me", &format!("{{{{base}}}}/need-cookie/{cookie}=ok"), ""),
            ("coded", "{{base}}/status/{{code}}", ""),
        ] {
            write_entity(
                root.ws_id,
                "requests",
                id,
                &json!({ "id": id, "name": id, "method": "GET", "url": url.replace("{{base}}", base), "headers": {}, "body": "",
                         "postScript": post }),
                None,
            )
            .unwrap();
        }
    }

    fn auth(retry: bool) -> Value {
        json!({ "login": { "kind": "request", "id": "login" }, "retryOn401": retry })
    }

    #[tokio::test]
    async fn a_401_runs_the_environments_login_once_and_sends_again() {
        let root = scratch_data_root();
        let base = test_server::start(HashMap::new()).await;
        login_setup(&root, &base, &format!("{base}/set-cookie/sidA=ok"), "sidA", auth(true));
        let send = |id: &'static str| call("request.send", json!({ "workspaceId": "ws1", "id": id }));
        // The jar of its own, so that cookies of this test reach no other.
        crate::cookies::private("login-test", async {
            let first = send("me").await.unwrap();
            assert_eq!((first["status"].clone(), first["reauthenticated"].clone()), (json!(200), json!(true)));
            // The cookie is in the jar now: no 401, no login.
            assert!(send("me").await.unwrap().get("reauthenticated").is_none());
            // A variable the login's script set is used by the retry (`code` 401 -> 200).
            login_setup(&root, &base, &format!("{base}/set-cookie/sidA=ok"), "sidA", auth(true));
            let coded = send("coded").await.unwrap();
            assert_eq!((coded["status"].clone(), coded["reauthenticated"].clone()), (json!(200), json!(true)));
        })
        .await;
    }

    #[tokio::test]
    async fn a_failed_login_returns_the_original_401_and_says_why() {
        let root = scratch_data_root();
        let base = test_server::start(HashMap::new()).await;
        login_setup(&root, &base, &format!("{base}/status/500"), "sidB", auth(true));
        let out = call("request.send", json!({ "workspaceId": "ws1", "id": "me" })).await.unwrap();
        assert_eq!(out["status"], 401);
        assert!(out.get("reauthenticated").is_none());
        assert_eq!(out["reauthentication"]["ok"], false);
        assert!(out["reauthentication"]["error"].as_str().unwrap().contains("500"));
    }

    #[tokio::test]
    async fn no_retry_when_it_is_off_and_no_loop_when_the_login_itself_is_refused() {
        let root = scratch_data_root();
        let base = test_server::start(HashMap::new()).await;
        login_setup(&root, &base, &format!("{base}/set-cookie/sidC=ok"), "sidC", auth(false));
        let off = call("request.send", json!({ "workspaceId": "ws1", "id": "me" })).await.unwrap();
        assert!(off["status"] == 401 && off.get("reauthenticated").is_none() && off.get("reauthentication").is_none());

        login_setup(&root, &base, &format!("{base}/status/401"), "sidC", auth(true));
        let login = call("request.send", json!({ "workspaceId": "ws1", "id": "login" })).await.unwrap();
        assert!(login["status"] == 401 && login.get("reauthentication").is_none());
        let me = call("request.send", json!({ "workspaceId": "ws1", "id": "me" })).await.unwrap();
        assert_eq!((me["status"].clone(), me["reauthentication"]["ok"].clone()), (json!(401), json!(false)));
    }
}
