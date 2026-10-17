//! Named sessions: what one agent keeps to itself while sharing the workspace with the app and with others.
//!
//! The command line names its session in `hello`. Every command such a connection runs is entered with
//! [`enter`]: the session's cookie jar (`cookies::jar_of`), a [`VariableScope`] with its runtime variables and
//! its selected environment, and a collector for what scripts set, which is merged back into the session
//! afterwards instead of being written to the environment's file. A connection with no name is the default
//! session: the app's jar, the stored environment and script writes to the file as always; only the runtime
//! variables the command line extracts (`--extract`, `vars set`) are kept, in a layer the app never sees.
//! Layers live as long as the engine and are never written to disk.

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, LazyLock},
};

use parking_lot::Mutex;
use serde_json::{Value, json};

use super::{VariableChanges, VariableScope, with_variable_scope};
use crate::{
    kernel::CommandRegistry,
    protocol::{EngineError, Result},
    utils::json::{array_of, opt_str, str_of},
    workspace::config::load_config,
};

/// What a session keeps: runtime variables (later pairs win) and the environment it selected.
#[derive(Default)]
struct Layer {
    variables: Vec<(String, String)>,
    environment_id: Option<String>,
}

/// By workspace id and session name; the default session's name is empty.
static LAYERS: LazyLock<Mutex<HashMap<(String, String), Layer>>> = LazyLock::new(Default::default);

tokio::task_local! {
    /// The workspace and the name of the session the running command belongs to.
    static CURRENT: (String, String);
}

/// The name of the named session this task runs in, if it runs in one.
pub fn named() -> Option<String> {
    CURRENT.try_with(|(_, name)| name.clone()).ok().filter(|name| !name.is_empty())
}

/// The workspace a command runs for: its `workspaceId`, else the active one.
pub fn workspace_of(payload: &Value) -> String {
    opt_str(payload, "workspaceId")
        .map(str::to_string)
        .unwrap_or_else(|| str_of(&load_config(), "activeWorkspaceId").to_string())
}

fn set(layer: &mut Layer, set: &[(String, String)], removed: &[String]) {
    layer.variables.retain(|(key, _)| !removed.contains(key) && !set.iter().any(|(k, _)| k == key));
    layer.variables.extend_from_slice(set);
}

/// Run `work` as a command of session `name` (empty: the default one) in workspace `ws_id`.
pub async fn enter<F: Future>(ws_id: &str, name: &str, work: F) -> F::Output {
    let key = (ws_id.to_string(), name.to_string());
    let (variables, environment_id) = {
        let layers = LAYERS.lock();
        layers.get(&key).map(|l| (l.variables.clone(), l.environment_id.clone())).unwrap_or_default()
    };
    let changes = (!name.is_empty()).then(|| Arc::new(Mutex::new(VariableChanges::default())));
    let scope = VariableScope { variables, environment_id, changes: changes.clone() };
    let out = CURRENT.scope(key.clone(), with_variable_scope(scope, work)).await;
    if let Some(changes) = changes {
        let done = std::mem::take(&mut *changes.lock());
        set(LAYERS.lock().entry(key).or_default(), &done.set, &done.removed);
    }
    out
}

/// `work` for a task about to be spawned: it enters the session of the task that calls this, as that task
/// would, since task-locals do not cross `tokio::spawn`.
pub fn carry<F: Future>(work: F) -> impl Future<Output = F::Output> {
    let current = CURRENT.try_with(Clone::clone).ok();
    let work = crate::workspace::changes::carry(work);
    async move {
        match current {
            Some((ws_id, name)) => enter(&ws_id, &name, work).await,
            None => work.await,
        }
    }
}

/// A named session's `env use`: `true` when the running command is in one and the selection was kept there.
pub fn select_environment(id: Option<&str>) -> bool {
    let Ok((ws_id, name)) = CURRENT.try_with(Clone::clone) else { return false };
    if name.is_empty() {
        return false;
    }
    LAYERS.lock().entry((ws_id, name)).or_default().environment_id = id.map(str::to_string);
    true
}

fn this() -> Result<(String, String)> {
    CURRENT.try_with(Clone::clone).map_err(|_| {
        EngineError::bad_request("session commands are for the command line: this connection has no session")
            .keyed("session.none", json!({}))
    })
}

fn layer_of<T>(work: impl FnOnce(&mut Layer) -> T) -> Result<T> {
    Ok(work(LAYERS.lock().entry(this()?).or_default()))
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("session.get", |_, _| {
        let (ws_id, name) = this()?;
        let (variables, environment_id) = layer_of(|l| (l.variables.clone(), l.environment_id.clone()))?;
        let variables: Vec<Value> =
            variables.iter().map(|(key, value)| json!({ "key": key, "value": value })).collect();
        let domains = (!name.is_empty()).then(|| crate::cookies::domain_count(&ws_id));
        let name = if name.is_empty() { "default" } else { &name };
        Ok(json!({ "name": name, "variables": variables, "environmentId": environment_id, "cookieDomains": domains }))
    });
    registry.register_sync("session.setVariables", |params, _| {
        let pairs: Vec<(String, String)> = params["variables"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
            .collect();
        layer_of(|l| set(l, &pairs, &[]))?;
        Ok(json!({ "ok": true, "count": pairs.len() }))
    });
    registry.register_sync("session.clearVariables", |params, _| {
        let keys: Vec<String> =
            array_of(&params, "keys").iter().filter_map(|k| k.as_str().map(str::to_string)).collect();
        let all = params.get("keys").is_none();
        let cleared = layer_of(|l| {
            let before = l.variables.len();
            l.variables.retain(|(key, _)| !all && !keys.contains(key));
            before - l.variables.len()
        })?;
        Ok(json!({ "ok": true, "cleared": cleared }))
    });
    registry.register_sync("session.reset", |_, _| {
        let (ws_id, name) = this()?;
        LAYERS.lock().remove(&(ws_id.clone(), name.clone()));
        if !name.is_empty() {
            crate::cookies::reset_session(&ws_id, &name);
        }
        Ok(json!({ "ok": true }))
    });
}

#[cfg(test)]
mod tests;
