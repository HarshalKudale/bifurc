//! A vault as the engine reports it (`vault.list`): its name, where its values are kept, whether it can be
//! used, and the *names* of its secrets. The engine never sends a value to the UI, so there is none here to
//! hold. Also the providers a vault can be linked to (`vault.providers`).

use serde_json::Value;

use crate::i18n::t;

/// The provider id of a vault kept on this device.
pub const LOCAL: &str = "local";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultState {
    /// Nothing is in the engine's memory: its secrets cannot be used until it is unsealed.
    Sealed,
    /// The engine holds the values until the vault is sealed or the engine stops: `{{vault.secret}}` resolves
    /// in requests, and `bifurc vault run` starts programs with its secrets.
    Unsealed,
}

impl VaultState {
    pub fn label(self) -> &'static str {
        t(match self {
            VaultState::Sealed => "model.vault.sealed",
            VaultState::Unsealed => "model.vault.unsealed",
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VaultInfo {
    pub id: String,
    pub name: String,
    pub state: VaultState,
    /// The secrets' names: a linked vault's as this device last pulled them.
    pub secrets: Vec<String>,
    /// `local`, or the secret manager the values are pulled from.
    pub provider: String,
    pub provider_label: String,
    /// A linked vault's file in the workspace repository; empty for a local vault, which is not synced.
    pub path: String,
    /// This device accepted where a linked vault's values come from; always true for a local vault.
    pub approved: bool,
    /// A linked vault's source in its provider, as `(key, value)`.
    pub settings: Vec<(String, String)>,
    /// A token for the provider is kept on this device.
    pub has_credential: bool,
    /// A local vault an older version kept in the repository, moved to this device and not yet acknowledged.
    pub moved: bool,
}

impl VaultInfo {
    fn from_value(value: &Value) -> Option<Self> {
        let state = match value.get("state")?.as_str()? {
            "sealed" => VaultState::Sealed,
            "unsealed" => VaultState::Unsealed,
            _ => return None,
        };
        let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        let provider = value.get("provider").and_then(Value::as_str).unwrap_or(LOCAL).to_owned();
        Some(Self {
            id: value.get("id")?.as_str()?.to_owned(),
            name: value.get("name")?.as_str()?.to_owned(),
            state,
            secrets: value
                .get("secrets")
                .and_then(Value::as_array)
                .map(|names| names.iter().filter_map(|n| n.as_str().map(str::to_owned)).collect())
                .unwrap_or_default(),
            provider_label: value.get("providerLabel").and_then(Value::as_str).unwrap_or(&provider).to_owned(),
            provider,
            path: text("path"),
            approved: value.get("approved").and_then(Value::as_bool).unwrap_or(true),
            settings: value
                .get("settings")
                .and_then(Value::as_object)
                .map(|map| map.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned())).collect())
                .unwrap_or_default(),
            has_credential: value.get("hasCredential").and_then(Value::as_bool).unwrap_or(false),
            moved: value.get("movedAt").is_some_and(Value::is_i64),
        })
    }

    pub fn list_from_value(value: &Value) -> Vec<Self> {
        value.as_array().map(|items| items.iter().filter_map(Self::from_value).collect()).unwrap_or_default()
    }

    pub fn is_local(&self) -> bool {
        self.provider == LOCAL
    }

    /// How the secret is written in a request.
    pub fn reference(&self, secret: &str) -> String {
        format!("{{{{{}.{}}}}}", self.name, secret)
    }
}

/// A setting a provider takes.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderField {
    pub key: String,
    pub required: bool,
}

/// Where a vault's values can be kept, as `vault.providers` describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderInfo {
    pub id: String,
    pub label: String,
    /// The program its values are read with; empty for `local`.
    pub program: String,
    pub installed: bool,
    pub install_url: String,
    pub fields: Vec<ProviderField>,
    /// It can take a token kept on this device.
    pub takes_credential: bool,
    pub credential_required: bool,
}

impl ProviderInfo {
    pub fn list_from_value(value: &Value) -> Vec<Self> {
        let text = |item: &Value, key: &str| item.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let credential = item.get("credential").filter(|c| c.is_object());
                        Some(Self {
                            id: item.get("id")?.as_str()?.to_owned(),
                            label: text(item, "label"),
                            program: text(item, "program"),
                            installed: item.get("installed").and_then(Value::as_bool).unwrap_or(false),
                            install_url: text(item, "installUrl"),
                            fields: item
                                .get("fields")
                                .and_then(Value::as_array)
                                .map(|fields| {
                                    fields
                                        .iter()
                                        .map(|f| ProviderField {
                                            key: text(f, "key"),
                                            required: f.get("required").and_then(Value::as_bool).unwrap_or(false),
                                        })
                                        .collect()
                                })
                                .unwrap_or_default(),
                            takes_credential: credential.is_some(),
                            credential_required: credential
                                .and_then(|c| c.get("required"))
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn is_local(&self) -> bool {
        self.id == LOCAL
    }
}

/// The label of a provider setting.
pub fn field_label(key: &str) -> &'static str {
    t(match key {
        "item" => "model.vault.field.item",
        "vault" => "model.vault.field.vault",
        "secretId" => "model.vault.field.secret_id",
        "filter" => "model.vault.field.filter",
        "path" => "model.vault.field.path",
        "projectId" => "model.vault.field.project_id",
        "account" => "model.vault.field.account",
        "region" => "model.vault.field.region",
        "profile" => "model.vault.field.profile",
        "vaultName" => "model.vault.field.vault_name",
        "project" => "model.vault.field.project",
        "address" => "model.vault.field.address",
        "namespace" => "model.vault.field.namespace",
        "serverUrl" => "model.vault.field.server_url",
        "config" => "model.vault.field.config",
        _ => "model.vault.field.other",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_engines_list_is_read_and_unknown_states_are_skipped() {
        let list = json!([
            { "id": "a", "name": "api_keys", "provider": "local", "state": "unsealed", "secrets": ["K1", "K2"],
              "path": null, "approved": true, "movedAt": 12 },
            { "id": "b", "name": "team", "provider": "aws", "providerLabel": "AWS Secrets Manager", "state": "sealed",
              "secrets": ["S"], "path": "vaults/b.json", "approved": false, "settings": { "region": "eu-west-1" },
              "hasCredential": false },
            { "id": "c", "name": "x", "state": "weird" },
        ]);
        let vaults = VaultInfo::list_from_value(&list);
        assert_eq!(vaults.len(), 2);
        assert_eq!(vaults[0].state, VaultState::Unsealed);
        assert_eq!(vaults[0].reference("K1"), "{{api_keys.K1}}");
        assert!(vaults[0].is_local() && vaults[0].moved && vaults[0].path.is_empty());
        assert_eq!(vaults[1].provider_label, "AWS Secrets Manager");
        assert!(!vaults[1].approved);
        assert_eq!(vaults[1].settings, [("region".to_owned(), "eu-west-1".to_owned())]);
    }

    #[test]
    fn providers_say_what_they_take() {
        let list = json!([
            { "id": "local", "label": "This device", "installed": true, "fields": [], "credential": null },
            { "id": "bitwarden", "label": "Bitwarden", "program": "bws", "installed": false,
              "fields": [{ "key": "serverUrl", "required": false }],
              "credential": { "env": "BWS_ACCESS_TOKEN", "required": true } },
        ]);
        let providers = ProviderInfo::list_from_value(&list);
        assert!(providers[0].is_local() && !providers[0].takes_credential);
        assert!(providers[1].takes_credential && providers[1].credential_required && !providers[1].installed);
        assert_eq!(providers[1].fields[0].key, "serverUrl");
    }
}
