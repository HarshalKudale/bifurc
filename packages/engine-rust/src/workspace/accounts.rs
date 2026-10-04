//! The git accounts signed in on this device (`accounts.json`), and which one each workspace uses.
//!
//! A sign-in belongs to a workspace: the workspace's settings name the account it commits and syncs as, and
//! two workspaces may name two different accounts. The file sits beside `app.json`, outside every workspace
//! repository, so a token never reaches a remote. On Windows the token is sealed to the signed-in user
//! (DPAPI); elsewhere the file is readable by its owner only.

use std::path::PathBuf;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use super::{git_store::device_name, paths::data_dir, settings::load_settings};
use crate::utils::{
    id::now_ms,
    json::{array_mut, array_of, read_json, str_of, write_json_pretty},
};

/// One signed-in git account. `token` is `None` once it was signed out while workspaces still use it.
#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    /// `<host>:<user id>`: the same person on the same host is the same account on every device.
    pub id: String,
    pub host: String,
    pub user_id: u64,
    pub login: String,
    pub name: String,
    pub avatar_url: String,
    pub token: Option<String>,
}

impl Account {
    /// What a client may see: everything but the token.
    pub fn public(&self) -> Value {
        json!({
            "id": self.id, "host": self.host, "userId": self.user_id, "login": self.login, "name": self.name,
            "avatarUrl": self.avatar_url, "signedIn": self.token.is_some(),
        })
    }

    /// The author its commits carry. The address is the one the host maps back to the account.
    pub fn commit_identity(&self) -> (String, String) {
        let name = if self.name.is_empty() { self.login.clone() } else { self.name.clone() };
        (name, format!("{}+{}@users.noreply.{}", self.user_id, self.login, self.host))
    }
}

pub fn account_id(host: &str, user_id: u64) -> String {
    format!("{host}:{user_id}")
}

fn accounts_path() -> PathBuf {
    data_dir().join("accounts.json")
}

#[cfg(windows)]
fn seal(token: &str) -> Option<String> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData},
    };
    let mut input = token.as_bytes().to_vec();
    let blob_in = CRYPT_INTEGER_BLOB { cbData: input.len() as u32, pbData: input.as_mut_ptr() };
    let mut blob_out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY: both blobs outlive the call; the output buffer is copied and then freed with LocalFree.
    unsafe {
        let ok = CryptProtectData(
            &blob_in,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            &mut blob_out,
        );
        if ok == 0 || blob_out.pbData.is_null() {
            return None;
        }
        let sealed = std::slice::from_raw_parts(blob_out.pbData, blob_out.cbData as usize).to_vec();
        LocalFree(blob_out.pbData.cast());
        Some(format!("dpapi:{}", STANDARD.encode(sealed)))
    }
}

#[cfg(windows)]
fn unseal_dpapi(encoded: &str) -> Option<String> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData},
    };
    let mut input = STANDARD.decode(encoded).ok()?;
    let blob_in = CRYPT_INTEGER_BLOB { cbData: input.len() as u32, pbData: input.as_mut_ptr() };
    let mut blob_out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY: as in `seal`.
    unsafe {
        let ok = CryptUnprotectData(
            &blob_in,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            &mut blob_out,
        );
        if ok == 0 || blob_out.pbData.is_null() {
            return None;
        }
        let plain = std::slice::from_raw_parts(blob_out.pbData, blob_out.cbData as usize).to_vec();
        LocalFree(blob_out.pbData.cast());
        String::from_utf8(plain).ok()
    }
}

#[cfg(not(windows))]
fn seal(token: &str) -> Option<String> {
    Some(format!("plain:{}", STANDARD.encode(token)))
}

fn unseal(secret: &str) -> Option<String> {
    if let Some(encoded) = secret.strip_prefix("plain:") {
        return String::from_utf8(STANDARD.decode(encoded).ok()?).ok();
    }
    #[cfg(windows)]
    if let Some(encoded) = secret.strip_prefix("dpapi:") {
        return unseal_dpapi(encoded);
    }
    None
}

fn load_file() -> Value {
    read_json(&accounts_path()).filter(Value::is_object).unwrap_or_else(|| json!({ "accounts": [] }))
}

fn save_file(file: &Value) -> std::io::Result<()> {
    let path = accounts_path();
    write_json_pretty(&path, file)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn from_stored(stored: &Value) -> Option<Account> {
    let id = str_of(stored, "id");
    if id.is_empty() {
        return None;
    }
    Some(Account {
        id: id.to_string(),
        host: str_of(stored, "host").to_string(),
        user_id: stored.get("userId").and_then(Value::as_u64)?,
        login: str_of(stored, "login").to_string(),
        name: str_of(stored, "name").to_string(),
        avatar_url: str_of(stored, "avatarUrl").to_string(),
        token: stored.get("secret").and_then(Value::as_str).and_then(unseal),
    })
}

/// Every account as a client sees it, without opening any token.
pub fn public_accounts() -> Vec<Value> {
    array_of(&load_file(), "accounts")
        .iter()
        .filter(|stored| !str_of(stored, "id").is_empty())
        .map(|stored| {
            json!({
                "id": str_of(stored, "id"), "host": str_of(stored, "host"),
                "userId": stored.get("userId").cloned().unwrap_or(Value::Null),
                "login": str_of(stored, "login"), "name": str_of(stored, "name"),
                "avatarUrl": str_of(stored, "avatarUrl"),
                "signedIn": stored.get("secret").is_some_and(Value::is_string),
            })
        })
        .collect()
}

pub fn list_accounts() -> Vec<Account> {
    array_of(&load_file(), "accounts").iter().filter_map(from_stored).collect()
}

pub fn find_account(id: &str) -> Option<Account> {
    list_accounts().into_iter().find(|account| account.id == id)
}

/// Record a sign-in, replacing what was stored for the same person.
pub fn save_account(account: &Account) -> std::io::Result<()> {
    let secret = match &account.token {
        Some(token) => Some(seal(token).ok_or_else(|| std::io::Error::other("the token could not be protected"))?),
        None => None,
    };
    let mut file = load_file();
    let accounts = array_mut(&mut file, "accounts");
    accounts.retain(|stored| str_of(stored, "id") != account.id);
    accounts.push(json!({
        "id": account.id, "host": account.host, "userId": account.user_id, "login": account.login,
        "name": account.name, "avatarUrl": account.avatar_url, "secret": secret, "addedAt": now_ms(),
    }));
    save_file(&file)
}

/// The ids of the workspaces that use the account.
pub fn workspaces_using(account_id: &str) -> Vec<String> {
    load_settings()
        .workspaces()
        .iter()
        .filter(|ws| str_of(ws, "accountId") == account_id)
        .map(|ws| str_of(ws, "id").to_string())
        .collect()
}

/// Drop the token GitHub no longer accepts, keeping the account: it shows as signed out, and signing in
/// again as the same person puts it back.
pub fn revoke_token(account_id: &str) -> std::io::Result<()> {
    let mut file = load_file();
    for stored in array_mut(&mut file, "accounts") {
        if str_of(stored, "id") == account_id {
            stored["secret"] = Value::Null;
        }
    }
    save_file(&file)
}

/// Forget the account and its token. The workspaces that used it are taken off the device by the caller.
pub fn sign_out(account_id: &str) -> std::io::Result<()> {
    let mut file = load_file();
    array_mut(&mut file, "accounts").retain(|stored| str_of(stored, "id") != account_id);
    save_file(&file)
}

/// The account a workspace commits and syncs as, if it has one.
pub fn workspace_account(ws_id: &str) -> Option<Account> {
    let settings = load_settings();
    let id = str_of(settings.workspace(ws_id)?, "accountId");
    if id.is_empty() { None } else { find_account(id) }
}

/// Who a workspace's commits are by: its account, or this device when it has none.
pub struct CommitIdentity {
    pub name: String,
    pub email: String,
    /// The `actor:` an audit entry shows.
    pub actor: String,
}

pub fn commit_identity(ws_id: &str) -> CommitIdentity {
    match workspace_account(ws_id) {
        Some(account) => {
            let (name, email) = account.commit_identity();
            CommitIdentity { name, email, actor: account.login.clone() }
        }
        None => {
            let name = device_name();
            CommitIdentity { email: format!("{name}@bifurc"), actor: name.clone(), name }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch_data_root;

    fn sample(token: Option<&str>) -> Account {
        Account {
            id: account_id("github.com", 42),
            host: "github.com".into(),
            user_id: 42,
            login: "octo".into(),
            name: "Octo Cat".into(),
            avatar_url: String::new(),
            token: token.map(str::to_string),
        }
    }

    #[test]
    fn a_token_is_stored_sealed_and_read_back() {
        let root = scratch_data_root();
        save_account(&sample(Some("gho_secret"))).unwrap();
        let raw = std::fs::read_to_string(root.dir.path().join("accounts.json")).unwrap();
        assert!(!raw.contains("gho_secret"), "the token is not written as it is");
        assert_eq!(find_account("github.com:42").unwrap().token.as_deref(), Some("gho_secret"));
        assert_eq!(sample(None).public()["signedIn"], false);
        assert!(sample(Some("t")).public().get("token").is_none());
    }

    #[test]
    fn signing_out_forgets_the_account() {
        let root = scratch_data_root();
        save_account(&sample(Some("t"))).unwrap();
        let mut settings = load_settings();
        settings.workspace_mut(root.ws_id).unwrap()["accountId"] = json!("github.com:42");
        crate::workspace::settings::save_settings(&settings).unwrap();
        assert_eq!(workspaces_using("github.com:42"), [root.ws_id]);
        sign_out("github.com:42").unwrap();
        assert!(find_account("github.com:42").is_none());
        assert!(workspace_account(root.ws_id).is_none());
    }

    #[test]
    fn commits_are_by_the_account_or_by_the_device() {
        let root = scratch_data_root();
        let device = commit_identity(root.ws_id);
        assert!(device.email.ends_with("@bifurc"));

        save_account(&sample(Some("t"))).unwrap();
        let mut settings = load_settings();
        settings.workspace_mut(root.ws_id).unwrap()["accountId"] = json!("github.com:42");
        crate::workspace::settings::save_settings(&settings).unwrap();
        let identity = commit_identity(root.ws_id);
        assert_eq!(identity.email, "42+octo@users.noreply.github.com");
        assert_eq!(identity.actor, "octo");
    }
}
