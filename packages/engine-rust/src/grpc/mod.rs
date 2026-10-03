//! gRPC commands. There is no gRPC runtime yet: `grpc.execute` and `grpc.reflect` answer that, and
//! `grpc.send` resolves a saved or inline request and hands it to `grpc.execute`.

use serde_json::{Map, Value, json};

use crate::{
    env::{load_merged_environment, resolve_vars},
    kernel::CommandRegistry,
    protocol::EngineError,
    utils::json::{opt_str, str_of},
    workspace::config::load_entity,
};

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("grpc.execute", |_, _| {
        Ok(json!({
            "ok": false,
            "error": "gRPC runtime not yet configured. Install @grpc/grpc-js and @grpc/proto-loader to enable gRPC calls.",
        }))
    });

    registry.register_sync("grpc.reflect", |_, _| {
        Ok(json!({
            "ok": false,
            "error": "gRPC runtime not yet configured. Install @grpc/grpc-js to enable server reflection.",
        }))
    });

    registry.register("grpc.send", |params: Value, ctx| async move {
        let ws_id = str_of(&params, "workspaceId");
        let stored = match opt_str(&params, "id") {
            Some(id) => load_entity(ws_id, "grpcRequests", id).ok_or_else(|| {
                EngineError::not_found(format!("No saved grpcRequests entity \"{id}\" in workspace \"{ws_id}\"."))
            })?,
            None => params.get("request").cloned().unwrap_or(Value::Null),
        };
        let env = load_merged_environment(ws_id);
        let env = env.as_ref();
        let mut metadata = Map::new();
        for (key, value) in stored.get("metadata").and_then(Value::as_object).into_iter().flatten() {
            metadata.insert(key.clone(), json!(resolve_vars(value.as_str().unwrap_or(""), env)));
        }
        let payload = json!({
            "serverAddress": resolve_vars(str_of(&stored, "serverAddress"), env),
            "serviceName": resolve_vars(str_of(&stored, "serviceName"), env),
            "methodName": resolve_vars(str_of(&stored, "methodName"), env),
            "requestBody": resolve_vars(str_of(&stored, "requestBody"), env),
            "metadata": metadata,
            "protoFileId": stored.get("protoFileId").filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null),
            "useReflection": stored.get("useReflection").filter(|v| !v.is_null()).cloned().unwrap_or(json!(false)),
        });
        ctx.registry.invoke("grpc.execute", payload, ctx.clone()).await
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{kernel::CommandContext, testing::scratch_data_root, workspace::fs::write_entity};

    async fn call(action: &str, payload: Value) -> crate::protocol::Result<Value> {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        let registry = Arc::new(registry);
        let ctx = CommandContext::internal(registry.clone());
        registry.invoke(action, payload, ctx).await
    }

    #[tokio::test]
    async fn execute_and_reflect_are_placeholders() {
        let execute = call(
            "grpc.execute",
            json!({
                "serverAddress": "a", "serviceName": "s", "methodName": "m", "requestBody": "{}",
                "metadata": {}, "protoFileId": null, "useReflection": false,
            }),
        )
        .await
        .unwrap();
        assert_eq!(execute["ok"], false);
        assert!(execute["error"].as_str().unwrap().contains("@grpc/proto-loader"));
        let reflect = call("grpc.reflect", json!({ "serverAddress": "a" })).await.unwrap();
        assert!(reflect["error"].as_str().unwrap().contains("server reflection"));
    }

    #[tokio::test]
    async fn send_goes_through_execute_and_a_missing_request_is_not_found() {
        let root = scratch_data_root();
        write_entity(
            root.ws_id,
            "grpcRequests",
            "r1",
            &json!({
                "id": "r1", "name": "r", "serverAddress": "x", "serviceName": "S", "methodName": "M",
                "requestBody": "{}", "metadata": { "k": "v" },
            }),
            None,
        )
        .unwrap();
        let sent = call("grpc.send", json!({ "workspaceId": root.ws_id, "id": "r1" })).await.unwrap();
        assert_eq!(sent["ok"], false);
        let missing = call("grpc.send", json!({ "workspaceId": root.ws_id, "id": "nope" })).await.unwrap_err();
        assert_eq!(missing.code, crate::protocol::ErrorCode::NotFound);
    }
}
