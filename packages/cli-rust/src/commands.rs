//! The engine-backed commands: each takes the parsed arguments and an engine session and returns the text to
//! print. The session is opened on first use, so commands that need none (`commands`) never connect.

use serde_json::{Map, Value, json};

use crate::{
    args::ParsedArgs,
    error::{CliError, Result},
    format::{describe_grpc, describe_reflection, describe_send, describe_status, js_string, js_template, json, table, truthy},
    prompt,
    protocol::{command_names, is_known_command},
    session::Engine,
    unlock,
};

/// Opens the engine session when a command first needs it.
pub type Connector<'a> = Box<dyn FnMut() -> Result<Box<dyn Engine>> + 'a>;

/// What a command runs against: its arguments, the output mode and a lazily opened engine session.
pub struct Context<'a> {
    pub args: &'a ParsedArgs,
    /// `--json`: print values as JSON instead of their human rendering.
    pub json: bool,
    connector: Connector<'a>,
    engine: Option<Box<dyn Engine>>,
}

impl<'a> Context<'a> {
    /// A context whose session is opened by `connector` on first use.
    pub fn new(args: &'a ParsedArgs, connector: Connector<'a>) -> Self {
        Self { args, json: args.has("json"), connector, engine: None }
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

    fn request(&mut self, command: &str, payload: Value) -> Result<Value> {
        self.engine()?.request(command, payload)
    }
}

/// An argument that must be present and non-empty.
fn need(value: Option<&str>, what: &str) -> Result<String> {
    match value {
        Some(v) if !v.is_empty() => Ok(v.to_string()),
        _ => Err(CliError::msg(format!("missing {what}"))),
    }
}

fn entities(response: &Value) -> Vec<Value> {
    response.get("entities").and_then(Value::as_array).cloned().unwrap_or_default()
}

/// `{...row, key: value}`: the row's own keys in order, `key` replaced in place or added last.
fn with_field(row: &Value, key: &str, value: Value) -> Value {
    let mut map: Map<String, Value> = row.as_object().cloned().unwrap_or_default();
    map.insert(key.to_string(), value);
    Value::Object(map)
}

/// The workspace a command acts on: `--workspace`, else the engine's active one.
fn workspace_id(ctx: &mut Context) -> Result<String> {
    if let Some(explicit) = ctx.args.flag("workspace") {
        return Ok(explicit.to_string());
    }
    let config = ctx.request("config.get", json!({}))?;
    Ok(js_template(config.get("activeWorkspaceId")))
}

fn list_entities(ctx: &mut Context, kind: &str) -> Result<Vec<Value>> {
    let workspace = workspace_id(ctx)?;
    let response = ctx.request("entity.list", json!({ "workspaceId": workspace, "kind": kind }))?;
    Ok(entities(&response))
}

/// `-H "name: value"` flags as a header map.
fn headers_from(args: &ParsedArgs) -> Result<Value> {
    let mut headers = Map::new();
    for header in args.flags("header") {
        match header.find(':') {
            Some(i) if i > 0 => {
                headers.insert(header[..i].trim().to_string(), Value::String(header[i + 1..].trim().to_string()));
            }
            _ => return Err(CliError::msg(format!("header \"{header}\" must be \"name: value\""))),
        }
    }
    Ok(Value::Object(headers))
}

fn body_from(args: &ParsedArgs) -> Result<String> {
    match args.flag("body-file") {
        Some(file) => std::fs::read_to_string(file).map_err(|e| CliError::msg(format!("cannot read {file}: {e}"))),
        None => Ok(args.flag("body").unwrap_or_default().to_string()),
    }
}

fn created_id(response: &Value) -> String {
    let id = response.get("id").filter(|v| !v.is_null()).or_else(|| response.pointer("/entity/id"));
    id.map(js_string).unwrap_or_else(|| "?".to_string())
}

/// `vault list`: the vaults of the workspace and their secrets' names. Values never come to the CLI here.
pub fn vaults(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    match sub.unwrap_or("list") {
        "list" => {
            let workspace = workspace_id(ctx)?;
            let listed = ctx.request("vault.list", json!({ "workspaceId": workspace }))?;
            let rows: Vec<Value> = listed
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| {
                    let names = v["secrets"].as_array().into_iter().flatten().filter_map(Value::as_str);
                    with_field(v, "secretNames", json!(names.collect::<Vec<_>>().join(", ")))
                })
                .collect();
            Ok(ctx.print(&listed, || table(&rows, &["name", "state", "secretNames"])))
        }
        "create" => {
            let name = need(ctx.args.positional(2), "vault name")?;
            let workspace = workspace_id(ctx)?;
            let passphrase = new_passphrase(ctx.args, &name)?;
            let created = ctx.request(
                "vault.create",
                json!({ "workspaceId": workspace, "name": name, "passphrase": passphrase }),
            )?;
            Ok(ctx.print(&created, || format!("Created vault {name}. It is unsealed until the engine restarts.")))
        }
        // The value is typed hidden (or piped with --value-stdin) and goes straight to the engine.
        "set" => {
            let (vault, secret) = (need(ctx.args.positional(2), "vault name")?, need(ctx.args.positional(3), "secret name")?);
            let workspace = workspace_id(ctx)?;
            let listed = ctx.request("vault.list", json!({ "workspaceId": workspace }))?;
            let id = unlock::find_vault(&listed, &vault)?;
            let passphrase = unlock::passphrase(ctx.args, &vault, 0)?;
            ctx.request("vault.unseal", json!({ "workspaceId": workspace, "id": id, "passphrase": passphrase }))?;
            let value = if ctx.args.has("value-stdin") {
                prompt::stdin_line()?
            } else {
                prompt::secret(&format!("Value of {secret}: "))?
            };
            let saved = ctx.request(
                "vault.setSecret",
                json!({ "workspaceId": workspace, "id": id, "name": secret, "value": value }),
            )?;
            Ok(ctx.print(&saved, || format!("Saved {{{{{vault}.{secret}}}}}")))
        }
        "unset" => {
            let (vault, secret) = (need(ctx.args.positional(2), "vault name")?, need(ctx.args.positional(3), "secret name")?);
            let workspace = workspace_id(ctx)?;
            let listed = ctx.request("vault.list", json!({ "workspaceId": workspace }))?;
            let id = unlock::find_vault(&listed, &vault)?;
            let removed =
                ctx.request("vault.deleteSecret", json!({ "workspaceId": workspace, "id": id, "name": secret }))?;
            Ok(ctx.print(&removed, || format!("Removed {secret} from {vault}")))
        }
        other => Err(CliError::msg(format!("unknown: vault {other}"))),
    }
}

/// The passphrase for a new vault: from the environment or stdin, or typed twice.
fn new_passphrase(args: &ParsedArgs, vault: &str) -> Result<String> {
    if args.has("passphrase-stdin") || std::env::var_os("BIFURC_VAULT_PASSPHRASE").is_some() {
        return unlock::passphrase(args, vault, 0);
    }
    let first = prompt::secret(&format!("New passphrase for {vault} (at least 8 characters): "))?;
    if first != prompt::secret("Repeat the passphrase: ")? {
        return Err(CliError::msg("the passphrases differ"));
    }
    Ok(first)
}

/// `workspaces list|use`.
pub fn workspaces(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    ctx.engine()?;
    match sub.unwrap_or("list") {
        "list" => {
            let config = ctx.request("config.get", json!({}))?;
            let active = config.get("activeWorkspaceId").cloned().unwrap_or(Value::Null);
            let rows: Vec<Value> = config
                .get("workspaces")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|w| with_field(w, "active", json!(if w.get("id") == Some(&active) { "*" } else { "" })))
                .collect();
            Ok(ctx.print(&Value::Array(rows.clone()), || table(&rows, &["active", "id", "name"])))
        }
        "use" => {
            let id = need(ctx.args.positional(2), "workspace id")?;
            ctx.request("workspace.setActive", json!({ "id": id }))?;
            Ok(ctx.print(&json!({ "ok": true, "activeWorkspaceId": id }), || format!("Active workspace: {id}")))
        }
        "add" => {
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
        "import" => {
            let repo = need(ctx.args.positional(2), "repository (owner/name)")?;
            let account = account_id(ctx, &need(ctx.args.flag("account"), "--account <login>")?)?;
            let result = refused(ctx.request("workspace.import", json!({ "accountId": account, "repo": repo }))?)?;
            Ok(ctx.print(&result, || {
                format!(
                    "Imported {repo} as workspace {} ({})",
                    js_template(result.pointer("/workspace/id")),
                    js_template(result.pointer("/workspace/name"))
                )
            }))
        }
        "publish" => {
            let name = need(ctx.args.positional(2), "new repository name")?;
            let account = account_id(ctx, &need(ctx.args.flag("account"), "--account <login>")?)?;
            let workspace = workspace_id(ctx)?;
            let payload = json!({
                "workspaceId": workspace, "accountId": account, "repoName": name, "private": !ctx.args.has("public"),
                "syncAll": ctx.args.has("all"),
            });
            let result = refused(ctx.request("sync.publish", payload)?)?;
            Ok(ctx.print(&result, || format!("Published to {}", js_template(result.pointer("/repo/fullName")))))
        }
        "sync" => {
            let workspace = workspace_id(ctx)?;
            let result = refused(ctx.request("sync.all", json!({ "workspaceId": workspace }))?)?;
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
                    line.push_str(&format!("\n{} deletion(s) still wait to be confirmed or reverted", count("pendingDeletions")));
                }
                line
            }))
        }
        other => Err(CliError::msg(format!("unknown: workspaces {other}"))),
    }
}

/// A command's `{ ok: false, error }` answer as an error.
fn refused(result: Value) -> Result<Value> {
    match result.get("ok").and_then(Value::as_bool) {
        Some(false) => Err(CliError::msg(js_template(result.get("error")))),
        _ => Ok(result),
    }
}

/// The id of a signed-in account, named by its id or its login.
fn account_id(ctx: &mut Context, wanted: &str) -> Result<String> {
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
        .ok_or_else(|| CliError::msg(format!("no account \"{wanted}\" is signed in (see: bifurc account list)")))
}

/// `account list|login|logout|repos`: the GitHub accounts workspaces sync as.
pub fn account(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    ctx.engine()?;
    match sub.unwrap_or("list") {
        "list" => {
            let listed = ctx.request("account.list", json!({}))?;
            let rows: Vec<Value> = listed.get("accounts").and_then(Value::as_array).cloned().unwrap_or_default();
            Ok(ctx.print(&Value::Array(rows.clone()), || table(&rows, &["login", "host", "signedIn", "id"])))
        }
        "login" => {
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
        "logout" => {
            let id = account_id(ctx, &need(ctx.args.positional(2), "account login")?)?;
            // Signing out takes the account's workspaces off this device, so what only this device has is
            // named first.
            let listed = ctx.request("account.list", json!({}))?;
            let account = listed
                .get("accounts")
                .and_then(Value::as_array)
                .and_then(|accounts| accounts.iter().find(|a| a.get("id").and_then(Value::as_str) == Some(&id)))
                .cloned()
                .unwrap_or(Value::Null);
            let unsynced: Vec<String> = account
                .get("workspaces")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|ws| {
                    ws.get("repo").is_none_or(Value::is_null)
                        || ws.get("changes").and_then(Value::as_u64).unwrap_or(0) > 0
                        || ws.get("unpushed").and_then(Value::as_bool).unwrap_or(false)
                })
                .map(|ws| js_template(ws.get("name")))
                .collect();
            if !unsynced.is_empty() && !ctx.args.has("force") {
                return Err(CliError::msg(format!(
                    "signing out removes this account's workspaces from this device, and these have changes that were never synced: {}. Sync them first, or pass --force to remove them anyway",
                    unsynced.join(", ")
                )));
            }
            let result = ctx.request("account.logout", json!({ "accountId": id }))?;
            let removed = result.get("removedWorkspaces").and_then(Value::as_array).map_or(0, Vec::len);
            Ok(ctx.print(&result, || format!("Signed out {id}; removed {removed} workspace(s) from this device")))
        }
        "repos" => {
            let id = account_id(ctx, &need(ctx.args.positional(2), "account login")?)?;
            let result = refused(ctx.request("account.repos", json!({ "accountId": id }))?)?;
            let rows: Vec<Value> = result.get("repos").and_then(Value::as_array).cloned().unwrap_or_default();
            Ok(ctx.print(&Value::Array(rows.clone()), || table(&rows, &["fullName", "private", "workspaceId"])))
        }
        other => Err(CliError::msg(format!("unknown: account {other}"))),
    }
}

/// `env list|show|use` for the active workspace. `Global` is always applied underneath; the selected one
/// overrides it. The selection is the engine's, so it is shared with the desktop UI.
pub fn env(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    ctx.engine()?;
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
        "show" => {
            let merged = ctx.request("env.getMerged", json!({ "workspaceId": workspace }))?;
            Ok(ctx.print(&merged, || json(&merged)))
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
                .ok_or_else(|| CliError::msg(format!("no environment \"{want}\" (see: bifurc env list)")))?;
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
        other => Err(CliError::msg(format!("unknown: env {other}"))),
    }
}

/// `mappings list|add|remove|enable|disable`.
pub fn mappings(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    ctx.engine()?;
    let args = ctx.args;
    match sub.unwrap_or("list") {
        "list" => {
            let rows = list_entities(ctx, "mappings")?;
            Ok(ctx.print(&Value::Array(rows.clone()), || table(&rows, &["id", "domain", "target", "label"])))
        }
        "add" => {
            let name = need(args.positional(2), "domain, e.g. api or api.localhost")?.to_lowercase();
            let domain = if name.ends_with(".localhost") { name } else { format!("{name}.localhost") };
            let target = need(args.positional(3), "target, e.g. 127.0.0.1:3000 or http://host:port")?;
            let workspace = workspace_id(ctx)?;
            let response = ctx.request(
                "entity.create",
                json!({
                    "kind": "mappings",
                    "workspaceId": workspace,
                    "entity": {
                        "domain": domain,
                        "target": target,
                        "label": args.flag("label").unwrap_or_default(),
                        "enabled": true,
                    },
                }),
            )?;
            Ok(ctx.print(&response, || format!("Mapped http://{domain} → {target}  (id {})", created_id(&response))))
        }
        "remove" => {
            let id = need(args.positional(2), "mapping id")?;
            ctx.request("entity.delete", json!({ "kind": "mappings", "id": id }))?;
            Ok(ctx.print(&json!({ "ok": true }), || format!("Removed mapping {id}")))
        }
        sub @ ("enable" | "disable") => {
            let id = need(args.positional(2), "mapping id")?;
            let workspace = workspace_id(ctx)?;
            let response = ctx.request(
                "entity.setEnabled",
                json!({ "workspaceId": workspace, "kind": "mappings", "id": id, "enabled": sub == "enable" }),
            )?;
            Ok(ctx
                .print(&response, || format!("{} mapping {id}", if sub == "enable" { "Enabled" } else { "Disabled" })))
        }
        other => Err(CliError::msg(format!("unknown: mappings {other}"))),
    }
}

/// `services list`.
pub fn services(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    if let Some(other) = sub.filter(|s| *s != "list") {
        return Err(CliError::msg(format!("unknown: services {other}")));
    }
    let response = ctx.request("services.discover", json!({}))?;
    let rows = response.as_array().cloned().unwrap_or_default();
    Ok(ctx.print(&response, || table(&rows, &["port", "pid", "processName", "address"])))
}

/// `requests list|show|add|send`.
pub fn requests(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    ctx.engine()?;
    let args = ctx.args;
    match sub.unwrap_or("list") {
        "list" => {
            let rows = list_entities(ctx, "requests")?;
            Ok(ctx.print(&Value::Array(rows.clone()), || table(&rows, &["id", "method", "name", "url"])))
        }
        "show" => {
            let id = need(args.positional(2), "request id")?;
            let workspace = workspace_id(ctx)?;
            let response =
                ctx.request("entity.load", json!({ "workspaceId": workspace, "kind": "requests", "id": id }))?;
            match response.get("entity").filter(|e| !e.is_null()) {
                Some(entity) if response.get("ok").is_some_and(truthy) => Ok(ctx.print(entity, || json(entity))),
                _ => Err(CliError::msg(format!("no request {id}"))),
            }
        }
        "add" => {
            let url = need(args.flag("url").or(args.positional(2)), "--url")?;
            let method = args.flag("method").unwrap_or("GET").to_uppercase();
            let workspace = workspace_id(ctx)?;
            let response = ctx.request(
                "entity.create",
                json!({
                    "kind": "requests",
                    "workspaceId": workspace,
                    "entity": {
                        "name": args.flag("name").map(str::to_string).unwrap_or_else(|| format!("{method} {url}")),
                        "method": method,
                        "url": url,
                        "headers": headers_from(args)?,
                        "body": body_from(args)?,
                        "protocol": "rest",
                    },
                }),
            )?;
            Ok(ctx.print(&response, || format!("Saved request {}", created_id(&response))))
        }
        "send" => {
            let workspace = workspace_id(ctx)?;
            // A saved request goes by id — the engine loads it, resolves it and runs its scripts. A request
            // that is not saved goes as a full payload.
            let payload = match args.flag("url") {
                None => json!({
                    "workspaceId": workspace,
                    "id": need(args.positional(2), "request id, or --url for an unsaved request")?,
                }),
                Some(url) => json!({
                    "workspaceId": workspace,
                    "request": {
                        "method": args.flag("method").unwrap_or("GET").to_uppercase(),
                        "url": url,
                        "headers": headers_from(args)?,
                        "body": body_from(args)?,
                    },
                    "scripts": {},
                }),
            };
            let response = ctx.request("request.send", payload)?;
            Ok(ctx.print(&response, || describe_send(&response)))
        }
        other => Err(CliError::msg(format!("unknown: requests {other}"))),
    }
}

/// `grpc list|reflect|call|send`.
pub fn grpc(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    ctx.engine()?;
    let args = ctx.args;
    // A result that says `ok: false` fails the command, unless the caller asked for the JSON.
    let checked = |ctx: &Context, response: Value| -> Result<Value> {
        match response.get("ok").and_then(Value::as_bool) {
            Some(false) if !ctx.json => Err(CliError::msg(
                response.get("error").and_then(Value::as_str).unwrap_or("the gRPC call failed").to_string(),
            )),
            _ => Ok(response),
        }
    };
    match sub.unwrap_or("list") {
        "list" => {
            let rows = list_entities(ctx, "grpcRequests")?;
            Ok(ctx.print(&Value::Array(rows.clone()), || {
                table(&rows, &["id", "name", "serverAddress", "serviceName", "methodName"])
            }))
        }
        "reflect" => {
            let address = need(args.positional(2).or(args.flag("server")), "server address")?;
            let response = ctx.request("grpc.reflect", json!({ "serverAddress": address }))?;
            let response = checked(ctx, response)?;
            Ok(ctx.print(&response, || describe_reflection(&response)))
        }
        "call" => {
            let address = need(args.positional(2).or(args.flag("server")), "server address")?;
            // `package.Service/Method` is one word; `package.Service Method` is two.
            let (service, method) = match (args.positional(3), args.positional(4)) {
                (Some(path), None) => match path.trim_start_matches('/').rsplit_once('/') {
                    Some((service, method)) => (service.to_string(), method.to_string()),
                    None => return Err(CliError::msg("the method is \"package.Service/Method\"")),
                },
                (Some(service), Some(method)) => (service.to_string(), method.to_string()),
                _ => return Err(CliError::msg("missing method (package.Service/Method)")),
            };
            let proto = args.flag("proto").filter(|p| !p.is_empty());
            let mut payload = json!({
                "serverAddress": address,
                "serviceName": service,
                "methodName": method,
                "requestBody": body_from(args)?,
                "metadata": headers_from(args)?,
                "protoFileId": proto,
                "useReflection": proto.is_none(),
            });
            if proto.is_some() {
                payload["workspaceId"] = json!(workspace_id(ctx)?);
            }
            let response = ctx.request("grpc.execute", payload)?;
            let response = checked(ctx, response)?;
            Ok(ctx.print(&response, || describe_grpc(&response)))
        }
        "send" => {
            let workspace = workspace_id(ctx)?;
            let id = need(args.positional(2), "gRPC request id")?;
            let response = ctx.request("grpc.send", json!({ "workspaceId": workspace, "id": id }))?;
            let response = checked(ctx, response)?;
            Ok(ctx.print(&response, || describe_grpc(&response)))
        }
        other => Err(CliError::msg(format!("unknown: grpc {other}"))),
    }
}

/// `call <command> [json]`: any protocol command with a JSON payload.
pub fn call(ctx: &mut Context) -> Result<String> {
    let command = need(ctx.args.positional(1), "command name (see: bifurc commands)")?;
    if !is_known_command(&command) {
        return Err(CliError::msg(format!("unknown command \"{command}\" (see: bifurc commands)")));
    }
    let raw = ctx.args.positional(2).or(ctx.args.flag("payload")).unwrap_or("{}");
    let payload: Value =
        serde_json::from_str(raw).map_err(|_| CliError::msg(format!("payload is not valid JSON: {raw}")))?;
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
    ctx.engine()?;
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
    use crate::{args::parse_args, discovery::RuntimeInfo};
    use std::{cell::RefCell, rc::Rc};

    type Calls = Rc<RefCell<Vec<(String, Value)>>>;

    /// An engine that answers each command from a table and records what it was asked.
    struct Fake {
        info: RuntimeInfo,
        answers: Vec<(&'static str, Result<Value>)>,
        calls: Calls,
    }

    impl Engine for Fake {
        fn info(&self) -> &RuntimeInfo {
            &self.info
        }

        fn request(&mut self, command: &str, payload: Value) -> Result<Value> {
            self.calls.borrow_mut().push((command.to_string(), payload));
            match self.answers.iter().find(|(name, _)| *name == command) {
                Some((_, answer)) => answer.clone(),
                None => Err(CliError::Engine { code: "UNKNOWN_COMMAND".into(), message: command.into() }),
            }
        }
    }

    fn info() -> RuntimeInfo {
        RuntimeInfo::from_json(json!({
            "pid": 5, "transport": "socket", "address": "addr", "dataDir": "/d", "engineVersion": "0.4.0"
        }))
        .unwrap()
    }

    fn config() -> Value {
        json!({
            "activeWorkspaceId": "w1",
            "activeEnvironmentId": "e2",
            "workspaces": [{"id": "w1", "name": "Default"}, {"id": "w2", "name": "Other"}],
        })
    }

    fn envs() -> Value {
        json!({"entities": [
            {"id": "__global__", "name": "Global", "variables": [{"k": "a"}]},
            {"id": "e1", "name": "Dev", "variables": []},
            {"id": "e2", "name": "Prod", "variables": [{"k": "a"}, {"k": "b"}]},
        ]})
    }

    /// Run `f` for the arguments, with an engine that gives `answers`; returns the result and the calls made.
    fn run(
        argv: &[&str],
        answers: Vec<(&'static str, Result<Value>)>,
        f: impl FnOnce(&mut Context) -> Result<String>,
    ) -> (Result<String>, Vec<(String, Value)>) {
        let args = parse_args(&argv.iter().map(|a| a.to_string()).collect::<Vec<_>>()).unwrap();
        let calls: Calls = Rc::default();
        let recorded = Rc::clone(&calls);
        let mut answers = Some(answers);
        let connector: Connector = Box::new(move || {
            let fake =
                Fake { info: info(), answers: answers.take().expect("opened once"), calls: Rc::clone(&recorded) };
            Ok(Box::new(fake) as Box<dyn Engine>)
        });
        let mut ctx = Context::new(&args, connector);
        let result = f(&mut ctx);
        let calls = calls.borrow().clone();
        (result, calls)
    }

    fn sub<'a>(argv: &'a [&'a str]) -> Option<&'a str> {
        argv.get(1).copied()
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
    fn workspaces_use_sets_the_active_workspace() {
        let argv = ["workspaces", "use", "w2"];
        let (out, calls) = run(&argv, vec![("workspace.setActive", Ok(json!({})))], |c| workspaces(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Active workspace: w2");
        assert_eq!(calls, vec![("workspace.setActive".to_string(), json!({"id": "w2"}))]);
    }

    #[test]
    fn workspaces_use_needs_an_id_and_unknown_subcommands_fail() {
        let argv = ["workspaces", "use"];
        let (out, _) = run(&argv, vec![], |c| workspaces(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("missing workspace id"));
        let argv = ["workspaces", "frob"];
        let (out, _) = run(&argv, vec![], |c| workspaces(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("unknown: workspaces frob"));
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
    fn env_show_prints_the_merged_variables() {
        let argv = ["env", "show"];
        let answers = vec![("config.get", Ok(config())), ("env.getMerged", Ok(json!({"a": "1"})))];
        let (out, calls) = run(&argv, answers, |c| env(c, sub(&argv)));
        assert_eq!(out.unwrap(), "{\n  \"a\": \"1\"\n}");
        assert_eq!(calls[1].1, json!({"workspaceId": "w1"}));
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
        assert_eq!(out.unwrap_err(), CliError::msg("no environment \"nope\" (see: bifurc env list)"));

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
        assert_eq!(out.unwrap_err(), CliError::msg("missing environment id or name"));
    }

    #[test]
    fn mappings_list_is_a_table_of_the_active_workspaces_mappings() {
        let argv = ["mappings"];
        let listing =
            json!({"entities": [{"id": "m1", "domain": "api.localhost", "target": "127.0.0.1:3000", "label": "API"}]});
        let answers = vec![("config.get", Ok(config())), ("entity.list", Ok(listing))];
        let (out, calls) = run(&argv, answers, |c| mappings(c, None));
        assert_eq!(out.unwrap(), "ID  DOMAIN         TARGET          LABEL\nm1  api.localhost  127.0.0.1:3000  API");
        assert_eq!(calls[1].1, json!({"workspaceId": "w1", "kind": "mappings"}));
    }

    #[test]
    fn mappings_add_appends_localhost_and_lowercases() {
        let argv = ["mappings", "add", "API", "127.0.0.1:3000", "--label", "My API"];
        let answers = vec![("config.get", Ok(config())), ("entity.create", Ok(json!({"id": "m9"})))];
        let (out, calls) = run(&argv, answers, |c| mappings(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Mapped http://api.localhost → 127.0.0.1:3000  (id m9)");
        assert_eq!(
            calls[1].1,
            json!({"kind": "mappings", "workspaceId": "w1",
                   "entity": {"domain": "api.localhost", "target": "127.0.0.1:3000", "label": "My API", "enabled": true}})
        );
    }

    #[test]
    fn mappings_add_keeps_a_full_domain_and_reads_a_nested_id() {
        let argv = ["mappings", "add", "Api.LOCALHOST", "t", "-w", "w2"];
        let answers = vec![("entity.create", Ok(json!({"entity": {"id": "n1"}})))];
        let (out, calls) = run(&argv, answers, |c| mappings(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Mapped http://api.localhost → t  (id n1)");
        assert_eq!(calls[0].1["workspaceId"], "w2");
        assert_eq!(calls[0].1["entity"]["label"], "");
    }

    #[test]
    fn mappings_add_needs_domain_and_target() {
        let argv = ["mappings", "add"];
        let (out, _) = run(&argv, vec![], |c| mappings(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("missing domain, e.g. api or api.localhost"));
        let argv = ["mappings", "add", "api"];
        let (out, _) = run(&argv, vec![], |c| mappings(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("missing target, e.g. 127.0.0.1:3000 or http://host:port"));
    }

    #[test]
    fn mappings_remove_enable_and_disable() {
        let argv = ["mappings", "remove", "m1"];
        let (out, calls) = run(&argv, vec![("entity.delete", Ok(json!({})))], |c| mappings(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Removed mapping m1");
        assert_eq!(calls[0].1, json!({"kind": "mappings", "id": "m1"}));

        for (verb, word, enabled) in [("enable", "Enabled", true), ("disable", "Disabled", false)] {
            let argv = ["mappings", verb, "m1"];
            let answers = vec![("config.get", Ok(config())), ("entity.setEnabled", Ok(json!({"ok": true})))];
            let (out, calls) = run(&argv, answers, |c| mappings(c, sub(&argv)));
            assert_eq!(out.unwrap(), format!("{word} mapping m1"));
            assert_eq!(calls[1].1, json!({"workspaceId": "w1", "kind": "mappings", "id": "m1", "enabled": enabled}));
        }

        let argv = ["mappings", "remove"];
        let (out, _) = run(&argv, vec![], |c| mappings(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("missing mapping id"));
        let argv = ["mappings", "frob"];
        let (out, _) = run(&argv, vec![], |c| mappings(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("unknown: mappings frob"));
    }

    #[test]
    fn services_list_is_a_table_and_rejects_other_subcommands() {
        let argv = ["services"];
        let found = json!([{"port": 3000, "pid": 12, "processName": "node", "address": "127.0.0.1"}]);
        let (out, _) = run(&argv, vec![("services.discover", Ok(found))], |c| services(c, None));
        assert_eq!(out.unwrap(), "PORT  PID  PROCESSNAME  ADDRESS\n3000  12   node         127.0.0.1");
        let argv = ["services", "kill"];
        let (out, calls) = run(&argv, vec![], |c| services(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("unknown: services kill"));
        assert!(calls.is_empty());
    }

    #[test]
    fn requests_list_is_a_table() {
        let argv = ["requests"];
        let listing = json!({"entities": [{"id": "r1", "method": "GET", "name": "Ping", "url": "http://x"}]});
        let answers = vec![("config.get", Ok(config())), ("entity.list", Ok(listing))];
        let (out, calls) = run(&argv, answers, |c| requests(c, None));
        assert_eq!(out.unwrap(), "ID  METHOD  NAME  URL\nr1  GET     Ping  http://x");
        assert_eq!(calls[1].1, json!({"workspaceId": "w1", "kind": "requests"}));
    }

    #[test]
    fn requests_show_prints_the_entity_or_fails() {
        let argv = ["requests", "show", "r1"];
        let loaded = json!({"ok": true, "entity": {"id": "r1"}});
        let answers = vec![("config.get", Ok(config())), ("entity.load", Ok(loaded))];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "{\n  \"id\": \"r1\"\n}");
        assert_eq!(calls[1].1, json!({"workspaceId": "w1", "kind": "requests", "id": "r1"}));

        let answers = vec![("config.get", Ok(config())), ("entity.load", Ok(json!({"ok": false})))];
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("no request r1"));

        let argv = ["requests", "show"];
        let (out, _) = run(&argv, vec![], |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("missing request id"));
    }

    #[test]
    fn requests_add_builds_the_entity_from_flags() {
        let argv =
            ["requests", "add", "--url", "http://x/a", "-X", "post", "-H", "Accept: a/b", "-H", "X-N:1", "-d", "{}"];
        let answers = vec![("config.get", Ok(config())), ("entity.create", Ok(json!({"id": "r7"})))];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Saved request r7");
        assert_eq!(
            calls[1].1,
            json!({"kind": "requests", "workspaceId": "w1", "entity": {
                "name": "POST http://x/a", "method": "POST", "url": "http://x/a",
                "headers": {"Accept": "a/b", "X-N": "1"}, "body": "{}", "protocol": "rest"}})
        );
    }

    #[test]
    fn requests_add_takes_the_url_positionally_and_a_name_and_a_body_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("body.txt");
        std::fs::write(&file, "from file").unwrap();
        let file = file.to_string_lossy().into_owned();
        let argv = ["requests", "add", "http://y", "--name", "Mine", "--body-file", &file, "--body", "ignored"];
        let answers = vec![("config.get", Ok(config())), ("entity.create", Ok(json!({"entity": {"id": "r8"}})))];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Saved request r8");
        assert_eq!(calls[1].1["entity"]["name"], "Mine");
        assert_eq!(calls[1].1["entity"]["url"], "http://y");
        assert_eq!(calls[1].1["entity"]["body"], "from file");
        assert_eq!(calls[1].1["entity"]["method"], "GET");
    }

    #[test]
    fn requests_add_rejects_a_missing_url_and_a_bad_header_and_a_missing_file() {
        let argv = ["requests", "add"];
        let (out, _) = run(&argv, vec![], |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("missing --url"));

        let argv = ["requests", "add", "--url", "u", "-H", "nocolon"];
        let (out, _) = run(&argv, vec![("config.get", Ok(config()))], |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("header \"nocolon\" must be \"name: value\""));

        let argv = ["requests", "add", "--url", "u", "-H", ":v"];
        let (out, _) = run(&argv, vec![("config.get", Ok(config()))], |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("header \":v\" must be \"name: value\""));

        let argv = ["requests", "add", "--url", "u", "--body-file", "/definitely/not/here"];
        let (out, _) = run(&argv, vec![("config.get", Ok(config()))], |c| requests(c, sub(&argv)));
        assert!(out.unwrap_err().to_string().starts_with("cannot read /definitely/not/here"));
    }

    #[test]
    fn requests_send_by_id_sends_only_the_id() {
        let argv = ["requests", "send", "r1"];
        let sent = json!({"status": 200, "durationMs": 3, "headers": {"a": "b"}, "body": "b2s="});
        let answers = vec![("config.get", Ok(config())), ("request.send", Ok(sent))];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "HTTP 200  (3 ms)\na: b\n\nok");
        assert_eq!(calls[1], ("request.send".to_string(), json!({"workspaceId": "w1", "id": "r1"})));
    }

    #[test]
    fn requests_send_inline_sends_the_whole_request() {
        let argv = ["requests", "send", "--url", "http://x", "-X", "put", "-H", "a: 1", "-d", "hi", "-w", "w3"];
        let sent = json!({"status": 201, "durationMs": 1, "headers": {}, "body": ""});
        let (out, calls) = run(&argv, vec![("request.send", Ok(sent))], |c| requests(c, sub(&argv)));
        assert!(out.unwrap().starts_with("HTTP 201"));
        assert_eq!(
            calls[0].1,
            json!({"workspaceId": "w3", "scripts": {},
                   "request": {"method": "PUT", "url": "http://x", "headers": {"a": "1"}, "body": "hi"}})
        );
    }

    #[test]
    fn requests_send_without_id_or_url_fails() {
        let argv = ["requests", "send"];
        let (out, _) = run(&argv, vec![("config.get", Ok(config()))], |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("missing request id, or --url for an unsaved request"));
        let argv = ["requests", "frob"];
        let (out, _) = run(&argv, vec![], |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("unknown: requests frob"));
    }

    #[test]
    fn grpc_call_sends_an_execute_by_reflection() {
        let argv = ["grpc", "call", "localhost:50051", "pkg.Svc/Do", "-d", "{\"a\":1}", "-H", "x-k: v"];
        let reply = json!({"ok": true, "status": 0, "durationMs": 3, "metadata": {"m": "1"}, "responses": ["{}"]});
        let (out, calls) = run(&argv, vec![("grpc.execute", Ok(reply))], |c| grpc(c, sub(&argv)));
        assert_eq!(out.unwrap(), "gRPC OK  (3 ms)\nm: 1\n\n{}");
        assert_eq!(
            calls[0],
            (
                "grpc.execute".to_string(),
                json!({"serverAddress": "localhost:50051", "serviceName": "pkg.Svc", "methodName": "Do",
                       "requestBody": "{\"a\":1}", "metadata": {"x-k": "v"}, "protoFileId": null, "useReflection": true})
            )
        );
    }

    #[test]
    fn grpc_call_failure_is_an_error_unless_json() {
        let argv = ["grpc", "call", "h:1", "S", "M"];
        let reply = json!({"ok": false, "error": "UNAVAILABLE: down", "status": 14});
        let (out, _) = run(&argv, vec![("grpc.execute", Ok(reply.clone()))], |c| grpc(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("UNAVAILABLE: down"));
        let argv = ["grpc", "call", "h:1", "S", "M", "--json"];
        let (out, _) = run(&argv, vec![("grpc.execute", Ok(reply))], |c| grpc(c, sub(&argv)));
        assert!(out.unwrap().contains("UNAVAILABLE"));
    }

    #[test]
    fn grpc_reflect_lists_methods_with_their_kind() {
        let argv = ["grpc", "reflect", "h:1"];
        let reply = json!({"ok": true, "services": [{"name": "p.S", "methods": [
            {"name": "A", "inputType": "p.I", "outputType": "p.O", "clientStreaming": false, "serverStreaming": true}]}]});
        let (out, _) = run(&argv, vec![("grpc.reflect", Ok(reply))], |c| grpc(c, sub(&argv)));
        assert_eq!(out.unwrap(), "p.S\n  A(p.I) -> p.O  [server streaming]");
    }

    #[test]
    fn grpc_send_goes_by_id() {
        let argv = ["grpc", "send", "g1"];
        let answers = vec![("config.get", Ok(config())), ("grpc.send", Ok(json!({"ok": true, "status": 0})))];
        let (_, calls) = run(&argv, answers, |c| grpc(c, sub(&argv)));
        assert_eq!(calls[1], ("grpc.send".to_string(), json!({"workspaceId": "w1", "id": "g1"})));
    }

    #[test]
    fn call_sends_the_payload_and_prints_the_result_as_json() {
        let argv = ["call", "server.status", r#"{"x":1}"#];
        let (out, calls) = run(&argv, vec![("server.status", Ok(json!({"running": true})))], |c| call(c));
        assert_eq!(out.unwrap(), "{\n  \"running\": true\n}");
        assert_eq!(calls, vec![("server.status".to_string(), json!({"x": 1}))]);
    }

    #[test]
    fn call_defaults_the_payload_and_accepts_it_as_a_flag() {
        let argv = ["call", "server.status"];
        let (_, calls) = run(&argv, vec![("server.status", Ok(json!({})))], |c| call(c));
        assert_eq!(calls[0].1, json!({}));
        let argv = ["call", "server.status", "--payload", "[1]"];
        let (_, calls) = run(&argv, vec![("server.status", Ok(json!({})))], |c| call(c));
        assert_eq!(calls[0].1, json!([1]));
    }

    #[test]
    fn call_validates_before_connecting() {
        let argv = ["call"];
        let (out, calls) = run(&argv, vec![], |c| call(c));
        assert_eq!(out.unwrap_err(), CliError::msg("missing command name (see: bifurc commands)"));
        let argv = ["call", "nope.nope"];
        let (out, _) = run(&argv, vec![], |c| call(c));
        assert_eq!(out.unwrap_err(), CliError::msg("unknown command \"nope.nope\" (see: bifurc commands)"));
        let argv = ["call", "server.status", "{bad"];
        let (out, calls2) = run(&argv, vec![], |c| call(c));
        assert_eq!(out.unwrap_err(), CliError::msg("payload is not valid JSON: {bad"));
        assert!(calls.is_empty() && calls2.is_empty());
    }

    #[test]
    fn call_passes_engine_errors_through_with_their_code() {
        let argv = ["call", "ws.list"];
        let (out, _) = run(&argv, vec![], |c| call(c));
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
        let (out, _) = run(&argv, answers, |c| status(c));
        assert_eq!(
            out.unwrap(),
            "engine     pid 5, socket addr, v0.4.0\ndata dir   /d\nproxy      listening on 8080\nworkspace  Default (w1)"
        );

        let argv = ["status", "--json"];
        let answers = vec![("server.status", Ok(json!({"running": false}))), ("config.get", Ok(config()))];
        let (out, _) = run(&argv, answers, |c| status(c));
        let parsed: Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!(parsed["engine"]["pid"], 5);
        assert_eq!(parsed["server"], json!({"running": false}));
        assert_eq!(parsed["activeWorkspace"], json!({"id": "w1", "name": "Default"}));
    }

    #[test]
    fn engine_errors_stop_a_command() {
        let argv = ["status"];
        let (out, _) = run(&argv, vec![], |c| status(c));
        assert_eq!(out.unwrap_err().line(), "bifurc: UNKNOWN_COMMAND: server.status");
    }
}
