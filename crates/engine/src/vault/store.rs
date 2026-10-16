//! Vault files: `<workspace>/vaults/<id>.json`. A vault holds its secrets' names in the clear and their values
//! sealed, so the file can be committed and reviewed (a diff shows which secret changed) without revealing
//! anything.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::crypto::{Kdf, Key, Sealed, open, seal};
use crate::{
    protocol::{EngineError, Result},
    utils::json::{read_json, write_json_pretty},
    workspace::fs::{entity_dir, flat_entity_rel_path},
};

pub const KIND: &str = "vaults";

/// What a correct key opens: proves a passphrase right even when the vault holds no secrets yet.
const CHECK_TEXT: &[u8] = b"bifurc-vault";
const MAX_NAME_LEN: usize = 64;
const MIN_PASSPHRASE_LEN: usize = 8;
/// Used in `{{random.*}}`, so a vault of that name could never be referenced.
const RESERVED_NAMES: [&str; 1] = ["random"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Secret {
    pub name: String,
    #[serde(flatten)]
    pub sealed: Sealed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultFile {
    pub id: String,
    pub name: String,
    pub version: u32,
    pub created_at: i64,
    pub kdf: Kdf,
    pub check: Sealed,
    pub secrets: Vec<Secret>,
}

fn aad(vault_id: &str, part: &str) -> String {
    format!("bifurc-vault/1/{vault_id}/{part}")
}

/// A name usable both in `{{vault.secret}}` and as an environment variable: a letter or underscore, then
/// letters, digits and underscores.
fn check_identifier(what: &str, name: &str) -> Result<()> {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if !first_ok || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || name.len() > MAX_NAME_LEN {
        return Err(EngineError::bad_request(format!(
            "A {what} name must start with a letter or underscore and use only letters, digits and underscores \
             (at most {MAX_NAME_LEN}), so that it works in {{{{vault.secret}}}} and as an environment variable."
        )));
    }
    Ok(())
}

pub fn check_vault_name(name: &str) -> Result<()> {
    check_identifier("vault", name)?;
    if RESERVED_NAMES.contains(&name.to_ascii_lowercase().as_str()) {
        return Err(EngineError::bad_request(format!("\"{name}\" is reserved; choose another vault name."))
            .keyed("vault.name_reserved", json!({ "name": name })));
    }
    Ok(())
}

pub fn check_secret_name(name: &str) -> Result<()> {
    check_identifier("secret", name)
}

pub fn check_passphrase(passphrase: &str) -> Result<()> {
    if passphrase.chars().count() < MIN_PASSPHRASE_LEN {
        return Err(EngineError::bad_request(format!(
            "The passphrase must be at least {MIN_PASSPHRASE_LEN} characters."
        ))
        .keyed("vault.passphrase_short", json!({ "min": MIN_PASSPHRASE_LEN })));
    }
    Ok(())
}

fn file_path(ws_id: &str, id: &str) -> PathBuf {
    entity_dir(ws_id, KIND).join(format!("{id}.json"))
}

/// The vault's path in its workspace's repository, as `entity.publish` takes it.
pub fn rel_path(id: &str) -> String {
    flat_entity_rel_path(KIND, id)
}

pub fn list(ws_id: &str) -> Vec<VaultFile> {
    let Ok(entries) = std::fs::read_dir(entity_dir(ws_id, KIND)) else { return Vec::new() };
    let mut vaults: Vec<VaultFile> = entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| read_json(&entry.path()))
        .filter_map(|value: Value| serde_json::from_value(value).ok())
        .collect();
    vaults.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    vaults
}

pub fn get(ws_id: &str, id: &str) -> Result<VaultFile> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(EngineError::bad_request("Not a vault id."));
    }
    read_json(&file_path(ws_id, id)).and_then(|value| serde_json::from_value(value).ok()).ok_or_else(|| {
        EngineError::not_found(format!("No vault \"{id}\".")).keyed("vault.not_found", json!({ "id": id }))
    })
}

pub fn find_by_name(ws_id: &str, name: &str) -> Option<VaultFile> {
    list(ws_id).into_iter().find(|vault| vault.name.eq_ignore_ascii_case(name))
}

pub fn save(ws_id: &str, vault: &VaultFile) -> Result<()> {
    let value = serde_json::to_value(vault).map_err(|e| EngineError::engine(e.to_string()))?;
    write_json_pretty(&file_path(ws_id, &vault.id), &value).map_err(|e| EngineError::engine(e.to_string()))
}

pub fn delete(ws_id: &str, id: &str) -> Result<()> {
    match std::fs::remove_file(file_path(ws_id, id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(EngineError::engine(e.to_string())),
    }
}

impl VaultFile {
    /// A new, empty vault and the key its passphrase gives.
    pub fn create(id: String, name: &str, passphrase: &str, now: i64) -> Result<(Self, Key)> {
        let kdf = Kdf::new();
        let key = kdf.derive(passphrase)?;
        let check = seal(&key, &aad(&id, "check"), CHECK_TEXT)?;
        Ok((Self { id, name: name.to_string(), version: 1, created_at: now, kdf, check, secrets: Vec::new() }, key))
    }

    /// The key `passphrase` gives, if it is the right one.
    pub fn unlock(&self, passphrase: &str) -> Result<Key> {
        let key = self.kdf.derive(passphrase)?;
        match open(&key, &aad(&self.id, "check"), &self.check) {
            Some(text) if text.as_slice() == CHECK_TEXT => Ok(key),
            _ => Err(EngineError::new(crate::protocol::ErrorCode::Forbidden, "Wrong passphrase.")
                .keyed("vault.wrong_passphrase", json!({}))),
        }
    }

    pub fn secret_names(&self) -> Vec<&str> {
        self.secrets.iter().map(|s| s.name.as_str()).collect()
    }

    pub fn set_secret(&mut self, key: &Key, name: &str, value: &str) -> Result<()> {
        let sealed = seal(key, &aad(&self.id, &format!("secret/{name}")), value.as_bytes())?;
        match self.secrets.iter_mut().find(|s| s.name.eq_ignore_ascii_case(name)) {
            Some(existing) => *existing = Secret { name: name.to_string(), sealed },
            None => self.secrets.push(Secret { name: name.to_string(), sealed }),
        }
        self.secrets.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(())
    }

    /// The value of one secret; `None` for an unknown name or a value this key does not open.
    pub fn read_secret(&self, key: &Key, name: &str) -> Option<zeroize::Zeroizing<String>> {
        let secret = self.secrets.iter().find(|s| s.name == name)?;
        let bytes = open(key, &aad(&self.id, &format!("secret/{}", secret.name)), &secret.sealed)?;
        String::from_utf8(bytes.to_vec()).ok().map(zeroize::Zeroizing::new)
    }

    /// Every secret as `(name, value)`; ones this key does not open are left out.
    pub fn read_all(&self, key: &Key) -> Vec<(String, zeroize::Zeroizing<String>)> {
        self.secrets
            .iter()
            .filter_map(|secret| self.read_secret(key, &secret.name).map(|value| (secret.name.clone(), value)))
            .collect()
    }

    /// Re-seal everything under the key a new passphrase gives (with a fresh salt).
    pub fn rekey(&mut self, old: &Key, new_passphrase: &str) -> Result<Key> {
        let values = self.read_all(old);
        if values.len() != self.secrets.len() {
            return Err(EngineError::engine("Some secrets in this vault do not open with its key; not re-encrypting."));
        }
        let kdf = Kdf::new();
        let key = kdf.derive(new_passphrase)?;
        self.kdf = kdf;
        self.check = seal(&key, &aad(&self.id, "check"), CHECK_TEXT)?;
        self.secrets.clear();
        for (name, value) in values {
            self.set_secret(&key, &name, &value)?;
        }
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_must_work_as_references_and_environment_variables() {
        for ok in ["api_keys", "_x", "A1", "STRIPE_KEY"] {
            assert!(check_secret_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "a b", "1a", "a-b", "a.b", "a{b}", "ünï", &"x".repeat(65)] {
            assert!(check_secret_name(bad).is_err(), "{bad}");
        }
        assert!(check_vault_name("random").is_err());
        assert!(check_vault_name("Random").is_err());
        assert!(check_vault_name("api_keys").is_ok());
    }

    #[test]
    fn secrets_round_trip_and_stay_sealed_in_the_file() {
        let (mut vault, key) = VaultFile::create("v1".into(), "keys", "long enough pass", 0).unwrap();
        vault.set_secret(&key, "TOKEN", "hunter2-hunter2").unwrap();
        vault.set_secret(&key, "TOKEN", "replaced-value").unwrap();
        assert_eq!(vault.secrets.len(), 1);
        assert_eq!(vault.read_secret(&key, "TOKEN").unwrap().as_str(), "replaced-value");
        assert!(!serde_json::to_string(&vault).unwrap().contains("replaced-value"));
        assert!(vault.unlock("long enough pass").is_ok());
        assert!(vault.unlock("another passphrase").is_err());
    }

    #[test]
    fn a_secret_moved_to_another_name_does_not_open() {
        let (mut vault, key) = VaultFile::create("v1".into(), "keys", "long enough pass", 0).unwrap();
        vault.set_secret(&key, "A", "value-of-a").unwrap();
        vault.secrets[0].name = "B".into();
        assert!(vault.read_secret(&key, "B").is_none());
    }

    #[test]
    fn rekeying_keeps_every_secret_and_drops_the_old_passphrase() {
        let (mut vault, key) = VaultFile::create("v1".into(), "keys", "old passphrase", 0).unwrap();
        vault.set_secret(&key, "A", "value-of-a").unwrap();
        let new_key = vault.rekey(&key, "new passphrase").unwrap();
        assert_eq!(vault.read_secret(&new_key, "A").unwrap().as_str(), "value-of-a");
        assert!(vault.unlock("old passphrase").is_err());
        assert!(vault.unlock("new passphrase").is_ok());
    }
}
