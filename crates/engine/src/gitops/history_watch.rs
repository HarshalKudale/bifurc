//! The history watch: one task that notices when a workspace's `HEAD` moves and says so (`history.changed`),
//! so a client showing the history reads it again. It does not matter who moved it: a sync, an auto-pull, or
//! git run by hand in the workspace's directory.
//!
//! Git appends a line to `.git/logs/HEAD` every time `HEAD` moves, so looking at that file's size and time is
//! the whole check: no git process is started to find that nothing happened.

use std::{collections::HashMap, time::Duration, time::SystemTime};

use parking_lot::Mutex;
use serde_json::json;
use tokio::task::JoinHandle;

use crate::{
    kernel::bus::bus,
    utils::json::str_of,
    workspace::{fs::ws_dir, policy, settings::load_settings},
};

const LOOK_EVERY: Duration = Duration::from_secs(2);

static WATCH: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

/// What `.git/logs/HEAD` looked like: `None` for a workspace with no commit yet, or one that is not a
/// repository.
type Mark = Option<(u64, SystemTime)>;

fn mark(ws_id: &str) -> Mark {
    let meta = std::fs::metadata(ws_dir(ws_id).join(".git").join("logs").join("HEAD")).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// The workspaces whose mark is not the one in `seen`, which is brought up to date. A workspace seen for the
/// first time has not changed: its history is whatever a client reads first.
fn moved<M: PartialEq>(seen: &mut HashMap<String, M>, now: Vec<(String, M)>) -> Vec<String> {
    let mut changed = Vec::new();
    for (ws_id, mark) in &now {
        if seen.get(ws_id).is_some_and(|before| before != mark) {
            changed.push(ws_id.clone());
        }
    }
    *seen = now.into_iter().collect();
    changed
}

pub fn start_history_watch() {
    let task = tokio::spawn(async {
        let mut seen = HashMap::new();
        let mut seen_policy = HashMap::new();
        let mut ticker = tokio::time::interval(LOOK_EVERY);
        loop {
            ticker.tick().await;
            let ids: Vec<String> = load_settings()
                .workspaces()
                .iter()
                .map(|ws| str_of(ws, "id").to_string())
                .filter(|id| !id.is_empty())
                .collect();
            let now = ids.iter().map(|id| (id.clone(), mark(id))).collect();
            for ws_id in moved(&mut seen, now) {
                bus().emit("history.changed", json!({ "workspaceId": ws_id }));
            }
            // The policy file is edited by hand: no command tells a client it changed.
            let policies = ids.iter().map(|id| (id.clone(), policy::stamp(id))).collect();
            for ws_id in moved(&mut seen_policy, policies) {
                bus().emit("policy.changed", json!({ "workspaceId": ws_id }));
            }
        }
    });
    if let Some(earlier) = WATCH.lock().replace(task) {
        earlier.abort();
    }
}

pub fn stop_history_watch() {
    if let Some(task) = WATCH.lock().take() {
        task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_has_moved_when_its_mark_is_not_the_one_seen_before() {
        let at = |len: u64| Some((len, SystemTime::UNIX_EPOCH));
        let mut seen = HashMap::new();
        let look = |seen: &mut HashMap<String, Mark>, a: Mark, b: Mark| {
            moved(seen, vec![("a".to_string(), a), ("b".to_string(), b)])
        };
        // First sight is not a change, and neither is a look that finds the same.
        assert!(look(&mut seen, at(10), None).is_empty());
        assert!(look(&mut seen, at(10), None).is_empty());
        // A commit in one, the first commit in the other.
        assert_eq!(look(&mut seen, at(20), None), ["a"]);
        assert_eq!(look(&mut seen, at(20), at(5)), ["b"]);
        assert!(look(&mut seen, at(20), at(5)).is_empty());
    }
}
