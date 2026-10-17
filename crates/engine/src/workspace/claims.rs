//! Claims: an agent marks an entity as the one it is editing (`bifurc claim mock "Get orders"`). Advisory only:
//! nothing is blocked, an update by someone else just carries `claimedBy` so the CLI can warn. In memory,
//! never stored; a claim lapses [`TTL_MS`] after it was made or renewed (checked when read).

use std::{collections::HashMap, sync::LazyLock};

use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::{
    env::session,
    kernel::{CommandRegistry, kinds::to_engine_kind},
    protocol::EngineError,
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
}

static CLAIMS: LazyLock<Mutex<HashMap<(String, String), Claim>>> = LazyLock::new(Default::default);

fn view(id: &str, c: &Claim) -> Value {
    json!({ "id": id, "kind": c.kind, "name": c.name, "by": c.by, "ts": c.ts })
}

/// Claim (or, with `None`, release) at time `now`: the claim, and the live claim of another asker it replaced.
fn set(ws: &str, id: &str, claim: Option<Claim>, now: i64) -> (Option<Value>, Option<Value>) {
    let mut all = CLAIMS.lock();
    all.retain(|_, c| now - c.ts < TTL_MS);
    let key = (ws.to_string(), id.to_string());
    let previous = all
        .get(&key)
        .filter(|old| claim.as_ref().is_none_or(|new| old.by != new.by))
        .map(|old| json!({ "by": old.by, "ts": old.ts }));
    match claim {
        Some(claim) => {
            let shown = view(id, &claim);
            all.insert(key, claim);
            (Some(shown), previous)
        }
        None => {
            all.remove(&key);
            (None, previous)
        }
    }
}

/// `{by, ts, name}` when another asker holds a live claim on the entity.
fn other_holder(ws: &str, id: &str, me: &str, now: i64) -> Option<Value> {
    let all = CLAIMS.lock();
    let c = all.get(&(ws.to_string(), id.to_string()))?;
    (c.by != me && now - c.ts < TTL_MS).then(|| json!({ "by": c.by, "ts": c.ts, "name": c.name }))
}

/// For `entity.update`: add `claimedBy` to its result when someone else holds the entity.
pub fn annotate(ws: &str, id: &str, result: &mut Value) {
    if let (Some(held), Some(map)) = (other_holder(ws, id, &changes::by(), now_ms()), result.as_object_mut()) {
        map.insert("claimedBy".into(), held);
    }
}

/// A deleted entity has no claim.
pub fn drop_claim(ws: &str, id: &str) {
    CLAIMS.lock().remove(&(ws.to_string(), id.to_string()));
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("claims.set", |params, _| {
        let ws = session::workspace_of(&params);
        let (wire, id) = (str_of(&params, "kind"), str_of(&params, "id"));
        let Some(entity) = load_entity(&ws, to_engine_kind(wire), id) else {
            return Err(EngineError::not_found(format!("No {wire} with id \"{id}\".")));
        };
        let claim = (!truthy(&params, "release")).then(|| Claim {
            by: changes::by(),
            ts: now_ms(),
            kind: wire.to_string(),
            name: str_of(&entity, "name").to_string(),
        });
        let (claim, previous) = set(&ws, id, claim, now_ms());
        Ok(json!({ "claim": claim, "previous": previous }))
    });
    registry.register_sync("claims.list", |params, _| {
        let ws = session::workspace_of(&params);
        let now = now_ms();
        let mut claims: Vec<Value> = CLAIMS
            .lock()
            .iter()
            .filter(|((w, _), c)| *w == ws && now - c.ts < TTL_MS)
            .map(|((_, id), c)| view(id, c))
            .collect();
        claims.sort_by_key(|c| c["ts"].as_i64());
        Ok(json!({ "claims": claims }))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(by: &str, ts: i64) -> Option<Claim> {
        Some(Claim { by: by.into(), ts, kind: "mocks".into(), name: "Get orders".into() })
    }

    #[test]
    fn claim_list_release_and_expiry() {
        let (ws, t0) = ("claims-test", now_ms());
        let (got, prev) = set(ws, "m1", claim("session:a", t0 + 1000), t0 + 1000);
        assert_eq!((got.unwrap()["by"].clone(), prev), (json!("session:a"), None));
        // Another asker may take it; the result names the previous holder. The holder renewing has none.
        let (_, prev) = set(ws, "m1", claim("session:b", t0 + 2000), t0 + 2000);
        assert_eq!(prev.unwrap()["by"], "session:a");
        let (_, prev) = set(ws, "m1", claim("session:b", t0 + 3000), t0 + 3000);
        assert!(prev.is_none());
        assert_eq!(other_holder(ws, "m1", "session:a", t0 + 3000).unwrap()["by"], "session:b");
        assert!(other_holder(ws, "m1", "session:b", t0 + 3000).is_none());
        // It lapses 30 minutes after the last renewal.
        assert!(other_holder(ws, "m1", "session:a", t0 + 3000 + TTL_MS).is_none());
        let (none, _) = set(ws, "m1", None, t0 + 4000);
        assert!(none.is_none() && other_holder(ws, "m1", "x", t0 + 4000).is_none());
    }

    #[tokio::test]
    async fn an_update_by_another_asker_carries_claimed_by() {
        let ws = "claims-annotate";
        let mine = changes::as_client(Some("cli"), async { changes::by() }).await;
        set(ws, "m1", claim("session:other", now_ms()), now_ms());
        let run = |client: &'static str| async move {
            let mut result = json!({ "ok": true });
            changes::as_client(Some(client), async { annotate(ws, "m1", &mut result) }).await;
            result
        };
        assert_eq!(mine, "cli");
        assert_eq!(run("cli").await["claimedBy"]["by"], "session:other");
        assert_eq!(run("desktop-ui").await["claimedBy"]["name"], "Get orders");
        set(ws, "m1", claim("cli", now_ms()), now_ms());
        assert!(run("cli").await.get("claimedBy").is_none());
        drop_claim(ws, "m1");
        assert!(run("desktop-ui").await.get("claimedBy").is_none());
    }
}
