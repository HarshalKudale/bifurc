//! Serialising git work per workspace, and getting past a leftover `.git/index.lock`.
//!
//! Every git command that writes takes `.git/index.lock`. A process killed mid-write leaves it behind and
//! from then on every `git add` fails while `git status` keeps working, so a publish would fail for a reason
//! nobody can see. A lock held by a live process clears in milliseconds, so a write is retried a few times
//! first; if the lock is still there it is stale, and it is removed (the index can be rebuilt, a permanently
//! broken Publish cannot be recovered without a terminal). The path to remove comes from git's own message
//! and is re-validated: it must be named `index.lock` and sit inside this workspace's `.git`.

use std::{collections::HashMap, path::Path, sync::Arc, time::Duration};

use parking_lot::Mutex;
use regress::Regex;

use crate::workspace::{
    fs::ws_dir,
    git_store::{GitError, git},
};

/// Attempts before the lock is treated as stale, spaced 80/160/240/320 ms.
const ATTEMPTS: u64 = 5;

static LOCKS: Mutex<Option<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = Mutex::new(None);

/// The workspace's operation lock: hold it across a git operation that must not interleave with another.
pub fn workspace_lock(ws_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    LOCKS.lock().get_or_insert_with(HashMap::new).entry(ws_id.to_string()).or_default().clone()
}

fn mentions_index_lock(message: &str) -> bool {
    message.to_lowercase().contains("index.lock")
}

/// Run a git write in the workspace, retrying while another process holds the index. Only `index.lock`
/// failures are retried.
pub async fn with_index_lock_retry(ws_id: &str, args: &[&str]) -> Result<String, GitError> {
    let mut attempt = 1;
    loop {
        match git(ws_id, args).await {
            Ok(value) => return Ok(value),
            Err(err) if !mentions_index_lock(&err.0) => return Err(err),
            Err(err) => {
                if attempt < ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(80 * attempt)).await;
                    attempt += 1;
                    continue;
                }
                remove_stale_index_lock(ws_id, &err.0);
                return git(ws_id, args).await;
            }
        }
    }
}

/// Remove the lock git said it could not create, only if it really is this workspace's index lock.
pub fn remove_stale_index_lock(ws_id: &str, git_message: &str) -> bool {
    let Ok(pattern) = Regex::with_flags(r"Unable to create '([^']+)'", "i") else { return false };
    let Some(claimed) = pattern.find(git_message).and_then(|m| m.group(1)).map(|range| &git_message[range]) else {
        return false;
    };
    let lock = Path::new(claimed);
    let git_dir = ws_dir(ws_id).join(".git");
    if lock.file_name().and_then(|n| n.to_str()) != Some("index.lock") {
        return false;
    }
    if lock.components().any(|c| matches!(c, std::path::Component::ParentDir)) || !lock.starts_with(&git_dir) {
        return false;
    }
    std::fs::remove_file(lock).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch_data_root;

    #[test]
    fn only_this_workspaces_index_lock_is_removed() {
        let root = scratch_data_root();
        let git_dir = ws_dir(root.ws_id).join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        let lock = git_dir.join("index.lock");
        std::fs::write(&lock, "").unwrap();
        let other = ws_dir(root.ws_id).join("other.lock");
        std::fs::write(&other, "").unwrap();

        let message = |p: &Path| format!("fatal: Unable to create '{}': File exists.", p.display());
        assert!(!remove_stale_index_lock(root.ws_id, &message(&other)));
        assert!(other.exists());
        assert!(!remove_stale_index_lock(root.ws_id, "fatal: something else"));
        assert!(remove_stale_index_lock(root.ws_id, &message(&lock)));
        assert!(!lock.exists());
    }

    #[tokio::test]
    async fn other_errors_are_not_retried() {
        let _root = scratch_data_root();
        let result = with_index_lock_retry("ws1", &["no-such-subcommand"]).await;
        assert!(result.is_err());
    }
}
