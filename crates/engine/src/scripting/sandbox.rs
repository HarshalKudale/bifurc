//! The QuickJS sandbox that runs user scripts: request pre/post/test scripts and proxy rule scripts.
//!
//! Every execution gets a fresh runtime and context with a memory limit, a stack limit and a wall-clock
//! interrupt. The script sees only the language built-ins plus `bc` (and `console` in test scripts), which the
//! JavaScript prelude builds from a JSON input. The result leaves the sandbox as JSON.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use rquickjs::{Context, Ctx, Error, Function, Object, Persistent, Runtime, Value, context::EvalOptions};
use serde_json::{Value as Json, json};
use tokio::runtime::Handle;

use crate::{
    kernel::traffic::Headers,
    utils::http::{RequestSettings, root_message},
};

/// How long a script may run before it is aborted.
pub const SCRIPT_TIMEOUT_MS: u64 = 5000;
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
const STACK_LIMIT: usize = 512 * 1024;
/// What `bc.sendRequest` may do in one script run: how many calls, how long each waits, how much body is kept.
const SEND_LIMIT: usize = 20;
const SEND_TIMEOUT: Duration = Duration::from_secs(30);
const SEND_BODY_LIMIT: usize = 5 * 1024 * 1024;
const PRELUDE: &str = include_str!("prelude.js");

/// What a proxy rule script produced.
pub struct ProxyScriptResult {
    pub headers: Headers,
    pub body: String,
    pub error: Option<String>,
}

/// One `bc.sendRequest`: `req` is `{url, method, headers: [[k, v]], body}` as JSON text; the answer is
/// `{status, headers, body, responseTime}` or `{error}`.
async fn send_request(req: String) -> Json {
    let go = async {
        let req: Json = serde_json::from_str(&req).map_err(|e| e.to_string())?;
        let method = req["method"].as_str().unwrap_or("GET").to_ascii_uppercase();
        let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?;
        let url = req["url"].as_str().unwrap_or_default();
        let mut builder = RequestSettings::default().client()?.request(method, url).timeout(SEND_TIMEOUT);
        for pair in req["headers"].as_array().into_iter().flatten() {
            if let (Some(k), Some(v)) = (pair[0].as_str(), pair[1].as_str()) {
                builder = builder.header(k, v);
            }
        }
        if let Some(body) = req["body"].as_str() {
            builder = builder.body(body.to_string());
        }
        let start = Instant::now();
        let mut res = builder.send().await.map_err(|e| root_message(&e))?;
        let mut headers = serde_json::Map::new();
        for (k, v) in res.headers() {
            let v = String::from_utf8_lossy(v.as_bytes()).into_owned();
            let joined = match headers.get(k.as_str()).and_then(Json::as_str) {
                Some(old) => format!("{old}, {v}"),
                None => v,
            };
            headers.insert(k.as_str().to_string(), json!(joined));
        }
        let status = res.status().as_u16();
        let mut body = Vec::new();
        while let Some(chunk) = res.chunk().await.map_err(|e| root_message(&e))? {
            body.extend_from_slice(&chunk);
            if body.len() >= SEND_BODY_LIMIT {
                body.truncate(SEND_BODY_LIMIT);
                break;
            }
        }
        let body = String::from_utf8_lossy(&body).into_owned();
        let responded = start.elapsed().as_millis() as u64;
        Ok::<_, String>(json!({ "status": status, "headers": headers, "body": body, "responseTime": responded }))
    };
    go.await.unwrap_or_else(|error| json!({ "error": error }))
}

/// Run `script` against `input` (the prelude's JSON input). Returns the prelude's collected result, or a message
/// when the sandbox itself failed before a result could be collected. `send` (the runtime to run requests on)
/// gives the script `bc.sendRequest`.
fn run_sandbox(input: String, script: String, timeout: Duration, send: Option<Handle>) -> Result<Json, String> {
    let rt = Runtime::new().map_err(|e| e.to_string())?;
    rt.set_memory_limit(MEMORY_LIMIT);
    rt.set_max_stack_size(STACK_LIMIT);

    let deadline = Instant::now() + timeout;
    let armed = Arc::new(AtomicBool::new(true));
    let timed_out = Arc::new(AtomicBool::new(false));
    // Microseconds spent waiting on `bc.sendRequest`, which do not count against the script's time.
    let waited = Arc::new(AtomicU64::new(0));
    {
        let (armed, timed_out, waited) = (armed.clone(), timed_out.clone(), waited.clone());
        rt.set_interrupt_handler(Some(Box::new(move || {
            let deadline = deadline + Duration::from_micros(waited.load(Ordering::Relaxed));
            if armed.load(Ordering::Relaxed) && Instant::now() >= deadline {
                timed_out.store(true, Ordering::Relaxed);
                true
            } else {
                false
            }
        })));
    }
    let ctx = Context::full(&rt).map_err(|e| e.to_string())?;

    type Saved = (Persistent<Function<'static>>, Persistent<Function<'static>>, Option<String>);
    let saved: Saved = ctx.with(|ctx| -> Result<Saved, String> {
        ctx.globals().set("__input", input).map_err(|e| e.to_string())?;
        if let Some(handle) = send {
            let sent = AtomicUsize::new(0);
            let host = Function::new(ctx.clone(), move |req: String| -> String {
                if sent.fetch_add(1, Ordering::Relaxed) >= SEND_LIMIT {
                    let error = format!("bc.sendRequest is limited to {SEND_LIMIT} calls per script");
                    return json!({ "error": error }).to_string();
                }
                let start = Instant::now();
                let out = handle.block_on(send_request(req));
                waited.fetch_add(start.elapsed().as_micros() as u64, Ordering::Relaxed);
                out.to_string()
            })
            .map_err(|e| e.to_string())?;
            ctx.globals().set("__send", host).map_err(|e| e.to_string())?;
        }
        let api: Object = ctx.eval(PRELUDE).map_err(|e| describe(&ctx, e))?;
        let collect: Function = api.get("collect").map_err(|e| e.to_string())?;
        let fmt: Function = api.get("fmt").map_err(|e| e.to_string())?;
        let mut options = EvalOptions::default();
        options.strict = false;
        let outcome = ctx.eval_with_options::<Value, _>(script, options);
        armed.store(false, Ordering::Relaxed);
        let error = match outcome {
            Ok(_) => None,
            Err(_) if timed_out.load(Ordering::Relaxed) => {
                Some(format!("Script execution timed out after {}ms", timeout.as_millis()))
            }
            Err(e) => Some(describe_with(&ctx, &fmt, e)),
        };
        Ok((Persistent::save(&ctx, collect), Persistent::save(&ctx, fmt), error))
    })?;

    // Leave room to collect the result even when the script ended by filling the heap.
    rt.set_memory_limit(MEMORY_LIMIT * 2);
    let (collect, fmt, error) = saved;
    let out: Result<String, String> = ctx.with(|ctx| {
        let _fmt = fmt.restore(&ctx).map_err(|e| e.to_string())?;
        let collect = collect.restore(&ctx).map_err(|e| e.to_string())?;
        collect.call::<_, String>((error,)).map_err(|e| describe(&ctx, e))
    });
    drop(ctx);
    drop(rt);
    serde_json::from_str(&out?).map_err(|e| e.to_string())
}

fn describe(ctx: &Ctx<'_>, e: Error) -> String {
    match e {
        Error::Exception => {
            let thrown = ctx.catch();
            match thrown.as_exception().and_then(|x| x.message()) {
                Some(m) => m,
                None => format!("{thrown:?}"),
            }
        }
        other => other.to_string(),
    }
}

/// `e instanceof Error ? e.message : String(e)`, evaluated by the prelude's own `fmt`.
fn describe_with<'js>(ctx: &Ctx<'js>, fmt: &Function<'js>, e: Error) -> String {
    match e {
        Error::Exception => {
            let thrown = ctx.catch();
            fmt.call::<_, String>((thrown,)).unwrap_or_else(|_| "Script error".to_string())
        }
        other => other.to_string(),
    }
}

async fn run_blocking(input: Json, script: &str, timeout: Duration, send: bool) -> Result<Json, String> {
    let (input, script) = (input.to_string(), script.to_string());
    let send = send.then(Handle::current);
    tokio::task::spawn_blocking(move || run_sandbox(input, script, timeout, send)).await.map_err(|e| e.to_string())?
}

async fn run_proxy(key: &str, script: &str, headers: &Headers, body: &str, timeout: Duration) -> ProxyScriptResult {
    let mode = if key == "request" { "proxyRequest" } else { "proxyResponse" };
    let input = json!({ "mode": mode, "headers": headers, "body": body });
    match run_blocking(input, script, timeout, false).await {
        Ok(out) => ProxyScriptResult {
            headers: out
                .get("headers")
                .and_then(Json::as_object)
                .map(|o| o.iter().map(|(k, v)| (k.clone(), json_string(v))).collect())
                .unwrap_or_default(),
            body: out.get("body").and_then(Json::as_str).unwrap_or_default().to_string(),
            error: out.get("error").and_then(Json::as_str).map(str::to_string),
        },
        Err(e) => ProxyScriptResult { headers: headers.clone(), body: body.to_string(), error: Some(e) },
    }
}

fn json_string(v: &Json) -> String {
    match v {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Proxy rule request script: `bc.request.headers` (a plain mutable object) and `bc.request.body` (get/set).
pub async fn execute_request_script(script: &str, headers: &Headers, body: &str) -> ProxyScriptResult {
    run_proxy("request", script, headers, body, Duration::from_millis(SCRIPT_TIMEOUT_MS)).await
}

/// Proxy rule response script: the same with `bc.response`.
pub async fn execute_response_script(script: &str, headers: &Headers, body: &str) -> ProxyScriptResult {
    run_proxy("response", script, headers, body, Duration::from_millis(SCRIPT_TIMEOUT_MS)).await
}

/// Request scripts (`pre`, `post`, `test`). Takes and returns the JSON shapes of the protocol's
/// `script.execute` params and result.
pub async fn execute_ipc_script(opts: Json) -> Json {
    execute_ipc_script_with(opts, Duration::from_millis(SCRIPT_TIMEOUT_MS)).await
}

pub(crate) async fn execute_ipc_script_with(opts: Json, timeout: Duration) -> Json {
    let env_vars = opts.get("envVars").cloned().filter(Json::is_object).unwrap_or_else(|| json!({}));
    let script = opts.get("script").and_then(Json::as_str).unwrap_or_default().to_string();
    let context = opts.get("context").and_then(Json::as_str).unwrap_or_default();
    let request = opts.get("request").filter(|v| v.is_object());
    let response = opts.get("response").filter(|v| v.is_object());

    let mut input = match (context, request, response) {
        ("pre", Some(req), _) => json!({ "mode": "pre", "request": req, "envVars": env_vars }),
        ("post", req, Some(res)) => json!({ "mode": "post", "request": req, "response": res, "envVars": env_vars }),
        ("test", req, Some(res)) => json!({ "mode": "test", "request": req, "response": res, "envVars": env_vars }),
        _ => {
            return json!({ "envVars": env_vars, "error": "Invalid script context or missing request/response" });
        }
    };
    // What `bc.cookies` reads; the script's changes come back as the result's `cookies`, for the caller to apply.
    input["cookies"] = opts.get("cookies").cloned().unwrap_or_default();
    match run_blocking(input, &script, timeout, true).await {
        Ok(out) => out,
        Err(e) => json!({ "envVars": env_vars, "error": e }),
    }
}

/// A flow node's after-execute script. `data` holds `output` (what the node produced, which the script reads
/// and replaces as `bc.output`), `input`, `nodes` and `flow`. Returns `{ output, envVars, logs, error? }`.
pub async fn execute_flow_script(script: &str, mut data: Json, env_vars: Json) -> Json {
    data["mode"] = json!("flow");
    data["envVars"] = env_vars;
    match run_blocking(data, script, Duration::from_millis(SCRIPT_TIMEOUT_MS), true).await {
        Ok(out) => out,
        Err(e) => json!({ "error": e }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_flow_script_reads_and_replaces_what_its_node_produced() {
        let data = || json!({ "output": { "body": "{\"id\":7}" }, "input": { "user": "1" }, "nodes": {}, "flow": {} });
        let out = execute_flow_script(
            "var id = JSON.parse(bc.output.body).id; bc.environment.set('id', id);\
             bc.output = { id: id, user: bc.input.user }; console.log('seen', id);",
            data(),
            json!({ "K": "v" }),
        )
        .await;
        assert_eq!(out["output"], json!({ "id": 7, "user": "1" }));
        assert_eq!(out["envVars"], json!({ "K": "v", "id": "7" }));
        assert_eq!(out["logs"], json!(["seen 7"]));
        assert!(out.get("error").is_none());

        let out = execute_flow_script("bc.output.extra = 1; throw new Error('boom');", data(), json!({})).await;
        assert_eq!(out["error"], "Error: boom");
        assert_eq!(out["output"]["extra"], 1);
        assert_eq!(execute_flow_script("bc.output = undefined", data(), json!({})).await["output"], Json::Null);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_scripts_run_off_the_async_workers() {
        let spin = "var t = Date.now(); while (Date.now() - t < 300) {} bc.output = { ok: 1 };";
        let data = || json!({ "output": {}, "input": {}, "nodes": {}, "flow": {} });
        let start = Instant::now();
        let ticker = tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Instant::now()
        });
        let (a, b) =
            tokio::join!(execute_flow_script(spin, data(), json!({})), execute_flow_script(spin, data(), json!({})));
        assert_eq!((a["output"]["ok"].clone(), b["output"]["ok"].clone()), (json!(1), json!(1)));
        // The timer fired long before the 300ms scripts finished: they did not occupy the async workers.
        assert!(ticker.await.unwrap().duration_since(start) < Duration::from_millis(250));
        assert!(start.elapsed() < Duration::from_millis(590), "the two scripts ran in parallel");
    }

    fn headers(pairs: &[(&str, &str)]) -> Headers {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn req() -> Json {
        json!({ "method": "GET", "url": "http://a/b", "headers": { "A": "1", "B": "2" }, "body": "x" })
    }

    fn res() -> Json {
        json!({ "status": 200, "headers": { "content-type": "application/json" }, "body": "{\"a\":[1,2],\"s\":\"hi\"}", "responseTime": 12 })
    }

    async fn pre(script: &str) -> Json {
        execute_ipc_script(json!({ "script": script, "context": "pre", "request": req(), "envVars": { "K": "v" } }))
            .await
    }

    async fn test(script: &str) -> Json {
        execute_ipc_script(json!({ "script": script, "context": "test", "response": res(), "envVars": { "K": "v" } }))
            .await
    }

    fn tests_of(out: &Json) -> Vec<(String, bool, Option<String>)> {
        out["testResults"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                (
                    t["name"].as_str().unwrap().to_string(),
                    t["passed"].as_bool().unwrap(),
                    t.get("error").and_then(Json::as_str).map(str::to_string),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn pre_script_mutates_request_and_env() {
        let out = pre("bc.request.url += '/x'; bc.request.method = 'POST'; bc.request.body = 'y';\
            bc.request.headers.set('C','3'); bc.request.headers.unset('A');\
            bc.environment.set('N', 5); bc.environment.unset('K');")
        .await;
        assert_eq!(out["request"]["url"], "http://a/b/x");
        assert_eq!(out["request"]["method"], "POST");
        assert_eq!(out["request"]["body"], "y");
        let keys: Vec<&String> = out["request"]["headers"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["B", "C"]);
        assert_eq!(out["envVars"], json!({ "N": "5" }));
        assert!(out.get("error").is_none());
    }

    #[tokio::test]
    async fn pre_script_error_keeps_partial_state() {
        let out = pre("bc.request.url = 'u'; bc.environment.set('Z','1'); throw new Error('boom');").await;
        assert_eq!(out["error"], "Error: boom");
        assert_eq!(out["request"]["url"], "u");
        assert_eq!(out["envVars"]["Z"], "1");
    }

    #[tokio::test]
    async fn thrown_error_vs_string_vs_syntax() {
        assert_eq!(pre("throw 'plain'").await["error"], "plain");
        assert_eq!(pre("throw 42").await["error"], "42");
        assert_eq!(pre("throw new TypeError('tt')").await["error"], "TypeError: tt");
        let syn = pre("var = ;").await;
        assert!(syn["error"].as_str().is_some_and(|s| !s.is_empty()));
        assert_eq!(syn["request"]["url"], "http://a/b");
    }

    #[tokio::test]
    async fn post_script_reads_response() {
        let out = execute_ipc_script(json!({
            "script": "var j = bc.response.json(); bc.environment.set('first', j.a[0]); bc.environment.set('st', bc.response.status);\
                bc.environment.set('h', bc.response.headers.get('content-type'));",
            "context": "post", "response": res(), "envVars": {}
        }))
        .await;
        assert_eq!(out["envVars"], json!({ "first": "1", "st": "200", "h": "application/json" }));
        assert_eq!(out["response"]["status"], 200);
        let bad = execute_ipc_script(json!({
            "script": "bc.environment.set('n', bc.response.json())", "context": "post",
            "response": { "status": 1, "headers": {}, "body": "nope" }, "envVars": {}
        }))
        .await;
        assert_eq!(bad["envVars"]["n"], "null");
    }

    #[tokio::test]
    async fn invalid_context_fallback() {
        let out = execute_ipc_script(json!({ "script": "1", "context": "pre", "envVars": { "a": "b" } })).await;
        assert_eq!(
            out,
            json!({ "envVars": { "a": "b" }, "error": "Invalid script context or missing request/response" })
        );
        let out =
            execute_ipc_script(json!({ "script": "1", "context": "test", "request": req(), "envVars": {} })).await;
        assert!(out["error"].is_string());
    }

    #[tokio::test]
    async fn test_script_passes_and_fails() {
        let out = test(
            "bc.test('ok', function(){ bc.expect(bc.response.status).to.equal(200); });\
             bc.test('bad', function(){ bc.expect(bc.response.status).to.equal(404); });\
             bc.test('thrown', function(){ throw 'str'; });",
        )
        .await;
        let t = tests_of(&out);
        assert_eq!(t[0], ("ok".into(), true, None));
        assert_eq!(t[1], ("bad".into(), false, Some("expected 200 to equal 404".into())));
        assert_eq!(t[2], ("thrown".into(), false, Some("str".into())));
        assert!(out["testResults"][0]["durationMs"].is_number());
        assert_eq!(out["testLogs"], json!([]));
        assert_eq!(out["envVars"], json!({ "K": "v" }));
    }

    #[tokio::test]
    async fn scripts_written_the_postman_way_run() {
        let script = "bc.test('response', function(){ bc.response.to.have.status(200); bc.response.to.be.ok; bc.response.to.be.success;\
               bc.response.to.not.be.error; bc.response.to.be.json; bc.response.to.have.jsonBody();\
               bc.response.to.have.header('Content-Type'); bc.response.to.have.header('content-type', 'application/json');\
               bc.response.to.have.body(/hi/); bc.expect(bc.response).to.have.status(200); bc.expect(bc.response.code).to.eql(200); });\
             bc.test('chai', function(){ var j = bc.response.json(); bc.expect(j.a).to.deep.equal([1,2]); bc.expect(j.s).to.exist;\
               bc.expect(j.none).to.not.exist; bc.expect(j).to.have.property('s', 'hi'); });\
             bc.test('scopes', function(){ bc.globals.set('g', 1); bc.collectionVariables.set('c', 2);\
               bc.expect(bc.variables.get('g')).to.equal('1'); bc.expect(bc.variables.replaceIn('{{K}}/{{ c }}/{{no}}')).to.equal('v/2/{{no}}'); });\
             bc.test('jar', function(){ var jar = bc.cookies.jar(), seen; jar.set('http://o.test/', 'far', 'x');\
               jar.set('http://o.test/', { name: 'obj', value: 'y' }, function(){}); jar.get('http://a/b', 'sid', function(e, v){ seen = v; });\
               bc.expect(seen).to.equal('s1'); jar.unset('http://o.test/', 'far'); });\
             bc.test('fail-status', function(){ bc.response.to.have.status(404); });\
             bc.test('fail-header', function(){ bc.response.to.have.header('x-none'); });\
             lp.test('old name', function(){ lp.expect(lp).to.equal(bc); });";
        let out = execute_ipc_script(json!({ "script": script, "context": "test", "request": req(), "response": res(),
            "envVars": { "K": "v" }, "cookies": { "sid": "s1" } }))
        .await;
        let t = tests_of(&out);
        let failed: Vec<_> = t.iter().filter(|t| !t.1).collect();
        assert_eq!(failed.len(), 2, "{t:?}");
        assert_eq!(t[4].2.as_deref(), Some("expected status 200 to be 404"));
        assert_eq!(t[5].2.as_deref(), Some("expected response to have header \"x-none\""));
        assert_eq!(out["envVars"], json!({ "K": "v", "g": "1", "c": "2" }));
        assert_eq!(out["cookies"][1], json!({ "op": "set", "name": "obj", "value": "y", "url": "http://o.test/" }));
        assert_eq!(out["cookies"][2], json!({ "op": "unset", "name": "far", "url": "http://o.test/" }));

        let out = pre(
            "bc.request.headers.add({ key: 'C', value: '3' }); bc.request.headers.upsert({ key: 'a', value: '9' });\
            bc.request.headers.remove('B');",
        )
        .await;
        assert_eq!(out["request"]["headers"], json!({ "a": "9", "C": "3" }));
    }

    #[tokio::test]
    async fn expect_chain_sample() {
        let out = test(
            "var e = bc.expect;\
             bc.test('equal', function(){ e(1).to.equal(1); e(1).to.not.equal(2); });\
             bc.test('eql', function(){ e({a:[1]}).to.eql({a:[1]}); });\
             bc.test('type', function(){ e([]).to.be.an('array'); e({}).to.be.an('object'); e(null).to.be.a('null'); e('s').to.be.a('string'); });\
             bc.test('include', function(){ e('abc').to.include('b'); e([1,2]).to.include(2); e({k:1}).to.include('k'); });\
             bc.test('property', function(){ e({k:1}).to.have.property('k'); });\
             bc.test('length', function(){ e([1,2,3]).to.have.lengthOf(3); e('ab').to.have.length(2); e([1,2]).to.have.lengthAbove(1); });\
             bc.test('numbers', function(){ e(5).to.be.above(4); e(5).to.be.below(6); e(5).to.be.at.least(5); e(5).to.be.at.most(5); });\
             bc.test('match', function(){ e('hello').to.match(/ell/); e('hello').to.match('^h'); });\
             bc.test('status', function(){ e(200).to.have.status(200); });\
             bc.test('empty', function(){ e('').to.be.empty; e([]).to.be.empty; e({}).to.be.empty; e([1]).to.not.be.empty; });\
             bc.test('oneOf', function(){ e(2).to.be.oneOf([1,2]); });\
             bc.test('flags', function(){ e(1).to.be.ok; e(true).to.be.true; e(false).to.be.false; e(null).to.be.null; e(undefined).to.be.undefined; e(0).to.not.be.ok; });\
             bc.test('fail-above', function(){ e(1).to.be.above(2); });\
             bc.test('fail-not', function(){ e('a').not.to.be.a('string'); });\
             bc.test('fail-property', function(){ e({}).to.have.property('z'); });\
             bc.test('fail-oneOf', function(){ e(3).to.be.oneOf([1,2]); });",
        )
        .await;
        for (name, passed, error) in tests_of(&out) {
            assert_eq!(passed, !name.starts_with("fail-"), "{name}: {error:?}");
        }
        let t = tests_of(&out);
        assert_eq!(t[12].2.as_deref(), Some("expected 1 to be above 2"));
        assert_eq!(t[13].2.as_deref(), Some("expected \"a\" not to be a(n) string"));
        assert_eq!(t[14].2.as_deref(), Some("expected object to have property \"z\""));
        assert_eq!(t[15].2.as_deref(), Some("expected 3 to be one of [1,2]"));
    }

    #[tokio::test]
    async fn console_capture_and_test_helpers() {
        let out = test(
            "console.log('a', {b:1}, 2); console.warn('w'); console.error('e'); console.info('i');\
             bc.test('t', function(){ bc.expect(bc.response.headers.get('Content-Type')).to.equal('application/json');\
               bc.expect(bc.response.headers.has('CONTENT-TYPE')).to.be.true; bc.expect(bc.response.responseTime).to.equal(12);\
               bc.expect(bc.response.text()).to.include('hi'); bc.expect(bc.environment.has('K')).to.be.true; });",
        )
        .await;
        assert_eq!(out["testLogs"], json!(["a {\"b\":1} 2", "[WARN] w", "[ERROR] e", "i"]));
        assert_eq!(tests_of(&out), vec![("t".to_string(), true, None)]);
    }

    #[tokio::test]
    async fn console_and_bare_test_helpers_exist_in_every_context() {
        let out =
            pre("console.log('a', 1); bc.environment.set('c', typeof console + typeof test + typeof expect)").await;
        assert_eq!(out["envVars"]["c"], "objectfunctionfunction");
        assert_eq!(out["logs"], json!(["a 1"]));
        let out = test("test('bare', () => { expect(bc.response.status).to.equal(200); expect(1).to.be.ok; });").await;
        assert_eq!(tests_of(&out), vec![("bare".to_string(), true, None)]);
    }

    #[tokio::test]
    async fn post_and_test_scripts_see_the_request_and_set_variables() {
        let both = |context: &str, script: &str| {
            execute_ipc_script(json!({
                "script": script, "context": context, "request": req(), "response": res(), "envVars": {}
            }))
        };
        let out = both(
            "post",
            "bc.environment.set('m', bc.request.method); bc.environment.set('u', bc.request.url);             bc.environment.set('t', bc.response.responseTime); bc.environment.set('ct', bc.response.headers.get('Content-Type'));             bc.environment.set('txt', bc.response.text().length); bc.environment.set('has', bc.response.headers.has('CONTENT-TYPE'));",
        )
        .await;
        assert_eq!(
            out["envVars"],
            json!({ "m": "GET", "u": "http://a/b", "t": "12", "ct": "application/json", "txt": "20", "has": "true" })
        );
        let out = both(
            "test",
            "bc.environment.set('fromTest', 1); bc.test('r', () => expect(bc.request.headers.get('a')).to.equal('1'));",
        )
        .await;
        assert_eq!(out["envVars"], json!({ "fromTest": "1" }));
        assert_eq!(tests_of(&out), vec![("r".to_string(), true, None)]);
        let out = pre("bc.request.headers.set('a', '9'); bc.environment.set('h', bc.request.headers.get('A'));").await;
        assert_eq!(out["request"]["headers"], json!({ "B": "2", "a": "9" }));
        assert_eq!(out["envVars"]["h"], "9");
    }

    #[tokio::test]
    async fn test_script_uncaught_errors() {
        let out = test("throw new Error('top')").await;
        assert_eq!(tests_of(&out), vec![("Script execution".to_string(), false, Some("Error: top".to_string()))]);
        assert_eq!(out["testResults"][0]["durationMs"], 0);
        let out = test("bc.test('one', function(){}); throw new Error('late')").await;
        assert_eq!(tests_of(&out).len(), 1);
        assert_eq!(out["testLogs"], json!(["[ERROR] Uncaught: Error: late"]));
    }

    #[tokio::test]
    async fn proxy_scripts() {
        let h = headers(&[("a", "1"), ("b", "2")]);
        let r = execute_request_script(
            "delete bc.request.headers.a; bc.request.headers['x-new'] = 'y'; bc.request.body = bc.request.body + '!';",
            &h,
            "hi",
        )
        .await;
        assert!(r.error.is_none());
        assert_eq!(r.headers.keys().collect::<Vec<_>>(), ["b", "x-new"]);
        assert_eq!(r.body, "hi!");
        let r = execute_response_script(
            "bc.response.headers.b = '3'; bc.response.body = 'new'; throw new Error('late');",
            &h,
            "hi",
        )
        .await;
        assert_eq!(r.error.as_deref(), Some("Error: late"));
        assert_eq!(r.headers["b"], "3");
        assert_eq!(r.body, "new");
        // The other side is not defined.
        let r = execute_request_script("bc.response.body = 'x'", &h, "hi").await;
        assert!(r.error.is_some());
        assert_eq!(r.body, "hi");
    }

    #[tokio::test]
    async fn infinite_loop_is_interrupted() {
        let out = execute_ipc_script_with(
            json!({ "script": "bc.request.url = 'set'; while(true){}", "context": "pre", "request": req(), "envVars": {} }),
            Duration::from_millis(150),
        )
        .await;
        assert_eq!(out["error"], "Script execution timed out after 150ms");
        assert_eq!(out["request"]["url"], "set");
        let out = execute_ipc_script_with(
            json!({ "script": "bc.test('a',function(){}); try { while(true){} } catch (e) {}", "context": "test", "response": res(), "envVars": {} }),
            Duration::from_millis(150),
        )
        .await;
        assert_eq!(out["testLogs"], json!(["[ERROR] Uncaught: Script execution timed out after 150ms"]));
    }

    #[tokio::test]
    async fn memory_blowup_is_contained() {
        let out = execute_ipc_script_with(
            json!({ "script": "var a=[]; while(true){ a.push(new Array(100000).fill(1)); }", "context": "pre", "request": req(), "envVars": {} }),
            Duration::from_secs(20),
        )
        .await;
        assert!(out["error"].is_string(), "{out}");
        assert_eq!(out["request"]["url"], "http://a/b");
        let out = pre("var s='x'; while(true){ s += s; }").await;
        assert!(out["error"].is_string());
    }

    #[tokio::test]
    async fn deep_recursion_is_an_error() {
        let out = pre("function f(){ return f()+1 } f()").await;
        assert!(out["error"].is_string());
    }

    #[tokio::test]
    async fn sandbox_has_no_host_access() {
        let out = pre(
            "var names = ['require','process','fetch','setTimeout','setInterval','setImmediate','XMLHttpRequest','Buffer','module','exports','std','os','__input','__dirname'];\
             var present = names.filter(function(n){ return typeof globalThis[n] !== 'undefined'; });\
             bc.environment.set('present', present.join(','));",
        )
        .await;
        assert_eq!(out["envVars"]["present"], "");
        assert!(pre("require('fs')").await["error"].is_string());
        assert!(pre("import('fs')").await.get("request").is_some());
    }

    #[tokio::test]
    async fn executions_do_not_share_globals() {
        pre("globalThis.leak = 1; var other = 2;").await;
        let out = pre("bc.environment.set('v', typeof leak + typeof other)").await;
        assert_eq!(out["envVars"]["v"], "undefinedundefined");
    }

    /// A one-shot HTTP server on localhost: after `delay` it answers the first request with a fixed JSON body
    /// and hands that request's text back.
    async fn serve_once(delay: Duration) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/p", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = socket.read(&mut buf).await.unwrap();
            tokio::time::sleep(delay).await;
            let body = "{\"ok\":true}";
            let reply = format!(
                "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        (url, task)
    }

    #[tokio::test]
    async fn send_request_returns_the_response_with_or_without_a_callback() {
        let (url, _) = serve_once(Duration::ZERO).await;
        let out = pre(&format!(
            "var seen; bc.sendRequest('{url}', function(e, r){{ seen = [e, r.code, r.json().ok]; bc.expect(r).to.have.status(201); }});\
             bc.environment.set('cb', seen.join());"
        ))
        .await;
        assert_eq!(out["envVars"]["cb"], ",201,true", "{out}");
        assert!(out.get("error").is_none(), "{out}");

        let (url, _) = serve_once(Duration::ZERO).await;
        let out = pre(&format!(
            "var r = bc.sendRequest({{ url: '{url}' }}); bc.environment.set('v', r.code + r.headers.get('Content-Type') + r.text());"
        ))
        .await;
        assert_eq!(out["envVars"]["v"], "201application/json{\"ok\":true}", "{out}");
    }

    #[tokio::test]
    async fn send_request_posts_a_body_and_headers() {
        let (url, server) = serve_once(Duration::ZERO).await;
        let out = test(&format!(
            "bc.sendRequest({{ url: '{url}', method: 'POST', header: [{{ key: 'X-A', value: '1' }}],\
               body: {{ mode: 'raw', raw: '{{\"a\":1}}' }} }}, function(){{}});"
        ))
        .await;
        assert!(out.get("error").is_none(), "{out}");
        let got = server.await.unwrap();
        assert!(got.starts_with("POST /p "), "{got}");
        assert!(got.to_ascii_lowercase().contains("x-a: 1"), "{got}");
        assert!(got.ends_with("{\"a\":1}"), "{got}");

        let (url, server) = serve_once(Duration::ZERO).await;
        pre(&format!(
            "bc.sendRequest({{ url: '{url}', method: 'POST', body: {{ mode: 'urlencoded', urlencoded: [{{ key: 'k', value: 'a b' }}] }} }});"
        ))
        .await;
        let got = server.await.unwrap();
        assert!(
            got.ends_with("k=a%20b") && got.to_ascii_lowercase().contains("application/x-www-form-urlencoded"),
            "{got}"
        );
    }

    #[tokio::test]
    async fn send_request_reports_a_failed_connection_to_the_callback() {
        let free = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", free.local_addr().unwrap());
        drop(free);
        let out = pre(&format!(
            "bc.sendRequest('{url}', function(e, r){{ bc.environment.set('e', (e instanceof Error) + ':' + r); }});"
        ))
        .await;
        assert_eq!(out["envVars"]["e"], "true:null", "{out}");
        let out = pre(&format!("bc.sendRequest('{url}');")).await;
        assert!(out["error"].is_string());
    }

    #[tokio::test]
    async fn send_request_is_for_request_and_flow_scripts_not_proxy_ones() {
        let out = execute_request_script("bc.sendRequest('http://127.0.0.1:1/')", &headers(&[]), "").await;
        assert_eq!(out.error.as_deref(), Some("Error: bc.sendRequest is not available in proxy scripts"));
        let out = execute_flow_script(
            "bc.sendRequest('http://127.0.0.1:1/', function(e){ bc.output = !!e; })",
            json!({ "output": 1 }),
            json!({}),
        )
        .await;
        assert_eq!(out["output"], true);
    }

    #[tokio::test]
    async fn send_request_is_capped_per_run() {
        let out = pre("var n = 0; for (var i = 0; i < 22; i++) bc.sendRequest('not a url', function(e){ if (/limited/.test(e.message)) n++; });\
            bc.environment.set('n', n);")
        .await;
        assert_eq!(out["envVars"]["n"], "2", "{out}");
    }

    #[tokio::test]
    async fn waiting_on_a_request_does_not_use_up_the_script_time() {
        let (url, _) = serve_once(Duration::from_millis(300)).await;
        let opts = json!({ "script": format!("bc.sendRequest('{url}'); for (var i = 0; i < 100000; i++);"),
            "context": "pre", "request": req(), "envVars": {} });
        let out = execute_ipc_script_with(opts, Duration::from_millis(100)).await;
        assert!(out.get("error").is_none(), "{out}");
        let out = execute_ipc_script_with(
            json!({ "script": "for(;;);", "context": "pre", "request": req(), "envVars": {} }),
            Duration::from_millis(100),
        )
        .await;
        assert!(out["error"].as_str().unwrap().contains("timed out"));
    }

    #[tokio::test]
    async fn postman_cookie_calls_read_and_write() {
        let out = execute_ipc_script(json!({
            "script": "bc.environment.set('g', bc.cookies.get('sid') + bc.cookies.has('sid'));\
                bc.cookies.jar().set('http://o.test/', 'n', 'v', function(e, c){ bc.environment.set('c', e + c.name); });",
            "context": "pre", "request": req(), "envVars": {}, "cookies": { "sid": "s1" } }))
        .await;
        assert_eq!(out["envVars"], json!({ "g": "s1true", "c": "nulln" }), "{out}");
        assert_eq!(out["cookies"][0], json!({ "op": "set", "name": "n", "value": "v", "url": "http://o.test/" }));
    }
}
