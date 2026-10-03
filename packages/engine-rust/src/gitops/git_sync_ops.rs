//! The git operations behind connecting to, cloning from and pulling from a remote.

use std::path::{Path, PathBuf};

use crate::workspace::git_store::{GitError, git, git_in};

/// A directory git can run in without a repository (remote queries and clones).
fn neutral_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir())
}

/// Whether the remote has no refs at all. An unreachable remote is an error.
pub async fn is_remote_empty(remote: &str) -> Result<bool, String> {
    match git_in(&neutral_dir(), &["ls-remote", "--refs", remote]).await {
        Ok(out) => Ok(out.trim().is_empty()),
        Err(e) => Err(format!("Cannot access remote: {e}")),
    }
}

/// The commit a remote branch points at, or `None` if it cannot be read or the branch does not exist.
pub async fn fetch_remote_head(remote: &str, branch: &str) -> Option<String> {
    let out = git_in(&neutral_dir(), &["ls-remote", remote, &format!("refs/heads/{branch}")]).await.ok()?;
    out.split_whitespace().next().filter(|sha| !sha.is_empty()).map(str::to_string)
}

/// Clone `branch` of `remote` into `temp_dir` (a directory that does not exist yet).
pub async fn clone_branch(remote: &str, branch: &str, temp_dir: &Path) -> Result<(), GitError> {
    let target = temp_dir.to_string_lossy();
    git_in(&neutral_dir(), &["clone", "--branch", branch, "--single-branch", remote, &target]).await.map(|_| ())
}

/// Give a freshly cloned workspace the committer identity it commits with.
pub async fn set_local_identity(dir: &Path) -> Result<(), GitError> {
    git_in(dir, &["config", "--local", "user.email", "bifurc@local"]).await?;
    git_in(dir, &["config", "--local", "user.name", "Bifurc"]).await?;
    Ok(())
}

/// What a pull brought in.
pub struct PullOutcome {
    pub updated: bool,
    pub changed_paths: Vec<String>,
}

/// Fetch `origin/<branch>` and merge it, setting local changes aside (stashed, then restored) around the
/// merge. A merge that fails is aborted and reported as `Pull merge failed`.
pub async fn perform_git_pull(ws_id: &str, branch: &str) -> Result<PullOutcome, GitError> {
    git(ws_id, &["fetch", "origin", branch]).await?;

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
        if git(ws_id, &["checkout", "--ours", "."]).await.is_ok() {
            let _ = git(ws_id, &["stash", "drop"]).await;
        }
    }

    Ok(PullOutcome { updated: true, changed_paths })
}
