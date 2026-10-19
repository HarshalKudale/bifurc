//! Claims are locks: a session that claims an entity (`bifurc claim mock "Get orders"`, or the app while a person
//! edits) is the only one who may update, delete or enable/disable it until it releases, its session ends or
//! [`TTL_MS`] passes without a write or renewal. Reads and uses (load, send, run, export) stay open. In memory,
//! never stored; a lapsed claim is dropped when next looked at.
//!
//! The holder is [`changes::by`]: `session:<name>` for a named session, else the client kind (`app`, `cli`,
//! `extension`). Two unnamed CLI runs are both `cli`, so they share one claim and never lock each other out;
//! use `--session` to be told apart. Engine-internal work (no client, `engine`) is not locked.
//!
//! Refusal: `CONFLICT`, key `claims.locked`, params `{id, kind, name, claimedBy, expiresAt}`.
//! Event `claims.changed`: `{workspaceId, claims, change?}` on set, release, take-over, lapse and session end.

use std::{collections::HashMap, sync::LazyLock};

use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::{
    env::session,
    kernel::{CommandRegistry, bus, kinds::to_engine_kind},
    protocol::{EngineError, Result},
    utils::{
        id::now_ms,
        json::{str_of, truthy},
    },
    workspace::{changes, config::load_entity},
};

const TTL_MS: i64 = 30 * 60_000;

struct Claim {
    by: String,
    ts: i64,
    kind: String,
    name: String,
    /// The connection that made it, when the claim should end with it (the app); `None` for a CLI or named
    /// session, whose connections last one command and whose claims outlive them until released or lapsed.
    owner: Option<String>,
}

type Key = (String, String);

static CLAIMS: LazyLock<Mutex<HashMap<Key, Claim>>> = LazyLock::new(Default::default);

fn view(id: &str, c: &Claim) -> Value {
    json!({ "id": id, "kind": c.kind, "name": c.name, "by": c.by, "ts": c.ts, "expiresAt": c.ts + TTL_MS })
}

fn list_of(ws: &str) -> Value {
    let mut claims: Vec<Value> =
        CLAIMS.lock().iter().filter(|((w, _), _)| w == ws).map(|((_, id), c)| view(id, c)).collect();
    claims.sort_by_key(|c| c["ts"].as_i64());
    json!({ "workspaceId": ws, "claims": claims })
}

fn announce(ws: &str, change: Option<Value>) {
    let mut payload = list_of(ws);
    if let Some(change) = change {
        payload["change"] = change;
    }
    bus().emit("claims.changed", payload);
}

/// Drop lapsed claims, then tell each workspace that lost one.
fn prune(now: i64) {
    let mut lost: Vec<String> = Vec::new();
    CLAIMS.lock().retain(|(ws, _), c| {
        let live = now - c.ts < TTL_MS;
        if !live && !lost.contains(ws) {
            lost.push(ws.clone());
        }
        live
    });
    for ws in lost {
        announce(&ws, Some(json!({ "action": "expired" })));
    }
}

fn locked(id: &str, c: &Claim) -> EngineError {
    let message = format!(
        "\"{name}\" is claimed by {by} and locked against edits. Take it over with `bifurc claim {kind} \"{name}\" --force`, or wait until it lapses.",
        name = c.name,
        by = c.by,
        kind = c.kind.strip_suffix('s').unwrap_or(&c.kind),
    );
    let params = json!({ "id": id, "kind": c.kind, "name": c.name, "claimedBy": c.by, "expiresAt": c.ts + TTL_MS });
    EngineError::conflict(message).keyed("claims.locked", params)
}

/// Refuse a write to the entity while another holder has a live claim; the holder's own write renews it.
pub fn guard(ws: &str, id: &str) -> Result<()> {
    guard_at(ws, id, &changes::by(), now_ms())
}

fn guard_at(ws: &str, id: &str, me: &str, now: i64) -> Result<()> {
    if me == "engine" {
        return Ok(());
    }
    prune(now);
    match CLAIMS.lock().get_mut(&(ws.to_string(), id.to_string())) {
        Some(c) if c.by != me => Err(locked(id, c)),
        Some(c) => {
            c.ts = now;
            Ok(())
        }
        None => Ok(()),
    }
}

/// Claim for `claim.by`. Another holder's live claim is refused unless `force`; the result is the new claim and
/// the claim of another holder it replaced.
fn take(ws: &str, id: &str, claim: Claim, force: bool, now: i64) -> Result<(Value, Option<Value>)> {
    prune(now);
    let key = (ws.to_string(), id.to_string());
    let (shown, previous) = {
        let mut all = CLAIMS.lock();
        let other = all.get(&key).filter(|old| old.by != claim.by);
        if let Some(old) = other.filter(|_| !force) {
            return Err(locked(id, old));
        }
        let previous = other.map(|old| json!({ "by": old.by, "ts": old.ts }));
        let shown = view(id, &claim);
        all.insert(key, claim);
        (shown, previous)
    };
    let action = if previous.is_some() { "taken-over" } else { "claimed" };
    announce(ws, Some(json!({ "action": action, "id": id, "by": shown["by"], "previous": previous })));
    Ok((shown, previous))
}

/// Release the claims on `ids` held by `me` (anyone's with `force`); the ids it released.
fn release(ws: &str, ids: &[String], me: &str, force: bool, now: i64) -> Result<Vec<String>> {
    prune(now);
    let released = {
        let mut all = CLAIMS.lock();
        for id in ids {
            if let Some(c) = all.get(&(ws.to_string(), id.clone())).filter(|c| c.by != me && !force) {
                return Err(locked(id, c));
            }
        }
        ids.iter().filter(|id| all.remove(&(ws.to_string(), (*id).clone())).is_some()).cloned().collect::<Vec<_>>()
    };
    if !released.is_empty() {
        announce(ws, Some(json!({ "action": "released", "ids": released, "by": me })));
    }
    Ok(released)
}

/// A deleted entity has no claim.
pub fn drop_claim(ws: &str, id: &str) {
    if CLAIMS.lock().remove(&(ws.to_string(), id.to_string())).is_some() {
        announce(ws, Some(json!({ "action": "released", "ids": [id] })));
    }
}

/// A connection ended: its claims end with it.
pub fn release_owned_by(session_id: &str) {
    let mut lost: Vec<String> = Vec::new();
    CLAIMS.lock().retain(|(ws, _), c| {
        let keep = c.owner.as_deref() != Some(session_id);
        if !keep && !lost.contains(ws) {
            lost.push(ws.clone());
        }
        keep
    });
    for ws in lost {
        announce(&ws, Some(json!({ "action": "session-ended" })));
    }
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("claims.set", |params, ctx| {
        let ws = session::workspace_of(&params);
        let (wire, id) = (str_of(&params, "kind"), str_of(&params, "id"));
        let me = changes::by();
        let force = truthy(&params, "force");
        if truthy(&params, "release") {
            let released = release(&ws, &[id.to_string()], &me, force, now_ms())?;
            return Ok(json!({ "claim": null, "previous": null, "released": released }));
        }
        let Some(entity) = load_entity(&ws, to_engine_kind(wire), id) else {
            return Err(EngineError::not_found(format!("No {wire} with id \"{id}\".")));
        };
        let name = str_of(&entity, "name").to_string();
        let persistent = session::named().is_some() || me == "cli";
        let owner = ctx.session.as_ref().map(|s| s.session_id.clone()).filter(|_| !persistent);
        let claim = Claim { by: me, ts: now_ms(), kind: wire.to_string(), name: name.clone(), owner };
        let (claim, previous) = take(&ws, id, claim, force, now_ms())?;
        if previous.is_some() {
            // The previous holder finds the take-over in `changes`.
            changes::record(&ws, to_engine_kind(wire), id, &name, "claim-taken-over", &[]);
        }
        Ok(json!({ "claim": claim, "previous": previous }))
    });
    registry.register_sync("claims.release", |params, _| {
        let ws = session::workspace_of(&params);
        let ids: Vec<String> =
            params["ids"].as_array().into_iter().flatten().filter_map(|id| id.as_str().map(str::to_string)).collect();
        let released = release(&ws, &ids, &changes::by(), truthy(&params, "force"), now_ms())?;
        Ok(json!({ "released": released }))
    });
    registry.register_sync("claims.list", |params, _| {
        let ws = session::workspace_of(&params);
        prune(now_ms());
        Ok(list_of(&ws))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(by: &str, ts: i64, owner: Option<&str>) -> Claim {
        Claim { by: by.into(), ts, kind: "mocks".into(), name: "Get orders".into(), owner: owner.map(Into::into) }
    }

    #[test]
    fn lock_take_over_release_and_expiry() {
        // Real time only: the claims map is shared with the other test.
        let (ws, now) = ("claims-test", now_ms());
        take(ws, "m1", claim("session:a", now - TTL_MS + 60_000, None), false, now).unwrap();
        // Others are refused with the holder named; the holder passes and renews.
        let err = guard_at(ws, "m1", "session:b", now).unwrap_err();
        assert_eq!(err.key(), Some("claims.locked"));
        assert_eq!(err.params().unwrap()["claimedBy"], "session:a");
        assert!(take(ws, "m1", claim("session:b", now, None), false, now).is_err());
        guard_at(ws, "m1", "session:a", now).unwrap();
        assert_eq!(CLAIMS.lock()[&(ws.to_string(), "m1".to_string())].ts, now);
        // Force takes over and names the previous holder.
        let (_, prev) = take(ws, "m1", claim("session:b", now, None), true, now).unwrap();
        assert_eq!(prev.unwrap()["by"], "session:a");
        // Only the holder releases, unless forced.
        let ids = ["m1".to_string()];
        assert!(release(ws, &ids, "session:a", false, now).is_err());
        assert_eq!(release(ws, &ids, "session:b", false, now).unwrap(), ids);
        assert!(guard_at(ws, "m1", "session:a", now).is_ok());
        // A claim lapses 30 minutes after its last renewal.
        CLAIMS.lock().insert((ws.into(), "m2".into()), claim("session:a", now - TTL_MS - 1, None));
        assert!(guard_at(ws, "m2", "session:b", now).is_ok());
        assert!(list_of(ws)["claims"].as_array().unwrap().is_empty());
    }

    #[test]
    fn a_connection_ending_releases_its_claims_and_announces() {
        let ws = "claims-session";
        let seen = std::sync::Arc::new(Mutex::new(0));
        let sink = seen.clone();
        let listener = bus().on("claims.changed", move |p| {
            if p["workspaceId"] == "claims-session" {
                *sink.lock() += 1;
            }
        });
        take(ws, "m1", claim("app", now_ms(), Some("conn-1")), false, now_ms()).unwrap();
        take(ws, "m2", claim("cli", now_ms(), None), false, now_ms()).unwrap();
        release_owned_by("conn-1");
        assert!(guard_at(ws, "m1", "cli", now_ms()).is_ok());
        assert!(guard_at(ws, "m2", "app", now_ms()).is_err());
        assert_eq!(*seen.lock(), 3);
        bus().off(listener);
        drop_claim(ws, "m2");
        // Engine-internal writes are never locked.
        take(ws, "m3", claim("app", now_ms(), None), false, now_ms()).unwrap();
        assert!(guard_at(ws, "m3", "engine", now_ms()).is_ok());
        drop_claim(ws, "m3");
    }
}
