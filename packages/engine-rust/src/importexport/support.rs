//! Helpers the importers and exporters share: JavaScript-flavoured text handling, the config snapshot lookups
//! they all start from, and the bookkeeping every imported entity needs.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value, json};

use crate::{
    runtime::instances::notify_config_changed,
    utils::{
        id::now_ms,
        json::{array_of, is_truthy, str_of},
    },
    workspace::{
        config::save_config,
        fs::{create_folder, ensure_default_folder, sync_enabled_set, write_entity},
    },
};

/// `String(new Error(message))`.
pub fn thrown(message: &str) -> String {
    format!("Error: {message}")
}

/// `String(err)` of a failed file operation.
pub fn io_failure(err: impl std::fmt::Display) -> String {
    thrown(&err.to_string())
}

/// `JSON.parse`, with the failure `String(err)` would give.
pub fn parse_json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|err| format!("SyntaxError: {err}"))
}

/// `JSON.stringify(value, null, 2)`.
pub fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// Whether `c` is whitespace to JavaScript's `\s` and `trim()`.
pub fn is_js_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// `s.trim()`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// A value as a template literal would print it.
pub fn js_str(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        Value::Array(items) => {
            items.iter().map(|v| if v.is_null() { String::new() } else { js_str(v) }).collect::<Vec<_>>().join(",")
        }
        Value::Object(_) => "[object Object]".to_string(),
        other => other.to_string(),
    }
}

/// `value[key]` unless it is absent or `null` (the `??` test).
pub fn present<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).filter(|v| !v.is_null())
}

/// `value[key] ?? default` as text.
pub fn text_or(value: &Value, key: &str, default: &str) -> String {
    present(value, key).map(js_str).unwrap_or_else(|| default.to_string())
}

/// Insert `value` under `key` when there is one.
pub fn put_opt(map: &mut Map<String, Value>, key: &str, value: Option<&Value>) {
    if let Some(value) = value {
        map.insert(key.to_string(), value.clone());
    }
}

/// The display name of workspace `ws_id` in a config snapshot, or `default`.
pub fn workspace_name(cfg: &Value, ws_id: &str, default: &str) -> String {
    array_of(cfg, "workspaces")
        .iter()
        .find(|w| str_of(w, "id") == ws_id)
        .and_then(|w| present(w, "name"))
        .map(js_str)
        .unwrap_or_else(|| default.to_string())
}

/// The entries of `cfg[key]` that belong to workspace `ws_id`.
pub fn in_workspace(cfg: &Value, key: &str, ws_id: &str) -> Vec<Value> {
    array_of(cfg, key).iter().filter(|item| str_of(item, "workspaceId") == ws_id).cloned().collect()
}

/// The ids of `items`.
pub fn ids_of(items: &[Value]) -> HashSet<String> {
    items.iter().map(|item| str_of(item, "id").to_string()).collect()
}

/// The ids of the entries of `cfg[key]` that belong to workspace `ws_id`.
pub fn workspace_ids(cfg: &Value, key: &str, ws_id: &str) -> HashSet<String> {
    ids_of(&in_workspace(cfg, key, ws_id))
}

/// Replace the entry of `cfg[key]` with `item`'s id, or append `item`.
pub fn upsert_by_id(cfg: &mut Value, key: &str, item: Value) {
    let list = crate::utils::json::array_mut(cfg, key);
    match list.iter().position(|existing| str_of(existing, "id") == str_of(&item, "id")) {
        Some(index) => list[index] = item,
        None => list.push(item),
    }
}

/// Switch an entity on or off in its kind's `enabled.json`.
pub fn sync_enabled(ws_id: &str, kind: &str, id: &str, enabled: &Value) -> Result<(), String> {
    sync_enabled_set(ws_id, kind, id, is_truthy(enabled)).map_err(io_failure)
}

/// Save the config snapshot an import changed and tell the proxies.
pub fn commit_config(cfg: &Value) -> Result<(), String> {
    save_config(cfg).map_err(io_failure)?;
    notify_config_changed();
    Ok(())
}

/// The Default folder of a kind, for an imported entity that came with none. It is listed in `cfg[folders_key]` so
/// a snapshot saved later finds its name.
pub fn default_folder(cfg: &mut Value, folders_key: &str, fs_kind: &str, ws_id: &str) -> Result<(String, String), String> {
    let (id, name) = ensure_default_folder(ws_id, fs_kind).map_err(io_failure)?;
    if !array_of(cfg, folders_key).iter().any(|f| str_of(f, "id") == id) {
        push_to(cfg, folders_key, new_folder(&id, &name, None, json!(now_ms()), ws_id));
    }
    Ok((id, name))
}

/// List a folder in `cfg[folders_key]` unless it is there.
fn list_folder(cfg: &mut Value, folders_key: &str, id: &str, name: &str, parent_id: Option<&str>, ws_id: &str) {
    if !array_of(cfg, folders_key).iter().any(|f| str_of(f, "id") == id) {
        push_to(cfg, folders_key, new_folder(id, name, parent_id, json!(now_ms()), ws_id));
    }
}

/// The folder one import run puts everything it brings in: `import-<date>_<time>`, at the top level.
pub fn import_folder(cfg: &mut Value, folders_key: &str, fs_kind: &str, ws_id: &str) -> Result<(String, String), String> {
    let stamp = iso_from_ms(now_ms() as i64)?;
    let name = format!("import-{}", stamp[..19].replace('T', "_").replace(':', "-"));
    let (id, name) = create_folder(ws_id, fs_kind, &name, None, true).map_err(io_failure)?;
    list_folder(cfg, folders_key, &id, &name, None, ws_id);
    Ok((id, name))
}

/// The import folder of one run, made when the first entity needs it so that an import of nothing leaves nothing.
pub struct ImportRoot {
    folders_key: &'static str,
    fs_kind: &'static str,
    made: Option<(String, String)>,
}

impl ImportRoot {
    pub fn new(folders_key: &'static str, fs_kind: &'static str) -> Self {
        Self { folders_key, fs_kind, made: None }
    }

    /// The `(id, name)` of the folder, which is made on the first call.
    pub fn get(&mut self, cfg: &mut Value, ws_id: &str) -> Result<(String, String), String> {
        if self.made.is_none() {
            self.made = Some(import_folder(cfg, self.folders_key, self.fs_kind, ws_id)?);
        }
        Ok(self.made.clone().unwrap_or_default())
    }
}

/// A folder of a source file: `key` is what the file calls it, `parent` the key of the folder it is in.
pub struct FolderSpec {
    pub key: String,
    pub name: String,
    pub parent: Option<String>,
}

/// Make the folders of a source file inside the import folder `root_id`, parents before the folders in them.
/// Returns each folder's id by its key. A folder whose parent is not in the file goes straight into the root.
pub fn add_import_folders(
    cfg: &mut Value,
    folders_key: &str,
    fs_kind: &str,
    ws_id: &str,
    root_id: &str,
    specs: &[FolderSpec],
) -> Result<HashMap<String, String>, String> {
    let known: HashSet<&str> = specs.iter().map(|s| s.key.as_str()).collect();
    let mut made: HashMap<String, String> = HashMap::new();
    let mut waiting: Vec<&FolderSpec> = specs.iter().collect();
    while !waiting.is_empty() {
        let before = waiting.len();
        let mut next = Vec::new();
        for spec in waiting {
            let parent = match spec.parent.as_deref().filter(|p| known.contains(p) && *p != spec.key) {
                None => Some(root_id.to_string()),
                Some(parent) => made.get(parent).cloned(),
            };
            match parent {
                Some(parent) => {
                    let (id, name) =
                        create_folder(ws_id, fs_kind, &spec.name, Some(&parent), false).map_err(io_failure)?;
                    list_folder(cfg, folders_key, &id, &name, Some(&parent), ws_id);
                    made.insert(spec.key.clone(), id);
                }
                None => next.push(spec),
            }
        }
        waiting = next;
        if waiting.len() == before {
            // A cycle of parents: what is left goes into the root.
            for spec in waiting.drain(..) {
                let (id, name) =
                    create_folder(ws_id, fs_kind, &spec.name, Some(root_id), false).map_err(io_failure)?;
                list_folder(cfg, folders_key, &id, &name, Some(root_id), ws_id);
                made.insert(spec.key.clone(), id);
            }
        }
    }
    Ok(made)
}

/// Write a request into the folder `folder` names by id: the Default one when it names none.
pub fn store_request(ws_id: &str, request: &Value, folder: Option<&str>) -> Result<(), String> {
    let id = str_of(request, "id");
    let folder_id = match folder.filter(|id| !id.is_empty()) {
        Some(folder_id) => folder_id.to_string(),
        None => ensure_default_folder(ws_id, "requests").map_err(io_failure)?.0,
    };
    write_entity(ws_id, "requests", id, request, Some(&folder_id)).map_err(io_failure)
}

/// A folder as the config snapshot lists it.
pub fn new_folder(id: &str, name: &str, parent_id: Option<&str>, created_at: Value, ws_id: &str) -> Value {
    json!({ "id": id, "name": name, "parentId": parent_id, "createdAt": created_at, "workspaceId": ws_id })
}

/// Append to `cfg[key]`.
pub fn push_to(cfg: &mut Value, key: &str, item: Value) {
    crate::utils::json::array_mut(cfg, key).push(item);
}

// ── dates ──────────────────────────────────────────────────────────────────────────────────────────────

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `new Date(ms).toISOString()`; the `RangeError` JavaScript throws for a time it cannot represent.
pub fn iso_from_ms(ms: i64) -> Result<String, String> {
    if ms.abs() > 8_640_000_000_000_000 {
        return Err("RangeError: Invalid time value".to_string());
    }
    let (days, rest) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second, milli) = (rest / 3_600_000, rest / 60_000 % 60, rest / 1000 % 60, rest % 1000);
    let year = if (0..=9999).contains(&year) {
        format!("{year:04}")
    } else {
        format!("{}{:06}", if year < 0 { '-' } else { '+' }, year.abs())
    };
    Ok(format!("{year}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milli:03}Z"))
}

/// `new Date(createdAt).toISOString()` for a stored `createdAt`.
pub fn iso_from_value(value: Option<&Value>) -> Result<String, String> {
    match value {
        Some(Value::Number(n)) => match n.as_f64() {
            Some(f) if f.is_finite() => iso_from_ms(f.trunc() as i64),
            _ => Err("RangeError: Invalid time value".to_string()),
        },
        Some(Value::String(s)) => {
            ms_from_iso(s).map(iso_from_ms).unwrap_or_else(|| Err("RangeError: Invalid time value".to_string()))
        }
        _ => Err("RangeError: Invalid time value".to_string()),
    }
}

/// `new Date(text).getTime()` for an ISO 8601 text; `None` where JavaScript would give `NaN`.
pub fn ms_from_iso(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date, time) = match text.split_once(['T', ' ']) {
        Some((date, time)) => (date, Some(time)),
        None => (text, None),
    };
    let mut parts = date.splitn(3, '-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next().map_or(Some(1), |m| m.parse().ok())?;
    let day: i64 = parts.next().map_or(Some(1), |d| d.parse().ok())?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut ms = days_from_civil(year, month, day) * 86_400_000;
    if let Some(time) = time {
        let (clock, offset) = match time.find(['Z', 'z', '+']).or_else(|| time.rfind('-')) {
            Some(at) => (&time[..at], Some(&time[at..])),
            None => (time, None),
        };
        let mut fields = clock.split(':');
        let hour: i64 = fields.next()?.parse().ok()?;
        let minute: i64 = fields.next().map_or(Some(0), |m| m.parse().ok())?;
        let seconds = fields.next().unwrap_or("0");
        let (second, fraction) = seconds.split_once('.').unwrap_or((seconds, "0"));
        let second: i64 = second.parse().ok()?;
        let milli: i64 = format!("{fraction:0<3}")[..3].parse().ok()?;
        if hour > 24 || minute > 59 || second > 59 {
            return None;
        }
        ms += ((hour * 60 + minute) * 60 + second) * 1000 + milli;
        if let Some(offset) = offset.filter(|o| o.starts_with(['+', '-'])) {
            let sign = if offset.starts_with('-') { -1 } else { 1 };
            let digits: String = offset[1..].chars().filter(char::is_ascii_digit).collect();
            if digits.len() < 2 {
                return None;
            }
            let (hours, minutes) =
                (digits[..2].parse::<i64>().ok()?, digits.get(2..4).map_or(Some(0), |m| m.parse::<i64>().ok())?);
            ms -= sign * (hours * 60 + minutes) * 60_000;
        }
    }
    Some(ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip_through_iso() {
        assert_eq!(iso_from_ms(1_700_000_000_123).unwrap(), "2023-11-14T22:13:20.123Z");
        assert_eq!(ms_from_iso("2023-11-14T22:13:20.123Z"), Some(1_700_000_000_123));
        assert_eq!(ms_from_iso("2023-11-14T23:13:20+01:00"), Some(1_700_000_000_000));
        assert_eq!(ms_from_iso("2023-11-14"), Some(1_699_920_000_000));
        assert_eq!(ms_from_iso("garbage"), None);
        assert!(iso_from_value(None).is_err());
    }

    #[test]
    fn javascript_text_rules() {
        assert_eq!(js_trim("\u{feff} a \n"), "a");
        assert_eq!(js_str(&json!(3)), "3");
        assert_eq!(js_str(&json!(null)), "null");
    }
}
