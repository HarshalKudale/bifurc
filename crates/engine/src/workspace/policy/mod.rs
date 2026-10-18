//! Agent guardrails: the workspace's `policy.yaml` (or `policy.json`) says which outbound calls the command
//! line and MCP may make. A human edits the file; no command writes it. It sits at the workspace root, is read
//! on each check and cached by its modification time (no watcher).
//!
//! ```yaml
//! apply_to: [cli, mcp]          # default; add `app` to hold the desktop app to it too
//! policies:
//!   - name: no-prod-writes
//!     deny: { env: prod, methods: [DELETE, PUT, PATCH, POST] }
//!   - name: local-only
//!     allow_hosts: ["*.localhost", "localhost", "127.0.0.1"]
//!   - name: staging-deletes
//!     require_human: { env: staging, methods: [DELETE] }
//! ```
//!
//! A flat file (top-level `deny:` / `allow_hosts:` / `require_human:`) is one unnamed rule per key, named
//! after the key. Both files present: `policy.yaml` wins and `policy.json` is ignored. `deny` and
//! `require_human` take `env` (environment name, glob or list), `methods` (case-insensitive, `*` any), `hosts`
//! (globs; `*.x.com` is its subdomains; the port is ignored) and `paths` (globs on the URL path: `*` stays in
//! a segment, `**` crosses them); no key matches everything. A request whose host matches none of an
//! `allow_hosts` list is blocked by that policy. Order: `deny` and `allow_hosts` first (first in the file
//! wins), then `require_human`. A file that cannot be read blocks every guarded call (fail closed).
//!
//! `cli` and `mcp` are one switch today: both connect to the engine as the client `cli`.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, LazyLock},
    time::SystemTime,
};

use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::{
    importexport::support::parse_yaml,
    kernel::CommandRegistry,
    protocol::{EngineError, ErrorCode, Result},
    utils::json::str_of,
    workspace::{changes, contracts::plain, fs::ws_dir},
};

#[cfg(test)]
mod tests;

const FILES: [&str; 2] = ["policy.yaml", "policy.json"];
const DEFAULT_APPLY_TO: [&str; 2] = ["cli", "mcp"];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Deny,
    AllowHosts,
    RequireHuman,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Deny => "deny",
            Kind::AllowHosts => "allow_hosts",
            Kind::RequireHuman => "require_human",
        }
    }
}

/// One rule of a policy. An `allow_hosts` rule keeps its globs in `hosts`.
#[derive(Clone, Debug, PartialEq)]
struct Clause {
    policy: String,
    kind: Kind,
    env: Vec<String>,
    methods: Vec<String>,
    hosts: Vec<String>,
    paths: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Policy {
    apply_to: Vec<String>,
    clauses: Vec<Clause>,
}

/// What a call is, for matching.
struct Call {
    env: String,
    method: String,
    host: String,
    path: String,
}

#[derive(Debug, PartialEq)]
pub enum Verdict {
    Allow,
    Deny {
        policy: String,
    },
    RequireHuman {
        policy: String,
    },
    /// The file cannot be used; every guarded call is blocked until a human fixes it.
    Invalid(String),
}

// ---- parsing ----

fn strings(value: &Value, what: &str) -> std::result::Result<Vec<String>, String> {
    match value {
        Value::String(one) => Ok(vec![one.clone()]),
        Value::Array(items) => items
            .iter()
            .map(|item| item.as_str().map(str::to_string).ok_or_else(|| format!("{what} must be text")))
            .collect(),
        _ => Err(format!("{what} must be text or a list of text")),
    }
}

fn clause(policy: &str, kind: Kind, value: &Value) -> std::result::Result<Clause, String> {
    let mut out =
        Clause { policy: policy.to_string(), kind, env: vec![], methods: vec![], hosts: vec![], paths: vec![] };
    if kind == Kind::AllowHosts {
        out.hosts = strings(value, &format!("allow_hosts of \"{policy}\""))?;
        if out.hosts.is_empty() {
            return Err(format!("allow_hosts of \"{policy}\" is empty"));
        }
        return Ok(out);
    }
    let name = kind.name();
    let map = match value {
        Value::Null => return Ok(out),
        Value::Object(map) => map,
        _ => return Err(format!("{name} of \"{policy}\" must be a mapping of env, methods, hosts, paths")),
    };
    for (key, value) in map {
        let list = strings(value, &format!("{name}.{key} of \"{policy}\""))?;
        match key.as_str() {
            "env" => out.env = list,
            "methods" => out.methods = list.iter().map(|m| m.to_uppercase()).collect(),
            "hosts" => out.hosts = list,
            "paths" => out.paths = list,
            _ => return Err(format!("{name} of \"{policy}\" has an unknown key \"{key}\"")),
        }
    }
    Ok(out)
}

/// A parser's error as one line: the rest is a code frame.
fn first_line(error: impl ToString) -> String {
    error.to_string().lines().next().unwrap_or_default().trim().to_string()
}

const KINDS: [Kind; 3] = [Kind::Deny, Kind::AllowHosts, Kind::RequireHuman];

fn parse(file: &str, text: &str) -> std::result::Result<Policy, String> {
    let doc: Value = if file.ends_with(".json") {
        serde_json::from_str(text).map_err(first_line)?
    } else {
        parse_yaml(text).map_err(first_line)?
    };
    let root = match &doc {
        Value::Null => return Ok(Policy { apply_to: DEFAULT_APPLY_TO.map(String::from).to_vec(), clauses: vec![] }),
        Value::Object(root) => root,
        _ => return Err("the file must be a mapping".into()),
    };
    if let Some(unknown) =
        root.keys().find(|k| !matches!(k.as_str(), "apply_to" | "policies" | "deny" | "allow_hosts" | "require_human"))
    {
        return Err(format!("unknown key \"{unknown}\""));
    }
    let mut apply_to = DEFAULT_APPLY_TO.map(String::from).to_vec();
    if let Some(listed) = root.get("apply_to") {
        apply_to = strings(listed, "apply_to")?;
        if let Some(bad) = apply_to.iter().find(|a| !matches!(a.as_str(), "cli" | "mcp" | "app")) {
            return Err(format!("apply_to has \"{bad}\"; use cli, mcp or app"));
        }
    }
    let mut clauses = Vec::new();
    let items: &[Value] = match root.get("policies") {
        None | Some(Value::Null) => &[],
        Some(Value::Array(items)) => items,
        Some(_) => return Err("policies must be a list".into()),
    };
    for (index, item) in items.iter().enumerate() {
        let Some(item) = item.as_object() else { return Err(format!("policies[{index}] must be a mapping")) };
        let name =
            item.get("name").and_then(Value::as_str).map_or_else(|| format!("policy-{}", index + 1), str::to_string);
        if let Some(unknown) =
            item.keys().find(|k| !matches!(k.as_str(), "name" | "deny" | "allow_hosts" | "require_human"))
        {
            return Err(format!("policy \"{name}\" has an unknown key \"{unknown}\""));
        }
        let before = clauses.len();
        for kind in KINDS {
            if let Some(value) = item.get(kind.name()) {
                clauses.push(clause(&name, kind, value)?);
            }
        }
        if clauses.len() == before {
            return Err(format!("policy \"{name}\" has none of deny, allow_hosts, require_human"));
        }
    }
    for kind in KINDS {
        if let Some(value) = root.get(kind.name()) {
            clauses.push(clause(kind.name(), kind, value)?);
        }
    }
    Ok(Policy { apply_to, clauses })
}

// ---- matching ----

/// `*` is any run of characters, except that with `by_segment` it stops at `/` and `**` goes across.
fn glob(pattern: &[char], text: &[char], by_segment: bool) -> bool {
    match pattern.first() {
        None => text.is_empty(),
        Some('*') => {
            let double = pattern.get(1) == Some(&'*');
            let rest = &pattern[if double { 2 } else { 1 }..];
            let crosses = double || !by_segment;
            (0..=text.len())
                .any(|at| glob(rest, &text[at..], by_segment) && text[..at].iter().all(|c| crosses || *c != '/'))
        }
        Some(c) => text.first() == Some(c) && glob(&pattern[1..], &text[1..], by_segment),
    }
}

fn globs(pattern: &str, text: &str, ignore_case: bool, by_segment: bool) -> bool {
    let chars =
        |s: &str| -> Vec<char> { if ignore_case { s.to_lowercase().chars().collect() } else { s.chars().collect() } };
    glob(&chars(pattern), &chars(text), by_segment)
}

fn any_glob(patterns: &[String], text: &str, ignore_case: bool, by_segment: bool) -> bool {
    patterns.iter().any(|p| globs(p, text, ignore_case, by_segment))
}

fn hits(clause: &Clause, call: &Call) -> bool {
    (clause.env.is_empty() || any_glob(&clause.env, &call.env, true, false))
        && (clause.methods.is_empty() || clause.methods.iter().any(|m| m == "*" || *m == call.method))
        && (clause.hosts.is_empty() || any_glob(&clause.hosts, &call.host, true, false))
        && (clause.paths.is_empty() || any_glob(&clause.paths, &call.path, false, true))
}

fn evaluate(policy: &Policy, call: &Call) -> Verdict {
    for c in &policy.clauses {
        let blocked = match c.kind {
            Kind::Deny => hits(c, call),
            Kind::AllowHosts => !any_glob(&c.hosts, &call.host, true, false),
            Kind::RequireHuman => false,
        };
        if blocked {
            return Verdict::Deny { policy: c.policy.clone() };
        }
    }
    match policy.clauses.iter().find(|c| c.kind == Kind::RequireHuman && hits(c, call)) {
        Some(c) => Verdict::RequireHuman { policy: c.policy.clone() },
        None => Verdict::Allow,
    }
}

fn applies(policy_apply_to: &[String], client: &str) -> bool {
    let wants = |names: &[&str]| policy_apply_to.iter().any(|a| names.contains(&a.as_str()));
    match client {
        "desktop-ui" => wants(&["app"]),
        "cli" => wants(&["cli", "mcp"]),
        _ => false,
    }
}

// ---- the file, cached by modification time ----

/// The file's name and what it holds: `None` when the workspace has no policy file.
type Loaded = Option<(&'static str, std::result::Result<Arc<Policy>, String>)>;

type Stamp = (&'static str, Option<SystemTime>, u64);

static CACHE: LazyLock<Mutex<HashMap<String, (Stamp, Loaded)>>> = LazyLock::new(Default::default);

// shortcut: with no policy file this is two stats per guarded send; one watcher/inotify-style cache if sends get hot.
fn load(ws_id: &str) -> Loaded {
    let dir: PathBuf = ws_dir(ws_id);
    let (file, meta) =
        FILES.iter().find_map(|f| std::fs::metadata(dir.join(f)).ok().filter(|m| m.is_file()).map(|m| (*f, m)))?;
    let stamp: Stamp = (file, meta.modified().ok(), meta.len());
    if let Some((seen, loaded)) = CACHE.lock().get(ws_id)
        && *seen == stamp
    {
        return loaded.clone();
    }
    let loaded = Some((
        file,
        std::fs::read_to_string(dir.join(file))
            .map_err(|e| format!("cannot read {file}: {e}"))
            .and_then(|text| parse(file, &text))
            .map(Arc::new),
    ));
    CACHE.lock().insert(ws_id.to_string(), (stamp, loaded.clone()));
    loaded
}

// ---- deciding ----

/// The host and path of `url`, and `url` as an error message may show it (no userinfo, query or fragment: a
/// resolved vault secret can be in them).
fn call_of(env: String, method: &str, url: &str) -> (Call, String) {
    let (host, path, shown) = match url::Url::parse(url) {
        Ok(mut parsed) => {
            let host = parsed.host_str().unwrap_or_default().trim_end_matches('.').to_lowercase();
            let path = parsed.path().to_string();
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.set_query(None);
            parsed.set_fragment(None);
            (host, path, parsed.to_string())
        }
        Err(_) => (String::new(), String::new(), url.split(['?', '#']).next().unwrap_or_default().to_string()),
    };
    (Call { env, method: method.to_uppercase(), host, path }, shown)
}

fn selected_env_name(ws_id: &str) -> String {
    crate::env::selected_environment(ws_id).map(|env| str_of(&env, "name").to_string()).unwrap_or_default()
}

/// What the workspace's policy says about `method url` from `client`. `env` is the environment name to judge by,
/// else the one the running command would send under. Also the shown form of the URL.
pub fn verdict(ws_id: &str, client: &str, env: Option<&str>, method: &str, url: &str) -> (Verdict, String) {
    let shown = call_of(String::new(), method, url).1;
    let Some((_, loaded)) = load(ws_id) else { return (Verdict::Allow, shown) };
    let policy = match loaded {
        Ok(policy) => policy,
        Err(reason) => {
            let default: Vec<String> = DEFAULT_APPLY_TO.map(String::from).to_vec();
            let verdict = if applies(&default, client) { Verdict::Invalid(reason) } else { Verdict::Allow };
            return (verdict, shown);
        }
    };
    if !applies(&policy.apply_to, client) {
        return (Verdict::Allow, shown);
    }
    let env = env.map_or_else(|| selected_env_name(ws_id), str::to_string);
    let (call, shown) = call_of(env, method, url);
    (evaluate(&policy, &call), shown)
}

fn blocked(policy: &str, method: &str, url: &str) -> EngineError {
    let method = method.to_uppercase();
    EngineError::new(ErrorCode::PolicyBlocked, format!("blocked by policy \"{policy}\": {method} {url}"))
        .keyed("policy.blocked", json!({ "policy": policy, "method": method, "url": url }))
}

fn needs_approval(policy: &str, method: &str, url: &str) -> EngineError {
    let method = method.to_uppercase();
    EngineError::new(
        ErrorCode::PolicyApproval,
        format!("needs approval: policy \"{policy}\" ({method} {url}); approvals arrive in a later version"),
    )
    .keyed("policy.approval", json!({ "policy": policy, "method": method, "url": url }))
}

fn invalid(reason: &str) -> EngineError {
    EngineError::new(ErrorCode::PolicyBlocked, format!("policy file invalid: {reason}"))
        .keyed("policy.invalid", json!({ "reason": reason }))
}

fn error_of(verdict: Verdict, method: &str, url: &str) -> Option<EngineError> {
    match verdict {
        Verdict::Allow => None,
        Verdict::Deny { policy } => Some(blocked(&policy, method, url)),
        Verdict::RequireHuman { policy } => Some(needs_approval(&policy, method, url)),
        Verdict::Invalid(reason) => Some(invalid(&reason)),
    }
}

/// The one place an outbound call of a client is allowed or stopped: `Err` carries `POLICY_BLOCKED` or
/// `POLICY_APPROVAL` and the caller must not send. Engine-internal work (no client) and clients a policy does
/// not apply to pass. Call it with the final method and URL, before anything goes on the wire.
pub async fn guard(method: &str, url: &str) -> Result<()> {
    guard_now(method, url)
}

/// [`guard`] for a sync handler (one that opens a connection: WebSocket, SSE, Socket.IO, MCP). It runs inside the
/// command's task, so it sees the same client.
pub fn guard_now(method: &str, url: &str) -> Result<()> {
    let client = changes::client_kind();
    if client.is_empty() {
        return Ok(());
    }
    // shortcut: outside a command-line session the workspace is the active one, not the command's own; use the command's payload when the app is held to a policy.
    let ws_id = crate::env::session::current_workspace()
        .unwrap_or_else(|| crate::workspace::settings::load_settings().active_workspace_id());
    let (verdict, shown) = verdict(&ws_id, &client, None, method, url);
    // D8b: a `RequireHuman` verdict queues the request for approval and waits here instead of failing.
    match error_of(verdict, method, &shown) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether `error` is a guardrail stopping a call (not a failure of the call).
pub fn is_policy(error: &EngineError) -> bool {
    matches!(error.code, ErrorCode::PolicyBlocked | ErrorCode::PolicyApproval)
}

// ---- commands ----

fn workspace_of(params: &Value) -> Result<&str> {
    let ws = str_of(params, "workspaceId");
    if !plain(ws) || !ws_dir(ws).is_dir() {
        return Err(EngineError::not_found(format!("No workspace \"{ws}\".")));
    }
    Ok(ws)
}

fn clause_json(c: &Clause) -> Value {
    json!({ "name": c.policy, "kind": c.kind.name(), "env": c.env, "methods": c.methods, "hosts": c.hosts, "paths": c.paths })
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("policy.show", |params, _| {
        let ws = workspace_of(&params)?;
        Ok(match load(ws) {
            None => json!({ "file": null, "applyTo": DEFAULT_APPLY_TO, "policies": [] }),
            Some((file, Ok(policy))) => json!({
                "file": file, "applyTo": policy.apply_to,
                "policies": policy.clauses.iter().map(clause_json).collect::<Vec<_>>(),
            }),
            Some((file, Err(error))) => {
                json!({ "file": file, "applyTo": DEFAULT_APPLY_TO, "policies": [], "error": error })
            }
        })
    });

    // A dry run as the asking client: nothing is sent.
    registry.register_sync("policy.check", |params, _| {
        let ws = workspace_of(&params)?;
        let (method, url) = (str_of(&params, "method"), str_of(&params, "url"));
        let client = changes::client_kind();
        let client = if client.is_empty() { "cli" } else { &client };
        let env = params.get("env").and_then(Value::as_str);
        let (verdict, shown) = verdict(ws, client, env, method, url);
        let decision = match &verdict {
            Verdict::Allow => "allow",
            Verdict::RequireHuman { .. } => "require_human",
            Verdict::Deny { .. } | Verdict::Invalid(_) => "deny",
        };
        let policy = match &verdict {
            Verdict::Deny { policy } | Verdict::RequireHuman { policy } => json!(policy),
            _ => Value::Null,
        };
        let reason = error_of(verdict, method, &shown).map_or_else(|| "no policy blocks it".to_string(), |e| e.message);
        Ok(json!({ "decision": decision, "policy": policy, "reason": reason }))
    });
}
