//! Publishing: `git add` -> commit -> push for one or more files or folders.
//!
//! The result's `commit` is the proof that something happened: `{ ok: true, commit: "<sha>" }` is a commit,
//! `{ ok: true, commit: null }` means nothing was staged because every named path is already committed, and
//! `{ ok: false, error }` means a step failed (a failure at the push step leaves the commit behind).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::{
    git_lock::{with_index_lock_retry, workspace_lock},
    git_sync_ops::perform_git_pull,
    remote::{ensure_owner, git_net},
    status_tracker::{parse_porcelain_z, unquote_git_path},
    sync_state::get_sync_config,
};
use crate::{
    runtime::instances::notify_config_changed,
    utils::json::{str_of, truthy},
    workspace::{
        accounts::commit_identity,
        fs::ws_dir,
        git_store::{GitError, git},
    },
};

pub struct PublishOptions {
    pub ws_id: String,
    /// Workspace-relative paths (files, or directories ending in a separator) to stage and commit.
    pub paths: Vec<String>,
    /// Overrides the generated commit message.
    pub message: Option<String>,
}

/// The entity's display name read from its file: `name`, else `label`, else `domain`, else `urlPattern`.
fn read_entity_name(ws_id: &str, rel_path: &str) -> Option<String> {
    let parsed = crate::utils::json::read_json(&ws_dir(ws_id).join(rel_path))?;
    ["name", "label", "domain", "urlPattern"]
        .iter()
        .find(|key| truthy(&parsed, key))
        .map(|key| str_of(&parsed, key).to_string())
}

fn entity_kind_from_path(rel_path: &str) -> String {
    rel_path.split(['/', '\\']).next().filter(|k| !k.is_empty()).unwrap_or("entity").to_string()
}

pub(super) fn singular_kind(kind: &str) -> String {
    match kind {
        "requests" => "request",
        "mocks" => "mock",
        "sockets" => "socket",
        "mappings" => "mapping",
        "rules" => "rule",
        "environments" => "environment",
        other => other,
    }
    .to_string()
}

fn entity_id_from_path(rel_path: &str) -> Option<String> {
    let base = rel_path.rsplit(['/', '\\']).next()?;
    base.strip_suffix(".json").map(str::to_string)
}

/// What is staged in the index, by category.
#[derive(Debug, Default)]
struct StagedStatus {
    staged: Vec<String>,
    created: Vec<String>,
    deleted: Vec<String>,
    renamed: Vec<String>,
}

impl StagedStatus {
    fn has_changes(&self) -> bool {
        !(self.staged.is_empty() && self.created.is_empty() && self.deleted.is_empty() && self.renamed.is_empty())
    }
}

async fn staged_status(ws_id: &str) -> Result<StagedStatus, GitError> {
    let raw = git(ws_id, &["status", "--porcelain", "-z", "-uno"]).await?;
    let mut out = StagedStatus::default();
    for (xy, path) in parse_porcelain_z(&raw) {
        let mut letters = xy.chars();
        let (index, work) = (letters.next().unwrap_or(' '), letters.next().unwrap_or(' '));
        if "MARCT".contains(index) {
            out.staged.push(path.clone());
        }
        if index == 'A' {
            out.created.push(path.clone());
        }
        if index == 'D' || work == 'D' {
            out.deleted.push(path.clone());
        }
        if index == 'R' {
            out.renamed.push(path);
        }
    }
    Ok(out)
}

/// The requested paths git will not stage, with the reason for each. Empty when every path is tracked (so
/// "nothing staged" really means "unchanged"). Folder pathspecs are skipped: an empty folder is a no-op.
async fn find_unstageable_paths(ws_id: &str, paths: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    for raw in paths {
        if raw.ends_with('/') || raw.ends_with('\\') {
            continue;
        }
        let rel = raw.replace('\\', "/");
        let tracked =
            git(ws_id, &["ls-files", "--error-unmatch", "--", &rel]).await.is_ok_and(|out| !out.trim().is_empty());
        if tracked {
            continue;
        }
        if ws_dir(ws_id).join(&rel).exists() {
            problems.push(format!(
                "{rel} exists but git will not stage it — check for a stale .git/index.lock (a killed git process leaves one), or a .gitignore rule"
            ));
        } else {
            problems.push(format!("{rel} does not exist in this workspace"));
        }
    }
    problems
}

/// `create` | `update` | `delete` for one staged entity path, from its status before staging.
fn commit_action(pre_xy: &str, status: &StagedStatus, path: &str) -> &'static str {
    let mut letters = pre_xy.chars();
    let (x, y) = (letters.next(), letters.next());
    let is_deleted = status.deleted.iter().any(|d| d == path) || x == Some('D') || y == Some('D');
    let is_new = pre_xy == "??" || pre_xy == "A " || pre_xy == "AM";
    if is_deleted {
        "delete"
    } else if is_new {
        "create"
    } else {
        "update"
    }
}

/// The commit message (subject, optional body) for a publish with no explicit message.
fn build_message(
    opts: &PublishOptions,
    status: &StagedStatus,
    pre_status: &std::collections::HashMap<String, String>,
    actor: &str,
) -> (String, Option<String>) {
    let ws_id = &opts.ws_id;
    if opts.paths.len() != 1 {
        return (format!("update {} files", opts.paths.len()), Some(format!("workspace-id: {ws_id}\nactor: {actor}")));
    }
    let path = &opts.paths[0];
    let single_entity = |path: &str, name_of_deleted: bool| {
        let kind = singular_kind(&entity_kind_from_path(path));
        let entity_id = entity_id_from_path(path);
        let action = commit_action(pre_status.get(path).map(String::as_str).unwrap_or(""), status, path);
        let id_or_unknown = entity_id.clone().unwrap_or_else(|| "unknown".to_string());
        let name = if action == "delete" && !name_of_deleted {
            id_or_unknown
        } else {
            read_entity_name(ws_id, path).unwrap_or(id_or_unknown)
        };
        let body = entity_id.map(|id| format!("entity-id: {id}\nworkspace-id: {ws_id}\nactor: {actor}"));
        (format!("{action} {kind} {name}"), body)
    };

    if !(path.ends_with('/') || path.ends_with('\\')) {
        return single_entity(path, true);
    }

    // A folder publish: every staged entity file, once each.
    let mut seen = HashSet::new();
    let staged_entities: Vec<&String> = status
        .staged
        .iter()
        .chain(&status.created)
        .chain(&status.deleted)
        .filter(|f| seen.insert((*f).clone()))
        .filter(|f| {
            f.ends_with(".json") && !["index.json", "enabled.json", "names.json"].iter().any(|s| f.ends_with(s))
        })
        .collect();

    if staged_entities.len() == 1 {
        return single_entity(staged_entities[0], false);
    }

    let trimmed = path.trim_end_matches(['/', '\\']);
    let mut parts = trimmed.split(['/', '\\']);
    let kind_part = parts.next().filter(|k| !k.is_empty()).unwrap_or("folder");
    let folder_part = parts.next();
    let subject_kind = singular_kind(kind_part);
    let subject = match folder_part {
        Some(folder) => format!("update {subject_kind} folder \"{folder}\""),
        None => format!("update {subject_kind} changes"),
    };
    let ids: Vec<String> =
        staged_entities.iter().filter_map(|f| entity_id_from_path(f)).filter(|id| !id.is_empty()).collect();
    let body =
        (!ids.is_empty()).then(|| format!("workspace-id: {ws_id}\nactor: {actor}\nentity-ids: {}", ids.join(",")));
    (subject, body)
}

/// Stage `opts.paths`, commit them and push when a remote is configured.
pub async fn publish_entities(opts: PublishOptions) -> Value {
    let ws_id = opts.ws_id.clone();
    let _guard = workspace_lock(&ws_id).lock_owned().await;
    match publish_locked(&opts).await {
        Ok(result) => result,
        Err(e) => json!({ "ok": false, "error": e.0 }),
    }
}

async fn publish_locked(opts: &PublishOptions) -> Result<Value, GitError> {
    let ws_id = opts.ws_id.as_str();
    let actor = commit_identity(ws_id).actor;

    // The commit goes on top of what the remote has now, so the push that follows is never behind it. A
    // remote that cannot be reached is met again at the push.
    let remote = get_sync_config(ws_id).filter(|c| truthy(c, "remote"));
    if let Some(config) = &remote {
        ensure_owner(ws_id).await?;
        if perform_git_pull(ws_id, str_of(config, "branch")).await.is_ok_and(|pulled| pulled.updated) {
            notify_config_changed();
        }
    }

    // The status before staging says which files are new, modified or deleted. Not `-z`, so keys are unquoted.
    let pre_raw = git(ws_id, &["status", "--porcelain", "-uall"]).await.unwrap_or_default();
    let mut pre_status = std::collections::HashMap::new();
    for line in pre_raw.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        let xy: String = line.chars().take(2).collect();
        let path: String = line.chars().skip(3).collect();
        pre_status.insert(unquote_git_path(path.trim()), xy);
    }

    // Failures are collected, not ignored: "git refused to stage this" must not look like "nothing to stage".
    let mut stage_failures = Vec::new();
    for path in &opts.paths {
        let result = with_index_lock_retry(ws_id, &["add", "-A", "--", path.as_str()]).await;
        if let Err(e) = result {
            stage_failures.push(format!("{path}: {}", e.0.lines().next().unwrap_or("")));
        }
    }

    let status = staged_status(ws_id).await?;
    if !status.has_changes() {
        let mut problems = stage_failures;
        problems.extend(find_unstageable_paths(ws_id, &opts.paths).await);
        if !problems.is_empty() {
            return Ok(json!({ "ok": false, "error": format!("git staged nothing — {}", problems.join("; ")) }));
        }
        return Ok(json!({ "ok": true, "commit": null }));
    }

    let (message, body) = match opts.message.clone().filter(|m| !m.is_empty()) {
        Some(message) => (message, None),
        None => build_message(opts, &status, &pre_status, &actor),
    };
    let full_message = match body {
        Some(body) => format!("{message}\n\n{body}"),
        None => message,
    };
    with_index_lock_retry(ws_id, &["commit", "-m", full_message.as_str()]).await?;
    let commit = git(ws_id, &["rev-parse", "HEAD"]).await?.trim().to_string();

    if let Some(config) = remote {
        git_net(ws_id, &["push", "origin", &format!("HEAD:{}", str_of(&config, "branch"))]).await?;
    }

    Ok(json!({ "ok": true, "commit": if commit.is_empty() { Value::Null } else { json!(commit) } }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_named_in_the_singular() {
        assert_eq!(singular_kind("requests"), "request");
        assert_eq!(singular_kind("graphqlRequests"), "graphqlRequests");
        assert_eq!(entity_kind_from_path("mocks/f/m1.json"), "mocks");
        assert_eq!(entity_id_from_path("mocks/f/m1.json").as_deref(), Some("m1"));
    }

    #[test]
    fn commit_action_follows_the_status_before_staging() {
        let status = StagedStatus::default();
        assert_eq!(commit_action("??", &status, "a.json"), "create");
        assert_eq!(commit_action(" M", &status, "a.json"), "update");
        assert_eq!(commit_action(" D", &status, "a.json"), "delete");
        assert_eq!(commit_action("", &status, "a.json"), "update");
    }
}
