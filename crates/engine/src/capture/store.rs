//! The captured-traffic history, per workspace. The engine records it whether or not any client is
//! watching, unless the workspace's capture is paused.
//!
//! Newest first: every pinned entry (`"pinned": true`) plus the newest `captureLimit` (default
//! [`DEFAULT_CAPTURE_LIMIT`]) unpinned ones per workspace. Persisted under `<dataDir>/.capture/`,
//! outside the git-tracked workspace directories: captured traffic carries real headers and bodies and must
//! never be committed with the workspace. Entries are kept as JSON so a file round-trips unchanged.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::OnceLock,
    time::Duration,
};

use parking_lot::Mutex;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;
use tokio::runtime::Handle;

use crate::{
    protocol::{EngineError, Result},
    runtime::notice,
    workspace::{paths::data_dir, settings::load_settings},
};

pub use crate::workspace::settings::DEFAULT_CAPTURE_LIMIT;

const WRITE_DELAY: Duration = Duration::from_millis(500);

/// What `encodeURIComponent` leaves alone besides letters and digits.
const URI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

#[derive(Default)]
struct Store {
    entries: HashMap<String, Vec<Value>>,
    /// Workspaces with a write scheduled, and the token of the timer that will perform it.
    pending: HashMap<String, u64>,
    next_token: u64,
    /// Workspaces whose capture is paused. In memory only: an engine restart resumes capturing.
    paused: HashSet<String>,
    /// The unpinned entries kept per workspace; read from the settings on first use, refreshed when they change.
    limit: Option<usize>,
    /// The trace header name (`None` = off); outer `None` = not read yet.
    trace_header: Option<Option<String>>,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);
static RUNTIME: OnceLock<Handle> = OnceLock::new();

fn with_store<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    let mut guard = STORE.lock();
    f(guard.get_or_insert_with(Store::default))
}

/// Remember the runtime so a write can be scheduled from a thread outside it.
pub(super) fn remember_runtime() {
    if let Ok(handle) = Handle::try_current() {
        let _ = RUNTIME.set(handle);
    }
}

fn capture_file(ws_id: &str) -> PathBuf {
    data_dir().join(".capture").join(format!("{}.json", utf8_percent_encode(ws_id, URI_COMPONENT)))
}

pub(super) fn is_pinned(entry: &Value) -> bool {
    entry.get("pinned").and_then(Value::as_bool) == Some(true)
}

/// Keep every pinned entry and the newest `limit` unpinned ones; the order stays.
fn evict(entries: &mut Vec<Value>, limit: usize) {
    let mut unpinned = 0;
    entries.retain(|entry| {
        is_pinned(entry) || {
            unpinned += 1;
            unpinned <= limit
        }
    });
}

fn limit_of(store: &mut Store) -> usize {
    *store.limit.get_or_insert_with(|| load_settings().capture_limit())
}

/// The limit changed in the settings: take it, and evict from the workspaces in memory.
pub fn refresh_capture_limit() {
    with_store(|store| {
        let limit = load_settings().capture_limit();
        store.limit = Some(limit);
        store.trace_header = Some(load_settings().trace_header());
        let shrunk: Vec<String> = store
            .entries
            .iter_mut()
            .filter_map(|(ws_id, entries)| {
                let before = entries.len();
                evict(entries, limit);
                (entries.len() < before).then(|| ws_id.clone())
            })
            .collect();
        for ws_id in shrunk {
            schedule_write(store, &ws_id);
        }
    });
}

/// The header the proxies stamp the capture id into (`None` = off); cached like the limit.
pub fn trace_header() -> Option<String> {
    with_store(|store| store.trace_header.get_or_insert_with(|| load_settings().trace_header()).clone())
}

/// The limit in force.
pub fn capture_limit() -> usize {
    with_store(limit_of)
}

fn entries_of<'a>(store: &'a mut Store, ws_id: &str) -> &'a mut Vec<Value> {
    let limit = limit_of(store);
    store.entries.entry(ws_id.to_string()).or_insert_with(|| {
        let loaded =
            std::fs::read(capture_file(ws_id)).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        match loaded {
            Some(Value::Array(mut entries)) => {
                evict(&mut entries, limit);
                // No stream outlives the engine that relayed it: one still open when it stopped is over.
                for entry in &mut entries {
                    if let Some(entry) = entry.as_object_mut() {
                        entry.remove("live");
                    }
                }
                entries
            }
            _ => Vec::new(),
        }
    })
}

fn write_now(ws_id: &str) {
    let snapshot = with_store(|store| {
        store.pending.remove(ws_id);
        store.entries.get(ws_id).cloned().unwrap_or_default()
    });
    let file = capture_file(ws_id);
    let written = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&file, serde_json::to_string(&snapshot).unwrap_or_else(|_| "[]".into())));
    if let Err(err) = written {
        notice(&format!("[capture] could not persist {}: {err}", file.display()));
        crate::analytics::failed("capture_persist");
    }
}

/// Traffic arrives in bursts; coalesce the writes. The first change arms a timer, later ones ride on it.
fn schedule_write(store: &mut Store, ws_id: &str) {
    if store.pending.contains_key(ws_id) {
        return;
    }
    let handle = Handle::try_current().ok().or_else(|| RUNTIME.get().cloned());
    let Some(handle) = handle else {
        // No runtime to time the write on: persist at the next flush.
        store.next_token += 1;
        let token = store.next_token;
        store.pending.insert(ws_id.to_string(), token);
        return;
    };
    store.next_token += 1;
    let token = store.next_token;
    store.pending.insert(ws_id.to_string(), token);
    let ws_id = ws_id.to_string();
    handle.spawn(async move {
        tokio::time::sleep(WRITE_DELAY).await;
        if with_store(|store| store.pending.get(&ws_id) == Some(&token)) {
            write_now(&ws_id);
        }
    });
}

/// The messages a captured stream keeps; older ones are dropped from the front.
pub const MAX_STREAM_MESSAGES: usize = 500;

/// Pause or resume recording for a workspace. Entries that pass while paused are dropped, not queued.
pub fn set_capture_paused(ws_id: &str, paused: bool) {
    with_store(|store| {
        if paused {
            store.paused.insert(ws_id.to_string());
        } else {
            store.paused.remove(ws_id);
        }
    });
}

pub fn is_capture_paused(ws_id: &str) -> bool {
    with_store(|store| store.paused.contains(ws_id))
}

/// Record one exchange (the JSON of a `RequestLogEntry`). An entry with no workspace is not attributable, so
/// it is not kept. An entry already kept (a stream logged when it opened) is updated where it is, keeping its
/// pin and its messages; that also happens while paused, which only stops new exchanges.
pub fn record_capture(entry: &Value) {
    let Some(ws_id) = entry.get("workspaceId").and_then(Value::as_str).filter(|id| !id.is_empty()) else { return };
    let id = entry.get("id");
    with_store(|store| {
        let paused = store.paused.contains(ws_id);
        let limit = limit_of(store);
        let entries = entries_of(store, ws_id);
        let mut entry = entry.clone();
        match entries.iter().position(|existing| existing.get("id") == id) {
            Some(at) => {
                if is_pinned(&entries[at]) {
                    entry["pinned"] = Value::Bool(true);
                }
                if entry.get("messages").is_none()
                    && let Some(messages) = entries[at].get("messages")
                {
                    entry["messages"] = messages.clone();
                }
                entries[at] = entry;
            }
            None if paused => return,
            None => {
                entries.insert(0, entry);
                evict(entries, limit);
            }
        }
        schedule_write(store, ws_id);
    });
}

/// Append one message to the captured stream it belongs to (a `log.message` payload). A stream that is not
/// kept (it opened while paused, or was removed) records nothing.
pub fn record_capture_message(payload: &Value) {
    let (Some(ws_id), Some(id)) =
        (payload.get("workspaceId").and_then(Value::as_str), payload.get("logId").and_then(Value::as_str))
    else {
        return;
    };
    with_store(|store| {
        let entries = entries_of(store, ws_id);
        let Some(entry) = entries.iter_mut().find(|e| e.get("id").and_then(Value::as_str) == Some(id)) else {
            return;
        };
        if !entry["messages"].is_array() {
            entry["messages"] = Value::Array(Vec::new());
        }
        if let Some(messages) = entry["messages"].as_array_mut() {
            messages.push(payload.get("message").cloned().unwrap_or(Value::Null));
            crate::kernel::streams::trim_log(messages, MAX_STREAM_MESSAGES);
        }
        schedule_write(store, ws_id);
    });
}

/// The workspace's captured exchanges, newest first.
pub fn list_capture(ws_id: &str) -> Vec<Value> {
    with_store(|store| entries_of(store, ws_id).clone())
}

pub fn get_capture(ws_id: &str, id: &str) -> Option<Value> {
    with_store(|store| {
        entries_of(store, ws_id).iter().find(|e| e.get("id").and_then(Value::as_str) == Some(id)).cloned()
    })
}

/// The entry a `capture.build*` call names: the whole `entry`, or a `{workspaceId, id}` reference into the
/// store.
pub fn resolve_capture_entry(params: &Value) -> Result<Value> {
    if let Some(entry) = params.get("entry") {
        return Ok(entry.clone());
    }
    let ws_id = params.get("workspaceId").and_then(Value::as_str).unwrap_or("");
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    get_capture(ws_id, id).ok_or_else(|| {
        EngineError::not_found(format!("No captured entry \"{id}\" in workspace \"{ws_id}\"."))
            .keyed("capture.entry_not_found_in", serde_json::json!({ "id": id, "workspace": ws_id }))
    })
}

pub fn clear_capture(ws_id: &str) {
    with_store(|store| {
        store.entries.insert(ws_id.to_string(), Vec::new());
        schedule_write(store, ws_id);
    });
}

/// Drop the given exchanges; returns how many were present.
pub fn remove_capture(ws_id: &str, ids: &[String]) -> usize {
    with_store(|store| {
        let entries = entries_of(store, ws_id);
        let before = entries.len();
        entries.retain(|e| !e.get("id").and_then(Value::as_str).is_some_and(|id| ids.iter().any(|drop| drop == id)));
        let removed = before - entries.len();
        if removed > 0 {
            schedule_write(store, ws_id);
        }
        removed
    })
}

/// Pin or unpin the given exchanges. Returns how many changed and which ids are not in the history. An
/// unpinned entry beyond the limit is evicted at once.
pub fn pin_capture(ws_id: &str, ids: &[String], pinned: bool) -> (usize, Vec<String>) {
    with_store(|store| {
        let limit = limit_of(store);
        let entries = entries_of(store, ws_id);
        let (mut changed, mut missing) = (0, Vec::new());
        for id in ids {
            match entries.iter_mut().find(|e| e.get("id").and_then(Value::as_str) == Some(id)) {
                None => missing.push(id.clone()),
                Some(entry) if is_pinned(entry) != pinned => {
                    changed += 1;
                    if pinned {
                        entry["pinned"] = Value::Bool(true);
                    } else if let Some(object) = entry.as_object_mut() {
                        object.remove("pinned");
                    }
                }
                Some(_) => {}
            }
        }
        if changed > 0 {
            evict(entries, limit);
            schedule_write(store, ws_id);
        }
        (changed, missing)
    })
}

/// Write every pending workspace now (shutdown, tests).
pub fn flush_capture() {
    let pending: Vec<String> = with_store(|store| store.pending.keys().cloned().collect());
    for ws_id in pending {
        write_now(&ws_id);
    }
}

/// Forget the in-memory state, so the next read comes from disk (tests, data-root changes).
pub fn reset_capture() {
    with_store(|store| *store = Store::default());
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::testing::scratch_data_root;

    fn entry(ws: &str, id: &str) -> Value {
        json!({ "id": id, "ts": 1, "method": "GET", "url": "http://h/", "workspaceId": ws })
    }

    fn ids(ws: &str) -> Vec<String> {
        list_capture(ws).iter().map(|e| e["id"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn a_stream_keeps_its_messages_and_updates_through_a_pause() {
        let _root = scratch_data_root();
        reset_capture();
        let message = |id: &str, n: u64| json!({ "logId": id, "workspaceId": "ws1", "message": { "data": n } });
        record_capture(&json!({ "id": "s", "ts": 1, "workspaceId": "ws1", "kind": "ws", "live": true }));
        for n in 0..(MAX_STREAM_MESSAGES as u64 + 2) {
            record_capture_message(&message("s", n));
        }
        record_capture_message(&message("gone", 0));
        set_capture_paused("ws1", true);
        record_capture(&entry("ws1", "new"));
        record_capture(&json!({ "id": "s", "ts": 1, "workspaceId": "ws1", "kind": "ws" }));
        set_capture_paused("ws1", false);

        assert_eq!(ids("ws1"), ["s"]);
        let stream = get_capture("ws1", "s").unwrap();
        assert!(stream.get("live").is_none());
        let messages = stream["messages"].as_array().unwrap();
        assert_eq!(messages.len(), MAX_STREAM_MESSAGES);
        assert_eq!(messages[0]["data"], 2);
        reset_capture();
    }

    #[test]
    fn newest_first_deduped_and_capped() {
        let _root = scratch_data_root();
        reset_capture();
        record_capture(&entry("ws1", "a"));
        record_capture(&entry("ws1", "b"));
        record_capture(&entry("ws1", "a"));
        // Logged again (a stream that closed), an entry is updated where it is.
        assert_eq!(ids("ws1"), ["b", "a"]);
        for i in 0..250 {
            record_capture(&entry("ws1", &format!("e{i}")));
        }
        let all = ids("ws1");
        assert_eq!(all.len(), DEFAULT_CAPTURE_LIMIT);
        assert_eq!(all[0], "e249");
        assert_eq!(all[199], "e50");
        reset_capture();
    }

    fn pinned_ids(ws: &str) -> Vec<String> {
        let all = list_capture(ws);
        all.iter().filter(|e| is_pinned(e)).map(|e| e["id"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn pinned_entries_survive_eviction_and_do_not_count_toward_the_limit() {
        let _root = scratch_data_root();
        reset_capture();
        record_capture(&entry("ws1", "keep"));
        assert_eq!(pin_capture("ws1", &["keep".into(), "nope".into()], true), (1, vec!["nope".to_string()]));
        assert_eq!(pin_capture("ws1", &["keep".into()], true).0, 0);
        for i in 0..DEFAULT_CAPTURE_LIMIT + 5 {
            record_capture(&entry("ws1", &format!("e{i}")));
        }
        let all = ids("ws1");
        assert_eq!(all.len(), DEFAULT_CAPTURE_LIMIT + 1);
        assert_eq!(all.last().unwrap(), "keep");
        // Re-recording the pinned exchange keeps its pin; unpinning it past the limit evicts it.
        record_capture(&entry("ws1", "keep"));
        assert_eq!(pinned_ids("ws1"), ["keep"]);
        assert_eq!(ids("ws1").len(), DEFAULT_CAPTURE_LIMIT + 1);
        assert_eq!(pin_capture("ws1", &["keep".into()], false).0, 1);
        assert_eq!(ids("ws1").len(), DEFAULT_CAPTURE_LIMIT);
        reset_capture();
    }

    #[test]
    fn a_lowered_limit_evicts_the_unpinned_entries_in_memory() {
        let _root = scratch_data_root();
        reset_capture();
        for i in 0..30 {
            record_capture(&entry("ws1", &format!("e{i}")));
        }
        pin_capture("ws1", &["e0".into()], true);
        let mut settings = load_settings();
        settings.set("captureLimit", json!(10));
        crate::workspace::settings::save_settings(&settings).unwrap();
        refresh_capture_limit();
        assert_eq!(capture_limit(), 10);
        let all = ids("ws1");
        assert_eq!((all.len(), all[0].as_str(), all[10].as_str()), (11, "e29", "e0"));
        reset_capture();
    }

    #[test]
    fn a_file_without_pinned_loads_and_one_with_it_round_trips() {
        let root = scratch_data_root();
        reset_capture();
        let dir = root.dir.path().join(".capture");
        std::fs::create_dir_all(&dir).unwrap();
        let pinned = json!({ "id": "p", "ts": 1, "workspaceId": "ws1", "pinned": true });
        std::fs::write(dir.join("ws1.json"), serde_json::to_string(&json!([entry("ws1", "a"), pinned])).unwrap())
            .unwrap();
        assert_eq!(ids("ws1"), ["a", "p"]);
        assert_eq!(pinned_ids("ws1"), ["p"]);
        pin_capture("ws1", &["a".into()], true);
        flush_capture();
        reset_capture();
        assert_eq!(pinned_ids("ws1"), ["a", "p"]);
        reset_capture();
    }

    #[test]
    fn entries_without_a_workspace_are_not_kept_and_pause_drops_entries() {
        let _root = scratch_data_root();
        reset_capture();
        record_capture(&json!({ "id": "x", "method": "GET" }));
        record_capture(&json!({ "id": "x", "workspaceId": "" }));
        assert!(list_capture("ws1").is_empty());

        set_capture_paused("ws1", true);
        assert!(is_capture_paused("ws1"));
        record_capture(&entry("ws1", "a"));
        assert!(list_capture("ws1").is_empty());
        set_capture_paused("ws1", false);
        record_capture(&entry("ws1", "b"));
        assert_eq!(ids("ws1"), ["b"]);
        reset_capture();
    }

    #[test]
    fn get_remove_clear_and_resolve() {
        let _root = scratch_data_root();
        reset_capture();
        for id in ["a", "b", "c"] {
            record_capture(&entry("ws1", id));
        }
        assert_eq!(get_capture("ws1", "b").unwrap()["id"], "b");
        assert!(get_capture("ws1", "zzz").is_none());
        assert_eq!(remove_capture("ws1", &["a".into(), "zzz".into(), "c".into()]), 2);
        assert_eq!(ids("ws1"), ["b"]);

        let whole = json!({ "entry": { "id": "inline" } });
        assert_eq!(resolve_capture_entry(&whole).unwrap()["id"], "inline");
        let by_id = json!({ "workspaceId": "ws1", "id": "b" });
        assert_eq!(resolve_capture_entry(&by_id).unwrap()["id"], "b");
        let missing = resolve_capture_entry(&json!({ "workspaceId": "ws1", "id": "nope" })).unwrap_err();
        assert_eq!(missing.code, crate::protocol::ErrorCode::NotFound);
        assert_eq!(missing.message, "No captured entry \"nope\" in workspace \"ws1\".");

        clear_capture("ws1");
        assert!(list_capture("ws1").is_empty());
        reset_capture();
    }

    #[test]
    fn persistence_round_trips_through_the_capture_directory() {
        let root = scratch_data_root();
        reset_capture();
        record_capture(&entry("ws 1/é", "a"));
        record_capture(&json!({ "id": "b", "ts": 1, "workspaceId": "ws 1/é", "kind": "ws", "live": true }));
        flush_capture();
        let file = root.dir.path().join(".capture").join("ws%201%2F%C3%A9.json");
        assert!(file.exists(), "{}", file.display());

        reset_capture();
        assert_eq!(ids("ws 1/é"), ["b", "a"]);
        // A stream that was open when the engine stopped is read back as ended.
        assert!(get_capture("ws 1/é", "b").unwrap().get("live").is_none());
        reset_capture();
    }

    #[tokio::test]
    async fn writes_are_debounced() {
        let root = scratch_data_root();
        reset_capture();
        remember_runtime();
        record_capture(&entry("ws1", "a"));
        let file = root.dir.path().join(".capture").join("ws1.json");
        assert!(!file.exists());
        tokio::time::sleep(WRITE_DELAY + Duration::from_millis(400)).await;
        assert!(file.exists());
        let stored: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(stored[0]["id"], "a");
        reset_capture();
    }
}
