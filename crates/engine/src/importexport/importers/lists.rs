//! The Bifurc JSON imports that are a plain list of flat entities: environments, mappings and proxy rules.

use serde_json::{Value, json};

use crate::{
    importexport::{
        support::{commit_config, io_failure, parse_json, sync_enabled, thrown, upsert_by_id, workspace_ids},
        types::{ImportOutcome, ImportSource, Imported, Preflight, PreflightOutcome, Strategy},
    },
    utils::{
        id::generate_id,
        json::{array_of, merged, opt_str, str_of},
    },
    workspace::{
        config::load_config,
        fs::{read_all_entities, write_flat_entity},
    },
};

/// Where a list-style file keeps its entities and where they land.
pub struct ListSpec {
    /// The file's `schema`.
    pub schema: &'static str,
    /// The array in the file.
    pub list_key: &'static str,
    /// The array in the config snapshot, used to find ids that are taken.
    pub cfg_key: &'static str,
    /// The store directory.
    pub fs_kind: &'static str,
    /// Whether the kind has an enabled set.
    pub tracks_enabled: bool,
}

fn parse_list(spec: &ListSpec, content: &str) -> Result<Vec<Value>, String> {
    let data = parse_json(content)?;
    if opt_str(&data, "schema") != Some(spec.schema) || !data.get(spec.list_key).is_some_and(Value::is_array) {
        return Err(thrown(&format!("Not a valid {} file", spec.schema)));
    }
    Ok(array_of(&data, spec.list_key).to_vec())
}

pub fn preflight_list(spec: &ListSpec, ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    let items = parse_list(spec, &source.content)?;
    let existing: std::collections::HashSet<String> =
        read_all_entities(ws_id, spec.fs_kind).iter().map(|e| str_of(e, "id").to_string()).collect();
    let collision_ids =
        items.iter().map(|i| str_of(i, "id")).filter(|id| existing.contains(*id)).map(str::to_string).collect();
    Ok(Preflight { item_count: items.len(), collision_ids })
}

pub fn run_list(spec: &ListSpec, ws_id: String, source: ImportSource, strategy: Strategy) -> ImportOutcome {
    let items = parse_list(spec, &source.content)?;
    let mut cfg = load_config();
    let existing = workspace_ids(&cfg, spec.cfg_key, &ws_id);

    let (mut imported, mut skipped) = (0, 0);
    for item in &items {
        if existing.contains(str_of(item, "id")) && strategy == Strategy::Keep {
            skipped += 1;
            continue;
        }
        let new_id = if strategy == Strategy::New { generate_id() } else { str_of(item, "id").to_string() };
        let stored = merged(item, &json!({ "id": new_id, "workspaceId": ws_id }));
        upsert_by_id(&mut cfg, spec.cfg_key, stored.clone());
        write_flat_entity(&ws_id, spec.fs_kind, &new_id, &stored).map_err(io_failure)?;
        if spec.tracks_enabled {
            sync_enabled(&ws_id, spec.fs_kind, &new_id, stored.get("enabled").unwrap_or(&Value::Null))?;
        }
        imported += 1;
    }

    commit_config(&cfg)?;
    Ok(Imported::with_skipped(imported, skipped))
}
