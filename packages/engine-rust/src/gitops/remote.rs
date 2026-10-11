//! A workspace and its remote repository: importing one that is on the account, publishing a local one as a
//! new repository, and the credentials and the access check every later sync goes through.
//!
//! The rules: a remote is assigned once and never changed; a local workspace is only ever pushed to a
//! repository created for it, and an imported one only ever lands in a directory of its own. Neither side
//! can overwrite the other.
//!
//! Who owns a published workspace is what GitHub says: the owner of its repository. Who else may use it is
//! what GitHub says too. With team sync ([`team_sync`]) everyone the repository is shared with may bring the
//! workspace to a device, and syncs it with the access they were given: someone who may push syncs like the
//! owner, someone who may only read has the workspace read-only (what the remote gets is pulled, what is
//! changed here stays here). Without team sync only the owner imports and syncs. The token an account
//! presents is asked who it belongs to. A repository is a workspace when the `workspace.json` at its root
//! says so.
//!
//! Someone else's workspace takes a second yes, from the Bifurc Console ([`console`]): the organization the
//! workspace belongs to has to have given it to the account. Being a collaborator on GitHub is not enough to
//! import one, and the console can give less than GitHub does (read where GitHub gives write, which is all
//! GitHub gives on a personal repository). The owner of a repository is never asked about.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde_json::{Value, json};

use super::{
    console::{self, Denial, Grant},
    git_lock::workspace_lock,
    github::{self, Repo},
    sync_state::{get_sync_config, save_sync_config, save_sync_meta, sync_meta_value},
};
use crate::{
    features::team_sync,
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

/// The `reason` of an import refused because the repository is another account's, which only happens with
/// team sync off.
pub const NOT_OWNER: &str = "notOwner";

/// The `reason` of an import refused because the console could not be asked.
pub const CONSOLE_UNREACHABLE: &str = "consoleUnreachable";

/// Why an import was refused: what the user is told, and a word for the refusals a client can tell apart.
struct Refusal {
    message: String,
    reason: Option<&'static str>,
}

impl From<String> for Refusal {
    fn from(message: String) -> Self {
        Refusal { message, reason: None }
    }
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

/// What the account a workspace syncs as may do with the workspace's repository, as GitHub says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// The repository is the account's own.
    Owner,
    /// Someone else's (a person's or an organization's), which the account may push to.
    Write,
    /// Someone else's, which the account may only read: the workspace is read-only on this device.
    Read,
}

impl Access {
    fn of(repo: &Repo, user_id: u64) -> Access {
        if repo.owner_id == user_id {
            Access::Owner
        } else if repo.can_push == Some(true) {
            Access::Write
        } else {
            Access::Read
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Access::Owner => "owner",
            Access::Write => "write",
            Access::Read => "read",
        }
    }
}

/// What a remote operation is about to do with the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    Pull,
    Push,
}

fn sync_config_for(repo: &Repo, access: Access) -> Value {
    json!({
        "remote": repo.clone_url, "branch": repo.default_branch, "repo": repo.full_name,
        "owner": { "host": github::host(), "userId": repo.owner_id, "login": repo.owner_login },
        "access": access.as_str(),
    })
}

/// A repository as an account's listing shows it: with what the account may do with it.
pub fn listed_repo(repo: &Repo, account: &Account) -> Value {
    let mut listed = repo.public();
    listed["access"] = json!(Access::of(repo, account.user_id).as_str());
    listed["workspaceId"] = json!(workspace_for_repo(&repo.full_name));
    listed
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

/// What GitHub said each (workspace, account, repository) link may do, and when it said so. A pull runs
/// whenever the remote moves, so the question is not asked every time.
static KNOWN_ACCESS: Mutex<Option<HashMap<String, (Access, Instant)>>> = Mutex::new(None);

/// How long access that was given by someone else is taken as still given, for a pull. Being the owner does
/// not lapse, and a push by anyone else always asks.
const ACCESS_TTL: Duration = Duration::from_secs(300);

/// Forget what GitHub said, as an engine that just started has.
#[cfg(test)]
pub fn forget_access() {
    *KNOWN_ACCESS.lock() = None;
}

/// The account a workspace syncs as and the repository it syncs with, where it has both.
fn remote_link(ws_id: &str) -> Option<(Account, String)> {
    let account = workspace_account(ws_id)?;
    let repo = get_sync_config(ws_id).map(|config| str_of(&config, "repo").to_string()).filter(|r| !r.is_empty())?;
    Some((account, repo))
}

fn access_key(ws_id: &str, account: &Account, repo_name: &str) -> String {
    format!("{ws_id}|{}|{repo_name}", account.id)
}

/// The access a workspace was last found to have, as its settings keep it: `owner`, `write` or `read`.
/// `None` without a remote, and for a remote assigned before access was recorded (its owner's, then).
pub fn workspace_access(ws_id: &str) -> Option<String> {
    get_sync_config(ws_id).map(|config| str_of(&config, "access").to_string()).filter(|access| !access.is_empty())
}

/// Keep what GitHub said in the workspace's settings, and tell the clients when it is news to them.
fn record_access(ws_id: &str, access: Access) {
    let Some(mut config) = get_sync_config(ws_id) else { return };
    if str_of(&config, "access") == access.as_str() {
        return;
    }
    config["access"] = json!(access.as_str());
    save_sync_config(ws_id, Some(config));
    crate::kernel::bus().emit("settings.changed", json!({}));
}

/// Write access as GitHub gives it, narrowed to what the console gives, for a workspace that is already on
/// this device. Only a yes for less, or a no, narrows it: a repository no organization has (one shared
/// before there was a console) keeps what GitHub gives, and so does a workspace when the console cannot be
/// asked, unless reading was all it had (`recorded`) when it last could.
async fn narrowed(github: Access, repo: &Repo, account: &Account, recorded: Option<String>) -> Access {
    if github != Access::Write {
        return github;
    }
    match console::check(account.user_id, repo.id).await {
        Ok(Grant::Allowed { write: true }) | Ok(Grant::Denied { denial: Denial::Unmanaged, .. }) => Access::Write,
        Ok(Grant::Allowed { write: false }) | Ok(Grant::Denied { .. }) => Access::Read,
        Err(_) if recorded.as_deref() == Some(Access::Read.as_str()) => Access::Read,
        Err(_) => Access::Write,
    }
}

/// Ask GitHub, and for someone else's repository the console, what the workspace's account may do with its
/// repository now. `None` for a workspace with no account or no repository.
pub async fn refresh_access(ws_id: &str) -> Result<Option<Access>, GitError> {
    let Some((account, repo_name)) = remote_link(ws_id) else { return Ok(None) };
    let token = account
        .token
        .clone()
        .ok_or_else(|| GitError(format!("Sign in to GitHub as {} to sync this workspace.", account.login)))?;
    let repo = github::get_repo(&token, &repo_name).await.map_err(|e| GitError(flag_if_rejected(&account.id, e)))?;
    let access = narrowed(Access::of(&repo, account.user_id), &repo, &account, workspace_access(ws_id)).await;
    KNOWN_ACCESS
        .lock()
        .get_or_insert_with(HashMap::new)
        .insert(access_key(ws_id, &account, &repo_name), (access, Instant::now()));
    record_access(ws_id, access);
    Ok(Some(access))
}

/// Refuse a remote operation the workspace's account may not do, by what GitHub says the account may do with
/// the repository. With team sync anyone the repository is shared with pulls, and pushes when they were
/// given write access; without it the repository has to be the account's own for either.
pub async fn ensure_access(ws_id: &str, need: Need) -> Result<(), GitError> {
    let Some((account, repo_name)) = remote_link(ws_id) else { return Ok(()) };
    let known = KNOWN_ACCESS
        .lock()
        .as_ref()
        .and_then(|known| known.get(&access_key(ws_id, &account, &repo_name)).copied())
        .filter(|(access, at)| *access == Access::Owner || (need == Need::Pull && at.elapsed() < ACCESS_TTL))
        .map(|(access, _)| access);
    let access = match known {
        Some(access) => access,
        None => match refresh_access(ws_id).await? {
            Some(access) => access,
            None => return Ok(()),
        },
    };
    let owner = named(str_of(&workspace_owner(ws_id), "login"));
    if !team_sync() && access != Access::Owner {
        return Err(GitError(format!("This workspace belongs to {owner}. Only its owner can sync it.")));
    }
    if need == Need::Push && access == Access::Read {
        return Err(GitError(format!(
            "@{} has read access to {repo_name}, so this workspace is read-only here: what you change stays on \
             this device. Ask {owner} for write access to sync it.",
            account.login
        )));
    }
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

/// Bring a workspace that lives in a repository the account owns, or with team sync one it was given access
/// to, to this device as a new workspace. Returns `{ ok: true, workspace }` or `{ ok: false, error }`, with
/// a `reason` for the refusals that are about whose workspace it is: `notOwner` when the repository is
/// another account's and team sync is off; `unmanaged`, `notGranted` or `seatInactive` when the console
/// says the account was not given it; `consoleUnreachable` when the console could not be asked.
pub async fn import_workspace(account_id: &str, repo_name: &str) -> Value {
    match import(account_id, repo_name).await {
        Ok(workspace) => json!({ "ok": true, "workspace": workspace }),
        Err(refusal) => {
            let mut result = refused(refusal.message);
            if let Some(reason) = refusal.reason {
                result["reason"] = json!(reason);
            }
            result
        }
    }
}

async fn import(account_id: &str, repo_name: &str) -> Result<Value, Refusal> {
    if !check_git_installed().await {
        return Err("Git is not installed, so a remote workspace cannot be imported.".to_string().into());
    }
    let (account, token) = signed_in(account_id).await?;
    let repo = github::get_repo(&token, repo_name).await?;
    // What the account may do with it goes with the workspace: someone who may only read has it read-only.
    let access = Access::of(&repo, account.user_id);
    if access != Access::Owner && !team_sync() {
        return Err(Refusal {
            message: format!(
                "{} belongs to {}, not @{}. A workspace can only be imported by the account that owns it.",
                repo.full_name,
                named(&repo.owner_login),
                account.login
            ),
            reason: Some(NOT_OWNER),
        });
    }
    if workspace_for_repo(&repo.full_name).is_some() {
        return Err(format!("{} is already a workspace on this device.", repo.full_name).into());
    }
    // Someone else's workspace: GitHub letting the account in is half of it. The organization it belongs to
    // has to have given it to the account, and no answer is not a yes.
    let access = match access {
        Access::Owner => access,
        _ => match console::check(account.user_id, repo.id).await {
            Ok(Grant::Allowed { write: true }) => access,
            Ok(Grant::Allowed { write: false }) => Access::Read,
            Ok(Grant::Denied { denial, message }) => {
                return Err(Refusal { message, reason: Some(denial.as_str()) });
            }
            Err(error) => {
                return Err(Refusal {
                    message: format!(
                        "{error}. A workspace that belongs to {} can only be imported once the console has confirmed                          you were given it. Check your connection and try again.",
                        named(&repo.owner_login)
                    ),
                    reason: Some(CONSOLE_UNREACHABLE),
                });
            }
        },
    };
    // Asked before anything is cloned: a repository that is not a workspace never reaches this device.
    let not_a_workspace = || format!("{} is not a Bifurc workspace: it has no workspace.json of one.", repo.full_name);
    let listed = github::read_json_file(&token, &repo.full_name, WORKSPACE_FILE).await?;
    let Some((id, _)) = listed.as_ref().and_then(workspace_identity).filter(|(id, _)| is_plain_id(id)) else {
        return Err(not_a_workspace().into());
    };
    if let Some(existing) = load_settings().workspace(&id) {
        return Err(format!(
            "A workspace with this id is already on this device (\"{}\"). A remote workspace never replaces a local one.",
            str_of(existing, "name")
        )
        .into());
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
    let Some((_, name)) = cloned.filter(|(cloned_id, _)| *cloned_id == id) else {
        return Err(not_a_workspace().into());
    };
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
        "syncConfig": sync_config_for(&repo, access),
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
    save_sync_config(ws_id, Some(sync_config_for(&repo, Access::Owner)));
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
