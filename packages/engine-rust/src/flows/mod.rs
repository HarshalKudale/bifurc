//! Flows: saved requests chained into a graph, where what one node produces is the input of the next.
//!
//! A flow is an entity of the kind `flows`, written by the client that draws it. The engine checks it
//! (`flow.validate`) and runs it (`flow.run`, `flow.cancel`, `flow.status`); a run reports itself with
//! `flow.progress` and `flow.finished` events.

mod condition;
pub mod model;
mod nodes;
mod reference;
mod run;
mod streams;
mod validate;

#[cfg(test)]
mod tests;

use serde_json::Value;

use self::model::Flow;
use crate::{
    kernel::CommandRegistry,
    protocol::{EngineError, Result},
    utils::json::{opt_str, str_of},
    workspace::config::load_entity,
};

/// Whether `flow` can be saved: it has a name and is written the way a flow is. What is wrong inside it (a
/// node without a request, a loop) is reported by `flow.validate` and does not keep a draft from being saved.
pub fn check_shape(flow: &Value) -> Result<()> {
    if str_of(flow, "name").trim().is_empty() {
        return Err(EngineError::engine("name is required for flows"));
    }
    Flow::parse(flow).map(|_| ())
}

/// The flow a command is about: the one given inline, or the saved one named by `id`.
fn flow_of(params: &Value) -> Result<Flow> {
    if let Some(flow) = params.get("flow") {
        return Flow::parse(flow);
    }
    let (ws_id, id) = (str_of(params, "workspaceId"), opt_str(params, "id"));
    let id =
        id.ok_or_else(|| EngineError::bad_request("flow.validate takes the `id` of a saved flow, or a `flow`."))?;
    let entity = load_entity(ws_id, "flows", id)
        .ok_or_else(|| EngineError::not_found(format!("No saved flow \"{id}\" in workspace \"{ws_id}\".")))?;
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
}
