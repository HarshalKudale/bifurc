//! The one question the engine asks the Bifurc Console: has this GitHub account been given this workspace
//! by the organization it belongs to?
//!
//! GitHub says whether an account can reach a repository. The console says whether it was meant to: someone
//! made a collaborator on GitHub by hand, outside the console, is on nobody's seat. The question carries two
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

/// Ask the console. An error means there was no answer (the console could not be reached, or said something
/// that is neither yes nor no), which is not a yes.
pub async fn check(github_user_id: u64, repo_id: u64) -> Result<Grant, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let unreachable = |detail: String| format!("The Bifurc Console could not be reached: {detail}");
    let response = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("{}/api/v1/access/check", url()))
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .body(json!({ "githubUserId": github_user_id, "repoId": repo_id }).to_string())
        .send()
        .await
        .map_err(|e| unreachable(e.to_string()))?;
    let status = response.status().as_u16();
    let text = response.text().await.map_err(|e| unreachable(e.to_string()))?;
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
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
