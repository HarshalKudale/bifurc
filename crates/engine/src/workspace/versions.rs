//! What each agent last read of an entity, so an update that would overwrite another agent's edit is refused.
//! A CLI process ends between `show` and `update`, so the engine remembers (in memory only: after a restart
//! nothing is stale). The version of an entity is a fingerprint of its stored file. The desktop app is never
//! checked and its reads are never remembered.

use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::LazyLock,
};

use crate::{
    protocol::{EngineError, Result},
    utils::{id::now_ms, json::str_of},
    workspace::{changes, config::load_entity},
};
use parking_lot::Mutex;

const CAP: usize = 10_000;

/// Fingerprint of the file as read, the change-ring position then, and when it was remembered (for trimming).
type Seen = (u64, u64, u64);

#[derive(Default)]
struct Book {
    clock: u64,
    seen: HashMap<(String, String, String), Seen>,
}

static BOOK: LazyLock<Mutex<Book>> = LazyLock::new(Default::default);

/// Agents are the named sessions and the `cli` client (MCP connects as it).
fn asker() -> Option<String> {
    let by = changes::by();
    (by.starts_with("session:") || by == "cli").then_some(by)
}

fn fingerprint(ws: &str, kind: &str, id: &str) -> Option<u64> {
    let mut hasher = DefaultHasher::new();
    load_entity(ws, kind, id)?.to_string().hash(&mut hasher);
    Some(hasher.finish())
}

fn put(ws: &str, id: &str, who: String, fingerprint: u64) {
    let mut book = BOOK.lock();
    book.clock += 1;
    let seen = (fingerprint, changes::cursor(), book.clock);
    book.seen.insert((ws.into(), who, id.into()), seen);
    if book.seen.len() > CAP {
        let keep_after = book.clock - CAP as u64 / 2;
        book.seen.retain(|_, s| s.2 > keep_after);
    }
}

/// An agent was shown the entity as it is now.
pub fn remember(ws: &str, kind: &str, id: &str) {
    if let (Some(who), Some(fp)) = (asker(), fingerprint(ws, kind, id)) {
        put(ws, id, who, fp);
    }
}

/// Refuse an update by an agent that was shown an older version, unless `force`.
pub fn check(ws: &str, kind: &str, id: &str, force: bool) -> Result<()> {
    let Some(who) = asker().filter(|_| !force) else { return Ok(()) };
    let Some(&(fp, seq, _)) = BOOK.lock().seen.get(&(ws.to_string(), who.clone(), id.to_string())) else {
        return Ok(());
    };
    if fingerprint(ws, kind, id).is_none_or(|now| now == fp) {
        return Ok(());
    }
    let name = load_entity(ws, kind, id).map(|e| str_of(&e, "name").to_string()).unwrap_or_default();
    let later: Vec<_> = changes::since(ws, id, seq).into_iter().filter(|c| c["by"] != who.as_str()).collect();
    let what = match later.last() {
        Some(last) => {
            let mut fields: Vec<&str> = Vec::new();
            for field in
                later.iter().flat_map(|c| c["fields"].as_array().into_iter().flatten()).filter_map(|f| f.as_str())
            {
                if !fields.contains(&field) {
                    fields.push(field);
                }
            }
            let ago = (now_ms() - last["ts"].as_i64().unwrap_or_default()).max(0) / 1000;
            let ago = if ago < 60 {
                format!("{ago}s")
            } else if ago < 3600 {
                format!("{}m", ago / 60)
            } else {
                format!("{}h", ago / 3600)
            };
            let by = last["by"].as_str().unwrap_or("someone");
            format!("{} by {by} {ago} ago", if fields.is_empty() { "updated".into() } else { fields.join(", ") })
        }
        None => "updated".into(),
    };
    Err(EngineError::conflict(format!("\"{name}\" changed since you read it: {what} — show it again, or --force")))
}

/// After the agent's own successful update: what it holds is now current (only when it held something).
pub fn wrote(ws: &str, kind: &str, id: &str) {
    let Some(who) = asker() else { return };
    let held = BOOK.lock().seen.contains_key(&(ws.to_string(), who.clone(), id.to_string()));
    if let (true, Some(fp)) = (held, fingerprint(ws, kind, id)) {
        put(ws, id, who, fp);
    }
}
