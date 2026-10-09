//! Whether a workspace is an organization's, and what that organization decided about its usage analytics.
//!
//! The Bifurc Console knows ([`console::policy`]); what it last said is kept with the workspace in `app.json`,
//! as `organization`: `{ publicId, name, workspaceId, analytics, githubUserId }`, or nothing for a workspace
//! no organization has. It is asked when a workspace's remote is first looked at and every half hour after
//! ([`super::auto_sync`]), and whenever the workspace's access is asked about. A console that cannot be
//! reached changes nothing: what was known stands.
//!
//! An organization's workspace has its analytics managed there, not on this device: on, what is done in the
//! workspace is reported with the organization and the account it was done as, whatever the app's own switch
//! says; off, nothing done in it is reported at all ([`crate::analytics`]).

use serde_json::{Value, json};

use super::{
    console::{self, Policy},
    github,
    sync_state::{get_sync_config, save_sync_config},
};
use crate::{
    utils::json::str_of,
    workspace::{
        accounts::{Account, workspace_account},
        settings::{load_settings, save_settings},
    },
};

/// What the console last said of the workspace's organization, as `app.json` keeps it.
pub fn workspace_organization(ws_id: &str) -> Option<Value> {
    load_settings().workspace(ws_id)?.get("organization").filter(|organization| organization.is_object()).cloned()
}

fn stored(policy: &Policy, account: &Account) -> Option<Value> {
    match policy {
        Policy::Unmanaged => None,
        Policy::Managed { public_id, name, workspace_id, analytics } => Some(json!({
            "publicId": public_id, "name": name, "workspaceId": workspace_id, "analytics": analytics,
            "githubUserId": account.user_id,
        })),
    }
}

/// Keep what the console said, and tell the clients and the analytics when it is news.
fn record(ws_id: &str, organization: Option<Value>) {
    let mut settings = load_settings();
    let Some(workspace) = settings.workspace_mut(ws_id) else { return };
    if workspace.get("organization").filter(|known| !known.is_null()) == organization.as_ref() {
        return;
    }
    workspace["organization"] = organization.unwrap_or(Value::Null);
    if save_settings(&settings).is_ok() {
        crate::analytics::policies_changed();
        crate::kernel::bus().emit("settings.changed", json!({}));
    }
}

/// Ask the console about a workspace whose account and repository are at hand.
pub async fn refresh_for(ws_id: &str, account: &Account, repo_id: u64) {
    if let Ok(policy) = console::policy(account.user_id, repo_id).await {
        record(ws_id, stored(&policy, account));
    }
}

/// GitHub's id of the workspace's repository: what the workspace's settings hold, else what GitHub says, kept
/// from then on (a remote assigned before the id was recorded has none).
async fn repo_id(ws_id: &str, account: &Account, config: &Value) -> Option<u64> {
    if let Some(id) = config.get("repoId").and_then(Value::as_u64) {
        return Some(id);
    }
    let repo = github::get_repo(account.token.as_deref()?, str_of(config, "repo")).await.ok()?;
    if let Some(mut config) = get_sync_config(ws_id) {
        config["repoId"] = json!(repo.id);
        save_sync_config(ws_id, Some(config));
    }
    Some(repo.id)
}

/// Ask the console whose the workspace is. Nothing is asked about a workspace with no account or no
/// repository: it is nobody's but its user's.
pub async fn refresh(ws_id: &str) {
    if !console::can_ask() {
        return;
    }
    let Some(account) = workspace_account(ws_id) else { return };
    let Some(config) = get_sync_config(ws_id).filter(|config| !str_of(config, "repo").is_empty()) else { return };
    if let Some(repo_id) = repo_id(ws_id, &account, &config).await {
        refresh_for(ws_id, &account, repo_id).await;
    }
}
