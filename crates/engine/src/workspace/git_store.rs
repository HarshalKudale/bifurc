//! Every workspace is a git repository; every saved change can become a commit, and the commit log is the
//! audit trail. Git is driven through its command line, as a user would.

use std::{path::Path, sync::LazyLock};

use regress::Regex;
use serde_json::{Value, json};

use super::{
    accounts::commit_identity,
    fs::{GLOBAL_ENV_PATH, ws_dir},
};
use crate::utils::process::hidden_command;

/// Variables that would make git open an editor or a pager, which a background engine can never answer.
const STRIPPED_GIT_VARS: [&str; 7] =
    ["EDITOR", "VISUAL", "GIT_EDITOR", "GIT_SEQUENCE_EDITOR", "PAGER", "GIT_PAGER", "GIT_EXTERNAL_DIFF"];

/// What a failed git invocation said.
#[derive(Debug, Clone)]
pub struct GitError(pub String, pub Option<(&'static str, serde_json::Value)>);

impl GitError {
    /// An error whose English `message` a client can translate by `key` with `params`.
    pub fn keyed(key: &'static str, message: impl Into<String>, params: serde_json::Value) -> Self {
        Self(message.into(), Some((key, params)))
    }

    /// Git's own output as the error, which a client wraps in `git.failed`.
    pub fn raw(detail: String) -> Self {
        let params = serde_json::json!({ "detail": detail });
        Self::keyed("git.failed", detail, params)
    }
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<GitError> for crate::protocol::EngineError {
    fn from(err: GitError) -> Self {
        let detail = err.0.clone();
        let error = crate::protocol::EngineError::engine(err.0);
        match err.1 {
            Some((key, params)) => error.keyed(key, params),
            None => error.keyed("git.failed", json!({ "detail": detail })),
        }
    }
}

pub fn device_name() -> String {
    let name = gethostname::gethostname().to_string_lossy().into_owned();
    if name.is_empty() { "unknown".to_string() } else { name }
}

/// The token a network operation presents to one host, in place of whatever credentials the machine has.
#[derive(Clone)]
pub struct RemoteAuth {
    /// `https://github.com`: the header is sent to this origin only.
    pub origin: String,
    pub token: String,
}

/// The `scheme://host[:port]` of an http(s) remote; `None` for a path or an ssh remote, which take no token.
pub fn remote_origin(remote: &str) -> Option<String> {
    let url = url::Url::parse(remote).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

/// Run `git <args>` in `dir`; stdout on success, stderr (or stdout) as the error otherwise.
pub async fn git_in(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    run_git(dir, args, None, false).await
}

/// `git_in` for an operation that talks to a remote. It never waits for a typed password. With `auth`, the
/// token is the only credential offered (the machine's credential helpers are left out), and it travels in
/// the environment, never on the command line.
pub async fn git_net_in(dir: &Path, args: &[&str], auth: Option<&RemoteAuth>) -> Result<String, GitError> {
    run_git(dir, args, auth, true).await
}

/// Where git was found when it is not on the engine's `PATH`.
static LOCATED_GIT: parking_lot::Mutex<Option<std::path::PathBuf>> = parking_lot::Mutex::new(None);

/// A git installed after the engine started is not on the `PATH` the engine was given, and the engine
/// outlives the windows that could restart it. So the places an installer puts it are looked at too.
fn locate_git() -> Option<std::path::PathBuf> {
    // `/usr/bin/git` on a Mac is a stand-in that asks for the developer tools when they are missing, and an app
    // started from the Dock has none of the folders a terminal's `PATH` adds. The real ones are looked for first.
    if cfg!(target_os = "macos") {
        return [
            "/opt/homebrew/bin/git",
            "/usr/local/bin/git",
            "/Library/Developer/CommandLineTools/usr/bin/git",
            "/Applications/Xcode.app/Contents/Developer/usr/bin/git",
        ]
        .into_iter()
        .map(std::path::PathBuf::from)
        .find(|path| path.is_file());
    }
    if !cfg!(windows) {
        return None;
    }
    let under = |var: &str, rest: &str| std::env::var_os(var).map(|base| Path::new(&base).join(rest));
    [
        under("ProgramFiles", "Git\\cmd\\git.exe"),
        under("ProgramW6432", "Git\\cmd\\git.exe"),
        under("ProgramFiles(x86)", "Git\\cmd\\git.exe"),
        under("LOCALAPPDATA", "Programs\\Git\\cmd\\git.exe"),
    ]
    .into_iter()
    .flatten()
    .find(|path| path.is_file())
}

static DEVELOPER_TOOLS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether a Mac has the developer tools `/usr/bin/git` hands over to. Without them that file is not git: it
/// puts up an install dialog and waits for it, which an engine with no window never gets answered. Once seen,
/// it stays true.
async fn developer_tools_installed() -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    if DEVELOPER_TOOLS.load(Relaxed) {
        return true;
    }
    let found = tokio::process::Command::new("/usr/bin/xcode-select")
        .arg("-p")
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .is_ok_and(|out| out.status.success());
    DEVELOPER_TOOLS.store(found, Relaxed);
    found
}

async fn run_git(dir: &Path, args: &[&str], auth: Option<&RemoteAuth>, network: bool) -> Result<String, GitError> {
    let located = LOCATED_GIT.lock().clone().or_else(|| if cfg!(target_os = "macos") { locate_git() } else { None });
    if cfg!(target_os = "macos") && located.is_none() && !developer_tools_installed().await {
        return Err(GitError::keyed(
            "git.not_installed",
            "could not run git: it is not installed",
            serde_json::json!({}),
        ));
    }
    let output = match git_command(located.as_deref(), dir, args, auth, network).output().await {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && located.is_none() => match locate_git() {
            Some(found) => {
                *LOCATED_GIT.lock() = Some(found.clone());
                git_command(Some(&found), dir, args, auth, network).output().await
            }
            None => Err(e),
        },
        other => other,
    }
    .map_err(|e| {
        GitError::keyed(
            "git.run_failed",
            format!("could not run git: {e}"),
            serde_json::json!({ "detail": e.to_string() }),
        )
    })?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(GitError::raw(if stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        stderr
    }))
}

fn git_command(
    program: Option<&Path>,
    dir: &Path,
    args: &[&str],
    auth: Option<&RemoteAuth>,
    network: bool,
) -> tokio::process::Command {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let mut command = hidden_command(program.unwrap_or(Path::new("git")));
    command.args(args).current_dir(dir).env("GIT_OPTIONAL_LOCKS", "0").stdin(std::process::Stdio::null());
    for name in STRIPPED_GIT_VARS {
        command.env_remove(name);
    }
    if network {
        command.env("GIT_TERMINAL_PROMPT", "0").env_remove("GIT_ASKPASS").env_remove("SSH_ASKPASS");
    }
    if let Some(auth) = auth {
        let basic = STANDARD.encode(format!("x-access-token:{}", auth.token));
        command
            .env("GIT_CONFIG_COUNT", "2")
            .env("GIT_CONFIG_KEY_0", "credential.helper")
            .env("GIT_CONFIG_VALUE_0", "")
            .env("GIT_CONFIG_KEY_1", format!("http.{}/.extraheader", auth.origin))
            .env("GIT_CONFIG_VALUE_1", format!("Authorization: Basic {basic}"));
    }
    command
}

/// Run `git <args>` in a workspace's repository.
pub async fn git(ws_id: &str, args: &[&str]) -> Result<String, GitError> {
    git_in(&ws_dir(ws_id), args).await
}

/// The installed git's version (`2.45.1`), or `None` when git cannot be run.
pub async fn git_version() -> Option<String> {
    // Bounded: an engine that waits on git has no endpoint for anyone to reach it by.
    let dir = std::env::current_dir().unwrap_or_default();
    let asking = git_in(&dir, &["--version"]);
    let out = tokio::time::timeout(std::time::Duration::from_secs(10), asking).await.ok()?.ok()?;
    Some(out.trim().strip_prefix("git version ").unwrap_or(out.trim()).to_string())
}

pub async fn check_git_installed() -> bool {
    git_version().await.is_some()
}

pub fn is_repo(ws_id: &str) -> bool {
    ws_dir(ws_id).join(".git").exists()
}

/// Make the workspace's commits be by its account, or by this device when it has none.
pub async fn apply_commit_identity(ws_id: &str) -> Result<(), GitError> {
    let identity = commit_identity(ws_id);
    git(ws_id, &["config", "--local", "user.email", &identity.email]).await?;
    git(ws_id, &["config", "--local", "user.name", &identity.name]).await?;
    Ok(())
}

/// Make the workspace directory a repository, and make sure it commits as the right identity. The first
/// commit creates the workspace (`.gitignore` and `workspace.json`).
pub async fn init_workspace_repo(ws_id: &str) -> Result<(), GitError> {
    let dir = ws_dir(ws_id);
    if dir.join(".git").exists() {
        let _ = apply_commit_identity(ws_id).await;
        return Ok(());
    }
    git_in(&dir, &["init"]).await?;
    super::tree::ensure_ignored(ws_id);
    // The same branch name on every device, whatever this machine's git defaults to.
    git_in(&dir, &["symbolic-ref", "HEAD", "refs/heads/main"]).await?;
    apply_commit_identity(ws_id).await?;
    if dir.join(".gitignore").exists() {
        git_in(&dir, &["add", ".gitignore"]).await?;
        if dir.join("workspace.json").exists() {
            git_in(&dir, &["add", "workspace.json"]).await?;
        }
        if dir.join(GLOBAL_ENV_PATH).exists() {
            git_in(&dir, &["add", GLOBAL_ENV_PATH]).await?;
        }
        let identity = commit_identity(ws_id);
        let name = crate::utils::json::read_json(&dir.join("workspace.json"))
            .and_then(|info| info.get("name").and_then(Value::as_str).map(str::to_string))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| ws_id.to_string());
        let message =
            format!("create workspace {name}\n\nentity-id: {ws_id}\nworkspace-id: {ws_id}\nactor: {}", identity.actor);
        git_in(&dir, &["commit", "-m", &message]).await?;
    }
    Ok(())
}

/// The `session:` trailer line of a commit of the entities `ids`: the named sessions the change feed saw change
/// them, and the session making the commit; nothing when there are none. Readers that do not know it ignore it.
pub fn session_trailer(ws_id: &str, ids: &[String]) -> String {
    let mut names = crate::workspace::changes::sessions_of(ws_id, ids);
    if let Some(own) = crate::env::session::named()
        && !names.contains(&own)
    {
        names.push(own);
    }
    if names.is_empty() { String::new() } else { format!("\nsession: {}", names.join(", ")) }
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

/// Stage one path and commit it with an audit message. Returns the commit's hash, or `""` when there
/// was nothing to commit.
pub async fn commit_mutation(opts: CommitMutation<'_>) -> Result<String, GitError> {
    let actor = opts.actor.map(str::to_string).unwrap_or_else(|| commit_identity(opts.workspace_id).actor);
    let fields =
        if opts.changed_fields.is_empty() { String::new() } else { format!(" [{}]", opts.changed_fields.join(",")) };
    let message = format!(
        "{} {}{} {}\n\nentity-id: {}\nworkspace-id: {}\nactor: {}{}",
        opts.action,
        opts.entity,
        fields,
        opts.entity_name,
        opts.entity_id,
        opts.workspace_id,
        actor,
        session_trailer(opts.workspace_id, &[opts.entity_id.to_string()])
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
    Ok(git(ws, &["rev-parse", "HEAD"]).await?.trim().to_string())
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

/// One audit entry from a commit's hash, timestamp, subject and body. A commit whose subject the user wrote
/// says what it did in its `change:` line, and carries the subject as `message`; any other commit (several
/// files at once, one made outside the engine) is an update of no one entity, named by its subject.
fn parse_audit_entry(block: &str) -> Option<Value> {
    let mut lines = block.trim().split('\n');
    let (hash, ts, subject) = (lines.next()?, lines.next()?, lines.next()?);
    if hash.is_empty() || ts.is_empty() || subject.is_empty() {
        return None;
    }
    let body = lines.collect::<Vec<_>>().join("\n");

    static PATTERNS: LazyLock<Option<(Regex, Regex)>> = LazyLock::new(|| {
        let current = Regex::new(r"^(create|update|delete) ([A-Za-z]\w*)(?: \[([^\]]*)\])? (.+)$").ok()?;
        let legacy = Regex::new(r"^(create|update|delete)\((\w+)\): (.+)$").ok()?;
        Some((current, legacy))
    });
    let (current, legacy) = PATTERNS.as_ref()?;
    let change = body_field(&body, "change");
    let described = if current.find(subject).is_some() || legacy.find(subject).is_some() {
        subject
    } else {
        change.as_deref().unwrap_or(subject)
    };
    let group = |m: &regress::Match, i: usize| m.group(i).map(|range| described[range].to_string());

    let (action, entity, name, changed_fields) = if let Some(m) = current.find(described) {
        let fields = group(&m, 3)
            .filter(|f| !f.is_empty())
            .map(|f| f.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect::<Vec<_>>());
        (group(&m, 1)?, group(&m, 2)?, group(&m, 4)?.trim().to_string(), fields)
    } else if let Some(m) = legacy.find(described) {
        let name = group(&m, 3)?;
        (group(&m, 1)?, group(&m, 2)?, name.split(" — ").next().unwrap_or("").trim().to_string(), None)
    } else {
        ("update".to_string(), String::new(), subject.trim().to_string(), None)
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
    if let Some(session) = body_field(&body, "session") {
        entry["session"] = json!(session);
    }
    if let Some(fields) = changed_fields {
        entry["changedFields"] = json!(fields);
    }
    if described != subject {
        entry["message"] = json!(subject.trim());
    }
    Some(entry)
}

/// The audit log, newest first: `{ entries, total }` after filtering, with `entries` paged.
pub async fn query_log(opts: &QueryLogOptions) -> (Vec<Value>, usize) {
    if !is_repo(&opts.workspace_id) {
        return (Vec::new(), 0);
    }
    // A merge changes nothing of its own: what it brought in is listed commit by commit.
    let mut args =
        vec!["log".to_string(), "--no-merges".to_string(), "--format=%H%n%at%n%s%n%b%n---END---".to_string()];
    if let Some(path) = opts.file_path.as_deref().filter(|p| !p.is_empty()) {
        // An entity moved to another folder is the same entity: its history goes on through the move.
        if path.ends_with(".json") {
            args.push("--follow".into());
        }
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
            opts.search.as_ref().filter(|s| !s.is_empty()).is_none_or(|s| {
                format!("{}\n{}", text(e, "entityName"), text(e, "message")).to_lowercase().contains(&s.to_lowercase())
            })
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

/// What a commit changed, as a patch against its first parent. With `rel_path`, only the file of that
/// entity, wherever it was then: an entity's file keeps its name when it moves to another folder.
pub async fn commit_patch(ws_id: &str, commit: &str, rel_path: Option<&str>) -> Result<String, GitError> {
    // The hash goes on git's command line: nothing but an object id may.
    if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(GitError::keyed("git.not_commit_id", "Not a commit id.", serde_json::json!({})));
    }
    let mut args = vec![
        "show",
        "--no-show-signature",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--format=",
        "--first-parent",
        "--patch",
        "--find-renames",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        commit,
        "--",
    ];
    let pathspec = rel_path
        .map(|path| path.replace('\\', "/"))
        .filter(|path| !path.is_empty())
        .map(|path| format!(":(glob)**/{}", path.rsplit('/').next().unwrap_or(&path)));
    if let Some(pathspec) = &pathspec {
        args.push(pathspec);
    }
    git(ws_id, &args).await
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

        // The user's own subject: what the commit did is in its `change:` line.
        let custom = parse_audit_entry(
            "jkl\n1\nFix the login path\nchange: update request Login\nentity-id: r1\nworkspace-id: w1\nactor: sam\n",
        )
        .unwrap();
        assert_eq!(custom["message"], "Fix the login path");
        assert_eq!(custom["action"], "update");
        assert_eq!(custom["entity"], "request");
        assert_eq!(custom["entityName"], "Login");
        assert_eq!(custom["entityId"], "r1");

        // Any other commit is listed too, named by its subject.
        for subject in ["chore: init workspace repo", "update 3 files"] {
            let other = parse_audit_entry(&format!("ghi\n1\n{subject}\n")).unwrap();
            assert_eq!(other["action"], "update");
            assert_eq!(other["entity"], "");
            assert_eq!(other["entityName"], subject);
            assert!(other.get("message").is_none());
        }
    }
}
