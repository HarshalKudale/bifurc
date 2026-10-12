//! `request.send`: the request pipeline on the wire, for an inline request or a saved one.

use serde_json::{Value, json};

use super::{
    pipeline::{PipelineScripts, run_request_pipeline},
    request_body::encode_request_body,
};
use crate::{
    kernel::CommandRegistry,
    protocol::EngineError,
    utils::json::{array_of, is_truthy, opt_str, str_of},
    workspace::config::load_entity,
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
    request
}

fn scripts_of(source: &Value, pre: &str, post: &str, test: &str) -> PipelineScripts {
    let get = |key: &str| opt_str(source, key).map(str::to_string);
    PipelineScripts { pre: get(pre), post: get(post), test: get(test) }
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register("request.send", |params: Value, ctx| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let (request, scripts) = if let Some(id) = opt_str(&params, "id") {
            let entity = load_entity(&ws_id, "requests", id).ok_or_else(|| {
                EngineError::not_found(format!("No saved requests entity \"{id}\" in workspace \"{ws_id}\"."))
            })?;
            let mut scripts = scripts_of(&entity, "preScript", "postScript", "testScript");
            for skipped in array_of(&params, "skipScripts") {
                match skipped.as_str() {
                    Some("pre") => scripts.pre = None,
                    Some("post") => scripts.post = None,
                    Some("test") => scripts.test = None,
                    _ => {}
                }
            }
            (request_from_entity(&entity), scripts)
        } else {
            let inline = params.get("request").cloned().unwrap_or(Value::Null);
            let scripts = scripts_of(params.get("scripts").unwrap_or(&Value::Null), "pre", "post", "test");
            (inline, scripts)
        };

        let jar = ws_id.clone();
        let result = run_request_pipeline(&ws_id, &request, &scripts, |resolved| async move {
            let encoded = encode_request_body(&resolved);
            let sent = ctx.registry.invoke(
                "request.replay",
                json!({
                    "method": resolved["method"],
                    "url": resolved["url"],
                    "headers": encoded.headers,
                    "body": encoded.body_base64,
                }),
                ctx.clone(),
            );
            crate::cookies::scoped(&jar, sent).await
        })
        .await?;

        let mut out = json!({
            "status": result["response"]["status"],
            "headers": result["response"]["headers"],
            "body": result["response"]["body"],
            "durationMs": result["durationMs"],
            "request": result["request"],
            "tests": result["tests"],
            "testLogs": result["testLogs"],
        });
        for key in ["preScriptError", "postScriptError"] {
            if let Some(error) = result.get(key) {
                out[key] = error.clone();
            }
        }
        Ok(out)
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
            [("status 200", true), ("post var visible", true), ("time + headers", true), ("json body", true), ("expected failure", false)]
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
}
