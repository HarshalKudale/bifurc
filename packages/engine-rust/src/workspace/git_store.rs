//! Every workspace is a git repository; every saved change can become a commit, and the commit log is the
//! audit trail. Git is driven through its command line, as a user would.

use std::path::Path;

use regress::Regex;
use serde_json::{Value, json};

use super::fs::ws_dir;
use crate::utils::process::hidden_command;

/// Variables that would make git open an editor or a pager, which a background engine can never answer.
const STRIPPED_GIT_VARS: [&str; 7] =
    ["EDITOR", "VISUAL", "GIT_EDITOR", "GIT_SEQUENCE_EDITOR", "PAGER", "GIT_PAGER", "GIT_EXTERNAL_DIFF"];

/// What a failed git invocation said.
#[derive(Debug, Clone)]
pub struct GitError(pub String);

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<GitError> for crate::protocol::EngineError {
    fn from(err: GitError) -> Self {
        crate::protocol::EngineError::engine(err.0)
    }
}

pub fn device_name() -> String {
    let name = gethostname::gethostname().to_string_lossy().into_owned();
    if name.is_empty() { "unknown".to_string() } else { name }
}

/// Run `git <args>` in `dir`; stdout on success, stderr (or stdout) as the error otherwise.
pub async fn git_in(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    let mut command = hidden_command("git");
    command.args(args).current_dir(dir).env("GIT_OPTIONAL_LOCKS", "0").stdin(std::process::Stdio::null());
    for name in STRIPPED_GIT_VARS {
        command.env_remove(name);
    }
    let output = command.output().await.map_err(|e| GitError(format!("could not run git: {e}")))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(GitError(if stderr.is_empty() { String::from_utf8_lossy(&output.stdout).trim().to_string() } else { stderr }))
}

/// Run `git <args>` in a workspace's repository.
pub async fn git(ws_id: &str, args: &[&str]) -> Result<String, GitError> {
    git_in(&ws_dir(ws_id), args).await
}

pub async fn check_git_installed() -> bool {
    git_in(&std::env::current_dir().unwrap_or_default(), &["--version"]).await.is_ok()
}

pub fn is_repo(ws_id: &str) -> bool {
    ws_dir(ws_id).join(".git").exists()
}

/// Make the workspace directory a repository (first commit: `.gitignore` and `workspace.json`), and make
/// sure it has a committer identity.
pub async fn init_workspace_repo(ws_id: &str) -> Result<(), GitError> {
    let dir = ws_dir(ws_id);
    let name = device_name();
    let email = format!("{name}@bifurc");
    if !dir.join(".git").exists() {
        git_in(&dir, &["init"]).await?;
        git_in(&dir, &["config", "--local", "user.email", &email]).await?;
        git_in(&dir, &["config", "--local", "user.name", &name]).await?;
        if dir.join(".gitignore").exists() {
            git_in(&dir, &["add", ".gitignore"]).await?;
            if dir.join("workspace.json").exists() {
                git_in(&dir, &["add", "workspace.json"]).await?;
            }
            git_in(&dir, &["commit", "-m", "chore: init workspace repo"]).await?;
        }
    } else {
        let _ = git_in(&dir, &["config", "--local", "user.email", &email]).await;
        let _ = git_in(&dir, &["config", "--local", "user.name", &name]).await;
    }
    Ok(())
}

pub struct CommitMutation<'a> {
    /// `create` | `update` | `delete`
    pub action: &'a str,
    /// `mock` | `mapping` | `rule` | `environment` | `request` | `wsConnection` | `folder` | `workspace`
    pub entity: &'a str,
    pub entity_id: &'a str,
    pub entity_name: &'a str,
    pub workspace_id: &'a str,
    pub rel_path: &'a str,
    pub actor: Option<&'a str>,
    pub changed_fields: &'a [String],
}

/// Stage one path and commit it with an audit message. Returns the commit's short hash, or `""` when there
/// was nothing to commit.
pub async fn commit_mutation(opts: CommitMutation<'_>) -> Result<String, GitError> {
    let actor = opts.actor.map(str::to_string).unwrap_or_else(device_name);
    let fields =
        if opts.changed_fields.is_empty() { String::new() } else { format!(" [{}]", opts.changed_fields.join(",")) };
    let message = format!(
        "{} {}{} {}\n\nentity-id: {}\nworkspace-id: {}\nactor: {}",
        opts.action, opts.entity, fields, opts.entity_name, opts.entity_id, opts.workspace_id, actor
    );
    let ws = opts.workspace_id;
    if opts.action == "delete" {
        let _ = git(ws, &["rm", "--cached", "--ignore-unmatch", opts.rel_path]).await;
    } else {
        let _ = git(ws, &["add", opts.rel_path]).await;
    }
    if git(ws, &["diff", "--cached", "--name-only"]).await?.trim().is_empty() {
        return Ok(String::new());
    }
    git(ws, &["commit", "-m", &message]).await?;
    Ok(git(ws, &["rev-parse", "--short", "HEAD"]).await?.trim().to_string())
}

#[derive(Debug, Clone, Default)]
pub struct QueryLogOptions {
    pub workspace_id: String,
    pub entity: Option<String>,
    pub action: Option<String>,
    pub entity_id: Option<String>,
    pub file_path: Option<String>,
    pub from_ts: Option<i64>,
    pub to_ts: Option<i64>,
    pub search: Option<String>,
    /// `Some(0)` means no limit; `None` means 200.
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

fn body_field(body: &str, name: &str) -> Option<String> {
    body.lines().find_map(|line| line.strip_prefix(&format!("{name}: ")).map(|v| v.trim().to_string()))
}

/// One audit entry from a commit's hash, timestamp, subject and body; `None` for a commit that is not an
/// entity mutation.
fn parse_audit_entry(block: &str) -> Option<Value> {
    let mut lines = block.trim().split('\n');
    let (hash, ts, subject) = (lines.next()?, lines.next()?, lines.next()?);
    if hash.is_empty() || ts.is_empty() || subject.is_empty() {
        return None;
    }
    let body = lines.collect::<Vec<_>>().join("\n");

    let current = Regex::new(r"^(create|update|delete) (\w+)(?: \[([^\]]*)\])? (.+)$").ok()?;
    let legacy = Regex::new(r"^(create|update|delete)\((\w+)\): (.+)$").ok()?;
    let group = |m: &regress::Match, i: usize| m.group(i).map(|range| subject[range].to_string());

    let (action, entity, name, changed_fields) = if let Some(m) = current.find(subject) {
        let fields = group(&m, 3)
            .filter(|f| !f.is_empty())
            .map(|f| f.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect::<Vec<_>>());
        (group(&m, 1)?, group(&m, 2)?, group(&m, 4)?.trim().to_string(), fields)
    } else if let Some(m) = legacy.find(subject) {
        let name = group(&m, 3)?;
        (group(&m, 1)?, group(&m, 2)?, name.split(" — ").next().unwrap_or("").trim().to_string(), None)
    } else {
        return None;
    };

    let mut entry = json!({
        "commitHash": hash.trim(),
        "ts": ts.trim().parse::<i64>().unwrap_or(0) * 1000,
        "action": action,
        "entity": entity,
        "entityName": name,
        "entityId": body_field(&body, "entity-id").unwrap_or_default(),
        "workspaceId": body_field(&body, "workspace-id").unwrap_or_default(),
        "actor": body_field(&body, "actor").unwrap_or_else(|| "local".to_string()),
    });
    if let Some(fields) = changed_fields {
        entry["changedFields"] = json!(fields);
    }
    Some(entry)
}

/// The audit log, newest first: `{ entries, total }` after filtering, with `entries` paged.
pub async fn query_log(opts: &QueryLogOptions) -> (Vec<Value>, usize) {
    if !is_repo(&opts.workspace_id) {
        return (Vec::new(), 0);
    }
    let mut args = vec!["log".to_string(), "--format=%H%n%at%n%s%n%b%n---END---".to_string()];
    if let Some(path) = opts.file_path.as_deref().filter(|p| !p.is_empty()) {
        args.push("--".into());
        args.push(path.replace('\\', "/"));
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let Ok(raw) = git(&opts.workspace_id, &arg_refs).await else { return (Vec::new(), 0) };

    let text = |entry: &Value, key: &str| entry.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let ts = |entry: &Value| entry.get("ts").and_then(Value::as_i64).unwrap_or(0);
    let entries: Vec<Value> = raw
        .split("---END---\n")
        .filter(|block| !block.trim().is_empty())
        .filter_map(parse_audit_entry)
        .filter(|e| opts.entity.as_ref().is_none_or(|want| &text(e, "entity") == want))
        .filter(|e| opts.action.as_ref().is_none_or(|want| &text(e, "action") == want))
        .filter(|e| opts.entity_id.as_ref().is_none_or(|want| &text(e, "entityId") == want))
        .filter(|e| opts.from_ts.filter(|t| *t != 0).is_none_or(|from| ts(e) >= from))
        .filter(|e| opts.to_ts.filter(|t| *t != 0).is_none_or(|to| ts(e) <= to))
        .filter(|e| {
            opts.search
                .as_ref()
                .filter(|s| !s.is_empty())
                .is_none_or(|s| text(e, "entityName").to_lowercase().contains(&s.to_lowercase()))
        })
        .collect();

    let total = entries.len();
    let offset = opts.offset.unwrap_or(0);
    let limit = match opts.limit {
        Some(0) => total,
        Some(limit) => limit,
        None => 200,
    };
    (entries.into_iter().skip(offset).take(limit).collect(), total)
}

/// The JSON content of `rel_path` as of `commit_ref`, or `None` if it did not exist or does not parse.
pub async fn get_entity_at_commit(commit_ref: &str, ws_id: &str, rel_path: &str) -> Option<Value> {
    let spec = format!("{commit_ref}:{}", rel_path.replace('\\', "/"));
    let content = git(ws_id, &["show", &spec]).await.ok()?;
    serde_json::from_str(&content).ok()
}

pub async fn get_commit_changed_files(commit_ref: &str, ws_id: &str) -> Vec<String> {
    match git(ws_id, &["diff-tree", "--root", "--no-commit-id", "-r", "--name-only", commit_ref]).await {
        Ok(raw) => raw.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_entries_parse_current_and_legacy_subjects() {
        let entry = parse_audit_entry(
            "abc123\n1700000000\nupdate mock [name,responseBody] Login ok\nentity-id: m1\nworkspace-id: w1\nactor: laptop\n",
        )
        .unwrap();
        assert_eq!(entry["action"], "update");
        assert_eq!(entry["entity"], "mock");
        assert_eq!(entry["entityName"], "Login ok");
        assert_eq!(entry["changedFields"], json!(["name", "responseBody"]));
        assert_eq!(entry["ts"], 1_700_000_000_000i64);
        assert_eq!(entry["actor"], "laptop");

        let legacy = parse_audit_entry("def\n1\ncreate(request): Ping — GET /ping\n").unwrap();
        assert_eq!(legacy["entityName"], "Ping");
        assert_eq!(legacy["actor"], "local");
        assert!(legacy.get("changedFields").is_none());

        assert!(parse_audit_entry("ghi\n1\nchore: init workspace repo\n").is_none());
    }
}
