//! What a proxy routes with: the served workspace's enabled mocks, mappings and rules, and its merged
//! environment.

use indexmap::IndexSet;
use serde_json::{Value, json};

use super::{config::WorkspaceDispatchSlice, fs::load_enabled_set};
use crate::{
    env::merge_env_vars,
    utils::json::{array_of, opt_str, str_of},
};

#[derive(Debug, Clone, Default)]
pub struct EnabledSets {
    pub mocks: IndexSet<String>,
    pub mappings: IndexSet<String>,
    pub rules: IndexSet<String>,
}

pub fn load_enabled_sets(ws_id: &str) -> EnabledSets {
    EnabledSets {
        mocks: load_enabled_set(ws_id, "mocks"),
        mappings: load_enabled_set(ws_id, "mappings"),
        rules: load_enabled_set(ws_id, "rules"),
    }
}

/// The global environment with the config's active environment merged over it.
pub fn active_env(cfg: &Value) -> Option<Value> {
    let environments = array_of(cfg, "environments");
    let find = |id: &str| environments.iter().find(|e| str_of(e, "id") == id);
    let global = find(super::config::GLOBAL_ENV_ID);
    let selected = opt_str(cfg, "activeEnvironmentId").filter(|id| !id.is_empty()).and_then(find);
    merge_env_vars(global, selected)
}

/// `cfg` narrowed to one workspace and to what is switched on. `full_rules` are the workspace's rule files
/// in full (a config snapshot only lists rules).
pub fn workspace_cfg(
    cfg: &Value,
    full_rules: &[Value],
    enabled: &EnabledSets,
    slice: &WorkspaceDispatchSlice,
) -> Value {
    let ws_id = slice.workspace_id.as_str();
    let serves = |entity: &&Value, set: &IndexSet<String>| {
        let owner = str_of(entity, "workspaceId");
        (owner.is_empty() || owner == ws_id) && set.contains(str_of(entity, "id"))
    };
    let mut out = cfg.clone();
    out["activeWorkspaceId"] = json!(ws_id);
    out["activeEnvironmentId"] = json!(slice.active_environment_id);
    out["environments"] = json!(slice.environments);
    out["mappings"] = slice.mappings.iter().filter(|m| serves(m, &enabled.mappings)).cloned().collect();
    out["proxyRules"] = full_rules
        .iter()
        .filter(|r| serves(r, &enabled.rules))
        .map(|rule| {
            // Rule files hold no `enabled`; everything that reaches a proxy is on.
            let mut rule = rule.clone();
            rule["enabled"] = json!(true);
            rule
        })
        .collect();
    out["mocks"] = slice.mocks.iter().filter(|m| serves(m, &enabled.mocks)).cloned().collect();
    out
}
