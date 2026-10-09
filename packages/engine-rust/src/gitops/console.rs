//! The two questions the engine asks the Bifurc Console. Has this GitHub account been given this workspace
//! by the organization it belongs to ([`check`])? And is the workspace an organization's at all, with usage
//! analytics that organization turned on ([`policy`])?
//!
//! GitHub says whether an account can reach a repository. The console says whether it was meant to: someone
//! made a collaborator on GitHub by hand, outside the console, is on nobody's seat. Each question carries two
//! numbers, the account's GitHub id and the repository's, and nothing else: the GitHub token is for GitHub
//! alone and is never sent here.
//!
//! The console is at `BIFURC_CONSOLE_URL` (when the engine runs, else when it was built), else at
//! `https://console.bifurc.app`.

use std::time::Duration;

use serde_json::{Value, json};

use crate::utils::json::str_of;

const DEFAULT_URL: &str = "https://console.bifurc.app";
const TIMEOUT: Duration = Duration::from_secs(15);
const USER_AGENT: &str = concat!("bifurc-engine/", env!("CARGO_PKG_VERSION"));

#[cfg(test)]
static TEST_URL: parking_lot::Mutex<Option<String>> = parking_lot::Mutex::new(None);

/// Point the engine at a stand-in for the console. Tests take turns through the scratch data root.
#[cfg(test)]
pub fn set_test_url(url: Option<String>) {
    *TEST_URL.lock() = url;
}

/// Where the console is, without a trailing slash.
pub fn url() -> String {
    #[cfg(test)]
    if let Some(url) = TEST_URL.lock().clone() {
        return url;
    }
    std::env::var("BIFURC_CONSOLE_URL")
        .ok()
        .or_else(|| option_env!("BIFURC_CONSOLE_URL").map(str::to_string))
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_URL.to_string())
        .trim()
        .trim_end_matches('/')
        .to_string()
}

/// Why the console said no, for a client to tell apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denial {
    /// The repository is not a workspace of any organization.
    Unmanaged,
    /// The organization has not given the account this workspace.
    NotGranted,
    /// The account was given it, but the organization has no seat for the account.
    SeatInactive,
}

impl Denial {
    pub fn as_str(self) -> &'static str {
        match self {
            Denial::Unmanaged => "unmanaged",
            Denial::NotGranted => "notGranted",
            Denial::SeatInactive => "seatInactive",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grant {
    /// The account was given the workspace: to change it (`write`), or only to read it.
    Allowed { write: bool },
    Denied { denial: Denial, message: String },
}

fn unreachable(detail: String) -> String {
    format!("The Bifurc Console could not be reached: {detail}")
}

/// Put the two numbers to the console at `path`. Returns the status and what it answered.
async fn ask(path: &str, github_user_id: u64, repo_id: u64) -> Result<(u16, Value), String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let response = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("{}{path}", url()))
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .body(json!({ "githubUserId": github_user_id, "repoId": repo_id }).to_string())
        .send()
        .await
        .map_err(|e| unreachable(e.to_string()))?;
    let status = response.status().as_u16();
    let text = response.text().await.map_err(|e| unreachable(e.to_string()))?;
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

/// Ask the console. An error means there was no answer (the console could not be reached, or said something
/// that is neither yes nor no), which is not a yes.
pub async fn check(github_user_id: u64, repo_id: u64) -> Result<Grant, String> {
    let (status, body) = ask("/api/v1/access/check", github_user_id, repo_id).await?;
    let Some(allowed) = body.get("allowed").and_then(Value::as_bool).filter(|_| status == 200) else {
        return Err(unreachable(format!("it answered {status}")));
    };
    if allowed {
        return Ok(Grant::Allowed { write: str_of(&body, "permission") != "read" });
    }
    let denial = match str_of(&body, "reason") {
        "unmanaged" => Denial::Unmanaged,
        "seatInactive" => Denial::SeatInactive,
        _ => Denial::NotGranted,
    };
    let message = match str_of(&body, "message") {
        "" => "The organization this workspace belongs to has not given your GitHub account access to it.".to_string(),
        message => message.to_string(),
    };
    Ok(Grant::Denied { denial, message })
}

/// What the console says of a workspace and the organization it may belong to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Policy {
    /// No organization has the repository, or none that gave it to the account.
    Unmanaged,
    Managed {
        /// What a usage event names the organization by.
        public_id: String,
        name: String,
        /// The console's id of the workspace: what a usage event names it by.
        workspace_id: String,
        /// Whether the organization wants what is done in its workspaces reported.
        analytics: bool,
    },
}

/// Whether there is a console to ask. In a test there is one only when the test set one up, so that no test
/// reaches the real console.
pub fn can_ask() -> bool {
    #[cfg(test)]
    return TEST_URL.lock().is_some();
    #[cfg(not(test))]
    true
}

/// Ask the console whose the workspace is. An error means there was no answer, which says nothing either
/// way: what was known before stands.
pub async fn policy(github_user_id: u64, repo_id: u64) -> Result<Policy, String> {
    if !can_ask() {
        return Err(unreachable("no console to ask".to_string()));
    }
    let (status, body) = ask("/api/v1/workspaces/policy", github_user_id, repo_id).await?;
    let Some(managed) = body.get("managed").and_then(Value::as_bool).filter(|_| status == 200) else {
        return Err(unreachable(format!("it answered {status}")));
    };
    if !managed {
        return Ok(Policy::Unmanaged);
    }
    let organization = body.get("organization").unwrap_or(&Value::Null);
    let (public_id, workspace_id) = (str_of(organization, "publicId"), str_of(body.get("workspace").unwrap_or(&Value::Null), "id"));
    if public_id.is_empty() || workspace_id.is_empty() {
        return Err(unreachable("it answered without saying which organization".to_string()));
    }
    Ok(Policy::Managed {
        public_id: public_id.to_string(),
        name: str_of(organization, "name").to_string(),
        workspace_id: workspace_id.to_string(),
        analytics: body.get("analytics").and_then(|analytics| analytics.get("enabled")).and_then(Value::as_bool) == Some(true),
    })
}
