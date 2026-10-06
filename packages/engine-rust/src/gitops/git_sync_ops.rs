//! The git operations behind connecting to, cloning from and pulling from a remote.

use std::path::PathBuf;

use super::remote::git_net;
use crate::workspace::{
    git_store::{GitError, RemoteAuth, git, git_net_in},
    tree,
};

/// A directory git can run in without a repository (remote queries and clones).
fn neutral_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir())
}

/// The commit a remote branch points at, or `None` if it cannot be read or the branch does not exist.
pub async fn fetch_remote_head(remote: &str, branch: &str, auth: Option<&RemoteAuth>) -> Option<String> {
    let out =
        git_net_in(&neutral_dir(), &["ls-remote", remote, &format!("refs/heads/{branch}")], auth).await.ok()?;
    out.split_whitespace().next().filter(|sha| !sha.is_empty()).map(str::to_string)
}

/// What a pull brought in.
pub struct PullOutcome {
    pub updated: bool,
    pub changed_paths: Vec<String>,
}

/// Fetch `origin/<branch>` and merge it, setting local changes aside (stashed, then restored on top) around
/// the merge. A merge that fails is aborted and reported as `Pull merge failed`.
pub async fn perform_git_pull(ws_id: &str, branch: &str) -> Result<PullOutcome, GitError> {
    let outcome = pull(ws_id, branch).await;
    // Files of the trees came and went without their indexes being told.
    tree::mark_stale(ws_id);
    outcome
}

async fn pull(ws_id: &str, branch: &str) -> Result<PullOutcome, GitError> {
    git_net(ws_id, &["fetch", "origin", branch]).await?;

    let remote_ref = format!("origin/{branch}");
    let (Ok(local_head), Ok(remote_head)) =
        (git(ws_id, &["rev-parse", "HEAD"]).await, git(ws_id, &["rev-parse", &remote_ref]).await)
    else {
        return Ok(PullOutcome { updated: false, changed_paths: Vec::new() });
    };
    let (local_head, remote_head) = (local_head.trim().to_string(), remote_head.trim().to_string());
    if local_head == remote_head {
        return Ok(PullOutcome { updated: false, changed_paths: Vec::new() });
    }

    let changed_paths = match git(ws_id, &["diff", "--name-only", &local_head, &remote_ref]).await {
        Ok(raw) => raw.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
        Err(_) => Vec::new(),
    };

    let mut stashed = false;
    if let Ok(status) = git(ws_id, &["status", "--porcelain", "--untracked-files=all"]).await {
        if !status.trim().is_empty()
            && git(ws_id, &["stash", "push", "--include-untracked", "-m", "auto-stash before pull"]).await.is_ok()
        {
            stashed = true;
        }
    }

    if git(ws_id, &["merge", &remote_ref, "--no-edit"]).await.is_err() {
        let _ = git(ws_id, &["merge", "--abort"]).await;
        if stashed {
            let _ = git(ws_id, &["stash", "pop"]).await;
        }
        return Err(GitError("Pull merge failed".to_string()));
    }

    if stashed && git(ws_id, &["stash", "pop"]).await.is_err() {
        // The remote changed a file that was also changed here. What was changed here stays on top (in the
        // pop, "theirs" is the stash), as a difference from the new `HEAD` for the user to sync or revert.
        // Only when the pop did apply and left conflicts: a pop that applied nothing must keep its stash.
        let conflicted = git(ws_id, &["ls-files", "--unmerged"]).await.is_ok_and(|out| !out.trim().is_empty());
        if !conflicted || git(ws_id, &["checkout", "--theirs", "--", "."]).await.is_err() {
            return Err(GitError(
                "The remote's changes were pulled, but the changes made here could not be put back on top of them. \
                 They are kept in the repository's stash."
                    .to_string(),
            ));
        }
        let _ = git(ws_id, &["reset", "--quiet"]).await;
        let _ = git(ws_id, &["stash", "drop"]).await;
    }

    Ok(PullOutcome { updated: true, changed_paths })
}
