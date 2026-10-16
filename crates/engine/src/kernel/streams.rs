//! The bookkeeping every live-connection registry (WebSocket, SSE, Socket.IO, MCP) shares: a map of entries
//! keyed by `(workspaceId, id)`, the generation check that lets a superseded connection's task no-op, the
//! bounded message log, and events that go out only after the lock is released.
//!
//! What an entry holds and how it changes stays in the protocol's own module; each implements [`Stream`].

use std::collections::HashMap;

use parking_lot::{Mutex, MutexGuard};
use serde_json::Value;

use super::bus;

/// What an entry gives the shared registry.
pub trait Stream {
    fn workspace_id(&self) -> &str;
    /// Whether a connection task is running for it.
    fn is_live(&self) -> bool;
    /// Bumped on every connect; a task whose generation is behind the entry's no-ops.
    fn generation(&self) -> u64;
    fn snapshot(&self) -> Value;
    fn clear_log(&mut self);
}

/// A bus event to emit once the registry lock is released.
pub struct Event {
    pub name: &'static str,
    pub payload: Value,
}

/// Events are emitted after the registry lock is released: a listener may call back into the registry.
pub fn emit(events: Vec<Event>) {
    for event in events {
        bus().emit(event.name, event.payload);
    }
}

/// `\0` separates the ids: an id containing a colon is legal, and a collision would attach one workspace's
/// connection to another's.
pub fn key_of(workspace_id: &str, id: &str) -> String {
    format!("{workspace_id}\0{id}")
}

/// Drop the oldest lines so the log holds at most `max`.
pub fn trim_log(messages: &mut Vec<Value>, max: usize) {
    if messages.len() > max {
        let excess = messages.len() - max;
        messages.drain(..excess);
    }
}

/// Finished entries kept per workspace. The UI re-reads a finished stream's log from the engine when its tab
/// reopens (`*.status`), and stream sessions are not in response history, so a finished entry is the only
/// copy: keep the latest few, drop the oldest.
pub const MAX_FINISHED_PER_WORKSPACE: usize = 20;

pub struct Streams<E> {
    entries: Mutex<HashMap<String, E>>,
    /// When each finished entry was first seen finished; lower is older. Only finished entries have one.
    finished: Mutex<HashMap<String, u64>>,
}

impl<E: Stream> Default for Streams<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Stream> Streams<E> {
    pub fn new() -> Self {
        Self { entries: Mutex::new(HashMap::new()), finished: Mutex::new(HashMap::new()) }
    }

    pub fn lock(&self) -> MutexGuard<'_, HashMap<String, E>> {
        self.entries.lock()
    }

    /// Drop the oldest finished entries of a workspace beyond `max`. Live entries are never dropped, and a
    /// later connect to a dropped id simply creates a fresh entry.
    fn evict_finished(&self, workspace_id: &str, max: usize) {
        let mut entries = self.entries.lock();
        let mut finished = self.finished.lock();
        finished.retain(|key, _| entries.get(key).is_some_and(|e| !e.is_live()));
        let mut next = finished.values().max().map_or(0, |n| n + 1);
        let mut mine: Vec<(u64, String)> = Vec::new();
        for (key, _) in entries.iter().filter(|(_, e)| !e.is_live() && e.workspace_id() == workspace_id) {
            let stamp = *finished.entry(key.clone()).or_insert_with(|| {
                next += 1;
                next - 1
            });
            mine.push((stamp, key.clone()));
        }
        mine.sort();
        let excess = mine.len().saturating_sub(max);
        for (_, key) in mine.into_iter().take(excess) {
            entries.remove(&key);
            finished.remove(&key);
        }
    }

    /// The snapshot for one entry, live or finished.
    pub fn get(&self, workspace_id: &str, id: &str) -> Option<Value> {
        self.lock().get(&key_of(workspace_id, id)).map(E::snapshot)
    }

    /// The live entries of a workspace; a finished one keeps its entry but does not appear here.
    pub fn list(&self, workspace_id: &str) -> Vec<Value> {
        self.lock().values().filter(|e| e.workspace_id() == workspace_id && e.is_live()).map(E::snapshot).collect()
    }

    pub fn live_count(&self, workspace_id: &str) -> usize {
        self.lock().values().filter(|e| e.workspace_id() == workspace_id && e.is_live()).count()
    }

    /// Drop the retained log of one entry; `false` when the id was never opened.
    pub fn clear_log(&self, workspace_id: &str, id: &str) -> bool {
        self.lock().get_mut(&key_of(workspace_id, id)).map(E::clear_log).is_some()
    }

    /// Change the entry of a connection that has not been superseded and emit what `change` returns. `false`
    /// when the connection has been superseded.
    pub fn record(&self, key: &str, generation: u64, change: impl FnOnce(&mut E) -> Vec<Event>) -> bool {
        let (events, workspace_id) = {
            let mut entries = self.lock();
            let Some(entry) = entries.get_mut(key).filter(|e| e.generation() == generation) else {
                return false;
            };
            (change(entry), entry.workspace_id().to_owned())
        };
        self.evict_finished(&workspace_id, MAX_FINISHED_PER_WORKSPACE);
        emit(events);
        true
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    struct Probe {
        workspace_id: String,
        live: bool,
        generation: u64,
        log: Vec<Value>,
    }

    impl Stream for Probe {
        fn workspace_id(&self) -> &str {
            &self.workspace_id
        }
        fn is_live(&self) -> bool {
            self.live
        }
        fn generation(&self) -> u64 {
            self.generation
        }
        fn snapshot(&self) -> Value {
            json!({ "workspaceId": self.workspace_id, "log": self.log })
        }
        fn clear_log(&mut self) {
            self.log.clear();
        }
    }

    fn probe(workspace_id: &str, live: bool) -> Probe {
        Probe { workspace_id: workspace_id.into(), live, generation: 1, log: vec![json!(1)] }
    }

    #[test]
    fn keys_do_not_collide_across_workspaces() {
        assert_ne!(key_of("a:b", "c"), key_of("a", "b:c"));
    }

    #[test]
    fn the_log_drops_from_the_front() {
        let mut log: Vec<Value> = (0..5).map(|n| json!(n)).collect();
        trim_log(&mut log, 3);
        assert_eq!(log, vec![json!(2), json!(3), json!(4)]);
    }

    #[test]
    fn list_and_count_cover_the_live_entries_of_one_workspace() {
        let streams = Streams::new();
        streams.lock().insert(key_of("w", "live"), probe("w", true));
        streams.lock().insert(key_of("w", "done"), probe("w", false));
        streams.lock().insert(key_of("x", "live"), probe("x", true));
        assert_eq!(streams.live_count("w"), 1);
        assert_eq!(streams.list("w").len(), 1);
        assert!(streams.get("w", "done").is_some());
        assert!(streams.get("w", "never").is_none());
    }

    #[test]
    fn clear_empties_the_log_and_reports_an_unknown_id() {
        let streams = Streams::new();
        streams.lock().insert(key_of("w", "a"), probe("w", true));
        assert!(streams.clear_log("w", "a"));
        assert_eq!(streams.get("w", "a").unwrap()["log"], json!([]));
        assert!(!streams.clear_log("w", "b"));
    }

    #[test]
    fn a_superseded_generation_changes_nothing() {
        let streams = Streams::new();
        let key = key_of("w", "a");
        streams.lock().insert(key.clone(), probe("w", true));
        assert!(!streams.record(&key, 0, |e| {
            e.live = false;
            Vec::new()
        }));
        assert!(streams.lock()[&key].live);
        assert!(streams.record(&key, 1, |e| {
            e.live = false;
            Vec::new()
        }));
        assert!(!streams.lock()[&key].live);
        assert!(!streams.record("missing", 1, |_| Vec::new()));
    }

    #[test]
    fn the_oldest_finished_entries_go_first_and_live_ones_stay() {
        let streams = Streams::new();
        streams.lock().insert(key_of("w", "live"), probe("w", true));
        streams.lock().insert(key_of("x", "other"), probe("x", false));
        for id in ["a", "b", "c"] {
            streams.lock().insert(key_of("w", id), probe("w", true));
            let key = key_of("w", id);
            assert!(streams.record(&key, 1, |e| {
                e.live = false;
                Vec::new()
            }));
            streams.evict_finished("w", 2);
        }
        assert!(streams.get("w", "a").is_none());
        assert!(streams.get("w", "b").is_some() && streams.get("w", "c").is_some());
        assert!(streams.get("w", "live").is_some() && streams.get("x", "other").is_some());
    }

    #[test]
    fn a_reconnect_after_eviction_starts_a_fresh_entry() {
        let streams = Streams::new();
        for id in ["a", "b"] {
            streams.lock().insert(key_of("w", id), probe("w", false));
        }
        streams.evict_finished("w", 1);
        assert!(streams.get("w", "a").is_none() || streams.get("w", "b").is_none());
        let gone = if streams.get("w", "a").is_none() { "a" } else { "b" };
        streams.lock().insert(key_of("w", gone), Probe { generation: 1, ..probe("w", true) });
        assert!(streams.record(&key_of("w", gone), 1, |_| Vec::new()));
        assert_eq!(streams.live_count("w"), 1);
    }
}
