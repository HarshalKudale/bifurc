//! The engine-backed commands: each takes the parsed arguments and an engine session and returns the text to
//! print. The session is opened on first use, so commands that need none (`commands`) never connect.

use serde_json::{Map, Value, json};

use crate::{
    args::ParsedArgs,
    error::{CliError, Result},
    format::{describe_status, js_string, js_template, json, table, truthy},
    protocol::{command_names, is_known_command},
    session::Engine,
};

/// Opens the engine session when a command first needs it.
pub type Connector<'a> = Box<dyn FnMut() -> Result<Box<dyn Engine>> + 'a>;

/// What a command runs against: its arguments, the output mode and a lazily opened engine session.
pub struct Context<'a> {
    pub args: &'a ParsedArgs,
    /// `--json`: print values as JSON instead of their human rendering.
    pub json: bool,
    /// `--raw`: the bytes the caller writes to stdout as they are, with nothing after them.
    pub(crate) raw: Option<Vec<u8>>,
    connector: Connector<'a>,
    engine: Option<Box<dyn Engine>>,
}

impl<'a> Context<'a> {
    /// A context whose session is opened by `connector` on first use.
    pub fn new(args: &'a ParsedArgs, connector: Connector<'a>) -> Self {
        Self { args, json: args.has("json"), raw: None, connector, engine: None }
    }

    /// The engine session, opened if this is the first use.
    pub fn engine(&mut self) -> Result<&mut dyn Engine> {
        if self.engine.is_none() {
            self.engine = Some((self.connector)()?);
        }
        Ok(self.engine.as_deref_mut().expect("opened above"))
    }

    /// `value` as JSON with `--json`, otherwise what `human` renders.
    pub fn print(&self, value: &Value, human: impl FnOnce() -> String) -> String {
        if self.json { json(value) } else { human() }
    }

    pub(crate) fn request(&mut self, command: &str, payload: Value) -> Result<Value> {
        self.engine()?.request(command, payload)
    }

    /// Refuse an option the command does not take, and more arguments than its `operands`. The command is the
    /// group and its verb.
    pub(crate) fn accept(&self, flags: &[&str], operands: usize) -> Result<()> {
        self.accept_after(2, flags, operands)
    }

    /// [`Context::accept`] for a command that is one word (`status`, `call`).
    pub(crate) fn accept_top(&self, flags: &[&str], operands: usize) -> Result<()> {
        self.accept_after(1, flags, operands)
    }

    fn accept_after(&self, words: usize, flags: &[&str], operands: usize) -> Result<()> {
        let positionals = &self.args.positionals;
        let command = positionals[..words.min(positionals.len())].join(" ");
        let help = positionals.first().map(String::as_str).unwrap_or_default();
        let known = |name: &&str| GLOBAL_FLAGS.contains(name) || flags.contains(name);
        if let Some(unknown) = self.args.names().into_iter().find(|name| !known(name)) {
            return Err(CliError::usage(format!(
                "unknown option --{unknown} for \"{command}\" (see: bifurc {help} --help)"
            )));
        }
        if let Some(extra) = positionals.get(words + operands) {
            return Err(CliError::usage(format!(
                "unexpected argument \"{extra}\" for \"{command}\" (see: bifurc {help} --help)"
            )));
        }
        Ok(())
    }
}

/// The options every command takes.
const GLOBAL_FLAGS: [&str; 5] = ["data-dir", "workspace", "no-start", "json", "help"];

/// An argument that must be present and non-empty.
pub(crate) fn need(value: Option<&str>, what: &str) -> Result<String> {
    match value {
        Some(v) if !v.is_empty() => Ok(v.to_string()),
        _ => Err(CliError::usage(format!("missing {what}"))),
    }
}

/// The error of a verb the group does not have. Deleting and syncing one item are the desktop app's.
pub(crate) fn unknown_verb(group: &str, verb: &str) -> CliError {
    let hint = match verb {
        "revert" | "push" | "pull" | "publish" | "commit" | "discard" => "; git is under: bifurc sync",
        _ => "",
    };
    CliError::usage(format!("unknown: {group} {verb}{hint} (see: bifurc {group} --help)"))
}

fn entities(response: &Value) -> Vec<Value> {
    response.get("entities").and_then(Value::as_array).cloned().unwrap_or_default()
}

/// `{...row, key: value}`: the row's own keys in order, `key` replaced in place or added last.
pub(crate) fn with_field(row: &Value, key: &str, value: Value) -> Value {
    let mut map: Map<String, Value> = row.as_object().cloned().unwrap_or_default();
    map.insert(key.to_string(), value);
    Value::Object(map)
}

/// The workspace a command acts on: `--workspace`, else the engine's active one.
pub(crate) fn workspace_id(ctx: &mut Context) -> Result<String> {
    if let Some(explicit) = ctx.args.flag("workspace") {
        return Ok(explicit.to_string());
    }
    let config = ctx.request("config.get", json!({}))?;
    Ok(js_template(config.get("activeWorkspaceId")))
}

/// The entities of `kind` in `workspace`, as their files hold them.
pub(crate) fn list_entities(ctx: &mut Context, workspace: &str, kind: &str) -> Result<Vec<Value>> {
    let response = ctx.request("entity.list", json!({ "workspaceId": workspace, "kind": kind }))?;
    Ok(entities(&response))
}

/// The one row `wanted` names, trying in order: its id, an exact name, a name whatever the case, a unique name
/// prefix, a unique id prefix (4+ characters). Several at the first step with any is a usage error that lists
/// them; none is not-found with close names suggested. `what` is the row's singular name and `group` the
/// command group that lists them.
pub(crate) fn find_one<'v>(
    rows: &'v [Value],
    wanted: &str,
    name_keys: &[&str],
    what: &str,
    group: &str,
) -> Result<&'v Value> {
    let names = |row: &Value| -> Vec<String> {
        name_keys.iter().filter_map(|key| row.get(*key).and_then(Value::as_str)).map(str::to_string).collect()
    };
    let lower = wanted.to_lowercase();
    let row_id = |row: &Value| row.get("id").and_then(Value::as_str).map(str::to_string);
    let matches = |step: usize, row: &Value| match step {
        0 => row_id(row).as_deref() == Some(wanted),
        1 => names(row).iter().any(|n| n == wanted),
        2 => names(row).iter().any(|n| n.to_lowercase() == lower),
        3 => names(row).iter().any(|n| n.to_lowercase().starts_with(&lower)),
        _ => wanted.chars().count() >= 4 && row_id(row).is_some_and(|id| id.starts_with(wanted)),
    };
    for step in 0..5 {
        let hits: Vec<&Value> = rows.iter().filter(|row| matches(step, row)).collect();
        match hits.as_slice() {
            [] => continue,
            [one] => return Ok(one),
            many => {
                let labels: Vec<String> =
                    many.iter().map(|row| names(row).first().cloned().unwrap_or_default()).collect();
                let repeated =
                    labels.iter().any(|l| labels.iter().filter(|o| o.to_lowercase() == l.to_lowercase()).count() > 1);
                let listed: Vec<String> = many
                    .iter()
                    .zip(&labels)
                    .map(|(row, label)| {
                        if repeated || step == 4 {
                            format!("\"{label}\" ({})", js_template(row.get("id")))
                        } else {
                            format!("\"{label}\"")
                        }
                    })
                    .collect();
                let how = if step >= 3 { "start with" } else { "are named" };
                let advice = if repeated { "use the id" } else { "use more of the name" };
                return Err(CliError::usage(format!(
                    "{} {what}s {how} \"{wanted}\": {} — {advice}",
                    many.len(),
                    listed.join(", ")
                )));
            }
        }
    }
    let mut close: Vec<(usize, String)> = rows
        .iter()
        .flat_map(names)
        .filter_map(|n| {
            let l = n.to_lowercase();
            let d = levenshtein(&l, &lower);
            (d <= 2.max(lower.chars().count() / 4) || l.contains(&lower)).then_some((d, n))
        })
        .collect();
    close.sort();
    close.dedup_by(|a, b| a.1 == b.1);
    close.truncate(3);
    let hint = if close.is_empty() {
        format!("(see: bifurc {group} list)")
    } else {
        let quoted: Vec<String> = close.iter().map(|(_, n)| format!("\"{n}\"")).collect();
        format!("— did you mean {}?", quoted.join(", "))
    };
    Err(CliError::not_found(format!("no {what} \"{wanted}\" {hint}")))
}

/// Edit distance between two strings (insert, delete, replace), for did-you-mean.
fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diag = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = (diag + usize::from(ca != *cb)).min(up + 1).min(row[j] + 1);
            diag = up;
        }
    }
    row[b.len()]
}

/// `-H "name: value"` flags in order. An empty value (`-H "name:"`) is kept: on an update it removes the header.
pub(crate) fn header_flags(args: &ParsedArgs) -> Result<Vec<(String, String)>> {
    args.flags("header")
        .iter()
        .map(|header| match header.find(':') {
            Some(i) if i > 0 => Ok((header[..i].trim().to_string(), header[i + 1..].trim().to_string())),
            _ => Err(CliError::usage(format!("header \"{header}\" must be \"name: value\""))),
        })
        .collect()
}

/// Put the `-H` flags into `headers`: a header replaces the one of the same name whatever its case, and an empty
/// value removes it. Returns the names touched.
pub(crate) fn merge_headers(headers: &mut Map<String, Value>, flags: &[(String, String)]) {
    for (name, value) in flags {
        headers.retain(|existing, _| !existing.eq_ignore_ascii_case(name));
        if !value.is_empty() {
            headers.insert(name.clone(), json!(value));
        }
    }
}

/// `--body-file <f>` or `--data <text>` (`-d`; `--data @<f>` reads a file, as curl does), when either is given.
pub(crate) fn body_from(args: &ParsedArgs) -> Result<Option<String>> {
    match args.flag("data").and_then(|data| data.strip_prefix('@')) {
        Some(file) if !args.has("body-file") => {
            std::fs::read_to_string(file).map(Some).map_err(|e| CliError::msg(format!("cannot read {file}: {e}")))
        }
        _ => text_from(args, "data", "body-file"),
    }
}

/// The text of `--<flag>`, or of the file `--<file_flag>` names, when either is given. The file wins.
pub(crate) fn text_from(args: &ParsedArgs, flag: &str, file_flag: &str) -> Result<Option<String>> {
    match args.flag(file_flag) {
        Some(file) => {
            std::fs::read_to_string(file).map(Some).map_err(|e| CliError::msg(format!("cannot read {file}: {e}")))
        }
        None => Ok(args.flag(flag).map(str::to_string)),
    }
}

/// `--<on>` or `--<off>` as a choice, when either is given.
pub(crate) fn switch(args: &ParsedArgs, on: &str, off: &str) -> Result<Option<bool>> {
    match (args.has(on), args.has(off)) {
        (true, true) => Err(CliError::usage(format!("--{on} and --{off} cannot both be given"))),
        (true, false) => Ok(Some(true)),
        (false, true) => Ok(Some(false)),
        (false, false) => Ok(None),
    }
}

/// The entity an `entity.create` answered with.
pub(crate) fn created(response: &Value) -> Value {
    response.get("entity").filter(|entity| entity.is_object()).cloned().unwrap_or_else(|| response.clone())
}

pub(crate) fn id_of(entity: &Value) -> String {
    entity.get("id").filter(|v| !v.is_null()).map(js_string).unwrap_or_else(|| "?".to_string())
}

/// A command's `{ ok: false, error }` answer as an error.
pub(crate) fn refused(result: Value) -> Result<Value> {
    match result.get("ok").and_then(Value::as_bool) {
        Some(false) => Err(CliError::msg(js_template(result.get("error")))),
        _ => Ok(result),
    }
}

/// The workspaces of the engine, each with whether it is the active one, and the active one's id.
pub(crate) fn workspace_rows(ctx: &mut Context) -> Result<(Vec<Value>, Value)> {
    let config = ctx.request("config.get", json!({}))?;
    let active = config.get("activeWorkspaceId").cloned().unwrap_or(Value::Null);
    let rows = config
        .get("workspaces")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|w| with_field(w, "active", json!(if w.get("id") == Some(&active) { "*" } else { "" })))
        .collect();
    Ok((rows, active))
}

/// `workspaces list|show|create|update|use|sync`.
pub fn workspaces(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    match sub.unwrap_or("list") {
        "list" => {
            ctx.accept(&[], 0)?;
            let (rows, _) = workspace_rows(ctx)?;
            Ok(ctx.print(&Value::Array(rows.clone()), || table(&rows, &["active", "id", "name"])))
        }
        "show" => {
            ctx.accept(&[], 1)?;
            let (rows, active) = workspace_rows(ctx)?;
            let wanted = ctx.args.positional(2).map(str::to_string).unwrap_or_else(|| js_template(Some(&active)));
            let workspace = find_one(&rows, &wanted, &["name"], "workspace", "workspaces")?;
            Ok(ctx.print(workspace, || json(workspace)))
        }
        "use" => {
            ctx.accept(&[], 1)?;
            let wanted = need(ctx.args.positional(2), "workspace id or name")?;
            let (rows, _) = workspace_rows(ctx)?;
            let workspace = find_one(&rows, &wanted, &["name"], "workspace", "workspaces")?;
            let (id, name) = (id_of(workspace), js_template(workspace.get("name")));
            ctx.request("workspace.setActive", json!({ "id": id }))?;
            Ok(ctx
                .print(&json!({ "ok": true, "activeWorkspaceId": id }), || format!("Active workspace: {name} ({id})")))
        }
        "update" => {
            ctx.accept(&["name"], 1)?;
            let wanted = need(ctx.args.positional(2), "workspace id or name")?;
            let name = need(ctx.args.flag("name").map(str::trim), "--name <new name>")?;
            let (rows, _) = workspace_rows(ctx)?;
            let workspace = find_one(&rows, &wanted, &["name"], "workspace", "workspaces")?;
            let id = id_of(workspace);
            ctx.request("workspace.rename", json!({ "id": id, "name": name }))?;
            let renamed = with_field(workspace, "name", json!(name));
            Ok(ctx.print(&renamed, || format!("Updated workspace {id} ({name})")))
        }
        "create" | "add" => {
            ctx.accept(&["account"], 1)?;
            let name = need(ctx.args.positional(2), "workspace name")?;
            let mut payload = json!({ "name": name });
            if let Some(account) = ctx.args.flag("account") {
                payload["accountId"] = json!(account_id(ctx, account)?);
            }
            let created = ctx.request("workspace.add", payload)?;
            let id = js_template(created.get("id"));
            ctx.request("workspace.setActive", json!({ "id": id }))?;
            Ok(ctx.print(&created, || {
                let tracked = if created.get("gitTracked").is_some_and(truthy) {
                    ""
                } else {
                    "\nGit is not installed: this workspace has no history, audit log or sync."
                };
                format!("Created workspace {id} ({name}){tracked}")
            }))
        }
        "sync" => {
            ctx.accept(&["message"], 0)?;
            let workspace = workspace_id(ctx)?;
            let mut payload = json!({ "workspaceId": workspace });
            if let Some(message) = ctx.args.flag("message") {
                payload["message"] = json!(message);
            }
            let result = refused(ctx.request("sync.all", payload)?)?;
            Ok(ctx.print(&result, || {
                let count = |key: &str| result.get(key).and_then(Value::as_u64).unwrap_or(0);
                let mut line = match count("synced") {
                    0 => "Nothing new to sync".to_string(),
                    synced => format!("Synced {synced} change(s), each in a commit of its own"),
                };
                if result.get("initialized").is_some_and(truthy) {
                    line = format!("The workspace is now a git repository.\n{line}");
                }
                if result.get("pushed").is_some_and(truthy) {
                    line.push_str(", and pushed them");
                }
                if count("pendingDeletions") > 0 {
                    line.push_str(&format!(
                        "\n{} deletion(s) still wait to be confirmed or reverted",
                        count("pendingDeletions")
                    ));
                }
                line
            }))
        }
        other => Err(unknown_verb("workspaces", other)),
    }
}

/// The id of a signed-in account, named by its id or its login.
pub(crate) fn account_id(ctx: &mut Context, wanted: &str) -> Result<String> {
    let listed = ctx.request("account.list", json!({}))?;
    listed
        .get("accounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|a| {
            a.get("id").and_then(Value::as_str) == Some(wanted)
                || a.get("login").and_then(Value::as_str).is_some_and(|login| login.eq_ignore_ascii_case(wanted))
        })
        .map(|a| js_template(a.get("id")))
        .ok_or_else(|| CliError::not_found(format!("no account \"{wanted}\" is signed in (see: bifurc account list)")))
}

/// `account list|login|repos`: the GitHub accounts workspaces sync as.
pub fn account(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    match sub.unwrap_or("list") {
        "list" => {
            ctx.accept(&[], 0)?;
            let listed = ctx.request("account.list", json!({}))?;
            let rows: Vec<Value> = listed.get("accounts").and_then(Value::as_array).cloned().unwrap_or_default();
            Ok(ctx.print(&Value::Array(rows.clone()), || table(&rows, &["login", "host", "signedIn", "id"])))
        }
        "login" => {
            ctx.accept(&[], 0)?;
            let started = ctx.request("account.login.start", json!({}))?;
            let login_id = js_template(started.get("loginId"));
            if started.get("status").and_then(Value::as_str) != Some("pending") {
                return Err(CliError::msg(js_template(started.get("error"))));
            }
            // The code goes to the terminal straight away: the command then waits for it to be used.
            eprintln!(
                "Open {} and enter the code {}",
                js_template(started.get("verificationUri")),
                js_template(started.get("userCode"))
            );
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let status = ctx.request("account.login.status", json!({ "loginId": login_id }))?;
                match status.get("status").and_then(Value::as_str) {
                    Some("pending") => continue,
                    Some("done") => {
                        let login = js_template(status.pointer("/account/login"));
                        return Ok(ctx.print(&status, || format!("Signed in as {login}")));
                    }
                    _ => return Err(CliError::msg(js_template(status.get("error")))),
                }
            }
        }
        "repos" => {
            ctx.accept(&[], 1)?;
            let id = account_id(ctx, &need(ctx.args.positional(2), "account login")?)?;
            let result = refused(ctx.request("account.repos", json!({ "accountId": id }))?)?;
            let rows: Vec<Value> = result.get("repos").and_then(Value::as_array).cloned().unwrap_or_default();
            Ok(ctx
                .print(&Value::Array(rows.clone()), || table(&rows, &["fullName", "private", "access", "workspaceId"])))
        }
        other => Err(unknown_verb("account", other)),
    }
}

/// `env list|show|use` for the active workspace. `Global` is always applied underneath; the selected one
/// overrides it. The selection is the engine's, so it is shared with the desktop UI.
pub fn env(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    match sub.unwrap_or("list") {
        "list" => ctx.accept(&[], 0)?,
        "use" => ctx.accept(&[], 1)?,
        other => return Err(unknown_verb("env", other)),
    }
    let config = ctx.request("config.get", json!({}))?;
    let active_workspace = config.get("activeWorkspaceId").cloned().unwrap_or(Value::Null);
    let active_environment = config.get("activeEnvironmentId").cloned().unwrap_or(Value::Null);
    let workspace =
        ctx.args.flag("workspace").map(str::to_string).unwrap_or_else(|| js_template(Some(&active_workspace)));
    let selectable = Value::String(workspace.clone()) == active_workspace;
    match sub.unwrap_or("list") {
        "list" => {
            let response = ctx.request("entity.list", json!({ "workspaceId": workspace, "kind": "environments" }))?;
            let rows: Vec<Value> = entities(&response)
                .iter()
                .map(|e| {
                    let id = e.get("id");
                    let active = if id.and_then(Value::as_str) == Some("__global__") {
                        "base"
                    } else if selectable && id == Some(&active_environment) {
                        "*"
                    } else {
                        ""
                    };
                    let vars = e.get("variables").and_then(Value::as_array).map_or(0, Vec::len);
                    with_field(&with_field(e, "active", json!(active)), "vars", json!(vars))
                })
                .collect();
            Ok(ctx.print(&Value::Array(rows.clone()), || table(&rows, &["active", "id", "name", "vars"])))
        }
        "use" => {
            if !selectable {
                return Err(CliError::msg(format!(
                    "Environments are selected for the active workspace; run: bifurc workspaces use {workspace}"
                )));
            }
            let want = need(ctx.args.positional(2), "environment id or name")?;
            let response = ctx.request("entity.list", json!({ "workspaceId": workspace, "kind": "environments" }))?;
            let all = entities(&response);
            let hit = all
                .iter()
                .find(|e| e.get("id").and_then(Value::as_str) == Some(want.as_str()))
                .or_else(|| all.iter().find(|e| js_template(e.get("name")).to_lowercase() == want.to_lowercase()))
                .ok_or_else(|| CliError::not_found(format!("no environment \"{want}\" (see: bifurc env list)")))?;
            let id = hit.get("id").cloned().unwrap_or(Value::Null);
            let result = ctx.request("env.setActive", json!({ "id": id }))?;
            if !result.get("ok").is_some_and(truthy) {
                return Err(CliError::msg(match result.get("error").and_then(Value::as_str) {
                    Some("cannot_activate_global") => "Global is always active; pick another environment.".to_string(),
                    _ => js_template(result.get("error")),
                }));
            }
            let (name, id_text) = (js_template(hit.get("name")), js_template(Some(&id)));
            Ok(ctx.print(&json!({ "ok": true, "activeEnvironmentId": id }), || {
                format!("Active environment: {name} ({id_text})")
            }))
        }
        other => Err(unknown_verb("env", other)),
    }
}

/// `services list`.
pub fn services(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    if let Some(other) = sub.filter(|s| *s != "list") {
        return Err(unknown_verb("services", other));
    }
    ctx.accept(&[], 0)?;
    let response = ctx.request("services.discover", json!({}))?;
    let rows = response.as_array().cloned().unwrap_or_default();
    Ok(ctx.print(&response, || table(&rows, &["port", "pid", "processName", "address"])))
}

/// `call <command> [json]`: any protocol command with a JSON payload.
pub fn call(ctx: &mut Context) -> Result<String> {
    ctx.accept_top(&["payload"], 2)?;
    let command = need(ctx.args.positional(1), "command name (see: bifurc commands)")?;
    if !is_known_command(&command) {
        return Err(CliError::usage(format!("unknown command \"{command}\" (see: bifurc commands)")));
    }
    let raw = ctx.args.positional(2).or(ctx.args.flag("payload")).unwrap_or("{}");
    let payload: Value =
        serde_json::from_str(raw).map_err(|_| CliError::usage(format!("payload is not valid JSON: {raw}")))?;
    let response = ctx.request(&command, payload)?;
    Ok(ctx.print(&response, || json(&response)))
}

/// `commands`: every engine command, sorted.
pub fn commands(ctx: &Context) -> String {
    let names = command_names();
    ctx.print(&Value::Array(names.iter().map(|n| json!(n)).collect()), || names.join("\n"))
}

/// `status`: the engine, the proxy and the active workspace.
pub fn status(ctx: &mut Context) -> Result<String> {
    ctx.accept_top(&[], 0)?;
    let server = ctx.request("server.status", json!({}))?;
    let config = ctx.request("config.get", json!({}))?;
    let active_id = config.get("activeWorkspaceId").cloned().unwrap_or(Value::Null);
    let active = config
        .get("workspaces")
        .and_then(Value::as_array)
        .and_then(|all| all.iter().find(|w| w.get("id") == Some(&active_id)))
        .cloned()
        .unwrap_or(active_id);
    let info = ctx.engine()?.info().clone();
    let value = json!({ "engine": info.raw, "server": server, "activeWorkspace": active });
    Ok(ctx.print(&value, || describe_status(&info, &server, &active)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        args::parse_args,
        testing::{config, run, sub},
    };

    fn envs() -> Value {
        json!({"entities": [
            {"id": "__global__", "name": "Global", "variables": [{"k": "a"}]},
            {"id": "e1", "name": "Dev", "variables": []},
            {"id": "e2", "name": "Prod", "variables": [{"k": "a"}, {"k": "b"}]},
        ]})
    }

    #[test]
    fn workspaces_list_marks_the_active_one() {
        let argv = ["workspaces", "list"];
        let (out, calls) = run(&argv, vec![("config.get", Ok(config()))], |c| workspaces(c, sub(&argv)));
        assert_eq!(out.unwrap(), "ACTIVE  ID  NAME\n*       w1  Default\n        w2  Other");
        assert_eq!(calls[0].0, "config.get");
    }

    #[test]
    fn workspaces_list_as_json_adds_the_active_column() {
        let argv = ["workspaces", "--json"];
        let (out, _) = run(&argv, vec![("config.get", Ok(config()))], |c| workspaces(c, None));
        let parsed: Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!(parsed[0], json!({"id": "w1", "name": "Default", "active": "*"}));
        assert_eq!(parsed[1]["active"], "");
    }

    #[test]
    fn workspaces_use_sets_the_active_workspace_named_by_id_or_name() {
        for wanted in ["w2", "other"] {
            let argv = ["workspaces", "use", wanted];
            let answers = vec![("config.get", Ok(config())), ("workspace.setActive", Ok(json!({})))];
            let (out, calls) = run(&argv, answers, |c| workspaces(c, sub(&argv)));
            assert_eq!(out.unwrap(), "Active workspace: Other (w2)");
            assert_eq!(calls[1], ("workspace.setActive".to_string(), json!({"id": "w2"})));
        }
        let argv = ["workspaces", "use", "nope"];
        let (out, calls) = run(&argv, vec![("config.get", Ok(config()))], |c| workspaces(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::not_found("no workspace \"nope\" (see: bifurc workspaces list)"));
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn workspaces_show_prints_the_named_or_the_active_one() {
        let argv = ["workspaces", "show"];
        let (out, _) = run(&argv, vec![("config.get", Ok(config()))], |c| workspaces(c, sub(&argv)));
        let shown: Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!(shown, json!({"id": "w1", "name": "Default", "active": "*"}));
        let argv = ["workspaces", "show", "Other"];
        let (out, _) = run(&argv, vec![("config.get", Ok(config()))], |c| workspaces(c, sub(&argv)));
        assert_eq!(serde_json::from_str::<Value>(&out.unwrap()).unwrap()["id"], "w2");
    }

    #[test]
    fn workspaces_update_renames() {
        let argv = ["workspaces", "update", "w2", "--name", " Staging "];
        let answers = vec![("config.get", Ok(config())), ("workspace.rename", Ok(json!({"ok": true})))];
        let (out, calls) = run(&argv, answers, |c| workspaces(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Updated workspace w2 (Staging)");
        assert_eq!(calls[1], ("workspace.rename".to_string(), json!({"id": "w2", "name": "Staging"})));

        let argv = ["workspaces", "update", "w2"];
        let (out, calls) = run(&argv, vec![], |c| workspaces(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("missing --name <new name>"));
        assert!(calls.is_empty());
    }

    #[test]
    fn workspaces_create_makes_it_active_and_add_still_works() {
        for verb in ["create", "add"] {
            let argv = ["workspaces", verb, "Mine"];
            let answers = vec![
                ("workspace.add", Ok(json!({"id": "w9", "gitTracked": true}))),
                ("workspace.setActive", Ok(json!({}))),
            ];
            let (out, calls) = run(&argv, answers, |c| workspaces(c, sub(&argv)));
            assert_eq!(out.unwrap(), "Created workspace w9 (Mine)");
            assert_eq!(calls[0], ("workspace.add".to_string(), json!({"name": "Mine"})));
            assert_eq!(calls[1], ("workspace.setActive".to_string(), json!({"id": "w9"})));
        }
    }

    #[test]
    fn workspaces_sync_reports_what_was_synced_and_what_waits() {
        let argv = ["workspaces", "sync"];
        let synced = json!({"ok": true, "synced": 3, "pushed": true, "pendingDeletions": 1});
        let answers = vec![("config.get", Ok(config())), ("sync.all", Ok(synced))];
        let (out, calls) = run(&argv, answers, |c| workspaces(c, sub(&argv)));
        assert_eq!(
            out.unwrap(),
            "Synced 3 change(s), each in a commit of its own, and pushed them\n\
             1 deletion(s) still wait to be confirmed or reverted"
        );
        assert_eq!(calls[1], ("sync.all".to_string(), json!({"workspaceId": "w1"})));

        let answers = vec![("config.get", Ok(config())), ("sync.all", Ok(json!({"ok": false, "error": "no git"})))];
        let (out, _) = run(&argv, answers, |c| workspaces(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("no git"));
    }

    #[test]
    fn workspaces_use_needs_an_id_and_unknown_subcommands_fail() {
        let argv = ["workspaces", "use"];
        let (out, _) = run(&argv, vec![], |c| workspaces(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("missing workspace id or name"));
        let argv = ["workspaces", "frob"];
        let (out, calls) = run(&argv, vec![], |c| workspaces(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("unknown: workspaces frob (see: bifurc workspaces --help)"));
        assert!(calls.is_empty());
    }

    #[test]
    fn an_option_or_argument_a_command_does_not_take_is_refused_before_the_engine_is_asked() {
        let argv = ["workspaces", "list", "--nmae", "x"];
        let (out, calls) = run(&argv, vec![], |c| workspaces(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::usage("unknown option --nmae for \"workspaces list\" (see: bifurc workspaces --help)")
        );
        assert!(calls.is_empty());

        let argv = ["workspaces", "use", "w1", "w2"];
        let (out, calls) = run(&argv, vec![], |c| workspaces(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::usage("unexpected argument \"w2\" for \"workspaces use\" (see: bifurc workspaces --help)")
        );
        assert!(calls.is_empty());

        let argv = ["status", "now"];
        let (out, _) = run(&argv, vec![], status);
        assert_eq!(
            out.unwrap_err(),
            CliError::usage("unexpected argument \"now\" for \"status\" (see: bifurc status --help)")
        );
    }

    #[test]
    fn find_one_tries_id_then_name_then_prefixes() {
        let rows = vec![
            json!({"id": "abcd1", "name": "Ping"}),
            json!({"id": "abcd2", "name": "ping"}),
            json!({"id": "c", "name": "Pong"}),
            json!({"id": "Pong", "name": "Id wins"}),
            json!({"id": "d", "name": "Get order"}),
            json!({"id": "e", "name": "Get orders"}),
            json!({"id": "f", "name": "Create user"}),
        ];
        let find = |wanted: &str| find_one(&rows, wanted, &["name"], "request", "requests");
        assert_eq!(find("abcd1").unwrap()["name"], "Ping");
        assert_eq!(find("Pong").unwrap()["name"], "Id wins");
        assert_eq!(find("Ping").unwrap()["id"], "abcd1"); // exact name beats the other case
        assert_eq!(find("pong").unwrap()["id"], "c");
        assert_eq!(
            find("PING").unwrap_err(),
            CliError::usage("2 requests are named \"PING\": \"Ping\" (abcd1), \"ping\" (abcd2) — use the id")
        );
        assert_eq!(find("create").unwrap()["id"], "f"); // unique name prefix
        assert_eq!(find("get order").unwrap()["id"], "d"); // exact (any case) name before prefix
        assert_eq!(
            find("get").unwrap_err(),
            CliError::usage("2 requests start with \"get\": \"Get order\", \"Get orders\" — use more of the name")
        );
        assert_eq!(find("abcd").unwrap_err().code(), 2); // id prefix shared by two
        let rows = vec![json!({"id": "mvbzxzympjvgk", "name": "Login"})];
        assert_eq!(find_one(&rows, "mvbz", &["name"], "request", "requests").unwrap()["name"], "Login");
        assert!(find_one(&rows, "mvb", &["name"], "request", "requests").is_err()); // under 4 characters
    }

    #[test]
    fn find_one_not_found_suggests_close_names() {
        let rows = vec![json!({"id": "a", "name": "Create user"}), json!({"id": "b", "name": "Login"})];
        let err = find_one(&rows, "Create usr", &["name"], "request", "requests").unwrap_err();
        assert_eq!(err, CliError::not_found("no request \"Create usr\" — did you mean \"Create user\"?"));
        let err = find_one(&rows, "zip", &["name"], "request", "requests").unwrap_err();
        assert_eq!(err, CliError::not_found("no request \"zip\" (see: bifurc requests list)"));
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }

    #[test]
    fn headers_merge_by_name_whatever_the_case_and_an_empty_value_removes() {
        let args = parse_args(&["-H", "accept: b", "-H", "X-Gone:", "-H", "New: 1"].map(String::from)).unwrap();
        let mut headers = json!({"Accept": "a", "x-gone": "1", "Keep": "k"}).as_object().cloned().unwrap();
        merge_headers(&mut headers, &header_flags(&args).unwrap());
        assert_eq!(Value::Object(headers), json!({"Keep": "k", "accept": "b", "New": "1"}));

        for bad in ["nocolon", ":v"] {
            let args = parse_args(&["-H", bad].map(String::from)).unwrap();
            assert_eq!(
                header_flags(&args).unwrap_err(),
                CliError::usage(format!("header \"{bad}\" must be \"name: value\""))
            );
        }
    }

    #[test]
    fn env_list_marks_base_and_selected_and_counts_variables() {
        let argv = ["env", "list"];
        let answers = vec![("config.get", Ok(config())), ("entity.list", Ok(envs()))];
        let (out, calls) = run(&argv, answers, |c| env(c, sub(&argv)));
        assert_eq!(
            out.unwrap(),
            "ACTIVE  ID          NAME    VARS\nbase    __global__  Global  1\n        e1          Dev     0\n*       e2          Prod    2"
        );
        assert_eq!(calls[1].1, json!({"workspaceId": "w1", "kind": "environments"}));
    }

    #[test]
    fn env_list_for_another_workspace_selects_nothing() {
        let argv = ["env", "list", "-w", "w2"];
        let answers = vec![("config.get", Ok(config())), ("entity.list", Ok(envs()))];
        let (out, calls) = run(&argv, answers, |c| env(c, sub(&argv)));
        assert!(!out.unwrap().contains('*'));
        assert_eq!(calls[1].1["workspaceId"], "w2");
    }

    #[test]
    fn env_use_finds_by_id_or_case_insensitive_name() {
        for want in ["e1", "dev", "DEV"] {
            let argv = ["env", "use", want];
            let answers = vec![
                ("config.get", Ok(config())),
                ("entity.list", Ok(envs())),
                ("env.setActive", Ok(json!({"ok": true}))),
            ];
            let (out, calls) = run(&argv, answers, |c| env(c, sub(&argv)));
            assert_eq!(out.unwrap(), "Active environment: Dev (e1)", "{want}");
            assert_eq!(calls[2], ("env.setActive".to_string(), json!({"id": "e1"})));
        }
    }

    #[test]
    fn env_use_reports_unknown_global_and_other_workspaces() {
        let list = || vec![("config.get", Ok(config())), ("entity.list", Ok(envs()))];

        let argv = ["env", "use", "nope"];
        let (out, _) = run(&argv, list(), |c| env(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::not_found("no environment \"nope\" (see: bifurc env list)"));

        let argv = ["env", "use", "Global"];
        let mut answers = list();
        answers.push(("env.setActive", Ok(json!({"ok": false, "error": "cannot_activate_global"}))));
        let (out, _) = run(&argv, answers, |c| env(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("Global is always active; pick another environment."));

        let argv = ["env", "use", "e1"];
        let mut answers = list();
        answers.push(("env.setActive", Ok(json!({"ok": false, "error": "other"}))));
        let (out, _) = run(&argv, answers, |c| env(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("other"));

        let argv = ["env", "use", "e1", "-w", "w2"];
        let (out, _) = run(&argv, list(), |c| env(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::msg("Environments are selected for the active workspace; run: bifurc workspaces use w2")
        );

        let argv = ["env", "use"];
        let (out, _) = run(&argv, list(), |c| env(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("missing environment id or name"));
    }

    #[test]
    fn services_list_is_a_table_and_rejects_other_subcommands() {
        let argv = ["services"];
        let found = json!([{"port": 3000, "pid": 12, "processName": "node", "address": "127.0.0.1"}]);
        let (out, _) = run(&argv, vec![("services.discover", Ok(found))], |c| services(c, None));
        assert_eq!(out.unwrap(), "PORT  PID  PROCESSNAME  ADDRESS\n3000  12   node         127.0.0.1");
        let argv = ["services", "kill"];
        let (out, calls) = run(&argv, vec![], |c| services(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("unknown: services kill (see: bifurc services --help)"));
        assert!(calls.is_empty());
    }

    #[test]
    fn call_sends_the_payload_and_prints_the_result_as_json() {
        let argv = ["call", "server.status", r#"{"x":1}"#];
        let (out, calls) = run(&argv, vec![("server.status", Ok(json!({"running": true})))], call);
        assert_eq!(out.unwrap(), "{\n  \"running\": true\n}");
        assert_eq!(calls, vec![("server.status".to_string(), json!({"x": 1}))]);
    }

    #[test]
    fn call_defaults_the_payload_and_accepts_it_as_a_flag() {
        let argv = ["call", "server.status"];
        let (_, calls) = run(&argv, vec![("server.status", Ok(json!({})))], call);
        assert_eq!(calls[0].1, json!({}));
        let argv = ["call", "server.status", "--payload", "[1]"];
        let (_, calls) = run(&argv, vec![("server.status", Ok(json!({})))], call);
        assert_eq!(calls[0].1, json!([1]));
    }

    #[test]
    fn call_validates_before_connecting() {
        let argv = ["call"];
        let (out, calls) = run(&argv, vec![], call);
        assert_eq!(out.unwrap_err(), CliError::usage("missing command name (see: bifurc commands)"));
        let argv = ["call", "nope.nope"];
        let (out, _) = run(&argv, vec![], call);
        assert_eq!(out.unwrap_err(), CliError::usage("unknown command \"nope.nope\" (see: bifurc commands)"));
        let argv = ["call", "server.status", "{bad"];
        let (out, calls2) = run(&argv, vec![], call);
        assert_eq!(out.unwrap_err(), CliError::usage("payload is not valid JSON: {bad"));
        assert!(calls.is_empty() && calls2.is_empty());
    }

    #[test]
    fn call_passes_engine_errors_through_with_their_code() {
        let argv = ["call", "ws.list"];
        let (out, _) = run(&argv, vec![], call);
        assert_eq!(out.unwrap_err().line(), "bifurc: UNKNOWN_COMMAND: ws.list");
    }

    #[test]
    fn commands_lists_the_names_sorted_without_connecting() {
        let args = parse_args(&["commands".to_string()]).unwrap();
        let connector: Connector = Box::new(|| panic!("commands must not connect"));
        let ctx = Context::new(&args, connector);
        let listing = commands(&ctx);
        assert_eq!(listing.lines().count(), command_names().len());
        assert_eq!(listing.lines().next(), Some("account.list"));

        let args = parse_args(&["commands".to_string(), "--json".to_string()]).unwrap();
        let connector: Connector = Box::new(|| panic!("commands must not connect"));
        let parsed: Value = serde_json::from_str(&commands(&Context::new(&args, connector))).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), command_names().len());
    }

    #[test]
    fn status_combines_engine_proxy_and_workspace() {
        let argv = ["status"];
        let answers = vec![("server.status", Ok(json!({"running": true, "port": 8080}))), ("config.get", Ok(config()))];
        let (out, _) = run(&argv, answers, status);
        assert_eq!(
            out.unwrap(),
            "engine     pid 5, socket addr, v0.4.0\ndata dir   /d\nproxy      listening on 8080\nworkspace  Default (w1)"
        );

        let argv = ["status", "--json"];
        let answers = vec![("server.status", Ok(json!({"running": false}))), ("config.get", Ok(config()))];
        let (out, _) = run(&argv, answers, status);
        let parsed: Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!(parsed["engine"]["pid"], 5);
        assert_eq!(parsed["server"], json!({"running": false}));
        assert_eq!(parsed["activeWorkspace"], json!({"id": "w1", "name": "Default"}));
    }

    #[test]
    fn engine_errors_stop_a_command() {
        let argv = ["status"];
        let (out, _) = run(&argv, vec![], status);
        assert_eq!(out.unwrap_err().line(), "bifurc: UNKNOWN_COMMAND: server.status");
    }
}
