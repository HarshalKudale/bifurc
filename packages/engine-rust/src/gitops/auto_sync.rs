//! Auto-sync: while a workspace has it switched on, the remote branch's tip is checked every 30 seconds and a
//! pull runs when it moved.

use std::{collections::HashMap, sync::Arc, time::Duration};

use parking_lot::Mutex;
use serde_json::Value;
use tokio::task::JoinHandle;

use super::{
    sync_manager::{get_remote_head, sync_pull},
    sync_state::get_sync_config,
};
use crate::{runtime::instances::notify_config_changed, utils::json::bool_of};

const POLL_INTERVAL: Duration = Duration::from_secs(30);

struct Poller {
    task: JoinHandle<()>,
    last_known_remote_head: Arc<Mutex<Option<String>>>,
}

static POLLERS: Mutex<Option<HashMap<String, Poller>>> = Mutex::new(None);

fn auto_sync_enabled(ws_id: &str) -> bool {
    get_sync_config(ws_id).is_some_and(|config| bool_of(&config, "autoSync", false))
}

/// Start polling the workspace's remote, replacing a poller that is already running. Does nothing unless the
/// workspace's auto-sync is switched on.
pub fn start_auto_sync(ws_id: &str) {
    stop_auto_sync(ws_id);
    if !auto_sync_enabled(ws_id) {
        return;
    }

    let last_known = Arc::new(Mutex::new(None));
    let task = {
        let (ws_id, last_known) = (ws_id.to_string(), last_known.clone());
        tokio::spawn(async move {
            // Seed the last-known head so the first tick does not pull straight away.
            let seed = get_remote_head(&ws_id).await;
            *last_known.lock() = seed;
            let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + POLL_INTERVAL, POLL_INTERVAL);
            loop {
                ticker.tick().await;
                if !auto_sync_enabled(&ws_id) {
                    stop_auto_sync(&ws_id);
                    return;
                }
                let Some(head) = get_remote_head(&ws_id).await else { continue };
                if last_known.lock().as_deref() == Some(head.as_str()) {
                    continue;
                }
                *last_known.lock() = Some(head);
                let result = sync_pull(&ws_id).await;
                if result.get("ok") == Some(&Value::Bool(true)) && result.get("updated") == Some(&Value::Bool(true)) {
                    notify_config_changed();
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
