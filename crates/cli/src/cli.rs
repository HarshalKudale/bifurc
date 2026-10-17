//! The command line's top level: which data directory, which command, how an error is reported, and the
//! `engine` subcommands that manage the engine process itself.

use std::{
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
};

use serde_json::json;

use crate::{
    agent,
    args::{ParsedArgs, parse_args},
    capture,
    commands::{self, Connector, Context, unknown_verb},
    curl, direct,
    discovery::{RuntimeInfo, find_running_engine, platform_default_data_dir},
    entities,
    error::{CliError, Result},
    extras, flows, health,
    locate::locate_engine,
    requests,
    session::{Engine, connect},
    setup,
    supervisor::{RESTART_TIMEOUT, START_TIMEOUT, STOP_TIMEOUT, restart_engine, start_engine, stop_engine},
    update,
    usage::{OVERVIEW, USAGE, canonical_group, examples, group_usage},
    vault, wait,
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
    connect_existing(data_dir)
}

fn connect_existing(data_dir: &Path) -> Result<Box<dyn Engine>> {
    Ok(Box::new(connect(data_dir, VERSION)?))
}

/// `--transport`, defaulting to `socket`.
pub fn transport_of(args: &ParsedArgs) -> Result<&str> {
    match args.flag("transport").unwrap_or("socket") {
        transport @ ("socket" | "ws") => Ok(transport),
        _ => Err(CliError::usage("--transport must be socket or ws")),
    }
}

fn engine_command(args: &ParsedArgs, ctx: &Context, data_dir: &Path) -> Result<String> {
    let started = |label: &str, info: &RuntimeInfo| ctx.print(&info.raw, || format!("{label}  {}", info.describe()));
    let verb = args.positional(1).unwrap_or("status");
    match verb {
        "start" => ctx.accept(&["transport", "no-tray"], 0)?,
        "status" | "restart" | "stop" => ctx.accept(&[], 0)?,
        other => return Err(unknown_verb("engine", other)),
    }
    match verb {
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
        other => Err(unknown_verb("engine", other)),
    }
}

/// What a command of `args` runs against: the engine of `data_dir`, reached when it is first needed.
fn context<'a>(args: &'a ParsedArgs, data_dir: &'a Path) -> Context<'a> {
    let no_start = args.has("no-start");
    let connector: Connector =
        Box::new(move || if no_start { connect_existing(data_dir) } else { connect_or_start(data_dir) });
    Context::new(args, connector)
}

/// Run the command in `args` and return the text to print. A command that reports as it goes writes those
/// lines to `stdout` itself.
fn dispatch(args: &ParsedArgs, data_dir: &Path, stdout: &mut dyn Write) -> Result<String> {
    let mut ctx = context(args, data_dir);
    let group = args.positional(0).unwrap_or_default();
    let sub = args.positional(1);
    let group = canonical_group(group);
    // The verbs that are one engine command, and the few beside them that take more than one.
    let verb = sub.unwrap_or("list");
    // Extras first: `cookies list` and `env show` there print what the table's plain row would show in clear.
    if let Some(done) = extras::run(&mut ctx, group, verb) {
        return done;
    }
    if let Some(row) = direct::find(group, verb) {
        return direct::run(&mut ctx, &row);
    }
    match group {
        "engine" => engine_command(args, &ctx, data_dir),
        "status" => commands::status(&mut ctx),
        "wait" => wait::wait(&mut ctx),
        "setup" => setup::setup(&mut ctx),
        "agent" => agent::agent(&mut ctx, sub),
        "workspaces" => commands::workspaces(&mut ctx, sub),
        "account" => commands::account(&mut ctx, sub),
        "env" => commands::env(&mut ctx, sub),
        "mappings" => entities::mappings(&mut ctx, sub),
        "rules" => entities::rules(&mut ctx, sub),
        "mocks" => entities::mocks(&mut ctx, sub),
        "requests" => {
            let done = requests::requests(&mut ctx, sub);
            if let Some(raw) = ctx.raw.take() {
                let _ = stdout.write_all(&raw);
            }
            done
        }
        "health" => health::health(&mut ctx, sub),
        "flows" => flows::flows(&mut ctx, sub, stdout),
        "capture" if verb == "watch" => capture::watch(&mut ctx, stdout),
        "services" => commands::services(&mut ctx, sub),
        "vault" => vault::vaults(&mut ctx, sub),
        "update" => update::update(&mut ctx, sub),
        "call" => commands::call(&mut ctx),
        "commands" => Ok(commands::commands(&ctx)),
        // gRPC requests are requests like the others.
        "grpc" => Err(CliError::usage(
            "gRPC is a type of request: bifurc requests list|create|send --type grpc, and bifurc requests reflect <address> (see: bifurc requests --help)",
        )),
        // A vault is unsealed once and then used; nothing is unlocked around one program any more.
        "unlock" | "unlock-vault" => Err(CliError::usage(
            "a vault is no longer unlocked for one program: bifurc vault run <vault> -- <command> (see: bifurc vault --help)",
        )),
        // A group that has only verbs of the tables above.
        _ if group_usage(group).is_some() => Err(unknown_verb(group, verb)),
        _ => Err(CliError::usage(format!("unknown command \"{group}\"\n\n{OVERVIEW}"))),
    }
}

/// Whether `head`, the arguments before a `--`, are `vault run <vault>`.
fn is_vault_run(head: &ParsedArgs) -> bool {
    head.positional(0).is_some_and(|group| canonical_group(group) == "vault") && head.positional(1) == Some("run")
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
    // `bifurc vault run <vault> -- <command…>`: what follows `--` is the command to run, not arguments of
    // ours, and the exit code is that command's.
    if let Some(split) = argv.iter().position(|a| a == "--")
        && let Ok(head) = parse_args(&argv[..split])
        && is_vault_run(&head)
        && !head.has("help")
    {
        let data_dir = data_dir_of(&head, env_data_dir);
        return match vault::run(&mut context(&head, &data_dir), &argv[split + 1..]) {
            Ok(code) => code,
            Err(err) => {
                let _ = writeln!(stderr, "{}", err.line());
                err.code()
            }
        };
    }
    // `bifurc help --all` is the whole overview (`--all` takes a value elsewhere, so it is read here).
    if argv.iter().any(|a| a == "--all") && matches!(argv.first().map(String::as_str), Some("help" | "--help" | "-h")) {
        let _ = writeln!(stdout, "{USAGE}");
        return 0;
    }
    // `bifurc help mocks --examples` and `bifurc mocks --help --examples`; `curl` would misread the line below.
    if argv.iter().any(|a| a == "--examples")
        && let Ok(args) = parse_args(argv)
    {
        let topic = if args.positional(0) == Some("help") { args.positional(1) } else { args.positional(0) };
        return match topic.and_then(examples) {
            Some(lines) => {
                let _ = writeln!(stdout, "{}", lines.join("\n"));
                0
            }
            None => {
                let message =
                    "examples exist for: agent, curl, requests, capture, mocks, flows, runner, env, cookies, wait";
                let _ = writeln!(stderr, "{}", CliError::usage(message).line());
                2
            }
        };
    }
    // `bifurc curl …`: curl's arguments, which `parse_args` would misread (`-I`, `--json <data>`).
    if curl::at(argv).is_some() {
        let result = curl::split(argv).and_then(|(args, call)| {
            if args.has("help") {
                return Ok(group_usage("curl").unwrap_or_default());
            }
            let data_dir = data_dir_of(&args, env_data_dir);
            let mut ctx = context(&args, &data_dir);
            let done = curl::run(&mut ctx, &call);
            if let Some(raw) = ctx.raw.take() {
                let _ = stdout.write_all(&raw);
            }
            done
        });
        return report(result, stdout, stderr);
    }
    let args = match parse_args(argv) {
        Ok(args) => args,
        Err(message) => {
            let _ = writeln!(stderr, "{}", CliError::usage(message).line());
            return 2;
        }
    };
    let group = args.positional(0);
    if group.is_none() || args.has("help") || group == Some("help") {
        // `bifurc help mocks` and `bifurc mocks --help` are the help of that group.
        let topic = if group == Some("help") { args.positional(1) } else { group };
        let _ = writeln!(stdout, "{}", topic.and_then(group_usage).unwrap_or_else(|| OVERVIEW.to_string()));
        return 0;
    }
    let data_dir = data_dir_of(&args, env_data_dir);
    let done = dispatch(&args, &data_dir, stdout);
    report(done, stdout, stderr)
}

/// Print what a command answered (or why it failed) and return the process exit code.
fn report(done: Result<String>, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    match done {
        Ok(text) => {
            // Nothing to print is not an empty line: `--quiet` and `--raw` print nothing after their output.
            if !text.is_empty() {
                let _ = writeln!(stdout, "{text}");
            }
            0
        }
        // What the command found is printed where an answer goes; only the exit code differs.
        Err(CliError::Failed(report)) => {
            if !report.is_empty() {
                let _ = writeln!(stdout, "{report}");
            }
            1
        }
        Err(err) => {
            let _ = writeln!(stderr, "{}", err.line());
            err.code()
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
            assert_eq!(outcome.stdout, format!("{OVERVIEW}\n"));
            assert_eq!(outcome.stderr, "");
        }
    }

    #[test]
    fn an_unknown_command_is_an_error_followed_by_the_usage() {
        let outcome = go(&["frobnicate"]);
        assert_eq!(outcome.code, 2);
        assert_eq!(outcome.stdout, "");
        assert_eq!(outcome.stderr, format!("bifurc: unknown command \"frobnicate\"\n\n{OVERVIEW}\n"));
    }

    #[test]
    fn a_flag_with_no_value_is_reported() {
        let outcome = go(&["status", "--data-dir"]);
        assert_eq!(outcome.code, 2);
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
        let runtime = json!({"pid": std::process::id(), "transport": "stdio", "address": "addr", "dataDir": "x"});
        std::fs::write(dir.path().join("runtime.json"), runtime.to_string()).unwrap();
        let path = dir.path().to_str().unwrap();
        let outcome = go(&["engine", "status", "--data-dir", path]);
        assert_eq!(outcome.stdout, format!("running  pid {}, stdio addr\n", std::process::id()));
        let outcome = go(&["engine", "status", "--data-dir", path, "--json"]);
        assert_eq!(serde_json::from_str::<Value>(&outcome.stdout).unwrap(), runtime);
    }

    #[test]
    fn engine_start_reports_a_running_engine_without_starting_another() {
        let dir = empty_dir();
        let runtime = json!({"pid": std::process::id(), "transport": "stdio", "address": "addr", "dataDir": "x"});
        std::fs::write(dir.path().join("runtime.json"), runtime.to_string()).unwrap();
        let outcome = go(&["engine", "start", "--data-dir", dir.path().to_str().unwrap()]);
        assert_eq!(outcome.code, 0);
        assert_eq!(outcome.stdout, format!("already running  pid {}, stdio addr\n", std::process::id()));
    }

    #[test]
    fn engine_start_validates_the_transport() {
        let dir = empty_dir();
        let outcome =
            go(&["engine", "start", "--transport", "carrier-pigeon", "--data-dir", dir.path().to_str().unwrap()]);
        assert_eq!((outcome.code, outcome.stderr.as_str()), (2, "bifurc: --transport must be socket or ws\n"));
    }

    #[test]
    fn stop_and_restart_need_a_running_engine() {
        let dir = empty_dir();
        let path = dir.path().to_str().unwrap();
        for sub in ["stop", "restart"] {
            let outcome = go(&["engine", sub, "--data-dir", path]);
            assert_eq!(outcome.code, 5);
            assert_eq!(
                outcome.stderr,
                format!("bifurc: No engine is running for {path}.\nStart one with: bifurc engine start\n")
            );
        }
    }

    #[test]
    fn an_unknown_engine_subcommand_is_an_error() {
        let outcome = go(&["engine", "explode"]);
        assert_eq!(outcome.stderr, "bifurc: unknown: engine explode (see: bifurc engine --help)\n");
    }

    #[test]
    fn a_group_has_its_own_help_under_any_of_its_names() {
        for argv in [vec!["mocks", "--help"], vec!["help", "mocks"], vec!["mock", "create", "-h"]] {
            let outcome = go(&argv);
            assert_eq!(outcome.code, 0, "{argv:?}");
            assert_eq!(outcome.stdout, format!("{}\n", group_usage("mocks").unwrap()));
        }
        assert_eq!(go(&["help", "nonsense"]).stdout, format!("{OVERVIEW}\n"));
        assert_eq!(go(&["help", "--all"]).stdout, format!("{USAGE}\n"));
    }

    #[test]
    fn examples_are_printed_for_a_group_in_either_spelling_and_refused_for_others() {
        let want = format!("{}\n", examples("curl").unwrap().join("\n"));
        for argv in [vec!["help", "curl", "--examples"], vec!["curl", "--help", "--examples"]] {
            assert_eq!(go(&argv).stdout, want, "{argv:?}");
        }
        assert_eq!(go(&["help", "mock", "--examples"]).stdout.lines().count(), 4);
        assert_eq!(go(&["help", "status", "--examples"]).code, 2);
    }

    #[test]
    fn waiting_for_a_port_needs_no_engine() {
        let dir = empty_dir();
        let path = dir.path().to_str().unwrap();
        let outcome = go(&["wait", "--port", "1", "--timeout", "0", "--no-start", "--data-dir", path]);
        assert_eq!(outcome.code, 4);
        assert!(outcome.stderr.starts_with("bifurc: timed out after 0s"), "{}", outcome.stderr);
        assert!(!dir.path().join("runtime.json").exists());
    }

    #[test]
    fn vault_run_takes_its_command_after_a_double_dash_and_checks_it_before_needing_an_engine() {
        let dir = empty_dir();
        let path = dir.path().to_str().unwrap();
        let example = "bifurc: missing the command to run, e.g.: bifurc vault run keys -- npm run dev\n";
        for argv in [
            vec!["vault", "run", "keys", "--no-start", "--data-dir", path],
            vec!["vault", "run", "keys", "--no-start", "--data-dir", path, "--"],
        ] {
            let outcome = go(&argv);
            assert_eq!((outcome.code, outcome.stderr.as_str()), (2, example), "{argv:?}");
        }
        // With a command, the vault is what is needed next, so the engine is.
        let outcome =
            go(&["vaults", "run", "keys", "--no-start", "--data-dir", path, "--", "npm", "run", "dev", "--help"]);
        assert_eq!(outcome.code, 5);
        assert!(outcome.stderr.starts_with("bifurc: No engine is running for"), "{}", outcome.stderr);
        assert_eq!(outcome.stdout, "");
    }

    #[test]
    fn unlock_is_no_longer_a_command_and_says_what_replaced_it() {
        for argv in [vec!["unlock", "keys", "--", "npm", "run", "dev"], vec!["unlock-vault", "keys"]] {
            let outcome = go(&argv);
            assert_eq!(outcome.code, 2, "{argv:?}");
            assert!(outcome.stderr.contains("bifurc vault run <vault> -- <command>"), "{}", outcome.stderr);
        }
    }

    #[test]
    fn grpc_is_no_longer_a_group_and_says_where_it_went() {
        let outcome = go(&["grpc", "list"]);
        assert_eq!(outcome.code, 2);
        assert!(outcome.stderr.starts_with("bifurc: gRPC is a type of request: bifurc requests"), "{}", outcome.stderr);
    }

    #[test]
    fn an_unknown_option_is_refused_without_starting_an_engine() {
        let dir = empty_dir();
        let outcome = go(&["mocks", "list", "--al", "x", "--data-dir", dir.path().to_str().unwrap()]);
        assert_eq!(outcome.code, 2);
        assert_eq!(outcome.stderr, "bifurc: unknown option --al for \"mocks list\" (see: bifurc mocks --help)\n");
        assert!(!dir.path().join("runtime.json").exists());
    }

    #[test]
    fn no_start_fails_instead_of_starting_an_engine() {
        let dir = empty_dir();
        let path = dir.path().to_str().unwrap();
        for argv in [
            vec!["status"],
            vec!["workspaces"],
            vec!["mappings", "list"],
            vec!["rules"],
            vec!["mocks"],
            vec!["requests", "list", "--type", "grpc"],
            vec!["health"],
            vec!["services"],
        ] {
            let mut argv = argv;
            argv.extend(["--no-start", "--data-dir", path]);
            let outcome = go(&argv);
            assert_eq!(outcome.code, 5, "{argv:?}");
            assert!(outcome.stderr.starts_with("bifurc: No engine is running for"), "{}", outcome.stderr);
        }
    }

    #[test]
    fn commands_needs_no_engine() {
        let dir = empty_dir();
        let outcome = go(&["commands", "--no-start", "--data-dir", dir.path().to_str().unwrap()]);
        assert_eq!(outcome.code, 0);
        assert!(outcome.stdout.starts_with("account.list\n"));
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
        for group in [
            "workspace",
            "environment",
            "environments",
            "mapping",
            "request",
            "rule",
            "proxy-rules",
            "mock",
            "healthchecks",
        ] {
            let outcome = go(&[group, "--no-start", "--data-dir", path]);
            assert!(outcome.stderr.starts_with("bifurc: No engine is running"), "{group}: {}", outcome.stderr);
        }
    }

    #[test]
    fn exit_codes_say_what_went_wrong() {
        let dir = empty_dir();
        let path = dir.path().to_str().unwrap();
        // 2: a wrong command line, found before any engine is needed.
        assert_eq!(go(&["status", "--nmae", "x", "--no-start", "--data-dir", path]).code, 2);
        assert_eq!(go(&["frobnicate"]).code, 2);
        assert_eq!(go(&["requests", "frob", "--no-start", "--data-dir", path]).code, 2);
        // 5: no engine and none to start.
        assert_eq!(go(&["requests", "list", "--no-start", "--data-dir", path]).code, 5);
    }

    #[test]
    fn curl_is_checked_before_an_engine_is_needed_and_has_its_own_help() {
        let dir = empty_dir();
        let path = dir.path().to_str().unwrap();
        let outcome = go(&["curl", "--proxy", "p", "http://h/", "--no-start", "--data-dir", path]);
        assert_eq!(outcome.code, 2);
        assert!(outcome.stderr.starts_with("bifurc: unsupported curl option --proxy"), "{}", outcome.stderr);
        // `-I` and `--json <data>` are curl's, not the options they would be anywhere else.
        let outcome = go(&["--no-start", "--data-dir", path, "curl", "-I", "--json", "{}", "http://h/"]);
        assert_eq!(outcome.code, 5);
        assert!(outcome.stderr.starts_with("bifurc: No engine is running for"), "{}", outcome.stderr);
        for argv in [vec!["curl", "--help"], vec!["help", "curl"], vec!["curl", "-sL", "-h"]] {
            let outcome = go(&argv);
            assert_eq!((outcome.code, outcome.stdout), (0, format!("{}\n", group_usage("curl").unwrap())), "{argv:?}");
        }
    }
}
