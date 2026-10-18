//! Environments: which one is selected, and substituting `{{variables}}` into text.
//!
//! An environment is `{id, name, variables: [{id, key, value}], createdAt, workspaceId}`. `__global__` always
//! exists and is merged under the selected environment.

pub mod session;

use std::{
    future::Future,
    sync::{Arc, LazyLock},
};

use parking_lot::Mutex;
use regress::Regex;
use serde_json::{Value, json};

use crate::{
    kernel::CommandRegistry,
    protocol::Result,
    runtime::instances::notify_config_changed,
    utils::{
        json::{array_of, opt_str, str_of, truthy},
        randomizer::resolve_randomizers,
    },
    workspace::{
        config::{GLOBAL_ENV_ID, load_config, save_config},
        fs::{entity_lock, read_all_entities, write_flat_entity},
        models::Environment,
        settings::load_settings,
    },
};

/// What scripts did to their variables while a [`VariableScope`] was collecting them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VariableChanges {
    pub set: Vec<(String, String)>,
    pub removed: Vec<String>,
}

/// Variables a caller lays over a workspace's environment for the commands it invokes: a flow run does, for
/// each of its nodes. They are data, not text an author wrote, so they are never read for vault references.
#[derive(Clone, Default)]
pub struct VariableScope {
    /// Later pairs win over earlier ones, and all of them over the environment.
    pub variables: Vec<(String, String)>,
    /// The environment to merge under Global instead of the stored selected one (and where script writes go).
    /// An id that names no environment merges Global alone: the CLI resolves names first. `None` keeps the selection.
    pub environment_id: Option<String>,
    /// Where the variables a script sets go instead of the environment's file. `None` writes the file as usual.
    pub changes: Option<Arc<Mutex<VariableChanges>>>,
}

tokio::task_local! {
    static SCOPE: VariableScope;
}

/// Run `future` with `scope` laid over every environment it loads. The scope follows the task, so it reaches
/// a command only while that command resolves its variables before spawning anything. Inside another scope
/// (a named session's, a `--var` send's) it builds on it: the outer variables stay, the inner ones win, and the
/// environment and the change collector are the outer ones unless this scope sets its own.
pub async fn with_variable_scope<F: Future>(mut scope: VariableScope, future: F) -> F::Output {
    if let Ok(mut outer) = SCOPE.try_with(Clone::clone) {
        outer.variables.append(&mut scope.variables);
        scope.variables = outer.variables;
        scope.environment_id = scope.environment_id.or(outer.environment_id);
        scope.changes = scope.changes.or(outer.changes);
    }
    SCOPE.scope(scope, future).await
}

/// Run `work` under the `variables` (object of text) and `environmentId` a one-shot send command was given;
/// without either it runs as it is, so an enclosing scope (a flow node's) stays in force.
pub async fn send_scoped<F: Future>(params: &Value, work: F) -> F::Output {
    let variables: Vec<(String, String)> = params
        .get("variables")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
        .collect();
    let environment_id = opt_str(params, "environmentId").map(str::to_string);
    if variables.is_empty() && environment_id.is_none() {
        return work.await;
    }
    with_variable_scope(VariableScope { variables, environment_id, changes: None }, work).await
}

/// `entity` with the fields of the send's `overrides` object laid over its own.
pub fn with_overrides(mut entity: Value, params: &Value) -> Value {
    for (field, value) in params.get("overrides").and_then(Value::as_object).into_iter().flatten() {
        entity[field] = value.clone();
    }
    entity
}

fn lay_over(env: &mut Value, variables: &[(String, String)]) {
    let Some(list) = env.get_mut("variables").and_then(Value::as_array_mut) else { return };
    for (key, value) in variables {
        let scoped = json!({ "id": key, "key": key, "value": value, "scoped": true });
        match list.iter_mut().find(|v| str_of(v, "key") == key) {
            Some(existing) => *existing = scoped,
            None => list.push(scoped),
        }
    }
}

/// The values of the variables an environment's author wrote, as one text: what a [`VariableScope`] laid over
/// it is left out.
pub fn authored_variables_text(env: &Value) -> String {
    let authored: Vec<&Value> = array_of(env, "variables").iter().filter(|v| !truthy(v, "scoped")).collect();
    json!(authored).to_string()
}

/// The environment in force: the stored pointer if it names a real, non-global environment; otherwise the
/// only non-global environment, if there is exactly one.
pub fn effective_environment_id(environments: &[Value], stored_id: Option<&str>) -> Option<String> {
    if let Some(stored) = stored_id.filter(|id| !id.is_empty() && *id != GLOBAL_ENV_ID)
        && environments.iter().any(|e| str_of(e, "id") == stored)
    {
        return Some(stored.to_string());
    }
    let mut others = environments.iter().filter(|e| str_of(e, "id") != GLOBAL_ENV_ID);
    match (others.next(), others.next()) {
        (Some(only), None) => Some(str_of(only, "id").to_string()),
        _ => None,
    }
}

/// The environment in force for `ws_id`: a [`VariableScope`]'s, else the stored selection's.
fn selected_environment_id(ws_id: &str, environments: &[Value]) -> Option<String> {
    if let Ok(Some(id)) = SCOPE.try_with(|scope| scope.environment_id.clone()) {
        return Some(id);
    }
    effective_environment_id(environments, load_settings().stored_environment_id(ws_id).as_deref())
}

/// The stored environment in force for `ws_id` (a scope's, else the selection), as the file holds it.
pub fn selected_environment(ws_id: &str) -> Option<Value> {
    let environments = read_all_entities(ws_id, "environments");
    let id = selected_environment_id(ws_id, &environments)?;
    environments.into_iter().find(|e| str_of(e, "id") == id)
}

/// What scripts of the running command set so far, where a scope collects them instead of writing the file
/// (a named session's): the later part of the same command does not see these unless they are laid over.
pub fn collected_variables() -> Vec<(String, String)> {
    SCOPE.try_with(|scope| scope.changes.as_ref().map(|c| c.lock().set.clone())).ok().flatten().unwrap_or_default()
}

/// `active`'s variables over `global`'s, as one environment named `merged`. `None` when both are absent.
pub fn merge_env_vars(global: Option<&Value>, active: Option<&Value>) -> Option<Value> {
    if global.is_none() && active.is_none() {
        return None;
    }
    let mut variables: Vec<Value> = Vec::new();
    for env in [global, active].into_iter().flatten() {
        for variable in array_of(env, "variables") {
            let key = str_of(variable, "key");
            match variables.iter_mut().find(|v| str_of(v, "key") == key) {
                Some(existing) => *existing = variable.clone(),
                None => variables.push(variable.clone()),
            }
        }
    }
    Some(json!({ "id": "merged", "name": "merged", "variables": variables, "createdAt": 0, "workspaceId": "" }))
}

/// The merged environment of a workspace, read from disk. It names its workspace, which lets
/// [`resolve_vars`] also resolve `{{vault.secret}}`; an environment without one (the proxies') never does.
pub fn load_merged_environment(ws_id: &str) -> Option<Value> {
    let environments = read_all_entities(ws_id, "environments");
    let find = |id: &str| environments.iter().find(|e| str_of(e, "id") == id);
    let active_id = selected_environment_id(ws_id, &environments);
    let mut merged = merge_env_vars(find(GLOBAL_ENV_ID), active_id.as_deref().and_then(find))
        .unwrap_or_else(|| json!({ "id": "merged", "name": "merged", "variables": [], "createdAt": 0 }));
    merged["workspaceId"] = json!(ws_id);
    let _ = SCOPE.try_with(|scope| lay_over(&mut merged, &scope.variables));
    Some(merged)
}

/// Persist what a script did to its variables: `set` is written into the selected environment (the global one
/// when none is selected), `removed` keys are deleted from both. Returns whether anything was written.
pub fn apply_variable_changes(ws_id: &str, set: &[(String, String)], removed: &[String]) -> bool {
    if set.is_empty() && removed.is_empty() {
        return false;
    }
    if let Ok(Some(changes)) = SCOPE.try_with(|scope| scope.changes.clone()) {
        let mut changes = changes.lock();
        changes.set.extend_from_slice(set);
        changes.removed.extend_from_slice(removed);
        return false;
    }
    let environments = read_all_entities(ws_id, "environments");
    // Script writes follow the environment a send was told to use, so a run against `staging` never edits `dev`.
    let active_id = selected_environment_id(ws_id, &environments).unwrap_or_else(|| GLOBAL_ENV_ID.to_string());
    let mut wrote = false;
    for env in environments {
        let id = str_of(&env, "id").to_string();
        let is_target = id == active_id;
        if !is_target && (id != GLOBAL_ENV_ID || removed.is_empty()) {
            continue;
        }
        let mut env = env;
        let mut variables: Vec<Value> = array_of(&env, "variables").to_vec();
        let before = variables.len();
        variables.retain(|v| !removed.iter().any(|key| key == str_of(v, "key")));
        let mut changed = variables.len() != before;
        if is_target {
            for (key, value) in set {
                match variables.iter_mut().find(|v| str_of(v, "key") == key) {
                    Some(existing) if str_of(existing, "value") == value => {}
                    Some(existing) => {
                        existing["value"] = json!(value);
                        changed = true;
                    }
                    None => {
                        variables.push(json!({ "id": key, "key": key, "value": value }));
                        changed = true;
                    }
                }
            }
        }
        if changed {
            env["variables"] = Value::Array(variables);
            wrote |= write_flat_entity(ws_id, "environments", &id, &env).is_ok();
        }
    }
    if wrote {
        notify_config_changed();
    }
    wrote
}

/// The value of `key` in an environment.
pub fn env_value<'a>(env: &'a Value, key: &str) -> Option<&'a str> {
    array_of(env, "variables").iter().find(|v| str_of(v, "key") == key).and_then(|v| v.get("value")?.as_str())
}

/// Replace `{{name}}` with the environment's value and `{{vault.secret}}` with the vault's (unknown names, and
/// vaults that are sealed, are left as written), then expand the `{{random.*}}` generators.
pub fn resolve_vars(text: &str, env: Option<&Value>) -> String {
    let ws_id = env.map(|env| str_of(env, "workspaceId")).unwrap_or("");
    substitute(text, env.map(|env| (ws_id, |key: &str| env_value(env, key))))
}

/// [`resolve_vars`] against a typed environment.
pub fn resolve_vars_in(text: &str, env: Option<&Environment>) -> String {
    substitute(text, env.map(|env| (env.workspace_id.as_str(), |key: &str| env.get(key))))
}

fn substitute<'a>(text: &str, env: Option<(&str, impl Fn(&str) -> Option<&'a str>)>) -> String {
    if text.is_empty() {
        return String::new();
    }
    let Some((ws_id, lookup)) = env else { return resolve_randomizers(text) };
    // Before the main pass: a quoted value must not be read as a `{{name}}`. Like a vault reference, it needs the
    // workspace (so not `env.resolveText`'s, nor the REST pipeline's, which mints on the wire copy instead).
    let minted = crate::jwt::expand(ws_id, text, None, &mut Vec::new());
    let text = minted.as_str();
    static PATTERN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\{\{(\w+)(?:\.(\w+))?\}\}").expect("a valid pattern"));
    let pattern = &*PATTERN;
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for found in pattern.find_iter(text) {
        out.push_str(&text[last..found.start()]);
        let key = found.group(1).map(|range| &text[range]).unwrap_or("");
        let secret = found.group(2).map(|range| &text[range]);
        match secret {
            Some(secret) => match crate::vault::resolve_reference(ws_id, key, secret) {
                Some(value) => out.push_str(&value),
                None => out.push_str(&text[found.range()]),
            },
            None => match lookup(key) {
                Some(value) => out.push_str(value),
                None => out.push_str(&text[found.range()]),
            },
        }
        last = found.end();
    }
    out.push_str(&text[last..]);
    resolve_randomizers(&out)
}

/// [`resolve_vars`] over every value of a header map.
pub fn resolve_headers(headers: &Value, env: Option<&Value>) -> Value {
    let mut out = serde_json::Map::new();
    for (name, value) in headers.as_object().into_iter().flatten() {
        out.insert(name.clone(), json!(resolve_vars(value.as_str().unwrap_or(""), env)));
    }
    Value::Object(out)
}

pub fn register(registry: &mut CommandRegistry) {
    session::register(registry);
    registry.register_sync("env.resolveText", |params, _| {
        // A client asks what text resolves to; secrets never go back to one, so vault references stay as written.
        let mut env = load_merged_environment(str_of(&params, "workspaceId"));
        if let Some(env) = env.as_mut() {
            env["workspaceId"] = json!("");
        }
        Ok(json!({ "text": resolve_vars(str_of(&params, "text"), env.as_ref()) }))
    });
    registry.register_sync("env.getMerged", |params, _| {
        let ws_id = str_of(&params, "workspaceId");
        let mut merged = load_merged_environment(ws_id).unwrap_or(Value::Null);
        // The selected environment's login, for `env show`; resolving never reads it.
        if let Some(auth) = selected_environment(ws_id).and_then(|env| env.get("auth").cloned()) {
            merged["auth"] = auth;
        }
        Ok(merged)
    });
    registry.register_sync("env.setActive", |params, _| -> Result<Value> {
        let id = params.get("id").cloned().unwrap_or(Value::Null);
        if id.as_str() == Some(GLOBAL_ENV_ID) {
            return Ok(json!({
                "ok": false, "error": "cannot_activate_global", "errorKey": "env.cannot_activate_global",
                "errorParams": {},
            }));
        }
        // A named session picks its own environment; the stored selection (the app's) stays.
        if session::select_environment(id.as_str()) {
            return Ok(json!({ "ok": true }));
        }
        let mut cfg = load_config();
        cfg["activeEnvironmentId"] = id.clone();
        let active_ws = str_of(&cfg, "activeWorkspaceId").to_string();
        if let Some(version) = id.as_str().and_then(|id| entity_lock(&active_ws, "environments", id)) {
            return Err(crate::workspace::stamp::update_required(&version));
        }
        if let Some(workspaces) = cfg.get_mut("workspaces").and_then(Value::as_array_mut)
            && let Some(ws) = workspaces.iter_mut().find(|w| str_of(w, "id") == active_ws)
        {
            ws["activeEnvironmentId"] = id;
        }
        save_config(&cfg)?;
        notify_config_changed();
        Ok(json!({ "ok": true }))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(id: &str, vars: &[(&str, &str)]) -> Value {
        let variables: Vec<Value> = vars.iter().map(|(k, v)| json!({ "id": k, "key": k, "value": v })).collect();
        json!({ "id": id, "name": id, "variables": variables })
    }

    #[test]
    fn selection_falls_back_to_the_lone_environment() {
        let envs = vec![env(GLOBAL_ENV_ID, &[]), env("dev", &[])];
        assert_eq!(effective_environment_id(&envs, None).as_deref(), Some("dev"));
        assert_eq!(effective_environment_id(&envs, Some("gone")).as_deref(), Some("dev"));
        let two = vec![env(GLOBAL_ENV_ID, &[]), env("dev", &[]), env("prod", &[])];
        assert_eq!(effective_environment_id(&two, None), None);
        assert_eq!(effective_environment_id(&two, Some("prod")).as_deref(), Some("prod"));
        assert_eq!(effective_environment_id(&two, Some(GLOBAL_ENV_ID)), None);
    }

    #[test]
    fn active_variables_override_global_ones_in_place() {
        let merged =
            merge_env_vars(Some(&env("g", &[("a", "1"), ("b", "2")])), Some(&env("d", &[("b", "3"), ("c", "4")])))
                .unwrap();
        let pairs: Vec<(&str, &str)> =
            array_of(&merged, "variables").iter().map(|v| (str_of(v, "key"), str_of(v, "value"))).collect();
        assert_eq!(pairs, vec![("a", "1"), ("b", "3"), ("c", "4")]);
        assert!(merge_env_vars(None, None).is_none());
    }

    #[test]
    fn variables_are_substituted_and_unknown_ones_kept() {
        let e = env("d", &[("host", "api.test"), ("empty", "")]);
        assert_eq!(resolve_vars("http://{{host}}/{{missing}}/{{empty}}x", Some(&e)), "http://api.test/{{missing}}/x");
        assert_eq!(resolve_vars("", Some(&e)), "");
    }

    #[test]
    fn the_typed_environment_resolves_exactly_as_the_stored_one() {
        // Older files: a non-text value, a repeated key, a variable with no key, unknown fields.
        let stored = json!({ "id": "d", "workspaceId": "", "future": [1], "variables": [
            { "key": "n", "value": 5 }, { "key": "n", "value": "late" }, { "value": "orphan" },
            { "key": "host", "value": "api.test", "extra": true },
        ] });
        let typed = Environment::from_value(&stored);
        for text in ["{{n}}|{{host}}|{{nope}}|{{vault.s}}|plain", "", "{{}}"] {
            assert_eq!(resolve_vars_in(text, Some(&typed)), resolve_vars(text, Some(&stored)));
        }
        assert_eq!(resolve_vars_in("{{n}}", None), resolve_vars("{{n}}", None));
    }
}
