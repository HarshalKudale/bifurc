//! Flows: saved requests chained into a graph, where what one node produces is the input of the next.
//!
//! A flow is an entity of the kind `flows`, written by the client that draws it. The engine checks it
//! (`flow.validate`) and runs it (`flow.run`, `flow.cancel`, `flow.status`); a run reports itself with
//! `flow.progress` and `flow.finished` events.

mod condition;
mod loadtest;
pub mod model;
mod nodes;
mod reference;
mod run;
mod streams;
mod validate;

#[cfg(test)]
mod tests;

use serde_json::{Value, json};

use self::model::Flow;
use crate::{
    kernel::CommandRegistry,
    protocol::{EngineError, Result},
    utils::json::{opt_str, str_of},
    workspace::{config::load_entity, fs::entity_lock},
};

/// An engine-made sentence of a flow: the English `message` and the key the UI translates it by.
pub(crate) fn said(key: &str, params: Value, message: impl Into<String>) -> EngineError {
    EngineError::bad_request(message).keyed(key, params)
}

/// The failure a command result or event carries (`error`, with `errorKey` and `errorParams` beside it), if any.
pub(crate) fn error_in(value: &Value) -> Option<EngineError> {
    let error = EngineError::engine(opt_str(value, "error")?);
    Some(match (opt_str(value, "errorKey"), value.get("errorParams")) {
        (Some(key), Some(params)) => error.keyed(key, params.clone()),
        _ => error,
    })
}

/// Writes `error` and, for a keyed one, `errorKey` and `errorParams` into the object `output`.
pub(crate) fn put_error(output: &mut Value, error: &EngineError) {
    output["error"] = json!(error.message);
    if let (Some(key), Some(params)) = (error.key(), error.params()) {
        output["errorKey"] = json!(key);
        output["errorParams"] = params.clone();
    }
}

/// Whether `flow` can be saved: it has a name and is written the way a flow is. What is wrong inside it (a
/// node without a request, a loop) is reported by `flow.validate` and does not keep a draft from being saved.
pub fn check_shape(flow: &Value) -> Result<()> {
    if str_of(flow, "name").trim().is_empty() {
        return Err(
            EngineError::engine("name is required for flows").keyed("validation.required", json!({ "field": "name" }))
        );
    }
    Flow::parse(flow).map(|_| ())
}

pub(crate) fn saved_flow_not_found(ws_id: &str, id: &str) -> EngineError {
    EngineError::not_found(format!("No saved flow \"{id}\" in workspace \"{ws_id}\"."))
        .keyed("flow.saved_not_found", json!({ "id": id, "workspace": ws_id }))
}

/// The flow a command is about: the one given inline, or the saved one named by `id`.
fn flow_of(params: &Value) -> Result<Flow> {
    if let Some(flow) = params.get("flow") {
        return Flow::parse(flow);
    }
    let (ws_id, id) = (str_of(params, "workspaceId"), opt_str(params, "id"));
    let id = id.ok_or_else(|| {
        EngineError::bad_request("flow.validate takes the `id` of a saved flow, or a `flow`.")
            .keyed("flow.validate_needs_id", json!({}))
    })?;
    let entity = load_entity(ws_id, "flows", id).ok_or_else(|| match entity_lock(ws_id, "flows", id) {
        Some(version) => crate::workspace::stamp::update_required(&version),
        None => saved_flow_not_found(ws_id, id),
    })?;
    Flow::parse(&entity)
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("flow.validate", |params, _| {
        Ok(validate::describe(str_of(&params, "workspaceId"), &flow_of(&params)?))
    });

    registry.register_sync("flow.run", |params, ctx| run::start(ctx, &params));

    registry.register_sync("flow.cancel", |params, _| {
        Ok(run::cancel(str_of(&params, "workspaceId"), str_of(&params, "id")))
    });

    registry.register_sync("flow.status", |params, _| {
        Ok(run::status(str_of(&params, "workspaceId"), str_of(&params, "id")).unwrap_or(Value::Null))
    });

    registry.register_sync("flow.loadtest.start", |params, ctx| loadtest::start(ctx, &params));

    registry.register_sync("flow.loadtest.cancel", |params, _| {
        Ok(loadtest::cancel(str_of(&params, "workspaceId"), str_of(&params, "id")))
    });

    registry.register_sync("flow.loadtest.status", |params, _| {
        Ok(loadtest::status(str_of(&params, "workspaceId"), str_of(&params, "id")).unwrap_or(Value::Null))
    });
}
