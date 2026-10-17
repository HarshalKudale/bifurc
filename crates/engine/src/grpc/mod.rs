//! gRPC commands: `grpc.reflect` reads a server's schema, `grpc.execute` makes a call, and `grpc.send`
//! resolves a saved or inline request and hands it to `grpc.execute`.
//!
//! The schema of a call comes from server reflection, or from a saved `.proto` file when the request names
//! one and does not ask for reflection.

mod call;
mod proto;
mod reflection;
mod stream;
#[cfg(test)]
pub(crate) mod tests;

use prost_reflect::{DescriptorPool, MethodDescriptor};
use serde_json::{Map, Value, json};

use crate::{
    env::{load_merged_environment, resolve_vars, send_scoped, with_overrides},
    kernel::CommandRegistry,
    protocol::{EngineError, ErrorCode},
    utils::json::{opt_str, str_of, string_map},
    workspace::{config::load_entity, response_history},
};

fn services_json(pool: &DescriptorPool, only: Option<&[String]>) -> Value {
    let mut services: Vec<_> = pool
        .services()
        .filter(|service| only.is_none_or(|names| names.iter().any(|name| name == service.full_name())))
        .filter(|service| !is_reflection_service(service.full_name()))
        .collect();
    services.sort_by(|a, b| a.full_name().cmp(b.full_name()));
    Value::Array(
        services
            .into_iter()
            .map(|service| {
                json!({
                    "name": service.full_name(),
                    "methods": service.methods().map(|method| json!({
                        "name": method.name(),
                        "inputType": method.input().full_name(),
                        "outputType": method.output().full_name(),
                        "clientStreaming": method.is_client_streaming(),
                        "serverStreaming": method.is_server_streaming(),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

/// The reflection service is how the schema is read, not part of what a user calls.
fn is_reflection_service(name: &str) -> bool {
    name.starts_with("grpc.reflection.")
}

/// Finds the method a request names, in the schema its request selects.
async fn find_method(
    params: &Value,
    channel: Option<&tonic::transport::Channel>,
) -> Result<MethodDescriptor, EngineError> {
    let use_reflection = params.get("useReflection").and_then(Value::as_bool).unwrap_or(false);
    let proto_file_id = opt_str(params, "protoFileId").filter(|id| !id.is_empty());
    let pool = match (proto_file_id, use_reflection) {
        (Some(id), false) => proto::pool_from_saved(str_of(params, "workspaceId"), id)?,
        _ => {
            let channel = channel
                .cloned()
                .ok_or_else(|| EngineError::engine("No connection.").keyed("grpc.no_connection", json!({})))?;
            reflection::reflect(channel).await?.pool
        }
    };
    let service_name = str_of(params, "serviceName").trim().trim_start_matches('/');
    let method_name = str_of(params, "methodName").trim();
    let service = pool.get_service_by_name(service_name).ok_or_else(|| {
        EngineError::not_found(format!("The schema has no service \"{service_name}\"."))
            .keyed("grpc.service_not_found", json!({ "service": service_name }))
    })?;
    service.methods().find(|method| method.name() == method_name).ok_or_else(|| {
        EngineError::not_found(format!("The service \"{service_name}\" has no method \"{method_name}\"."))
            .keyed("grpc.method_not_found", json!({ "service": service_name, "method": method_name }))
    })
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register("grpc.execute", |params: Value, _| async move {
        let started = std::time::Instant::now();
        let elapsed = |value: Value| {
            let mut value = value;
            value["durationMs"] = json!(started.elapsed().as_millis() as u64);
            value
        };
        let channel = match call::connect(str_of(&params, "serverAddress")).await {
            Ok(channel) => channel,
            Err(error) => return Ok(elapsed(crate::runtime::failed(&error))),
        };
        let method = match find_method(&params, Some(&channel)).await {
            Ok(method) => method,
            Err(error) => return Ok(elapsed(crate::runtime::failed(&error))),
        };
        let messages = match call::parse_messages(str_of(&params, "requestBody"), &method) {
            Ok(messages) => messages,
            Err(error) => return Ok(elapsed(crate::runtime::failed(&error))),
        };
        let metadata = match call::to_metadata(&string_map(&params, "metadata")) {
            Ok(metadata) => metadata,
            Err(error) => return Ok(elapsed(crate::runtime::failed(&error))),
        };
        let outcome = call::invoke(channel, &method, messages, metadata).await;
        let ok = outcome.status == tonic::Code::Ok;
        let mut result = json!({
            "ok": ok,
            "responses": outcome.responses,
            "metadata": outcome.metadata,
            "status": outcome.status as i32,
            "statusMessage": outcome.status_message,
        });
        if !ok {
            let (status, message) = (status_name(outcome.status), outcome.status_message.clone());
            let error = EngineError::upstream(format!("{status}: {message}"))
                .keyed("grpc.status", json!({ "status": status, "message": message }));
            crate::runtime::set_failure(&mut result, &error);
        }
        Ok(elapsed(result))
    });

    registry.register("grpc.reflect", |params: Value, _| async move {
        // The address may be written with environment variables, as it is in a saved request.
        let address = match opt_str(&params, "workspaceId") {
            Some(ws_id) => resolve_vars(str_of(&params, "serverAddress"), load_merged_environment(ws_id).as_ref()),
            None => str_of(&params, "serverAddress").to_owned(),
        };
        let channel = match call::connect(&address).await {
            Ok(channel) => channel,
            Err(error) => return Ok(crate::runtime::failed(&error)),
        };
        // A server that is not gRPC may accept the connection and never answer.
        let asked = tokio::time::timeout(std::time::Duration::from_secs(15), reflection::reflect(channel))
            .await
            .unwrap_or_else(|_| {
                Err(EngineError::new(ErrorCode::Timeout, "The server did not answer the reflection request.")
                    .keyed("grpc.reflection_timeout", json!({})))
            });
        Ok(match asked {
            Ok(found) => json!({ "ok": true, "services": services_json(&found.pool, Some(&found.services)) }),
            Err(error) => crate::runtime::failed(&error),
        })
    });

    registry.register("grpc.send", |params: Value, ctx| async move {
        send_scoped(&params.clone(), async move {
            let ws_id = str_of(&params, "workspaceId");
            let stored = match opt_str(&params, "id") {
                Some(id) => load_entity(ws_id, "grpcRequests", id).ok_or_else(|| {
                    EngineError::not_found(format!("No saved grpcRequests entity \"{id}\" in workspace \"{ws_id}\"."))
                        .keyed(
                            "request.saved_not_found",
                            json!({ "kind": "grpcRequests", "id": id, "workspace": ws_id }),
                        )
                })?,
                None => params.get("request").cloned().unwrap_or(Value::Null),
            };
            let stored = with_overrides(stored, &params);
            let payload = resolve_saved(ws_id, &stored);
            let outcome = ctx.registry.invoke("grpc.execute", payload.clone(), ctx.clone()).await;
            if let Some(id) = response_history::entity_id(&params) {
                let method = format!("{}/{}", str_of(&payload, "serviceName"), str_of(&payload, "methodName"));
                let address = str_of(&payload, "serverAddress").to_string();
                let capture = match &outcome {
                    Ok(v) => response_history::Capture {
                        status: v["status"].as_i64().unwrap_or(0),
                        status_text: opt_str(v, "statusMessage").map(str::to_string),
                        headers: v["metadata"].clone(),
                        body: serde_json::to_vec_pretty(&v["responses"]).unwrap_or_default(),
                        duration_ms: v["durationMs"].as_u64().unwrap_or(0),
                        error: opt_str(v, "error").map(str::to_string),
                        error_key: v["errorKey"]
                            .as_str()
                            .zip(v.get("errorParams"))
                            .map(|(k, p)| (k.to_owned(), p.clone())),
                        method,
                        url: address,
                    },
                    Err(e) => response_history::failed(e, method, address),
                };
                response_history::record(ws_id, id, capture);
            }
            let mut result = outcome?;
            crate::vault::redact_value(ws_id, &mut result);
            Ok(result)
        })
        .await
    });

    stream::register(registry);
}

/// A saved (or inline) request with its environment variables resolved, in the shape `grpc.execute` takes.
fn resolve_saved(ws_id: &str, stored: &Value) -> Value {
    let env = load_merged_environment(ws_id);
    let env = env.as_ref();
    let mut metadata = Map::new();
    for (key, value) in stored.get("metadata").and_then(Value::as_object).into_iter().flatten() {
        metadata.insert(key.clone(), json!(resolve_vars(value.as_str().unwrap_or(""), env)));
    }
    json!({
        "serverAddress": resolve_vars(str_of(stored, "serverAddress"), env),
        "serviceName": resolve_vars(str_of(stored, "serviceName"), env),
        "methodName": resolve_vars(str_of(stored, "methodName"), env),
        "requestBody": resolve_vars(str_of(stored, "requestBody"), env),
        "metadata": metadata,
        "protoFileId": stored.get("protoFileId").filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null),
        "useReflection": stored.get("useReflection").filter(|v| !v.is_null()).cloned().unwrap_or(json!(false)),
        "workspaceId": ws_id,
    })
}

/// The canonical upper-snake name of a status code (`UNAVAILABLE`), which is how gRPC tools print it.
fn status_name(code: tonic::Code) -> String {
    let debug = format!("{code:?}");
    let mut out = String::new();
    for (ix, ch) in debug.chars().enumerate() {
        if ch.is_ascii_uppercase() && ix > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_uppercase());
    }
    out
}
