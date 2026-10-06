//! The browser extensions connected right now. A companion is a session whose client is `bifurc-extension`;
//! what it said about itself in its handshake (`clientInfo`) is kept beside the session's identity so Settings
//! can show which browser, on which computer, is on the other end.

use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{Map, Value, json};

use crate::{kernel::SessionIdentity, utils::id::now_ms};

/// The client name the browser extension gives in its handshake.
pub const COMPANION_CLIENT: &str = "bifurc-extension";

/// The `clientInfo` members kept, all strings. Anything else a client sends is dropped.
const INFO_KEYS: [&str; 7] = ["browser", "browserVersion", "os", "arch", "extensionId", "userAgent", "language"];

const MAX_INFO_CHARS: usize = 300;

/// The part of a handshake's `clientInfo` worth keeping: known string members, bounded in length.
/// `None` when the client sent nothing usable.
pub fn sanitize_client_info(info: &Value) -> Option<Value> {
    let kept: Map<String, Value> = INFO_KEYS
        .iter()
        .filter_map(|key| {
            let text = info.get(*key)?.as_str()?.trim();
            (!text.is_empty()).then(|| ((*key).to_string(), json!(text.chars().take(MAX_INFO_CHARS).collect::<String>())))
        })
        .collect();
    (!kept.is_empty()).then_some(Value::Object(kept))
}

#[derive(Debug, Clone)]
struct Companion {
    session_id: String,
    client_version: Option<String>,
    peer: Option<String>,
    info: Option<Value>,
    connected_at: i64,
}

impl Companion {
    fn to_value(&self) -> Value {
        let mut value = json!({
            "sessionId": self.session_id,
            "connectedAt": self.connected_at,
        });
        if let Some(version) = &self.client_version {
            value["extensionVersion"] = json!(version);
        }
        if let Some(peer) = &self.peer {
            value["peer"] = json!(peer);
        }
        if let Some(Value::Object(info)) = &self.info {
            for (key, member) in info {
                value[key] = member.clone();
            }
        }
        value
    }
}

#[derive(Default)]
pub struct CompanionRegistry {
    companions: Mutex<Vec<Companion>>,
}

impl CompanionRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Record a session that just finished its handshake. Returns whether it is a companion.
    pub fn connected(&self, identity: &SessionIdentity) -> bool {
        if identity.client_name.as_deref() != Some(COMPANION_CLIENT) {
            return false;
        }
        let mut companions = self.companions.lock();
        companions.retain(|c| c.session_id != identity.session_id);
        companions.push(Companion {
            session_id: identity.session_id.clone(),
            client_version: identity.client_version.clone(),
            peer: identity.peer.clone(),
            info: identity.client_info.clone(),
            connected_at: now_ms(),
        });
        true
    }

    /// Forget a session that ended. Returns whether it was a companion.
    pub fn disconnected(&self, identity: &SessionIdentity) -> bool {
        let mut companions = self.companions.lock();
        let before = companions.len();
        companions.retain(|c| c.session_id != identity.session_id);
        companions.len() != before
    }

    /// The `engine.companions` reply, and the payload of `event.companions.changed`.
    pub fn snapshot(&self) -> Value {
        let companions: Vec<Value> = self.companions.lock().iter().map(Companion::to_value).collect();
        json!({ "companions": companions })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(id: &str, client: &str, info: Option<Value>) -> SessionIdentity {
        SessionIdentity {
            session_id: id.into(),
            client_name: Some(client.into()),
            client_version: Some("0.5.0".into()),
            peer: Some("127.0.0.1".into()),
            client_info: info,
            ..Default::default()
        }
    }

    #[test]
    fn only_extensions_are_listed_and_leave_when_they_disconnect() {
        let registry = CompanionRegistry::new();
        let info = sanitize_client_info(&json!({ "browser": "Chrome", "browserVersion": "126", "ignored": "x" }));
        let extension = identity("a", COMPANION_CLIENT, info);
        assert!(registry.connected(&extension));
        assert!(!registry.connected(&identity("b", "cli", None)));

        let listed = registry.snapshot();
        let companions = listed["companions"].as_array().unwrap();
        assert_eq!(companions.len(), 1);
        assert_eq!(companions[0]["browser"], "Chrome");
        assert_eq!(companions[0]["extensionVersion"], "0.5.0");
        assert!(companions[0].get("ignored").is_none());

        assert!(!registry.disconnected(&identity("b", "cli", None)));
        assert!(registry.disconnected(&extension));
        assert_eq!(registry.snapshot()["companions"], json!([]));
    }

    #[test]
    fn client_info_keeps_known_strings_only() {
        assert_eq!(sanitize_client_info(&json!({ "browser": 3, "os": "  " })), None);
        assert_eq!(sanitize_client_info(&json!("Chrome")), None);
        let long = "x".repeat(1000);
        let kept = sanitize_client_info(&json!({ "userAgent": long })).unwrap();
        assert_eq!(kept["userAgent"].as_str().unwrap().len(), MAX_INFO_CHARS);
    }
}
