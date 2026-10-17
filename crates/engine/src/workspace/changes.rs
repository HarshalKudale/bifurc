//! The change feed: the last changes made to a workspace's entities, in memory, with who made each. It is
//! how one agent learns what another changed (`bifurc changes`). The audit log keeps the full history; this
//! ring holds the last [`CAP`] changes since the engine started and is never written to disk.

use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    sync::LazyLock,
};

use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::{env::session, kernel::CommandRegistry, utils::id::now_ms};

const CAP: usize = 2000;

tokio::task_local! {
    /// The client kind (`desktop-ui`, `cli`, ...) the running command came from.
    static CLIENT: String;
}

/// Run `work` as a command of `client`, so changes it records say who made them.
pub async fn as_client<F: Future>(client: Option<&str>, work: F) -> F::Output {
    CLIENT.scope(client.unwrap_or_default().to_string(), work).await
}

/// `work` for a task about to be spawned, which keeps the client of the task that calls this.
pub fn carry<F: Future>(work: F) -> impl Future<Output = F::Output> {
    let client = CLIENT.try_with(Clone::clone).ok();
    async move {
        match client {
            Some(client) => CLIENT.scope(client, work).await,
            None => work.await,
        }
    }
}

/// `session:<name>` in a named session, else the client kind (`app`, `cli`, `extension`); `engine` when no
/// client is known. The command line and MCP both connect as `cli`.
pub fn by() -> String {
    if let Some(name) = session::named() {
        return format!("session:{name}");
    }
    match CLIENT.try_with(Clone::clone).unwrap_or_default().as_str() {
        "" => "engine".into(),
        "desktop-ui" => "app".into(),
        "bifurc-extension" => "extension".into(),
        other => other.into(),
    }
}

#[derive(Default)]
struct Log {
    next: u64,
    ring: VecDeque<Value>,
    /// By workspace and asker (`by()`): the last seq it was shown.
    cursors: HashMap<(String, String), u64>,
}

static LOG: LazyLock<Mutex<Log>> = LazyLock::new(Default::default);

impl Log {
    fn push(&mut self, mut change: Value) {
        self.next += 1;
        change["seq"] = json!(self.next);
        if self.ring.len() == CAP {
            self.ring.pop_front();
        }
        self.ring.push_back(change);
    }

    fn list(&mut self, ws: &str, me: &str, params: &Value) -> Value {
        let after = params.get("after").and_then(Value::as_u64);
        let since_ts = params.get("sinceTs").and_then(Value::as_i64);
        let explicit = after.is_some() || since_ts.is_some();
        let advance = params.get("advance").and_then(Value::as_bool) == Some(true) && !explicit;
        let mine = params.get("mine").and_then(Value::as_bool) == Some(true);
        let key = (ws.to_string(), me.to_string());
        let used = after.unwrap_or_else(|| if advance { self.cursors.get(&key).copied().unwrap_or(0) } else { 0 });
        let changes: Vec<&Value> = self
            .ring
            .iter()
            .filter(|c| c["workspaceId"] == ws)
            .filter(|c| c["seq"].as_u64() > Some(used))
            .filter(|c| since_ts.is_none_or(|ts| c["ts"].as_i64() >= Some(ts)))
            .filter(|c| mine || c["by"] != me)
            .collect();
        let oldest = self.ring.front().and_then(|c| c["seq"].as_u64()).unwrap_or(self.next + 1);
        // How many changes the ring dropped that this ask would have shown.
        let dropped = if since_ts.is_some() { 0 } else { (oldest - 1).saturating_sub(used) };
        let out = json!({ "changes": changes, "cursor": self.next, "oldest": oldest, "dropped": dropped });
        if advance {
            self.cursors.insert(key, self.next);
        }
        out
    }
}

/// The top-level fields that differ between two versions of an entity (bookkeeping stamps left out).
pub fn changed_fields(old: &Value, new: &Value) -> Vec<String> {
    let (Some(old), Some(new)) = (old.as_object(), new.as_object()) else { return Vec::new() };
    let mut keys: Vec<&String> = old
        .keys()
        .chain(new.keys())
        .filter(|k| !matches!(k.as_str(), "updatedAt" | "createdAt" | "engineVersion" | "enabled"))
        .collect();
    keys.sort();
    keys.dedup();
    // Missing, null and empty are the same: a client may spell "no folder" any of the three.
    let value =
        |map: &serde_json::Map<String, Value>, k: &String| map.get(k).filter(|v| !v.is_null() && *v != "").cloned();
    keys.into_iter().filter(|k| value(old, k) != value(new, k)).cloned().collect()
}

/// The singular name of an entity kind, as the feed shows it.
fn label(kind: &str) -> &str {
    match kind {
        "proxyRules" | "rules" => "rule",
        "wsConnections" | "sockets" => "socket",
        other => other.strip_suffix('s').unwrap_or(other),
    }
}

/// Note a change made by the running command. `action` is `created`, `updated`, `deleted`, `enabled`, ...
pub fn record(ws: &str, kind: &str, id: &str, name: &str, action: &str, fields: &[String]) {
    LOG.lock().push(json!({
        "ts": now_ms(), "workspaceId": ws, "kind": label(kind), "id": id, "name": name,
        "action": action, "fields": fields, "by": by(),
    }));
}

/// The named sessions that changed any of `ids` in the workspace, first change first: a commit made later (on
/// publish) says who made what it commits.
pub fn sessions_of(ws: &str, ids: &[String]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for change in LOG.lock().ring.iter().filter(|c| c["workspaceId"] == ws) {
        let name = change["by"].as_str().and_then(|by| by.strip_prefix("session:"));
        if let Some(name) = name
            && ids.iter().any(|id| change["id"] == id.as_str())
            && !names.iter().any(|n| n == name)
        {
            names.push(name.to_string());
        }
    }
    names
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("changes.list", |params, _| {
        let ws = session::workspace_of(&params);
        Ok(LOG.lock().list(&ws, &by(), &params))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(ws: &str, by: &str) -> Value {
        json!({ "ts": 1, "workspaceId": ws, "kind": "mock", "id": "m", "name": "n", "action": "updated", "fields": [], "by": by })
    }

    fn count(got: &Value) -> usize {
        got["changes"].as_array().unwrap().len()
    }

    #[test]
    fn the_ring_keeps_the_last_changes_and_says_what_it_dropped() {
        let mut log = Log::default();
        for _ in 0..CAP + 5 {
            log.push(entry("w", "app"));
        }
        let got = log.list("w", "cli", &json!({}));
        assert_eq!(count(&got), CAP);
        assert_eq!(
            (got["oldest"].clone(), got["cursor"].clone(), got["dropped"].clone()),
            (json!(6), json!(2005), json!(5))
        );
    }

    #[test]
    fn a_cursor_advances_per_asker_and_own_changes_are_left_out() {
        let mut log = Log::default();
        log.push(entry("w", "session:backend"));
        log.push(entry("w", "session:frontend"));
        log.push(entry("other", "app"));
        let advance = json!({ "advance": true });
        let first = log.list("w", "session:frontend", &advance);
        assert_eq!(first["changes"][0]["by"], "session:backend");
        assert_eq!(count(&first), 1);
        assert_eq!(count(&log.list("w", "session:frontend", &advance)), 0);
        // Another asker has its own cursor; `mine` brings back its own changes; `after` does not move a cursor.
        assert_eq!(log.list("w", "session:backend", &advance)["changes"][0]["by"], "session:frontend");
        assert_eq!(count(&log.list("w", "session:backend", &json!({ "mine": true, "after": 0 }))), 2);
        log.push(entry("w", "app"));
        assert_eq!(count(&log.list("w", "session:frontend", &advance)), 1);
    }

    #[tokio::test]
    async fn changes_say_who_made_them() {
        let ws = "changes-by-test";
        let fields = ["responseBody".to_string()];
        let named = as_client(Some("cli"), async { record(ws, "mocks", "m1", "Get orders", "updated", &fields) });
        session::enter(ws, "backend", named).await;
        as_client(Some("cli"), async { record(ws, "requests", "r1", "Ping", "created", &[]) }).await;
        as_client(Some("desktop-ui"), async { record(ws, "flows", "f1", "Checkout", "deleted", &[]) }).await;
        let got = LOG.lock().list(ws, "nobody", &json!({}));
        let by: Vec<&str> = got["changes"].as_array().unwrap().iter().map(|c| c["by"].as_str().unwrap()).collect();
        assert_eq!(by, ["session:backend", "cli", "app"]);
        assert_eq!(got["changes"][0]["kind"], "mock");
        assert_eq!(got["changes"][0]["fields"], json!(["responseBody"]));
        // A later commit of m1 (on publish, by anyone) names the session that changed it.
        assert_eq!(sessions_of(ws, &["m1".into(), "r1".into()]), ["backend"]);
        assert_eq!(crate::workspace::git_store::session_trailer(ws, &["m1".into()]), "\nsession: backend");
        assert!(sessions_of(ws, &["f1".into()]).is_empty());
    }

    #[test]
    fn changed_fields_ignore_stamps() {
        let old = json!({ "name": "a", "updatedAt": 1, "x": 1 });
        let new = json!({ "name": "a", "updatedAt": 2, "x": 2, "y": 3 });
        assert_eq!(changed_fields(&old, &new), ["x", "y"]);
        assert!(changed_fields(&json!({ "folderId": null, "enabled": true }), &json!({ "folderId": "" })).is_empty());
    }
}
