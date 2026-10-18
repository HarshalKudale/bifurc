//! Claims are locks: the workspace's entities a session is editing, as `claims.list` and `claims.changed` give them.
//!
//! The holder of a claim may write the entity; everyone else may read and use it (send, run) but not edit or
//! delete it. The app's own claims are held as [`ME`].

use serde_json::Value;

use super::traffic::clock_utc;

/// The holder name the engine gives this app (`changes::by()` for the `desktop-ui` client).
pub const ME: &str = "app";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub id: String,
    /// As `entity.update` takes it: `mocks`, `requests`, ...
    pub kind: String,
    pub by: String,
    pub name: String,
    /// When it lapses, in epoch milliseconds.
    pub expires_at: u64,
}

impl Claim {
    pub fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            id: value.get("id")?.as_str()?.to_owned(),
            kind: value.get("kind").and_then(Value::as_str).unwrap_or_default().to_owned(),
            by: value.get("by")?.as_str()?.to_owned(),
            name: value.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
            expires_at: value.get("expiresAt").and_then(Value::as_u64).unwrap_or(0),
        })
    }

    /// The claim a `claims.locked` refusal describes (`{id, name, claimedBy, expiresAt}`).
    pub fn from_refusal(params: &Value) -> Option<Self> {
        let mut shown = params.clone();
        shown["by"] = params.get("claimedBy")?.clone();
        Self::from_value(&shown)
    }

    pub fn is_mine(&self) -> bool {
        self.by == ME
    }

    /// When it lapses, as a clock time in UTC.
    pub fn until(&self) -> String {
        format!("{} UTC", &clock_utc(self.expires_at)[..5])
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Claims(Vec<Claim>);

impl Claims {
    /// From `claims.list` / `claims.changed`: `{ claims: [...] }`.
    pub fn from_value(value: &Value) -> Self {
        let rows = value.get("claims").and_then(Value::as_array);
        Self(rows.into_iter().flatten().filter_map(Claim::from_value).collect())
    }

    pub fn of(&self, id: &str) -> Option<&Claim> {
        self.0.iter().find(|claim| claim.id == id)
    }

    /// The claim that keeps this app from editing `id`.
    pub fn held_by_other(&self, id: &str) -> Option<&Claim> {
        self.of(id).filter(|claim| !claim.is_mine())
    }

    pub fn held_by_me(&self, id: &str) -> bool {
        self.of(id).is_some_and(Claim::is_mine)
    }

    /// Notes a claim a refusal told about, which this list had not heard of yet.
    pub fn insert(&mut self, claim: Claim) {
        self.0.retain(|old| old.id != claim.id);
        self.0.push(claim);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_list_tells_mine_from_others() {
        let claims = Claims::from_value(&json!({"claims": [
            {"id": "a", "by": "app", "name": "A", "expiresAt": 1},
            {"id": "b", "by": "session:bot", "name": "B", "expiresAt": 3_600_000},
        ]}));
        assert!(claims.held_by_me("a") && claims.held_by_other("a").is_none());
        assert_eq!(claims.held_by_other("b").unwrap().until(), "01:00 UTC");
        assert!(claims.of("c").is_none());
    }
}
