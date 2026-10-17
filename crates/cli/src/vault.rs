//! `vault`: the workspace's vaults of encrypted secrets, and `vault run <vault> -- <command…>`, which starts a
//! program with a vault's secrets as environment variables.
//!
//! A vault is sealed or unsealed, and that is the engine's state: once unsealed, by the desktop app or by a
//! command here, it stays unsealed until it is sealed or the engine stops. So a passphrase is asked for only
//! when the vault a command needs is sealed; an unsealed one is used as it is.

use std::process::{Command, ExitStatus};

use serde_json::{Value, json};

use crate::{
    args::ParsedArgs,
    commands::{Context, need, unknown_verb, with_field, workspace_id},
    error::{CliError, Result},
    format::table,
    prompt,
};

const PASSPHRASE_ENV: &str = "BIFURC_VAULT_PASSPHRASE";
const NEW_PASSPHRASE_ENV: &str = "BIFURC_VAULT_NEW_PASSPHRASE";
const PROMPT_ATTEMPTS: usize = 3;

/// The vault named `name` (any case) in a `vault.list` answer.
pub fn find_vault<'v>(list: &'v Value, name: &str) -> Result<&'v Value> {
    let vaults = list.as_array().map(Vec::as_slice).unwrap_or_default();
    match vaults.iter().find(|v| v["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name))) {
        Some(vault) => Ok(vault),
        None => {
            let known: Vec<&str> = vaults.iter().filter_map(|v| v["name"].as_str()).collect();
            let hint = if known.is_empty() {
                "this workspace has no vaults".to_string()
            } else {
                format!("known: {}", known.join(", "))
            };
            Err(CliError::not_found(format!("no vault named \"{name}\" ({hint})")))
        }
    }
}

#[cfg(unix)]
fn exit_code(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status.code().or_else(|| status.signal().map(|signal| 128 + signal)).unwrap_or(1)
}

#[cfg(not(unix))]
fn exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

/// The passphrase from the environment, stdin or the terminal. `attempt` is 0 for the first ask.
fn passphrase(args: &ParsedArgs, vault: &str, attempt: usize) -> Result<String> {
    if attempt == 0 {
        if let Some(from_env) = std::env::var(PASSPHRASE_ENV).ok().filter(|p| !p.is_empty()) {
            return Ok(from_env);
        }
        if args.has("passphrase-stdin") {
            return prompt::stdin_line();
        }
    } else if args.has("passphrase-stdin") || std::env::var_os(PASSPHRASE_ENV).is_some() {
        return Err(CliError::msg("wrong passphrase"));
    }
    if attempt > 0 {
        eprintln!("bifurc: wrong passphrase, try again");
    }
    if !prompt::interactive() {
        return Err(CliError::needs_human(format!(
            "vault \"{vault}\" is sealed — ask the user to unseal it in Bifurc (or: bifurc vault unseal {vault} in a terminal)"
        )));
    }
    prompt::secret(&format!("Passphrase for vault {vault}: "))
}

/// The passphrase for a new vault: from the environment or stdin, or typed twice.
fn new_passphrase(args: &ParsedArgs, vault: &str) -> Result<String> {
    if args.has("passphrase-stdin") || std::env::var_os(PASSPHRASE_ENV).is_some() {
        return passphrase(args, vault, 0);
    }
    let first = prompt::secret(&format!("New passphrase for {vault} (at least 8 characters): "))?;
    if first != prompt::secret("Repeat the passphrase: ")? {
        return Err(CliError::msg("the passphrases differ"));
    }
    Ok(first)
}

/// The workspace and the vault the command's first operand names, as `vault.list` describes it.
fn named(ctx: &mut Context) -> Result<(String, Value)> {
    let name = need(ctx.args.positional(2), "vault name")?;
    let workspace = workspace_id(ctx)?;
    let listed = ctx.request("vault.list", json!({ "workspaceId": workspace }))?;
    let vault = find_vault(&listed, &name)?.clone();
    Ok((workspace, vault))
}

/// Unseal the vault if it is sealed, asking for the passphrase until it is right. An unsealed vault is left
/// as it is and nothing is asked. Returns whether it was sealed.
fn unseal(ctx: &mut Context, workspace: &str, vault: &Value) -> Result<bool> {
    if vault["state"] != "sealed" {
        return Ok(false);
    }
    let name = vault["name"].as_str().unwrap_or_default();
    let mut attempt = 0;
    loop {
        let pass = passphrase(ctx.args, name, attempt)?;
        let payload = json!({ "workspaceId": workspace, "id": vault["id"], "passphrase": pass });
        match ctx.request("vault.unseal", payload) {
            Err(CliError::Engine { code, message })
                if code == "FORBIDDEN" && !message.starts_with("Too many") && attempt + 1 < PROMPT_ATTEMPTS =>
            {
                attempt += 1;
            }
            Err(error) => return Err(error),
            Ok(_) => return Ok(true),
        }
    }
}

/// `vault list|create|set|unseal|seal|run`. Values never come to the CLI here; `run` is [`run`].
pub fn vaults(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    match sub.unwrap_or("list") {
        "list" => {
            ctx.accept(&[], 0)?;
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
            ctx.accept(&["passphrase-stdin"], 1)?;
            let name = need(ctx.args.positional(2), "vault name")?;
            let workspace = workspace_id(ctx)?;
            let passphrase = new_passphrase(ctx.args, &name)?;
            let created = ctx
                .request("vault.create", json!({ "workspaceId": workspace, "name": name, "passphrase": passphrase }))?;
            Ok(ctx.print(&created, || format!("Created vault {name}. It is unsealed until the engine restarts.")))
        }
        // The value is typed hidden (or piped with --value-stdin) and goes straight to the engine.
        "set" => {
            ctx.accept(&["passphrase-stdin", "value-stdin"], 2)?;
            let secret = need(ctx.args.positional(3), "secret name")?;
            let (workspace, vault) = named(ctx)?;
            unseal(ctx, &workspace, &vault)?;
            let value = if ctx.args.has("value-stdin") {
                prompt::stdin_line()?
            } else {
                prompt::secret(&format!("Value of {secret}: "))?
            };
            let saved = ctx.request(
                "vault.setSecret",
                json!({ "workspaceId": workspace, "id": vault["id"], "name": secret, "value": value }),
            )?;
            let name = vault["name"].as_str().unwrap_or_default();
            Ok(ctx.print(&saved, || format!("Saved {{{{{name}.{secret}}}}}")))
        }
        "unseal" => {
            ctx.accept(&["passphrase-stdin"], 1)?;
            let (workspace, vault) = named(ctx)?;
            let was_sealed = unseal(ctx, &workspace, &vault)?;
            let name = vault["name"].as_str().unwrap_or_default();
            let unsealed = with_field(&vault, "state", json!("unsealed"));
            Ok(ctx.print(&unsealed, || {
                if was_sealed {
                    format!("Unsealed vault {name}. It stays unsealed until it is sealed or the engine stops.")
                } else {
                    format!("Vault {name} is already unsealed")
                }
            }))
        }
        "seal" => {
            ctx.accept(&[], 1)?;
            let (workspace, vault) = named(ctx)?;
            let sealed = ctx.request("vault.seal", json!({ "workspaceId": workspace, "id": vault["id"] }))?;
            let name = vault["name"].as_str().unwrap_or_default();
            Ok(ctx.print(&sealed, || format!("Sealed vault {name}")))
        }
        "passphrase" => {
            ctx.accept(&["passphrase-stdin"], 1)?;
            let (workspace, vault) = named(ctx)?;
            let name = vault["name"].as_str().unwrap_or_default();
            let current = passphrase(ctx.args, name, 0)?;
            // The new one cannot come the way the current one does: its own variable, or typed twice.
            let new = match std::env::var(NEW_PASSPHRASE_ENV).ok().filter(|p| !p.is_empty()) {
                Some(from_env) => from_env,
                None => {
                    let first = prompt::secret(&format!("New passphrase for {name} (at least 8 characters): "))?;
                    if first != prompt::secret("Repeat the passphrase: ")? {
                        return Err(CliError::msg("the passphrases differ"));
                    }
                    first
                }
            };
            let changed = ctx.request(
                "vault.changePassphrase",
                json!({ "workspaceId": workspace, "id": vault["id"], "passphrase": current, "newPassphrase": new }),
            )?;
            Ok(ctx.print(&changed, || format!("Changed the passphrase of vault {name}")))
        }
        // With a command after `--` this is `run` below; here there was none.
        "run" => {
            let name = ctx.args.positional(2).unwrap_or("<vault>");
            Err(CliError::usage(format!("missing the command to run, e.g.: bifurc vault run {name} -- npm run dev")))
        }
        other => Err(unknown_verb("vault", other)),
    }
}

/// While the program runs, Ctrl+C belongs to it: this process stays up to pass on its exit code.
fn leave_interrupts_to_the_program() {
    #[cfg(windows)]
    {
        unsafe extern "system" fn handled(_: u32) -> i32 {
            1
        }
        // SAFETY: registers a handler that does nothing; unlike "ignore", it is not inherited by the child.
        unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(handled), 1);
        }
    }
    #[cfg(unix)]
    {
        extern "C" fn nothing(_: libc::c_int) {}
        // SAFETY: installs a handler that does nothing; unlike SIG_IGN it is reset to the default on exec.
        unsafe {
            libc::signal(libc::SIGINT, nothing as usize);
        }
    }
}

/// On Windows a program is found through PATH and PATHEXT by the shell, not by `CreateProcess`.
#[cfg(windows)]
fn resolve_program(program: &str) -> std::path::PathBuf {
    use std::path::{Path, PathBuf};
    let has_dir = program.contains(['/', '\\']);
    let dirs: Vec<PathBuf> = if has_dir {
        vec![PathBuf::new()]
    } else {
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default()
    };
    for dir in dirs {
        for extension in ["", ".exe", ".cmd", ".bat", ".com"] {
            let candidate = dir.join(format!("{program}{extension}"));
            if Path::new(&candidate).is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from(program)
}

#[cfg(not(windows))]
fn resolve_program(program: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(program)
}

/// The secrets of the vault `name`, unsealing it first if it is sealed.
pub fn secrets_of(ctx: &mut Context, workspace: &str, name: &str) -> Result<Vec<(String, String)>> {
    let listed = ctx.request("vault.list", json!({ "workspaceId": workspace }))?;
    let vault = find_vault(&listed, name)?.clone();
    unseal(ctx, workspace, &vault)?;
    let read = ctx.request("vault.secrets", json!({ "workspaceId": workspace, "id": vault["id"] }))?;
    Ok(read["secrets"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_string()))
        .collect())
}

/// The secrets of the vault `vault run` names.
fn secrets_for(ctx: &mut Context) -> Result<Vec<(String, String)>> {
    ctx.accept(&["passphrase-stdin"], 1)?;
    let name = need(ctx.args.positional(2), "vault name")?;
    let workspace = workspace_id(ctx)?;
    secrets_of(ctx, &workspace, &name)
}

/// `vault run <vault> -- <command…>`: run `command` with the vault's secrets as environment variables and
/// return its exit code. The passphrase is asked for only if the vault is sealed, and the vault is left
/// unsealed. `ctx` holds what came before the `--`.
pub fn run(ctx: &mut Context, command: &[String]) -> Result<i32> {
    let Some((program, program_args)) = command.split_first() else {
        let name = ctx.args.positional(2).unwrap_or("<vault>");
        return Err(CliError::usage(format!(
            "missing the command to run, e.g.: bifurc vault run {name} -- npm run dev"
        )));
    };
    let secrets = secrets_for(ctx)?;

    let mut child = Command::new(resolve_program(program));
    child.args(program_args).env_remove(PASSPHRASE_ENV);
    for (key, value) in &secrets {
        child.env(key, value);
    }
    leave_interrupts_to_the_program();
    let mut child = child.spawn().map_err(|e| CliError::msg(format!("cannot run \"{program}\": {e}")))?;
    let status = child.wait().map_err(|e| CliError::msg(format!("lost the program: {e}")))?;
    Ok(exit_code(status))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run as with_engine, sub};

    fn listed(state: &str) -> Value {
        json!([
            {"id": "a1", "name": "api_keys", "state": state, "secrets": ["TOKEN", "DB_URL"]},
            {"id": "b2", "name": "db", "state": "sealed", "secrets": []},
        ])
    }

    fn engine(state: &str) -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            ("vault.list", Ok(listed(state))),
            ("vault.unseal", Ok(json!({"id": "a1", "state": "unsealed"}))),
            ("vault.seal", Ok(json!({"id": "a1", "state": "sealed"}))),
            ("vault.secrets", Ok(json!({"id": "a1", "name": "api_keys", "secrets": {"TOKEN": "t0ken-value"}}))),
        ]
    }

    #[test]
    fn a_vault_is_found_by_name_in_any_case_and_a_miss_lists_the_known_ones() {
        let list = listed("unsealed");
        assert_eq!(find_vault(&list, "API_KEYS").unwrap()["id"], "a1");
        let err = find_vault(&list, "nope").unwrap_err().to_string();
        assert_eq!(err, "no vault named \"nope\" (known: api_keys, db)");
        assert!(find_vault(&json!([]), "x").unwrap_err().to_string().contains("no vaults"));
    }

    #[test]
    fn a_sealed_vault_without_a_terminal_needs_a_human() {
        // The prompt is real when run from a terminal; this is about the agent case.
        if prompt::interactive() || std::env::var_os(PASSPHRASE_ENV).is_some() {
            return;
        }
        let argv = ["vault", "unseal", "db"];
        let (out, calls) = with_engine(&argv, engine("sealed"), |c| vaults(c, sub(&argv)));
        let err = out.unwrap_err();
        assert_eq!(err.code(), 6);
        assert!(err.to_string().starts_with("vault \"db\" is sealed"), "{err}");
        assert!(payloads(&calls, "vault.unseal").is_empty());
    }

    #[test]
    fn list_says_sealed_or_unsealed_and_names_the_secrets() {
        let argv = ["vault", "list"];
        let (out, _) = with_engine(&argv, engine("unsealed"), |c| vaults(c, sub(&argv)));
        assert_eq!(
            out.unwrap(),
            "NAME      STATE     SECRETNAMES\napi_keys  unsealed  TOKEN, DB_URL\ndb        sealed"
        );
    }

    #[test]
    fn an_unsealed_vault_is_used_without_asking_for_anything() {
        let argv = ["vault", "run", "API_KEYS"];
        let (out, calls) = with_engine(&argv, engine("unsealed"), |c| {
            Ok(secrets_for(c)?.into_iter().map(|(name, value)| format!("{name}={value}")).collect())
        });
        assert_eq!(out.unwrap(), "TOKEN=t0ken-value");
        assert!(payloads(&calls, "vault.unseal").is_empty());
        assert_eq!(payloads(&calls, "vault.secrets"), [json!({"workspaceId": "w1", "id": "a1"})]);

        let argv = ["vault", "unseal", "api_keys"];
        let (out, calls) = with_engine(&argv, engine("unsealed"), |c| vaults(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Vault api_keys is already unsealed");
        assert!(payloads(&calls, "vault.unseal").is_empty());
    }

    #[test]
    fn seal_goes_by_the_vaults_id() {
        let argv = ["vault", "seal", "api_keys"];
        let (out, calls) = with_engine(&argv, engine("unsealed"), |c| vaults(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Sealed vault api_keys");
        assert_eq!(payloads(&calls, "vault.seal"), [json!({"workspaceId": "w1", "id": "a1"})]);
    }

    #[test]
    fn run_needs_a_vault_that_exists_and_a_command() {
        let argv = ["vault", "run", "nope"];
        let command = ["npm".to_string()];
        let (out, calls) = with_engine(&argv, engine("unsealed"), |c| run(c, &command).map(|code| code.to_string()));
        assert_eq!(out.unwrap_err(), CliError::not_found("no vault named \"nope\" (known: api_keys, db)"));
        assert!(payloads(&calls, "vault.secrets").is_empty());

        let argv = ["vault", "run", "keys"];
        let (out, calls) = with_engine(&argv, vec![], |c| run(c, &[]).map(|code| code.to_string()));
        assert_eq!(
            out.unwrap_err(),
            CliError::usage("missing the command to run, e.g.: bifurc vault run keys -- npm run dev")
        );
        assert!(calls.is_empty());
        let (out, _) = with_engine(&argv, vec![], |c| vaults(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::usage("missing the command to run, e.g.: bifurc vault run keys -- npm run dev")
        );

        let argv = ["vault", "run"];
        let (out, _) = with_engine(&argv, vec![], |c| run(c, &command).map(|code| code.to_string()));
        assert_eq!(out.unwrap_err(), CliError::usage("missing vault name"));
    }

    #[test]
    fn run_gives_the_program_the_secrets_and_passes_on_its_exit_code() {
        // The program is this test binary's own interpreter of choice: a shell that every platform has.
        let (program, flag, script) = if cfg!(windows) {
            ("cmd", "/C", "if \"%TOKEN%\"==\"t0ken-value\" (exit 7) else (exit 1)")
        } else {
            ("sh", "-c", "[ \"$TOKEN\" = t0ken-value ] && exit 7 || exit 1")
        };
        let command = [program.to_string(), flag.to_string(), script.to_string()];
        let argv = ["vault", "run", "api_keys"];
        let (out, _) = with_engine(&argv, engine("unsealed"), |c| run(c, &command).map(|code| code.to_string()));
        assert_eq!(out.unwrap(), "7");
    }
}
