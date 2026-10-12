//! SOAP commands: fetch a WSDL, execute an envelope, and send a saved or inline request with its
//! `{{variables}}` resolved.

use std::time::Instant;

use serde_json::{Value, json};

use crate::{
    env::{load_merged_environment, resolve_headers, resolve_vars},
    kernel::CommandRegistry,
    protocol::EngineError,
    rest::direct_http::{exchange, header_pairs},
    utils::json::{opt_str, str_of},
    workspace::config::load_entity,
};

pub fn register(registry: &mut CommandRegistry) {
    registry.register("soap.fetchWsdl", |params: Value, _| async move {
        Ok(match exchange("GET", str_of(&params, "url"), &[], None).await {
            Ok(response) => json!({ "ok": true, "content": response.body }),
            Err(error) if error.is_empty() => json!({ "ok": false, "error": "Failed to fetch WSDL" }),
            Err(error) => json!({ "ok": false, "error": error }),
        })
    });

    registry.register("soap.execute", |params: Value, _| async move {
        let mut headers = vec![("Content-Type".to_string(), "text/xml; charset=utf-8".to_string())];
        headers.extend(header_pairs(params.get("headers").unwrap_or(&Value::Null)));
        let action = str_of(&params, "soapAction");
        if !action.is_empty() {
            headers.push(("SOAPAction".to_string(), action.to_string()));
        }
        let start = Instant::now();
        let response = exchange("POST", str_of(&params, "endpointUrl"), &headers, Some(str_of(&params, "body")))
            .await
            .map_err(EngineError::engine)?;
        Ok(json!({
            "status": response.status,
            "headers": response.headers,
            "body": response.body,
            "durationMs": start.elapsed().as_millis() as u64,
        }))
    });

    registry.register("soap.send", |params: Value, ctx| async move {
        let ws_id = str_of(&params, "workspaceId");
        let stored = match opt_str(&params, "id") {
            Some(id) => load_entity(ws_id, "soapRequests", id).ok_or_else(|| {
                EngineError::not_found(format!("No saved soapRequests entity \"{id}\" in workspace \"{ws_id}\"."))
            })?,
            None => params.get("request").cloned().unwrap_or(Value::Null),
        };
        let env = load_merged_environment(ws_id);
        let env = env.as_ref();
        let payload = json!({
            "endpointUrl": resolve_vars(str_of(&stored, "endpointUrl"), env),
            "soapAction": resolve_vars(str_of(&stored, "soapAction"), env),
            "headers": resolve_headers(stored.get("headers").unwrap_or(&Value::Null), env),
            "body": resolve_vars(str_of(&stored, "body"), env),
        });
        let mut result =
            crate::cookies::scoped(ws_id, ctx.registry.invoke("soap.execute", payload, ctx.clone())).await?;
        crate::vault::redact_value(ws_id, &mut result);
        Ok(result)
    });
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use super::*;
    use crate::{kernel::CommandContext, rest::test_server, testing::scratch_data_root, workspace::fs::write_entity};

    async fn call(action: &str, payload: Value) -> crate::protocol::Result<Value> {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        let registry = Arc::new(registry);
        let ctx = CommandContext::internal(registry.clone());
        registry.invoke(action, payload, ctx).await
    }

    #[tokio::test]
    async fn execute_sends_the_envelope_with_the_soap_headers() {
        let base = test_server::start(HashMap::new()).await;
        let result = call(
            "soap.execute",
            json!({ "endpointUrl": format!("{base}/svc"), "soapAction": "urn:Do", "headers": {}, "body": "<x/>" }),
        )
        .await
        .unwrap();
        assert_eq!(result["status"], 200);
        let echoed: Value = serde_json::from_str(result["body"].as_str().unwrap()).unwrap();
        assert_eq!(echoed["headers"]["content-type"], "text/xml; charset=utf-8");
        assert_eq!(echoed["headers"]["soapaction"], "urn:Do");
        assert_eq!(echoed["body"], "<x/>");
    }

    #[tokio::test]
    async fn fetch_wsdl_returns_the_content_or_an_error_value() {
        let canned = HashMap::from([("/w".to_string(), ("text/xml".to_string(), "<wsdl/>".to_string()))]);
        let base = test_server::start(canned).await;
        let ok = call("soap.fetchWsdl", json!({ "url": format!("{base}/w") })).await.unwrap();
        assert_eq!(ok, json!({ "ok": true, "content": "<wsdl/>" }));
        let failed = call("soap.fetchWsdl", json!({ "url": "nope" })).await.unwrap();
        assert_eq!(failed, json!({ "ok": false, "error": "Invalid URL" }));
    }

    #[tokio::test]
    async fn send_resolves_a_saved_request_and_a_missing_one_is_not_found() {
        let root = scratch_data_root();
        let base = test_server::start(HashMap::new()).await;
        write_entity(
            root.ws_id,
            "soapRequests",
            "s1",
            &json!({
                "id": "s1", "name": "s", "endpointUrl": format!("{base}/s"), "soapAction": "",
                "headers": {}, "body": "<y/>",
            }),
            None,
        )
        .unwrap();
        let result = call("soap.send", json!({ "workspaceId": root.ws_id, "id": "s1" })).await.unwrap();
        let echoed: Value = serde_json::from_str(result["body"].as_str().unwrap()).unwrap();
        assert_eq!(echoed["body"], "<y/>");
        assert!(echoed["headers"].get("soapaction").is_none());
        let missing = call("soap.send", json!({ "workspaceId": root.ws_id, "id": "nope" })).await.unwrap_err();
        assert_eq!(missing.code, crate::protocol::ErrorCode::NotFound);
    }
}
