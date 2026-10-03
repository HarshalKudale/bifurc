//! The command line's top level: which data directory, which command, how an error is reported, and the
//! `engine` subcommands that manage the engine process itself.

use std::{
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
};

use serde_json::json;

use crate::{
    args::{ParsedArgs, parse_args},
    commands::{self, Connector, Context},
    discovery::{RuntimeInfo, find_running_engine, platform_default_data_dir},
    error::{CliError, Result},
    locate::locate_engine,
    session::{Engine, connect},
    supervisor::{RESTART_TIMEOUT, START_TIMEOUT, STOP_TIMEOUT, restart_engine, start_engine, stop_engine},
    usage::USAGE,
};

/// The version this client reports in the handshake.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The data directory: `--data-dir`, else `BIFURC_DATA_DIR`, else the desktop app's.
pub fn data_dir_of(args: &ParsedArgs, env_value: Option<OsString>) -> PathBuf {
    match args.flag("data-dir") {
        Some(dir) => PathBuf::from(dir),
        None => env_value.filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(platform_default_data_dir),
    }
}

/// The engine for `data_dir`, started in the background first if none is running.
fn connect_or_start(data_dir: &Path) -> Result<Box<dyn Engine>> {
    if find_running_engine(data_dir).is_none() {
        start_engine(&locate_engine()?, data_dir, "socket", true, START_TIMEOUT)?;
    }
    Ok(Box::new(connect(data_dir, VERSION)?))
}

fn connect_existing(data_dir: &Path) -> Result<Box<dyn Engine>> {
    Ok(Box::new(connect(data_dir, VERSION)?))
}

/// `--transport`, defaulting to `socket`.
pub fn transport_of(args: &ParsedArgs) -> Result<&str> {
    match args.flag("transport").unwrap_or("socket") {
        transport @ ("socket" | "ws") => Ok(transport),
        _ => Err(CliError::msg("--transport must be socket or ws")),
    }
}

fn engine_command(args: &ParsedArgs, ctx: &Context, data_dir: &Path) -> Result<String> {
    let started = |label: &str, info: &RuntimeInfo| ctx.print(&info.raw, || format!("{label}  {}", info.describe()));
    match args.positional(1).unwrap_or("status") {
        "status" => Ok(match find_running_engine(data_dir) {
            Some(info) => ctx.print(&info.raw, || format!("running  {}", info.describe())),
            None => ctx.print(&json!({ "running": false, "dataDir": data_dir.to_string_lossy() }), || {
                format!("not running for {}", data_dir.display())
            }),
        }),
        "start" => {
            if let Some(running) = find_running_engine(data_dir) {
                return Ok(started("already running", &running));
            }
            let transport = transport_of(args)?;
            let info = start_engine(&locate_engine()?, data_dir, transport, !args.has("no-tray"), START_TIMEOUT)?;
            Ok(started("started", &info))
        }
        "restart" => {
            let info = restart_engine(connect_existing(data_dir)?, data_dir, RESTART_TIMEOUT)?;
            Ok(started("restarted", &info))
        }
        "stop" => {
            let engine = connect_existing(data_dir)?;
            let pid = engine.info().pid;
            stop_engine(engine, data_dir, STOP_TIMEOUT)?;
            Ok(ctx.print(&json!({ "ok": true }), || format!("stopped  pid {pid}")))
        }
        other => Err(CliError::msg(format!("unknown: engine {other}"))),
    }
}

/// Run the command in `args` and return the text to print.
fn dispatch(args: &ParsedArgs, data_dir: &Path) -> Result<String> {
    let no_start = args.has("no-start");
    let connector: Connector =
        Box::new(move || if no_start { connect_existing(data_dir) } else { connect_or_start(data_dir) });
    let mut ctx = Context::new(args, connector);
    let group = args.positional(0).unwrap_or_default();
    let sub = args.positional(1);
    match group {
        "engine" => engine_command(args, &ctx, data_dir),
        "status" => commands::status(&mut ctx),
        "workspaces" | "workspace" => commands::workspaces(&mut ctx, sub),
        "env" | "environments" | "environment" => commands::env(&mut ctx, sub),
        "mappings" | "mapping" => commands::mappings(&mut ctx, sub),
        "services" => commands::services(&mut ctx, sub),
        "requests" | "request" => commands::requests(&mut ctx, sub),
        "grpc" => commands::grpc(&mut ctx, sub),
        "call" => commands::call(&mut ctx),
        "commands" => Ok(commands::commands(&ctx)),
        other => Err(CliError::msg(format!("unknown command \"{other}\"\n\n{USAGE}"))),
    }
}

/// Run one command line and return the process exit code. Output goes to `stdout`, errors to `stderr`.
pub fn run(argv: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    run_with_env(argv, std::env::var_os("BIFURC_DATA_DIR"), stdout, stderr)
}

/// [`run`] with the value of `BIFURC_DATA_DIR` given.
pub fn run_with_env(
    argv: &[String],
    env_data_dir: Option<OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let args = match parse_args(argv) {
        Ok(args) => args,
        Err(message) => {
            let _ = writeln!(stderr, "{}", CliError::msg(message).line());
            return 1;
        }
    };
    let group = args.positional(0);
    if group.is_none() || args.has("help") || group == Some("help") {
        let _ = writeln!(stdout, "{USAGE}");
        return 0;
    }
    let data_dir = data_dir_of(&args, env_data_dir);
    match dispatch(&args, &data_dir) {
        Ok(text) => {
            let _ = writeln!(stdout, "{text}");
            0
        }
        Err(err) => {
            let _ = writeln!(stderr, "{}", err.line());
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    struct Outcome {
        code: i32,
        stdout: String,
        stderr: String,
    }

    fn go(argv: &[&str]) -> Outcome {
        go_env(argv, None)
    }

    fn go_env(argv: &[&str], env: Option<&str>) -> Outcome {
        let argv: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run_with_env(&argv, env.map(OsString::from), &mut out, &mut err);
        Outcome { code, stdout: String::from_utf8(out).unwrap(), stderr: String::from_utf8(err).unwrap() }
    }

    fn empty_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn no_command_help_flag_and_help_command_print_usage() {
        for argv in [vec![], vec!["--help"], vec!["-h"], vec!["help"], vec!["status", "--help"]] {
            let outcome = go(&argv);
            assert_eq!(outcome.code, 0, "{argv:?}");
            assert_eq!(outcome.stdout, format!("{USAGE}\n"));
            assert_eq!(outcome.stderr, "");
        }
    }

    #[test]
    fn an_unknown_command_is_an_error_followed_by_the_usage() {
        let outcome = go(&["frobnicate"]);
        assert_eq!(outcome.code, 1);
        assert_eq!(outcome.stdout, "");
        assert_eq!(outcome.stderr, format!("bifurc: unknown command \"frobnicate\"\n\n{USAGE}\n"));
    }

    #[test]
    fn a_flag_with_no_value_is_reported() {
        let outcome = go(&["status", "--data-dir"]);
        assert_eq!(outcome.code, 1);
        assert_eq!(outcome.stderr, "bifurc: --data-dir needs a value\n");
    }

    #[test]
    fn data_dir_prefers_the_flag_then_the_environment_then_the_default() {
        let parse = |argv: &[&str]| parse_args(&argv.iter().map(|a| a.to_string()).collect::<Vec<_>>()).unwrap();
        assert_eq!(data_dir_of(&parse(&["--data-dir", "/a"]), Some("/b".into())), PathBuf::from("/a"));
        assert_eq!(data_dir_of(&parse(&[]), Some("/b".into())), PathBuf::from("/b"));
        assert_eq!(data_dir_of(&parse(&[]), Some("".into())), platform_default_data_dir());
        assert_eq!(data_dir_of(&parse(&[]), None), platform_default_data_dir());
    }

    #[test]
    fn the_environment_variable_names_the_data_dir_when_the_flag_is_absent() {
        let dir = empty_dir();
        let outcome = go_env(&["engine", "status"], Some(dir.path().to_str().unwrap()));
        assert_eq!(outcome.stdout, format!("not running for {}\n", dir.path().display()));
    }

    #[test]
    fn engine_status_without_an_engine_says_so_in_both_formats() {
        let dir = empty_dir();
        let path = dir.path().to_str().unwrap();
        let outcome = go(&["engine", "--data-dir", path]);
        assert_eq!((outcome.code, outcome.stdout), (0, format!("not running for {path}\n")));

        let outcome = go(&["engine", "status", "--data-dir", path, "--json"]);
        let parsed: Value = serde_json::from_str(&outcome.stdout).unwrap();
        assert_eq!(parsed["running"], false);
        assert_eq!(parsed["dataDir"], path);
    }

    #[test]
    fn engine_status_describes_a_live_engine() {
        let dir = empty_dir();
        let runtime = json!({"pid": std::process::id(), "transport": "socket", "address": "addr", "dataDir": "x"});
        std::fs::write(dir.path().join("runtime.json"), runtime.to_string()).unwrap();
        let path = dir.path().to_str().unwrap();
        let outcome = go(&["engine", "status", "--data-dir", path]);
        assert_eq!(outcome.stdout, format!("running  pid {}, socket addr\n", std::process::id()));
        let outcome = go(&["engine", "status", "--data-dir", path, "--json"]);
        assert_eq!(serde_json::from_str::<Value>(&outcome.stdout).unwrap(), runtime);
    }

    #[test]
    fn engine_start_reports_a_running_engine_without_starting_another() {
        let dir = empty_dir();
        let runtime = json!({"pid": std::process::id(), "transport": "socket", "address": "addr", "dataDir": "x"});
        std::fs::write(dir.path().join("runtime.json"), runtime.to_string()).unwrap();
        let outcome = go(&["engine", "start", "--data-dir", dir.path().to_str().unwrap()]);
        assert_eq!(outcome.code, 0);
        assert_eq!(outcome.stdout, format!("already running  pid {}, socket addr\n", std::process::id()));
    }

    #[test]
    fn engine_start_validates_the_transport() {
        let dir = empty_dir();
        let outcome =
            go(&["engine", "start", "--transport", "carrier-pigeon", "--data-dir", dir.path().to_str().unwrap()]);
        assert_eq!((outcome.code, outcome.stderr.as_str()), (1, "bifurc: --transport must be socket or ws\n"));
    }

    #[test]
    fn stop_and_restart_need_a_running_engine() {
        let dir = empty_dir();
        let path = dir.path().to_str().unwrap();
        for sub in ["stop", "restart"] {
            let outcome = go(&["engine", sub, "--data-dir", path]);
            assert_eq!(outcome.code, 1);
            assert_eq!(
                outcome.stderr,
                format!("bifurc: No engine is running for {path}.\nStart one with: bifurc engine start\n")
            );
        }
    }

    #[test]
    fn an_unknown_engine_subcommand_is_an_error() {
        let outcome = go(&["engine", "explode"]);
        assert_eq!(outcome.stderr, "bifurc: unknown: engine explode\n");
    }

    #[test]
    fn no_start_fails_instead_of_starting_an_engine() {
        let dir = empty_dir();
        let path = dir.path().to_str().unwrap();
        for argv in [vec!["status"], vec!["workspaces"], vec!["mappings", "list"], vec!["services"]] {
            let mut argv = argv;
            argv.extend(["--no-start", "--data-dir", path]);
            let outcome = go(&argv);
            assert_eq!(outcome.code, 1, "{argv:?}");
            assert!(outcome.stderr.starts_with("bifurc: No engine is running for"), "{}", outcome.stderr);
        }
    }

    #[test]
    fn commands_needs_no_engine() {
        let dir = empty_dir();
        let outcome = go(&["commands", "--no-start", "--data-dir", dir.path().to_str().unwrap()]);
        assert_eq!(outcome.code, 0);
        assert!(outcome.stdout.starts_with("application.checkPort\n"));
        assert!(outcome.stdout.contains("\nworkspace.setActive\n"));
    }

    #[test]
    fn call_checks_its_arguments_before_needing_an_engine() {
        let dir = empty_dir();
        let path = dir.path().to_str().unwrap();
        let outcome = go(&["call", "nope.nope", "--no-start", "--data-dir", path]);
        assert_eq!(outcome.stderr, "bifurc: unknown command \"nope.nope\" (see: bifurc commands)\n");
        let outcome = go(&["call", "--no-start", "--data-dir", path]);
        assert_eq!(outcome.stderr, "bifurc: missing command name (see: bifurc commands)\n");
        let outcome = go(&["call", "config.get", "{oops", "--no-start", "--data-dir", path]);
        assert_eq!(outcome.stderr, "bifurc: payload is not valid JSON: {oops\n");
    }

    #[test]
    fn command_aliases_reach_the_same_commands() {
        let dir = empty_dir();
        let path = dir.path().to_str().unwrap();
        for group in ["workspace", "environment", "environments", "mapping", "request"] {
            let outcome = go(&[group, "--no-start", "--data-dir", path]);
            assert!(outcome.stderr.starts_with("bifurc: No engine is running"), "{group}: {}", outcome.stderr);
        }
    }
}
