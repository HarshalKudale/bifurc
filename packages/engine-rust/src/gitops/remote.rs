//! A workspace and its remote repository: importing one that is on the account, publishing a local one as a
//! new repository, and the credentials and the owner check every later sync goes through.
//!
//! The rules: a workspace has one owner and only the owner may bring it to a device or sync it; a remote is
//! assigned once and never changed; a local workspace is only ever pushed to a repository created for it,
//! and an imported one only ever lands in a directory of its own. Neither side can overwrite the other.
//!
//! Who owns a published workspace is what GitHub says: the owner of its repository. The account a workspace
//! syncs as has to be that owner, and the token it presents is asked who it belongs to. A repository is a
//! workspace when the `workspace.json` at its root says so.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use parking_lot::Mutex;
use serde_json::{Value, json};

use super::{
    git_lock::workspace_lock,
    github::{self, Repo},
    sync_state::{get_sync_config, save_sync_config, save_sync_meta, sync_meta_value},
};
use crate::{
    utils::{
        id::{generate_id, now_ms},
        json::{read_json, str_of, write_json_pretty},
    },
    workspace::{
        accounts::{Account, find_account, workspace_account},
        config::GLOBAL_ENV_ID,
        fs::{
            GLOBAL_ENV_PATH, WORKSPACE_FILE, WORKSPACE_KIND, WORKSPACE_VERSION, data_root, ensure_global_environment,
            init_workspace_dir, workspace_identity, ws_dir,
        },
        git_store::{
            CommitMutation, GitError, RemoteAuth, apply_commit_identity, check_git_installed, commit_mutation, git,
            git_in, git_net_in, init_workspace_repo, remote_origin,
        },
        settings::{load_settings, save_settings},
    },
};

fn refused(message: impl Into<String>) -> Value {
    json!({ "ok": false, "error": message.into() })
}

/// The account and its token, once GitHub has said the token is that account's: what this device has stored
/// about who signed in is not taken on trust.
async fn signed_in(account_id: &str) -> Result<(Account, String), String> {
    let account = find_account(account_id).ok_or("That account is not signed in on this device.")?;
    let token = account.token.clone().ok_or_else(|| format!("Sign in to GitHub as {} again first.", account.login))?;
    let user = github::fetch_user(&token).await.map_err(|e| flag_if_rejected(account_id, e))?;
    if user.id != account.user_id {
        return Err(format!("This sign-in is not @{0}'s. Sign in to GitHub as {0} again.", account.login));
    }
    Ok((account, token))
}

/// When GitHub refused the account's token, put the account in the signed-out state (its workspaces pause
/// until it signs in again) and tell the clients. Returns the error unchanged.
pub fn flag_if_rejected(account_id: &str, error: String) -> String {
    if github::is_rejected(&error) && crate::workspace::accounts::revoke_token(account_id).is_ok() {
        crate::kernel::bus().emit("account.changed", json!({ "accountId": account_id, "status": "needsRelogin" }));
    }
    error
}

fn sync_config_for(repo: &Repo) -> Value {
    json!({
        "remote": repo.clone_url, "branch": repo.default_branch, "repo": repo.full_name,
        "owner": { "host": github::host(), "userId": repo.owner_id, "login": repo.owner_login },
    })
}

fn named(login: &str) -> String {
    if login.is_empty() { "another account".to_string() } else { format!("@{login}") }
}

fn auth_for(remote: &str, token: &str) -> Option<RemoteAuth> {
    remote_origin(remote).map(|origin| RemoteAuth { origin, token: token.to_string() })
}

/// The token a workspace's remote operations present. `None` for a workspace with no account (it uses
/// whatever the machine's git has); an error when its account was signed out.
pub fn workspace_auth(ws_id: &str) -> Result<Option<RemoteAuth>, GitError> {
    let Some(account) = workspace_account(ws_id) else { return Ok(None) };
    let token = account
        .token
        .ok_or_else(|| GitError(format!("Sign in to GitHub as {} to sync this workspace.", account.login)))?;
    let remote = get_sync_config(ws_id).map(|config| str_of(&config, "remote").to_string()).unwrap_or_default();
    Ok(auth_for(&remote, &token))
}

/// `git <args>` in a workspace's repository, for an operation that reaches its remote: as the workspace's
/// account, whichever workspace is in front.
pub async fn git_net(ws_id: &str, args: &[&str]) -> Result<String, GitError> {
    let auth = workspace_auth(ws_id)?;
    let result = git_net_in(&ws_dir(ws_id), args, auth.as_ref()).await;
    // A push or pull the remote refused for its credentials: ask GitHub whether the token is still good.
    if let (Err(e), Some(auth), Some(account)) = (&result, &auth, workspace_account(ws_id)) {
        let text = e.0.to_lowercase();
        if text.contains("authentication failed") || text.contains("invalid username or password") || text.contains("401") {
            if let Err(error) = github::fetch_user(&auth.token).await {
                flag_if_rejected(&account.id, error);
            }
        }
    }
    result
}

/// The (workspace, account, repository) links GitHub has confirmed since the engine started. A sync runs
/// every few seconds, so the question is asked once.
static CONFIRMED_OWNERS: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Refuse to sync a workspace whose repository belongs to another account than the one it syncs as. The
/// owner is the repository's owner, as GitHub reports it to that account.
pub async fn ensure_owner(ws_id: &str) -> Result<(), GitError> {
    let Some(account) = workspace_account(ws_id) else { return Ok(()) };
    let repo_name = get_sync_config(ws_id).map(|config| str_of(&config, "repo").to_string()).unwrap_or_default();
    if repo_name.is_empty() {
        return Ok(());
    }
    let key = format!("{ws_id}|{}|{repo_name}", account.id);
    if CONFIRMED_OWNERS.lock().as_ref().is_some_and(|confirmed| confirmed.contains(&key)) {
        return Ok(());
    }
    let token = account
        .token
        .clone()
        .ok_or_else(|| GitError(format!("Sign in to GitHub as {} to sync this workspace.", account.login)))?;
    let repo = github::get_repo(&token, &repo_name).await.map_err(|e| GitError(flag_if_rejected(&account.id, e)))?;
    if repo.owner_id != account.user_id {
        return Err(GitError(format!(
            "This workspace belongs to {}. Only its owner can sync it.",
            named(&repo.owner_login)
        )));
    }
    CONFIRMED_OWNERS.lock().get_or_insert_with(HashSet::new).insert(key);
    Ok(())
}

/// Who owns a workspace, for a client to show: the owner of its repository once it has one, and until then
/// the account it was created as. `null` for a workspace with neither.
pub fn workspace_owner(ws_id: &str) -> Value {
    let config = get_sync_config(ws_id);
    if let Some(owner) = config.as_ref().and_then(|config| config.get("owner")).filter(|owner| owner.is_object()) {
        return owner.clone();
    }
    // A remote assigned before the owner was recorded: `owner/name` names them.
    if let Some((login, _)) = config.as_ref().and_then(|config| str_of(config, "repo").split_once('/')) {
        return json!({ "host": github::host(), "userId": null, "login": login });
    }
    workspace_account(ws_id).map_or(Value::Null, |account| {
        json!({ "host": account.host, "userId": account.user_id, "login": account.login })
    })
}

/// The local workspace that is connected to `repo` (`owner/name`), if any.
pub fn workspace_for_repo(repo: &str) -> Option<String> {
    load_settings()
        .workspaces()
        .iter()
        .find(|ws| ws.get("syncConfig").is_some_and(|config| str_of(config, "repo").eq_ignore_ascii_case(repo)))
        .map(|ws| str_of(ws, "id").to_string())
}

/// A workspace id is a directory name; one read from a repository is not trusted to be a plain one.
fn is_plain_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

/// Move a workspace's directory out of the data that is in use, into `.trash`: what a deleted workspace left
/// behind when another takes its id, or a workspace that is taken off this device.
fn set_aside(dir: &Path, id: &str) -> std::io::Result<()> {
    let trash = data_root().join(".trash");
    std::fs::create_dir_all(&trash)?;
    std::fs::rename(dir, trash.join(format!("{id}-{}", now_ms())))
}

/// Take workspaces off this device: out of the settings, their directories set aside. The workspace in front
/// gives way to one that is left, or to none.
pub fn remove_workspaces(ids: &[String]) -> std::io::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut settings = load_settings();
    settings.workspaces_mut().retain(|ws| !ids.iter().any(|id| id == str_of(ws, "id")));
    if ids.contains(&settings.active_workspace_id()) {
        let next = settings.workspaces().first().map(|ws| str_of(ws, "id").to_string()).unwrap_or_default();
        settings.set("activeWorkspaceId", json!(next));
    }
    save_settings(&settings)?;
    for id in ids.iter().filter(|id| is_plain_id(id)) {
        let dir = ws_dir(id);
        if dir.exists() {
            // A directory something still holds open stays where it is; the workspace is gone from the app.
            let _ = set_aside(&dir, id);
        }
    }
    Ok(())
}

/// Make `workspace.json` say the directory is a workspace, in a commit of its own when it did not yet.
async fn mark_as_workspace(ws_id: &str, name: &str) -> Result<(), String> {
    let path = ws_dir(ws_id).join(WORKSPACE_FILE);
    let mut info = read_json(&path).filter(Value::is_object).unwrap_or_else(|| json!({ "createdAt": now_ms() }));
    if info.get("kind").and_then(Value::as_str) == Some(WORKSPACE_KIND) && str_of(&info, "id") == ws_id {
        return Ok(());
    }
    info["kind"] = json!(WORKSPACE_KIND);
    info["version"] = json!(WORKSPACE_VERSION);
    info["id"] = json!(ws_id);
    if str_of(&info, "name").is_empty() {
        info["name"] = json!(name);
    }
    write_json_pretty(&path, &info).map_err(|e| e.to_string())?;
    commit_mutation(CommitMutation {
        action: "update",
        entity: "workspace",
        entity_id: ws_id,
        entity_name: name,
        workspace_id: ws_id,
        rel_path: WORKSPACE_FILE,
        actor: None,
        changed_fields: &["kind".to_string()],
    })
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Commit the Global environment the engine made by itself when it never was, so the first push carries
/// what every device would otherwise make again. One the user has put variables in is theirs to sync.
async fn commit_default_global_environment(ws_id: &str) -> Result<(), String> {
    let tracked = git(ws_id, &["ls-files", "--", GLOBAL_ENV_PATH]).await.map_err(|e| e.to_string())?;
    if !tracked.trim().is_empty() {
        return Ok(());
    }
    ensure_global_environment(ws_id).map_err(|e| e.to_string())?;
    let global = read_json(&ws_dir(ws_id).join(GLOBAL_ENV_PATH));
    let untouched =
        global.as_ref().and_then(|env| env.get("variables")).and_then(Value::as_array).is_none_or(Vec::is_empty);
    if !untouched {
        return Ok(());
    }
    commit_mutation(CommitMutation {
        action: "create",
        entity: "environment",
        entity_id: GLOBAL_ENV_ID,
        entity_name: "Global",
        workspace_id: ws_id,
        rel_path: GLOBAL_ENV_PATH,
        actor: None,
        changed_fields: &[],
    })
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Bring a workspace that lives in one of the account's repositories to this device, as a new workspace.
/// Returns `{ ok: true, workspace }` or `{ ok: false, error }`.
pub async fn import_workspace(account_id: &str, repo_name: &str) -> Value {
    match import(account_id, repo_name).await {
        Ok(workspace) => json!({ "ok": true, "workspace": workspace }),
        Err(message) => refused(message),
    }
}

async fn import(account_id: &str, repo_name: &str) -> Result<Value, String> {
    if !check_git_installed().await {
        return Err("Git is not installed, so a remote workspace cannot be imported.".to_string());
    }
    let (account, token) = signed_in(account_id).await?;
    let repo = github::get_repo(&token, repo_name).await?;
    if repo.owner_id != account.user_id {
        return Err(format!(
            "{} belongs to {}, not @{}. A workspace can only be imported by the account that owns it.",
            repo.full_name,
            named(&repo.owner_login),
            account.login
        ));
    }
    if workspace_for_repo(&repo.full_name).is_some() {
        return Err(format!("{} is already a workspace on this device.", repo.full_name));
    }
    // Asked before anything is cloned: a repository that is not a workspace never reaches this device.
    let not_a_workspace = || format!("{} is not a Bifurc workspace: it has no workspace.json of one.", repo.full_name);
    let listed = github::read_json_file(&token, &repo.full_name, WORKSPACE_FILE).await?;
    let Some((id, _)) = listed.as_ref().and_then(workspace_identity).filter(|(id, _)| is_plain_id(id)) else {
        return Err(not_a_workspace());
    };
    if let Some(existing) = load_settings().workspace(&id) {
        return Err(format!(
            "A workspace with this id is already on this device (\"{}\"). A remote workspace never replaces a local one.",
            str_of(existing, "name")
        ));
    }

    std::fs::create_dir_all(data_root()).map_err(|e| e.to_string())?;
    let temp = TempDir(data_root().join(format!(".import-{}", generate_id())));
    let target = temp.0.to_string_lossy().into_owned();
    let auth = auth_for(&repo.clone_url, &token);
    git_net_in(
        &data_root(),
        &["clone", "--branch", &repo.default_branch, "--single-branch", &repo.clone_url, &target],
        auth.as_ref(),
    )
    .await
    .map_err(|e| format!("Couldn't clone {}: {e}", repo.full_name))?;

    // What landed is what counts: it has to be the workspace the repository said it was.
    let cloned = read_json(&temp.0.join(WORKSPACE_FILE)).as_ref().and_then(workspace_identity);
    let Some((_, name)) = cloned.filter(|(cloned_id, _)| *cloned_id == id) else { return Err(not_a_workspace()) };
    let name = if name.is_empty() { repo.name.clone() } else { name };

    let mut settings = load_settings();
    let dir = ws_dir(&id);
    if dir.exists() {
        set_aside(&dir, &id).map_err(|e| format!("Couldn't make room for the workspace: {e}"))?;
    }
    std::fs::rename(&temp.0, &dir).map_err(|e| format!("Couldn't move the workspace into place: {e}"))?;
    init_workspace_dir(&id, &name).map_err(|e| e.to_string())?;

    let workspace = json!({
        "id": id, "name": name, "activeEnvironmentId": null, "accountId": account.id,
        "syncConfig": sync_config_for(&repo),
        "syncMeta": sync_meta_value(None, Some(now_ms()), None),
    });
    settings.workspaces_mut().push(workspace.clone());
    settings.set("activeWorkspaceId", json!(id));
    save_settings(&settings).map_err(|e| e.to_string())?;
    apply_commit_identity(&id).await.map_err(|e| e.to_string())?;
    Ok(workspace)
}

/// Publish a local workspace as a new repository on the account. Returns `{ ok: true, repo }` or
/// `{ ok: false, error }`.
pub async fn publish_workspace(ws_id: &str, account_id: &str, repo_name: &str, private: bool) -> Value {
    let _guard = workspace_lock(ws_id).lock_owned().await;
    match publish(ws_id, account_id, repo_name, private).await {
        Ok(repo) => json!({ "ok": true, "repo": repo.public() }),
        Err(message) => refused(message),
    }
}

async fn publish(ws_id: &str, account_id: &str, repo_name: &str, private: bool) -> Result<Repo, String> {
    let settings = load_settings();
    let Some(workspace) = settings.workspace(ws_id).cloned() else { return Err("No such workspace.".to_string()) };
    if get_sync_config(ws_id).is_some() {
        return Err("This workspace already has a remote repository. It cannot be changed.".to_string());
    }
    if !check_git_installed().await {
        return Err("Git is not installed, so this workspace cannot be published. Install Git first.".to_string());
    }
    // One that was made while git was missing becomes a repository now.
    init_workspace_repo(ws_id).await.map_err(|e| format!("Couldn't make this workspace a git repository: {e}"))?;
    let bound = str_of(&workspace, "accountId");
    if !bound.is_empty() && bound != account_id {
        return Err(
            "This workspace belongs to another account. Publish it with the account it was created with.".to_string()
        );
    }
    let (account, token) = signed_in(account_id).await?;
    let dir = ws_dir(ws_id);
    let repo_name = repo_name.trim();
    if repo_name.is_empty() || repo_name.starts_with('-') {
        return Err("Give the repository a name.".to_string());
    }

    // Always a repository made for this workspace: an existing one is never pushed into.
    let name = str_of(&workspace, "name").to_string();
    let repo = github::create_repo(&token, repo_name, private, &format!("Bifurc workspace: {name}")).await?;

    // From here the remote is assigned, whatever the push does: a failed push is retried by a sync.
    let mut settings = load_settings();
    if let Some(workspace) = settings.workspace_mut(ws_id) {
        workspace["accountId"] = json!(account.id);
    }
    save_settings(&settings).map_err(|e| e.to_string())?;
    save_sync_config(ws_id, Some(sync_config_for(&repo)));
    apply_commit_identity(ws_id).await.map_err(|e| e.to_string())?;
    // What is pushed says it is a workspace, so the repository can be told from any other on import.
    mark_as_workspace(ws_id, &name).await?;
    commit_default_global_environment(ws_id).await?;

    let _ = git(ws_id, &["remote", "remove", "origin"]).await;
    git(ws_id, &["remote", "add", "origin", &repo.clone_url]).await.map_err(|e| e.to_string())?;
    let _ = git_in(&dir, &["branch", "-M", &repo.default_branch]).await;
    let auth = auth_for(&repo.clone_url, &token);
    git_net_in(&dir, &["push", "--set-upstream", "origin", &format!("HEAD:{}", repo.default_branch)], auth.as_ref())
        .await
        .map_err(|e| format!("{} was created, but the first push failed: {e}. Sync to try again.", repo.full_name))?;
    save_sync_meta(ws_id, Some(sync_meta_value(Some(now_ms()), None, None)));
    github::mark_as_workspace(&token, &repo.full_name).await;
    Ok(repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_id_from_a_repository_must_be_a_plain_name() {
        assert!(is_plain_id("mf3k2a9x_b-1"));
        for bad in ["", "../x", "a/b", "a\\b", "c:", "."] {
            assert!(!is_plain_id(bad), "{bad}");
        }
    }
}
