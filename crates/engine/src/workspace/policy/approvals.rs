//! The approval queue behind `require_human` policies. A guarded call that a policy holds is queued here and
//! its command waits (default 5 minutes, `approvalTimeoutSeconds`) for a person to run `bifurc approvals approve
//! <id>` or `deny <id>` in a terminal. Approved: the waiting call goes on. Denied: it fails as blocked (exit 7).
//! Timed out: it fails as needing approval (exit 6) with the id, and the item stays pending (30 minutes) so a
//! late approval still counts: it leaves a one-time pass (10 minutes) that the next identical call consumes.
//!
//! In memory only: a restart forgets the queue (the waiting calls die with it). Every block, request and
//! decision is written to the change feed and, as a `[host]` line, to the engine's output (`engine.log` when
//! the command line started the engine). URLs here are already without userinfo, query and fragment.

use std::{collections::VecDeque, sync::LazyLock, time::Duration};

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::{Notify, oneshot};

use crate::{
    kernel::{CommandRegistry, bus},
    protocol::{EngineError, ErrorCode, Result},
    utils::{
        id::{mk_id, now_ms},
        json::{opt_str, str_of},
    },
    workspace::changes,
};

const DECIDED_KEPT: usize = 100;
const PENDING_MS: i64 = 30 * 60_000;
const PASS_MS: i64 = 10 * 60_000;

#[cfg(test)]
pub(super) static TEST_TIMEOUT_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

enum Outcome {
    Approved,
    Denied { by: String, reason: Option<String> },
}

struct Entry {
    item: Value,
    /// Set while a command waits on this item.
    tx: Option<oneshot::Sender<Outcome>>,
}

struct Pass {
    key: (String, String, String, String),
    until: i64,
}

#[derive(Default)]
struct State {
    pending: Vec<Entry>,
    decided: VecDeque<Value>,
    passes: Vec<Pass>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);
/// Woken when the engine is stopping, so no call keeps waiting.
static STOP: Notify = Notify::const_new();

impl State {
    /// Drop what ran out; the items that expired, to be announced once the lock is released.
    fn sweep(&mut self, now: i64) -> Vec<Value> {
        self.passes.retain(|p| p.until > now);
        let mut expired = Vec::new();
        let mut at = 0;
        while at < self.pending.len() {
            if self.pending[at].item["expiresAt"].as_i64().is_some_and(|t| t <= now) {
                let entry = self.pending.remove(at);
                expired.push(self.record(entry.item, "expired", "engine", None, now));
            } else {
                at += 1;
            }
        }
        expired
    }

    fn record(&mut self, mut item: Value, decision: &str, by: &str, reason: Option<&str>, now: i64) -> Value {
        item["decision"] = json!(decision);
        item["decidedAt"] = json!(now);
        item["decidedBy"] = json!(by);
        if let Some(reason) = reason.filter(|r| !r.is_empty()) {
            item["reason"] = json!(reason);
        }
        if self.decided.len() == DECIDED_KEPT {
            self.decided.pop_back();
        }
        self.decided.push_front(item.clone());
        item
    }
}

fn pass_key(ws: &str, by: &str, method: &str, url: &str) -> (String, String, String, String) {
    (ws.into(), by.into(), method.to_uppercase(), url.into())
}

/// How long a held call waits, in the setting `approvalTimeoutSeconds`.
fn timeout() -> Duration {
    #[cfg(test)]
    if let ms @ 1.. = TEST_TIMEOUT_MS.load(std::sync::atomic::Ordering::SeqCst) {
        return Duration::from_millis(ms);
    }
    Duration::from_secs(crate::workspace::settings::load_settings().approval_timeout_seconds())
}

// ---- the audit trail ----

fn describe(item: &Value) -> String {
    format!("policy \"{}\" {} {}", str_of(item, "policy"), str_of(item, "method"), str_of(item, "url"))
}

/// A line for the engine log and a row in the change feed.
/// One JSON line per block, request and decision in `<data dir>/audit/policy.jsonl`, so a person can review
/// what agents were stopped from doing after a restart, whoever launched the engine. URLs are already redacted.
fn persist(action: &str, item: &Value) {
    use std::io::Write as _;
    const ROTATE_AT: u64 = 5 * 1024 * 1024;
    let dir = crate::workspace::paths::data_dir().join("audit");
    let path = dir.join("policy.jsonl");
    let mut record = json!({ "ts": crate::utils::id::now_ms(), "action": action, "by": changes::by() });
    for key in ["id", "workspaceId", "policy", "method", "url", "decidedBy", "reason"] {
        if let Some(value) = item.get(key).filter(|v| !v.is_null() && v.as_str() != Some("")) {
            record[key] = value.clone();
        }
    }
    let _ = std::fs::create_dir_all(&dir);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > ROTATE_AT) {
        let _ = std::fs::rename(&path, dir.join("policy.jsonl.1"));
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{record}");
    }
}

fn audit(action: &str, item: &Value, line: String) {
    crate::runtime::notice(&line);
    persist(action, item);
    let name = format!("{} {}", str_of(item, "method"), str_of(item, "url"));
    changes::record(str_of(item, "workspaceId"), "approval", str_of(item, "id"), &name, action, &[]);
}

fn announce(event: &str, item: &Value) {
    let id = str_of(item, "id");
    match event {
        "approval.requested" => {
            audit("requested", item, format!("approval requested #{id} by {}: {}", str_of(item, "by"), describe(item)))
        }
        _ => audit(
            str_of(item, "decision"),
            item,
            format!("approval {} #{id} by {}: {}", str_of(item, "decision"), str_of(item, "decidedBy"), describe(item)),
        ),
    }
    bus().emit(event, item.clone());
}

/// A policy stopped a call outright (deny, allow_hosts, an invalid file).
pub(super) fn log_block(ws: &str, policy: &str, method: &str, url: &str) {
    let by = changes::by();
    let item = json!({ "id": "", "workspaceId": ws, "policy": policy, "method": method.to_uppercase(), "url": url });
    let name = format!("{} {url}", str_of(&item, "method"));
    crate::runtime::notice(&format!("blocked for {by}: {}", describe(&item)));
    persist("blocked", &item);
    changes::record(ws, "approval", "", &name, "blocked", &[]);
}

// ---- holding a call ----

fn needs(policy: &str, method: &str, url: &str, id: &str) -> EngineError {
    EngineError::new(
        ErrorCode::PolicyApproval,
        format!(
            "needs approval: policy \"{policy}\" ({method} {url}) — still pending as #{id}; approve with: bifurc approvals approve {id}"
        ),
    )
    .keyed("policy.approval", json!({ "policy": policy, "method": method, "url": url, "id": id }))
}

fn denied(policy: &str, method: &str, url: &str, by: &str, reason: Option<String>) -> EngineError {
    let why = reason.filter(|r| !r.is_empty()).map(|r| format!(": {r}")).unwrap_or_default();
    EngineError::new(ErrorCode::PolicyBlocked, format!("denied by {by}: policy \"{policy}\" ({method} {url}){why}"))
        .keyed("policy.blocked", json!({ "policy": policy, "method": method, "url": url }))
}

enum Step {
    Pass,
    Reused(String),
    Queued(Value, Option<oneshot::Receiver<Outcome>>),
}

/// Queue (or pass, or reuse) a held call. Takes the lock, never awaits.
fn enqueue(ws: &str, policy: &str, method: &str, url: &str, wait: bool) -> Step {
    let (by, now) = (changes::by(), now_ms());
    let ttl = PENDING_MS.max(timeout().as_millis() as i64);
    let (step, expired) = {
        let mut state = STATE.lock();
        let expired = state.sweep(now);
        let key = pass_key(ws, &by, method, url);
        let step = if let Some(at) = state.passes.iter().position(|p| p.key == key) {
            state.passes.remove(at);
            Step::Pass
        } else if let Some(same) = state.pending.iter().find(|e| {
            let i = &e.item;
            !wait
                && e.tx.as_ref().is_none_or(|tx| tx.is_closed())
                && (str_of(i, "workspaceId"), str_of(i, "by"), str_of(i, "method"), str_of(i, "url"))
                    == (ws, by.as_str(), method, url)
        }) {
            Step::Reused(str_of(&same.item, "id").to_string())
        } else {
            let item = json!({
                "id": mk_id(), "policy": policy, "method": method, "url": url, "workspaceId": ws, "by": by,
                "createdAt": now, "expiresAt": now + ttl,
            });
            let (tx, rx) = oneshot::channel();
            state.pending.push(Entry { item: item.clone(), tx: wait.then_some(tx) });
            Step::Queued(item, wait.then_some(rx))
        };
        (step, expired)
    };
    for item in &expired {
        announce("approval.decided", item);
    }
    if let Step::Queued(item, _) = &step {
        announce("approval.requested", item);
    }
    step
}

/// [`hold`] for a sync handler: it cannot wait, so a held call fails at once with its id.
pub(super) fn hold_now(ws: &str, policy: &str, method: &str, url: &str) -> Result<()> {
    let method = method.to_uppercase();
    match enqueue(ws, policy, &method, url, false) {
        Step::Pass => Ok(()),
        Step::Reused(id) => Err(needs(policy, &method, url, &id)),
        Step::Queued(item, _) => Err(needs(policy, &method, url, str_of(&item, "id"))),
    }
}

/// Let a call a `require_human` policy holds go on: with a pass, at once; else queue it for a person and wait
/// for the decision, timing out as `approvalTimeoutSeconds` says. No lock is held while waiting.
pub(super) async fn hold(ws: &str, policy: &str, method: &str, url: &str) -> Result<()> {
    let method = method.to_uppercase();
    let (item, rx) = match enqueue(ws, policy, &method, url, true) {
        Step::Pass => return Ok(()),
        Step::Reused(_) => unreachable!("a waiting call never reuses an item"),
        Step::Queued(item, rx) => (item, rx),
    };
    let wait_for = timeout();
    let id = str_of(&item, "id").to_string();
    let Some(mut rx) = rx else { unreachable!("a waiting call has a receiver") };

    let outcome = tokio::select! {
        outcome = &mut rx => outcome.ok(),
        () = tokio::time::sleep(wait_for) => None,
        () = STOP.notified() => None,
    };
    // A decision may have landed just as the wait ended.
    let outcome = outcome.or_else(|| {
        rx.close();
        rx.try_recv().ok()
    });
    match outcome {
        Some(Outcome::Approved) => Ok(()),
        Some(Outcome::Denied { by, reason }) => Err(denied(policy, &method, url, &by, reason)),
        None => {
            // Still pending for a person, but nobody waits on it now: an approval will leave a pass.
            if let Some(entry) = STATE.lock().pending.iter_mut().find(|e| str_of(&e.item, "id") == id) {
                entry.tx = None;
            }
            Err(needs(policy, &method, url, &id))
        }
    }
}

// ---- commands ----

fn decide(params: &Value) -> Result<Value> {
    // shortcut: this only keeps an agent's non-interactive shell from deciding (the CLI sets `interactive` after
    // checking for a terminal). It is no defence against a process that speaks the engine protocol directly;
    // upgrade to a confirmation only the desktop app or a TTY-bound channel can give if that threat matters.
    if params.get("interactive").and_then(Value::as_bool) != Some(true) {
        return Err(EngineError::bad_request("approvals are decided by a person in a terminal"));
    }
    let (id, approve) = (
        str_of(params, "id"),
        match str_of(params, "decision") {
            "approve" => true,
            "deny" => false,
            other => return Err(EngineError::bad_request(format!("decision is approve or deny, not \"{other}\""))),
        },
    );
    let (by, now) = (changes::by(), now_ms());
    let reason = opt_str(params, "reason");
    let (decided, expired) = {
        let mut state = STATE.lock();
        let expired = state.sweep(now);
        let Some(at) = state.pending.iter().position(|e| str_of(&e.item, "id") == id) else {
            let done = state.decided.iter().find(|d| str_of(d, "id") == id).map(|d| str_of(d, "decision").to_string());
            return Err(match done {
                Some(done) => EngineError::conflict(format!("#{id} was already {done}")),
                None => EngineError::not_found(format!("no pending approval #{id} (see: bifurc approvals list)")),
            });
        };
        let entry = state.pending.remove(at);
        let mut decided = state.record(entry.item, if approve { "approved" } else { "denied" }, &by, reason, now);
        let waiter = entry.tx;
        if approve {
            // A waiting call takes the approval itself; otherwise it becomes a pass for the next identical call.
            if waiter.is_none_or(|tx| tx.send(Outcome::Approved).is_err()) {
                let key = pass_key(
                    str_of(&decided, "workspaceId"),
                    str_of(&decided, "by"),
                    str_of(&decided, "method"),
                    str_of(&decided, "url"),
                );
                state.passes.push(Pass { key, until: now + PASS_MS });
                decided["passUntil"] = json!(now + PASS_MS);
            }
        } else if let Some(tx) = waiter {
            let _ = tx.send(Outcome::Denied { by: by.clone(), reason: reason.map(str::to_string) });
        }
        (decided, expired)
    };
    for item in expired.iter().chain([&decided]) {
        announce("approval.decided", item);
    }
    Ok(decided)
}

pub fn register(registry: &mut CommandRegistry) {
    static STOP_HOOK: std::sync::Once = std::sync::Once::new();
    STOP_HOOK.call_once(|| {
        bus().on("engine.stopping", |_| STOP.notify_waiters());
    });

    registry.register_sync("approvals.list", |params, _| {
        let ws = opt_str(&params, "workspaceId");
        let all = params.get("all").and_then(Value::as_bool) == Some(true);
        let (expired, pending, decided) = {
            let mut state = STATE.lock();
            let expired = state.sweep(now_ms());
            let mine = |item: &Value| ws.is_none_or(|ws| str_of(item, "workspaceId") == ws);
            let pending: Vec<Value> = state.pending.iter().map(|e| e.item.clone()).filter(|i| mine(i)).collect();
            let decided: Vec<Value> = state.decided.iter().filter(|d| mine(d)).cloned().collect();
            (expired, pending, decided)
        };
        for item in &expired {
            announce("approval.decided", item);
        }
        Ok(if all { json!({ "pending": pending, "decided": decided }) } else { json!({ "pending": pending }) })
    });

    registry.register_sync("approvals.decide", |params, _| decide(&params));
}

/// Forget everything (tests share the process).
#[cfg(test)]
pub(super) fn reset() {
    *STATE.lock() = State::default();
}

/// Make the pending item `id` expire now (tests).
#[cfg(test)]
pub(super) fn expire_now(id: &str) {
    for entry in STATE.lock().pending.iter_mut().filter(|e| str_of(&e.item, "id") == id) {
        entry.item["expiresAt"] = json!(1);
    }
}
