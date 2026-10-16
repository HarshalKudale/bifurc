//! GraphQL commands: introspect an endpoint, execute an operation, and send a saved or inline request with
//! its `{{variables}}` resolved.

use std::time::Instant;

use serde_json::{Map, Value, json};

use crate::{
    env::{load_merged_environment, resolve_headers, resolve_vars},
    kernel::CommandRegistry,
    protocol::EngineError,
    rest::direct_http::{exchange_with, header_pairs},
    utils::{
        http::RequestSettings,
        json::{opt_str, str_of},
    },
    workspace::{config::load_entity, response_history},
};

const INTROSPECTION_QUERY: &str = r#"
      query IntrospectionQuery {
        __schema {
          queryType { name }
          mutationType { name }
          subscriptionType { name }
          types { ...FullType }
          directives { name description locations args { ...InputValue } }
        }
      }
      fragment FullType on __Type {
        kind name description
        fields(includeDeprecated: true) { name description args { ...InputValue } type { ...TypeRef } isDeprecated deprecationReason }
        inputFields { ...InputValue }
        interfaces { ...TypeRef }
        enumValues(includeDeprecated: true) { name description isDeprecated deprecationReason }
        possibleTypes { ...TypeRef }
      }
      fragment InputValue on __InputValue { name description type { ...TypeRef } defaultValue }
      fragment TypeRef on __Type { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name } } } } } } } }
    "#;

/// The JSON POST headers followed by the caller's, which win on a name clash.
fn post_headers(headers: &Value) -> Vec<(String, String)> {
    let mut out = vec![("Content-Type".to_string(), "application/json".to_string())];
    out.extend(header_pairs(headers));
    out
}

/// The body of an `execute`: `variables` only when they parse as JSON, `operationName` only when not empty.
fn execute_body(query: &str, variables: &str, operation_name: &str) -> String {
    let mut body = Map::new();
    body.insert("query".into(), json!(query));
    if !variables.trim().is_empty() {
        if let Ok(parsed) = serde_json::from_str::<Value>(variables) {
            body.insert("variables".into(), parsed);
        }
    }
    if !operation_name.is_empty() {
        body.insert("operationName".into(), json!(operation_name));
    }
    Value::Object(body).to_string()
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register("graphql.introspect", |params: Value, _| async move {
        let body = json!({ "query": INTROSPECTION_QUERY }).to_string();
        let headers = post_headers(params.get("headers").unwrap_or(&Value::Null));
        let settings = RequestSettings::resolve(params.get("settings"));
        Ok(match exchange_with("POST", str_of(&params, "url"), &headers, Some(&body), &settings).await {
            Ok(response) => json!({ "ok": true, "sdl": response.body }),
            Err(error) => json!({ "ok": false, "error": error }),
        })
    });

    registry.register("graphql.execute", |params: Value, _| async move {
        let body =
            execute_body(str_of(&params, "query"), str_of(&params, "variables"), str_of(&params, "operationName"));
        let headers = post_headers(params.get("headers").unwrap_or(&Value::Null));
        let start = Instant::now();
        let settings = RequestSettings::resolve(params.get("settings"));
        let response = exchange_with("POST", str_of(&params, "url"), &headers, Some(&body), &settings)
            .await
            .map_err(EngineError::engine)?;
        Ok(json!({
            "status": response.status,
            "headers": response.headers,
            "body": response.body,
            "durationMs": start.elapsed().as_millis() as u64,
        }))
    });

    registry.register("graphql.send", |params: Value, ctx| async move {
        let ws_id = str_of(&params, "workspaceId");
        let stored = match opt_str(&params, "id") {
            Some(id) => load_entity(ws_id, "graphqlRequests", id).ok_or_else(|| {
                EngineError::not_found(format!("No saved graphqlRequests entity \"{id}\" in workspace \"{ws_id}\"."))
            })?,
            None => params.get("request").cloned().unwrap_or(Value::Null),
        };
        let env = load_merged_environment(ws_id);
        let env = env.as_ref();
        let mut payload = json!({
            "url": resolve_vars(str_of(&stored, "endpointUrl"), env),
            "headers": resolve_headers(stored.get("headers").unwrap_or(&Value::Null), env),
            "query": resolve_vars(str_of(&stored, "query"), env),
            "variables": resolve_vars(str_of(&stored, "variables"), env),
            "operationName": resolve_vars(str_of(&stored, "operationName"), env),
        });
        if let Some(settings) = stored.get("settings").filter(|v| v.is_object()) {
            payload["settings"] = settings.clone();
        }
        let url = str_of(&payload, "url").to_string();
        let outcome = crate::cookies::scoped(ws_id, ctx.registry.invoke("graphql.execute", payload, ctx.clone())).await;
        response_history::record_text(ws_id, &params, "POST", &url, &outcome);
        let mut result = outcome?;
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

    #[test]
    fn the_body_keeps_valid_variables_and_a_named_operation() {
        let body: Value = serde_json::from_str(&execute_body("{ a }", r#"{"x":1}"#, "Op")).unwrap();
        assert_eq!(body, json!({ "query": "{ a }", "variables": { "x": 1 }, "operationName": "Op" }));
        let body: Value = serde_json::from_str(&execute_body("{ a }", "not json", "")).unwrap();
        assert_eq!(body, json!({ "query": "{ a }" }));
        let body: Value = serde_json::from_str(&execute_body("{ a }", "  ", "")).unwrap();
        assert_eq!(body, json!({ "query": "{ a }" }));
    }

    #[tokio::test]
    async fn execute_posts_the_operation_and_reports_the_answer() {
        let base = test_server::start(HashMap::new()).await;
        let result = call(
            "graphql.execute",
            json!({
                "url": format!("{base}/graphql"), "headers": { "X-A": "b" },
                "query": "{ a }", "variables": "", "operationName": "",
            }),
        )
        .await
        .unwrap();
        assert_eq!(result["status"], 200);
        let echoed: Value = serde_json::from_str(result["body"].as_str().unwrap()).unwrap();
        assert_eq!(echoed["method"], "POST");
        assert_eq!(echoed["headers"]["content-type"], "application/json");
        assert_eq!(echoed["headers"]["x-a"], "b");
        assert_eq!(
            serde_json::from_str::<Value>(echoed["body"].as_str().unwrap()).unwrap(),
            json!({ "query": "{ a }" })
        );
        assert!(result["durationMs"].is_u64());
    }

    #[tokio::test]
    async fn introspect_returns_the_raw_text_or_an_error_value() {
        let canned = HashMap::from([("/g".to_string(), ("application/json".to_string(), "{\"data\":1}".to_string()))]);
        let base = test_server::start(canned).await;
        let ok = call("graphql.introspect", json!({ "url": format!("{base}/g"), "headers": {} })).await.unwrap();
        assert_eq!(ok, json!({ "ok": true, "sdl": "{\"data\":1}" }));
        let failed = call("graphql.introspect", json!({ "url": "nope", "headers": {} })).await.unwrap();
        assert_eq!(failed, json!({ "ok": false, "error": "Invalid URL" }));
    }

    #[tokio::test]
    async fn send_resolves_a_saved_request_and_a_missing_one_is_not_found() {
        let root = scratch_data_root();
        let base = test_server::start(HashMap::new()).await;
        write_entity(
            root.ws_id,
            "graphqlRequests",
            "g1",
            &json!({
                "id": "g1", "name": "g", "endpointUrl": format!("{base}/q"), "headers": {},
                "query": "{ a }", "variables": "", "operationName": "",
            }),
            None,
        )
        .unwrap();
        let result = call("graphql.send", json!({ "workspaceId": root.ws_id, "id": "g1" })).await.unwrap();
        assert_eq!(result["status"], 200);
        let missing = call("graphql.send", json!({ "workspaceId": root.ws_id, "id": "nope" })).await.unwrap_err();
        assert_eq!(missing.code, crate::protocol::ErrorCode::NotFound);
    }
}
