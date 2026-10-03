//! Where the workspace stands relative to its remote: the second axis of "is this entity synced?".
//!
//! Measured against `refs/remotes/origin/<branch>`, the last remote state this clone knows about, and never
//! over the network: this runs on every status refresh. So `ahead` can be stale after someone else pushes;
//! the pull path is the remedy. A repository with no `origin` is never ahead.

use serde_json::{Value, json};

use crate::workspace::git_store::git;

fn no_git_state() -> Value {
    json!({ "hasRemote": false, "branch": null, "ahead": false, "behind": false })
}

/// `{ hasRemote, branch, ahead, behind }`. Never fails: a missing or broken repository reports no remote.
pub async fn get_workspace_git_state(ws_id: &str) -> Value {
    let Ok(remotes) = git(ws_id, &["remote"]).await else { return no_git_state() };
    if !remotes.lines().any(|r| r.trim() == "origin") {
        return no_git_state();
    }

    // `--abbrev-ref HEAD` prints the literal "HEAD" when detached, which is not a branch name; an unborn
    // branch fails outright.
    let branch = match git(ws_id, &["rev-parse", "--abbrev-ref", "HEAD"]).await {
        Ok(head) if !head.trim().is_empty() && head.trim() != "HEAD" => head.trim().to_string(),
        _ => return json!({ "hasRemote": true, "branch": null, "ahead": false, "behind": false }),
    };

    // Tested by output, not exit code: `rev-parse --verify --quiet` of a missing ref can exit 0 with
    // nothing printed.
    let remote_ref = git(ws_id, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")])
        .await
        .unwrap_or_default();
    if remote_ref.trim().is_empty() {
        return json!({ "hasRemote": true, "branch": branch, "ahead": true, "behind": false });
    }

    // `A...B --left-right` prints "<only in A>\t<only in B>"; A is origin, B is HEAD.
    let range = format!("origin/{branch}...HEAD");
    let Ok(counts) = git(ws_id, &["rev-list", "--count", "--left-right", &range]).await else {
        return no_git_state();
    };
    let mut parts = counts.split_whitespace().map(|n| n.parse::<u64>().unwrap_or(0));
    let (behind, ahead) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    json!({ "hasRemote": true, "branch": branch, "ahead": ahead > 0, "behind": behind > 0 })
}
