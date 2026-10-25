//! Vault files. No value, sealed or not, is ever in a workspace's repository: a copy that was once pushed stays
//! in its history, and opens for anyone who learns the passphrase it had then.
//!
//! - A **local** vault is `<data dir>/vaults/<workspace>/<id>.json`, outside every repository: its secrets'
//!   names in the clear and their values sealed with its passphrase. It is on this device only.
//! - A **linked** vault is `<workspace>/vaults/<id>.json`, synced like any entity: its provider and the
//!   source there that its secrets are pulled from. Nothing in it is a secret, and nothing of it is edited
//!   here but the source: the secrets are the provider's. What this device adds to it is
//!   `<data dir>/vaults/<workspace>/<id>.link.json`: the token given for the provider, if any, the source as
//!   last approved here, and the names last pulled. A synced file that names another source (someone pointed
//!   it at another item, or the provider at another address) is not pulled from until it is approved again on
//!   this device.
//!
//! Vaults made before linked ones existed are local vaults that were kept in the repository; [`migrate`]
//! moves them out.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::crypto::{Kdf, Key, Sealed, open, seal};
use crate::{
    protocol::{EngineError, Result},
    utils::{
        id::now_ms,
        json::{read_json, write_json_pretty},
    },
    workspace::{
        fs::{entity_dir, flat_entity_rel_path},
        paths::data_dir,
    },
};

/// The directory of linked vaults in a workspace.
pub const KIND: &str = "vaults";
const LINK_SUFFIX: &str = ".link.json";

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
    /// When it was moved out of the workspace's repository, until the user has been told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedVault {
    pub id: String,
    pub name: String,
    pub version: u32,
    pub created_at: i64,
    pub provider: String,
    /// The source in the provider that every secret is pulled from.
    #[serde(default)]
    pub settings: Map<String, Value>,
    /// The names it gave when this device last pulled it (from [`Link::names`]); not in the synced file.
    #[serde(skip)]
    pub names: Vec<String>,
}

impl LinkedVault {
    /// What approving it on a device agrees to: where values are pulled from, not what the vault is called.
    pub fn fingerprint(&self) -> String {
        json!({ "provider": self.provider, "settings": self.settings }).to_string()
    }
}

/// What this device keeps for a linked vault.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    /// The provider's token, sealed to this device's user where the platform can.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    /// [`LinkedVault::fingerprint`] as approved here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<String>,
    /// The secrets' names as last pulled here, so a sealed vault still shows what it holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
}

impl Link {
    pub fn credential(&self) -> Option<String> {
        self.credential.as_deref().and_then(crate::workspace::accounts::unseal_secret)
    }

    pub fn set_credential(&mut self, token: Option<&str>) -> Result<()> {
        self.credential = match token.map(str::trim).filter(|t| !t.is_empty()) {
            Some(token) => Some(crate::workspace::accounts::seal_secret(token).ok_or_else(|| {
                EngineError::engine("The token could not be protected on this device.")
                    .keyed("vault.credential_unprotected", json!({}))
            })?),
            None => None,
        };
        Ok(())
    }
}

/// A vault of either kind.
#[derive(Debug, Clone)]
pub enum Vault {
    Local(VaultFile),
    Linked(LinkedVault),
}

impl Vault {
    pub fn id(&self) -> &str {
        match self {
            Vault::Local(v) => &v.id,
            Vault::Linked(v) => &v.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Vault::Local(v) => &v.name,
            Vault::Linked(v) => &v.name,
        }
    }

    pub fn provider(&self) -> &str {
        match self {
            Vault::Local(_) => super::providers::LOCAL,
            Vault::Linked(v) => &v.provider,
        }
    }

    pub fn secret_names(&self) -> Vec<&str> {
        match self {
            Vault::Local(v) => v.secret_names(),
            Vault::Linked(v) => v.names.iter().map(String::as_str).collect(),
        }
    }

    pub fn created_at(&self) -> i64 {
        match self {
            Vault::Local(v) => v.created_at,
            Vault::Linked(v) => v.created_at,
        }
    }
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
        ))
        .keyed("vault.identifier_invalid", json!({ "what": what, "max": MAX_NAME_LEN })));
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

/// This device's vault files of a workspace: local vaults and what it keeps for linked ones.
fn device_dir(ws_id: &str) -> PathBuf {
    data_dir().join("vaults").join(ws_id)
}

fn local_path(ws_id: &str, id: &str) -> PathBuf {
    device_dir(ws_id).join(format!("{id}.json"))
}

fn link_path(ws_id: &str, id: &str) -> PathBuf {
    device_dir(ws_id).join(format!("{id}{LINK_SUFFIX}"))
}

fn linked_path(ws_id: &str, id: &str) -> PathBuf {
    entity_dir(ws_id, KIND).join(format!("{id}.json"))
}

/// A linked vault's path in its workspace's repository, as `entity.publish` takes it.
pub fn rel_path(id: &str) -> String {
    flat_entity_rel_path(KIND, id)
}

fn json_files(dir: &std::path::Path) -> Vec<(PathBuf, Value)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter(|path| !path.to_string_lossy().ends_with(LINK_SUFFIX))
        .filter_map(|path| read_json(&path).map(|value| (path, value)))
        .collect()
}

/// A vault file of the repository from before linked vaults: a local vault, which it holds sealed values of.
fn is_old_local(value: &Value) -> bool {
    value.get("kdf").is_some() && value.get("check").is_some()
}

static MIGRATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Moves the local vaults found in the workspace's repository to this device, and returns their repository
/// paths, which the caller commits as deleted so they stop being synced. A vault this device already has
/// (moved before, then brought back by a pull from an older version) keeps this device's copy.
pub fn migrate(ws_id: &str) -> Vec<(String, String)> {
    let _guard = MIGRATE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut moved = Vec::new();
    for (path, value) in json_files(&entity_dir(ws_id, KIND)).into_iter().filter(|(_, v)| is_old_local(v)) {
        let Ok(mut vault) = serde_json::from_value::<VaultFile>(value) else { continue };
        if !local_path(ws_id, &vault.id).exists() {
            vault.moved_at = Some(now_ms());
            if save(ws_id, &vault).is_err() {
                continue;
            }
        }
        if std::fs::remove_file(&path).is_ok() {
            moved.push((rel_path(&vault.id), vault.name));
        }
    }
    moved
}

/// Every vault of the workspace, local and linked, by name.
pub fn list(ws_id: &str) -> Vec<Vault> {
    let locals = json_files(&device_dir(ws_id))
        .into_iter()
        .filter_map(|(_, value)| serde_json::from_value(value).ok())
        .map(Vault::Local);
    let linked = json_files(&entity_dir(ws_id, KIND))
        .into_iter()
        .filter(|(_, value)| !is_old_local(value))
        .filter_map(|(_, value)| linked_from(ws_id, value))
        .map(Vault::Linked);
    let mut vaults: Vec<Vault> = locals.chain(linked).collect();
    vaults.sort_by(|a, b| a.name().to_lowercase().cmp(&b.name().to_lowercase()).then(a.id().cmp(b.id())));
    vaults
}

fn check_id(id: &str) -> Result<()> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(EngineError::bad_request("Not a vault id.").keyed("vault.id_invalid", json!({})));
    }
    Ok(())
}

fn not_found(id: &str) -> EngineError {
    EngineError::not_found(format!("No vault \"{id}\".")).keyed("vault.not_found", json!({ "id": id }))
}

pub fn get(ws_id: &str, id: &str) -> Result<Vault> {
    check_id(id)?;
    if let Some(local) = read_json(&local_path(ws_id, id)).and_then(|v| serde_json::from_value(v).ok()) {
        return Ok(Vault::Local(local));
    }
    read_json(&linked_path(ws_id, id))
        .filter(|value| !is_old_local(value))
        .and_then(|value| linked_from(ws_id, value))
        .map(Vault::Linked)
        .ok_or_else(|| not_found(id))
}

/// A linked vault's file, with the names this device last pulled.
fn linked_from(ws_id: &str, value: Value) -> Option<LinkedVault> {
    let mut vault: LinkedVault = serde_json::from_value(value).ok()?;
    vault.names = link(ws_id, &vault.id).names;
    Some(vault)
}

/// The local vault `id`; a linked one is refused.
pub fn get_local(ws_id: &str, id: &str) -> Result<VaultFile> {
    match get(ws_id, id)? {
        Vault::Local(vault) => Ok(vault),
        Vault::Linked(vault) => Err(EngineError::bad_request(format!(
            "The values of \"{}\" are kept in its provider; change them there.",
            vault.name
        ))
        .keyed("vault.values_in_provider", json!({ "name": vault.name }))),
    }
}

/// The linked vault `id`; a local one is refused.
pub fn get_linked(ws_id: &str, id: &str) -> Result<LinkedVault> {
    match get(ws_id, id)? {
        Vault::Linked(vault) => Ok(vault),
        Vault::Local(vault) => Err(EngineError::bad_request(format!(
            "\"{}\" is kept on this device; it has no provider or references.",
            vault.name
        ))
        .keyed("vault.not_linked", json!({ "name": vault.name }))),
    }
}

pub fn find_by_name(ws_id: &str, name: &str) -> Option<Vault> {
    list(ws_id).into_iter().find(|vault| vault.name().eq_ignore_ascii_case(name))
}

/// Writes a file of this device's only, readable by its owner alone where the platform says so.
fn write_private(path: &std::path::Path, value: &Value) -> Result<()> {
    write_json_pretty(path, value).map_err(io_failed)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(io_failed)?;
    }
    Ok(())
}

pub fn save(ws_id: &str, vault: &VaultFile) -> Result<()> {
    write_private(&local_path(ws_id, &vault.id), &serde_json::to_value(vault).map_err(io_failed)?)
}

pub fn save_linked(ws_id: &str, vault: &LinkedVault) -> Result<()> {
    let value = serde_json::to_value(vault).map_err(io_failed)?;
    write_json_pretty(&linked_path(ws_id, &vault.id), &value).map_err(io_failed)
}

pub fn link(ws_id: &str, id: &str) -> Link {
    read_json(&link_path(ws_id, id)).and_then(|value| serde_json::from_value(value).ok()).unwrap_or_default()
}

pub fn save_link(ws_id: &str, id: &str, link: &Link) -> Result<()> {
    write_private(&link_path(ws_id, id), &serde_json::to_value(link).map_err(io_failed)?)
}

fn remove(path: PathBuf) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_failed(e)),
    }
}

/// Removes the vault's files, of either kind: a linked vault's repository file is then a deletion to sync.
pub fn delete(ws_id: &str, id: &str) -> Result<()> {
    check_id(id)?;
    remove(local_path(ws_id, id))?;
    remove(link_path(ws_id, id))?;
    remove(linked_path(ws_id, id))
}

/// A vault file that could not be written or removed, under its wrapper key with the raw text as `{detail}`.
fn io_failed(e: impl std::fmt::Display) -> EngineError {
    let detail = e.to_string();
    EngineError::engine(detail.clone()).keyed("vault.io_failed", json!({ "detail": detail }))
}

impl VaultFile {
    /// A new, empty vault and the key its passphrase gives.
    pub fn create(id: String, name: &str, passphrase: &str, now: i64) -> Result<(Self, Key)> {
        let kdf = Kdf::new();
        let key = kdf.derive(passphrase)?;
        let check = seal(&key, &aad(&id, "check"), CHECK_TEXT)?;
        let vault = Self {
            id,
            name: name.to_string(),
            version: 1,
            created_at: now,
            kdf,
            check,
            secrets: Vec::new(),
            moved_at: None,
        };
        Ok((vault, key))
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
            return Err(EngineError::engine("Some secrets in this vault do not open with its key; not re-encrypting.")
                .keyed("vault.rekey_unreadable", json!({})));
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
