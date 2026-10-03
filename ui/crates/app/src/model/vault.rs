//! A vault as the engine reports it (`vault.list`): its name, whether it can be used, and the *names* of
//! its secrets. The engine never sends a value to the UI, so there is none here to hold.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultState {
    /// The engine has no key for it: its secrets cannot be used until the passphrase is given.
    Sealed,
    /// The engine holds the key: `{{vault.secret}}` resolves in requests.
    Ready,
    /// Ready, and a program launched by `bifurc unlock` is running with its secrets.
    Open,
}

impl VaultState {
    pub fn label(self) -> &'static str {
        match self {
            VaultState::Sealed => "Sealed",
            VaultState::Ready => "Ready",
            VaultState::Open => "Open",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VaultInfo {
    pub id: String,
    pub name: String,
    pub state: VaultState,
    pub secrets: Vec<String>,
    /// How many programs launched by `bifurc unlock` hold it open.
    pub opened_by: usize,
    /// The vault's file in the workspace repository.
    pub path: String,
}

impl VaultInfo {
    fn from_value(value: &Value) -> Option<Self> {
        let state = match value.get("state")?.as_str()? {
            "sealed" => VaultState::Sealed,
            "ready" => VaultState::Ready,
            "open" => VaultState::Open,
            _ => return None,
        };
        Some(Self {
            id: value.get("id")?.as_str()?.to_owned(),
            name: value.get("name")?.as_str()?.to_owned(),
            state,
            secrets: value
                .get("secrets")
                .and_then(Value::as_array)
                .map(|names| names.iter().filter_map(|n| n.as_str().map(str::to_owned)).collect())
                .unwrap_or_default(),
            opened_by: value.get("openedBy").and_then(Value::as_u64).unwrap_or(0) as usize,
            path: value.get("path").and_then(Value::as_str).unwrap_or_default().to_owned(),
        })
    }

    pub fn list_from_value(value: &Value) -> Vec<Self> {
        value.as_array().map(|items| items.iter().filter_map(Self::from_value).collect()).unwrap_or_default()
    }

    /// How the secret is written in a request.
    pub fn reference(&self, secret: &str) -> String {
        format!("{{{{{}.{}}}}}", self.name, secret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_engines_list_is_read_and_unknown_states_are_skipped() {
        let list = json!([
            { "id": "a", "name": "api_keys", "state": "open", "secrets": ["K1", "K2"], "openedBy": 2, "path": "vaults/a.json" },
            { "id": "b", "name": "db", "state": "sealed", "secrets": [] },
            { "id": "c", "name": "x", "state": "weird" },
        ]);
        let vaults = VaultInfo::list_from_value(&list);
        assert_eq!(vaults.len(), 2);
        assert_eq!(vaults[0].state, VaultState::Open);
        assert_eq!(vaults[0].opened_by, 2);
        assert_eq!(vaults[0].reference("K1"), "{{api_keys.K1}}");
        assert_eq!(vaults[1].state, VaultState::Sealed);
    }
}
