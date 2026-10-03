//! `bifurc unlock <vault> -- <command…>`: open a vault for as long as a program runs.
//!
//! An environment variable can only reach the processes a program starts, so the vault is opened *around* a
//! program rather than into the shell: this process asks the engine for the vault's secrets (after the
//! passphrase proves the right to them), starts the command with them as environment variables, and the
//! vault closes when the command ends. The engine ties the opening to this session, so if this process dies
//! the vault closes too, and if the engine closes the vault (it is sealed from the UI, or the engine stops)
//! the command is stopped, because a program cannot be made to forget variables it already has.

use std::{
    process::{Command, ExitStatus},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use serde_json::{Value, json};

use crate::{
    args::ParsedArgs,
    error::{CliError, Result},
    prompt,
    session::{EngineConnection, Session, Stream},
};

const PASSPHRASE_ENV: &str = "BIFURC_VAULT_PASSPHRASE";
const PROMPT_ATTEMPTS: usize = 3;

/// The id of the vault named `name` (any case) in a `vault.list` answer.
pub fn find_vault(list: &Value, name: &str) -> Result<String> {
    let vaults = list.as_array().map(Vec::as_slice).unwrap_or_default();
    match vaults.iter().find(|v| v["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name))) {
        Some(vault) => Ok(vault["id"].as_str().unwrap_or_default().to_string()),
        None => {
            let known: Vec<&str> = vaults.iter().filter_map(|v| v["name"].as_str()).collect();
            let hint = if known.is_empty() { "this workspace has no vaults".to_string() } else { format!("known: {}", known.join(", ")) };
            Err(CliError::msg(format!("no vault named \"{name}\" ({hint})")))
        }
    }
}

/// Whether `frame` tells us that our opening of the vault was ended by the engine.
fn ends_our_lease(frame: &Value, lease_id: &str) -> bool {
    frame["event"] == "event.vault.revoked" && frame["payload"]["leaseId"] == lease_id
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
pub fn passphrase(args: &ParsedArgs, vault: &str, attempt: usize) -> Result<String> {
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
    prompt::secret(&format!("Passphrase for vault {vault}: "))
}

/// Ask the engine for the vault, asking the user for the passphrase until it is right.
fn checkout(session: &mut Session<Stream>, args: &ParsedArgs, workspace: &str, id: &str, name: &str) -> Result<Value> {
    let mut attempt = 0;
    loop {
        let pass = passphrase(args, name, attempt)?;
        match session.call("vault.checkout", json!({ "workspaceId": workspace, "id": id, "passphrase": pass })) {
            Err(CliError::Engine { code, message })
                if code == "FORBIDDEN" && !message.starts_with("Too many") && attempt + 1 < PROMPT_ATTEMPTS =>
            {
                attempt += 1;
            }
            other => return other,
        }
    }
}

/// Stop the program and what it started.
fn stop_program(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).output();
    }
    #[cfg(unix)]
    // SAFETY: sends a signal to a process id this process started.
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
}

/// While the program runs, Ctrl+C belongs to it: this process stays up to close the vault after it exits.
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

/// The vault name and the program, checked before any connection is made.
pub fn check<'a>(args: &'a ParsedArgs, command: &'a [String]) -> Result<(&'a str, &'a [String])> {
    let name = args.positional(1).filter(|n| !n.is_empty()).ok_or_else(|| CliError::msg("missing vault name"))?;
    if command.is_empty() {
        return Err(CliError::msg(format!("missing the command to run, e.g.: bifurc unlock {name} -- npm run dev")));
    }
    Ok((name, command))
}

/// Open the vault, run `command` with its secrets as environment variables and close the vault when it ends.
/// Returns the command's exit code.
pub fn run(args: &ParsedArgs, command: &[String], connection: EngineConnection) -> Result<i32> {
    let (name, command) = check(args, command)?;
    let (program, program_args) = command.split_first().expect("check found a command");

    let mut session = connection.into_session();
    session.call("subscribe", json!({ "events": ["event.vault.revoked"] }))?;
    let workspace = match args.flag("workspace") {
        Some(explicit) => explicit.to_string(),
        None => session.call("config.get", json!({}))?["activeWorkspaceId"].as_str().unwrap_or_default().to_string(),
    };
    let id = find_vault(&session.call("vault.list", json!({ "workspaceId": workspace }))?, name)?;
    let opened = checkout(&mut session, args, &workspace, &id, name)?;
    let lease_id = opened["leaseId"].as_str().unwrap_or_default().to_string();

    let mut child = Command::new(resolve_program(program));
    child.args(program_args).env_remove(PASSPHRASE_ENV);
    for (key, value) in opened["secrets"].as_object().into_iter().flatten() {
        child.env(key, value.as_str().unwrap_or_default());
    }
    leave_interrupts_to_the_program();
    let mut child = child.spawn().map_err(|e| CliError::msg(format!("cannot run \"{program}\": {e}")))?;

    // From here the connection is only read: the engine tells us if it closes the vault, and the end of the
    // connection (the engine stopped) means the same.
    let closed_by_engine = Arc::new(AtomicBool::new(false));
    let flag = closed_by_engine.clone();
    let pid = child.id();
    std::thread::spawn(move || {
        loop {
            match session.read_frame() {
                Ok(Some(frame)) if !ends_our_lease(&frame, &lease_id) => continue,
                _ => break,
            }
        }
        flag.store(true, Ordering::SeqCst);
        stop_program(pid);
    });

    let status = child.wait().map_err(|e| CliError::msg(format!("lost the program: {e}")))?;
    if closed_by_engine.load(Ordering::SeqCst) {
        eprintln!("bifurc: the vault {name} was closed, so \"{program}\" was stopped");
        return Ok(1);
    }
    Ok(exit_code(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vault_is_found_by_name_in_any_case_and_a_miss_lists_the_known_ones() {
        let list = json!([{ "id": "a1", "name": "api_keys" }, { "id": "b2", "name": "db" }]);
        assert_eq!(find_vault(&list, "API_KEYS").unwrap(), "a1");
        let err = find_vault(&list, "nope").unwrap_err().to_string();
        assert_eq!(err, "no vault named \"nope\" (known: api_keys, db)");
        assert!(find_vault(&json!([]), "x").unwrap_err().to_string().contains("no vaults"));
    }

    #[test]
    fn only_our_own_revocation_ends_the_program() {
        let ours = json!({ "event": "event.vault.revoked", "seq": 1, "payload": { "leaseId": "L1" } });
        assert!(ends_our_lease(&ours, "L1"));
        assert!(!ends_our_lease(&ours, "L2"));
        assert!(!ends_our_lease(&json!({ "event": "event.vault.changed", "payload": { "leaseId": "L1" } }), "L1"));
    }

    #[test]
    fn the_vault_and_the_command_are_both_required() {
        let parse = |argv: &[&str]| crate::args::parse_args(&argv.iter().map(|a| a.to_string()).collect::<Vec<_>>()).unwrap();
        let command = vec!["npm".to_string(), "run".to_string(), "dev".to_string()];
        assert_eq!(check(&parse(&["unlock", "keys"]), &command).unwrap().0, "keys");
        assert_eq!(check(&parse(&["unlock"]), &command).unwrap_err().to_string(), "missing vault name");
        assert!(check(&parse(&["unlock", "keys"]), &[]).unwrap_err().to_string().contains("bifurc unlock keys -- npm run dev"));
    }
}
