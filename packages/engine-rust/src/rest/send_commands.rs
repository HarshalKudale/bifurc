//! `request.send`: the request pipeline on the wire, for an inline request or a saved one.

use serde_json::{Value, json};

use super::{
    pipeline::{PipelineScripts, run_request_pipeline},
    request_body::encode_request_body,
};
use crate::{
    kernel::CommandRegistry,
    protocol::EngineError,
    utils::json::{is_truthy, opt_str, str_of},
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
            let scripts = scripts_of(&entity, "preScript", "postScript", "testScript");
            (request_from_entity(&entity), scripts)
        } else {
            let inline = params.get("request").cloned().unwrap_or(Value::Null);
            let scripts = scripts_of(params.get("scripts").unwrap_or(&Value::Null), "pre", "post", "test");
            (inline, scripts)
        };

        let result = run_request_pipeline(&ws_id, &request, &scripts, |resolved| async move {
            let encoded = encode_request_body(&resolved);
            ctx.registry
                .invoke(
                    "request.replay",
                    json!({
                        "method": resolved["method"],
                        "url": resolved["url"],
                        "headers": encoded.headers,
                        "body": encoded.body_base64,
                    }),
                    ctx.clone(),
                )
                .await
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
