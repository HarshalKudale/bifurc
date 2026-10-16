//! Auto-pull: a workspace with a remote has that remote's tip checked every few seconds, and a pull runs when
//! it moved. The workspace's `HEAD` so stays what the remote has, with what was changed here and not yet
//! synced kept on top of it. There is no switch: nothing is pushed until the user syncs.

use std::{collections::HashMap, sync::Arc, time::Duration};

use parking_lot::Mutex;
use serde_json::Value;
use tokio::task::JoinHandle;

use super::{
    remote::{refresh_access, workspace_access},
    status_tracker::invalidate_cache,
    sync_manager::{get_remote_head, sync_pull},
    sync_state::get_sync_config,
};
use crate::{
    features::team_sync,
    runtime::{entity_status::emit_entity_status, instances::notify_config_changed},
};

const POLL_INTERVAL: Duration = Duration::from_secs(10);
/// How many looks at the remote pass between two questions to GitHub about what the account may do with a
/// repository that is someone else's: five minutes' worth. Access given or taken there shows by then.
const ACCESS_CHECK_EVERY: u32 = 30;

struct Poller {
    task: JoinHandle<()>,
    last_known_remote_head: Arc<Mutex<Option<String>>>,
}

static POLLERS: Mutex<Option<HashMap<String, Poller>>> = Mutex::new(None);

/// Pull, and tell clients when it changed what the workspace holds.
pub async fn pull_and_announce(ws_id: &str) -> Value {
    let result = sync_pull(ws_id).await;
    if result.get("ok") == Some(&Value::Bool(true)) && result.get("updated") == Some(&Value::Bool(true)) {
        notify_config_changed();
        invalidate_cache(ws_id);
        emit_entity_status(ws_id);
    }
    result
}

/// Start polling the workspace's remote, replacing a poller that is already running. Does nothing for a
/// workspace with no remote.
pub fn start_auto_sync(ws_id: &str) {
    stop_auto_sync(ws_id);
    if get_sync_config(ws_id).is_none() {
        return;
    }

    let last_known = Arc::new(Mutex::new(None));
    let task = {
        let (ws_id, last_known) = (ws_id.to_string(), last_known.clone());
        tokio::spawn(async move {
            // The first look pulls too: the remote may have moved while the engine was not running.
            let mut ticker = tokio::time::interval(POLL_INTERVAL);
            let mut ticks: u32 = 0;
            loop {
                ticker.tick().await;
                if get_sync_config(&ws_id).is_none() {
                    stop_auto_sync(&ws_id);
                    return;
                }
                // The owner's access does not change; anyone else's is what the owner last made it.
                ticks = ticks.wrapping_add(1);
                let shared = workspace_access(&ws_id).is_some_and(|access| access != "owner");
                if shared && team_sync() && ticks.is_multiple_of(ACCESS_CHECK_EVERY) {
                    let _ = refresh_access(&ws_id).await;
                }
                // Unreadable while offline or signed out: looked at again on the next tick.
                let Some(head) = get_remote_head(&ws_id).await else { continue };
                if last_known.lock().as_deref() == Some(head.as_str()) {
                    continue;
                }
                // A pull that failed is tried again on the next tick.
                if pull_and_announce(&ws_id).await.get("ok") == Some(&Value::Bool(true)) {
                    *last_known.lock() = Some(head);
                }
            }
        })
    };
    POLLERS
        .lock()
        .get_or_insert_with(HashMap::new)
        .insert(ws_id.to_string(), Poller { task, last_known_remote_head: last_known });
}

pub fn stop_auto_sync(ws_id: &str) {
    let poller = POLLERS.lock().as_mut().and_then(|pollers| pollers.remove(ws_id));
    if let Some(poller) = poller {
        poller.task.abort();
    }
}

pub fn stop_all_auto_sync() {
    let pollers = POLLERS.lock().take();
    for poller in pollers.into_iter().flatten().map(|(_, p)| p) {
        poller.task.abort();
    }
}

/// Record the remote tip after this engine published to it, so the next poll does not pull its own push.
pub fn update_last_known_head(ws_id: &str, sha: &str) {
    if let Some(poller) = POLLERS.lock().as_ref().and_then(|pollers| pollers.get(ws_id)) {
        *poller.last_known_remote_head.lock() = Some(sha.to_string());
    }
}

#[cfg(test)]
pub fn poller_count() -> usize {
    POLLERS.lock().as_ref().map_or(0, HashMap::len)
}
