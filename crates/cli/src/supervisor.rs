//! Starting, stopping and restarting the standalone engine process for a data directory.

use std::{
    ffi::OsString,
    fs,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::{
    discovery::{RuntimeInfo, find_running_engine},
    error::{CliError, Result},
    session::Engine,
};

/// How long `engine start` waits for the engine to publish its address.
pub const START_TIMEOUT: Duration = Duration::from_secs(20);
/// How long `engine stop` waits for the engine to be gone.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(15);
/// How long `engine restart` waits for the successor engine.
pub const RESTART_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(150);

/// The engine's command line: `--data-dir <dir> --transport <kind>` and `--no-tray` unless it may show its
/// tray icon.
pub fn engine_args(data_dir: &Path, transport: &str, tray: bool) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["--data-dir".into(), data_dir.into(), "--transport".into(), transport.into()];
    if !tray {
        args.push("--no-tray".into());
    }
    args
}

/// The engine outlives this process and the terminal it ran in: no console window, and no share in this
/// process's signals.
#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

/// Windows hands every inheritable handle of this process to a child, including a pipe this process's own
/// stdout is. An engine holding that pipe would keep whoever reads our output (`bifurc … | more`, a script
/// capturing it) waiting for an end of stream until the engine exits. The child gets its standard handles
/// explicitly, so ours are made non-inheritable first.
#[cfg(windows)]
fn keep_own_stdio_from_the_child() {
    use windows_sys::Win32::{
        Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation},
        System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
    };
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: the handle comes from GetStdHandle and is only given a flag; a null or invalid one is
        // rejected by the call and ignored.
        unsafe {
            SetHandleInformation(GetStdHandle(which), HANDLE_FLAG_INHERIT, 0);
        }
    }
}

#[cfg(not(windows))]
fn keep_own_stdio_from_the_child() {}

/// The last `lines` lines of `file`, or nothing if it cannot be read.
pub fn tail_of(file: &Path, lines: usize) -> String {
    let text = fs::read_to_string(file).unwrap_or_default();
    let all: Vec<&str> = text.trim_end().lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// Start `binary` as a standalone engine for `data_dir` in the background and wait until it has published
/// its address. Its output goes to `<data_dir>/engine.log`.
pub fn start_engine(
    binary: &Path,
    data_dir: &Path,
    transport: &str,
    tray: bool,
    timeout: Duration,
) -> Result<RuntimeInfo> {
    let io = |what: &str, e: std::io::Error| CliError::engine_unreachable(format!("{what}: {e}"));
    fs::create_dir_all(data_dir).map_err(|e| io(&format!("cannot create {}", data_dir.display()), e))?;
    let log_path = data_dir.join("engine.log");
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| io(&format!("cannot open {}", log_path.display()), e))?;
    let mut command = Command::new(binary);
    command
        .args(engine_args(data_dir, transport, tray))
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| io("cannot share the engine log", e))?)
        .stderr(log);
    detach(&mut command);
    keep_own_stdio_from_the_child();
    let mut child = command.spawn().map_err(|e| io(&format!("could not start the engine {}", binary.display()), e))?;

    let deadline = Instant::now() + timeout;
    let mut exit_code = None;
    while Instant::now() < deadline {
        if let Some(info) = find_running_engine(data_dir).filter(|info| info.pid == child.id()) {
            return Ok(info);
        }
        if let Ok(Some(status)) = child.try_wait() {
            exit_code = Some(status.code().unwrap_or(1));
            break;
        }
        thread::sleep(POLL_INTERVAL);
    }
    let tail = tail_of(&log_path, 12);
    Err(CliError::engine_unreachable(match exit_code {
        Some(code) => format!("The engine exited (code {code}) before it was ready.\n{tail}"),
        None => format!(
            "The engine did not become ready within {}s. See {}.\n{tail}",
            timeout.as_secs(),
            log_path.display()
        ),
    }))
}

/// Ask the engine to shut down and wait until it is gone.
pub fn stop_engine(mut engine: Box<dyn Engine>, data_dir: &Path, timeout: Duration) -> Result<()> {
    let pid = engine.info().pid;
    engine.request("engine.shutdown", serde_json::json!({}))?;
    drop(engine);
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if find_running_engine(data_dir).is_none() {
            return Ok(());
        }
        thread::sleep(POLL_INTERVAL);
    }
    Err(CliError::msg(format!("The engine (pid {pid}) acknowledged the shutdown but is still running.")))
}

/// Ask the engine to restart itself and wait until its successor has published its address.
pub fn restart_engine(mut engine: Box<dyn Engine>, data_dir: &Path, timeout: Duration) -> Result<RuntimeInfo> {
    let pid = engine.info().pid;
    engine.request("engine.shutdown", serde_json::json!({ "restart": true }))?;
    drop(engine);
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(info) = find_running_engine(data_dir).filter(|info| info.pid != pid) {
            return Ok(info);
        }
        thread::sleep(POLL_INTERVAL);
    }
    let log = data_dir.join("engine.log");
    Err(CliError::msg(format!(
        "The engine (pid {pid}) acknowledged the restart but no new engine appeared. See {}.\n{}",
        log.display(),
        tail_of(&log, 12)
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: Vec<OsString>) -> Vec<String> {
        args.into_iter().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn the_engine_gets_its_data_dir_and_transport() {
        assert_eq!(
            strings(engine_args(Path::new("/d"), "socket", true)),
            ["--data-dir", "/d", "--transport", "socket"]
        );
    }

    #[test]
    fn no_tray_is_passed_through_to_the_engine() {
        assert_eq!(
            strings(engine_args(Path::new("/d"), "ws", false)),
            ["--data-dir", "/d", "--transport", "ws", "--no-tray"]
        );
    }

    #[test]
    fn the_log_tail_keeps_the_last_lines() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("engine.log");
        let text: String = (1..=20).map(|n| format!("line {n}\r\n")).collect();
        fs::write(&log, text).unwrap();
        let tail = tail_of(&log, 3);
        assert_eq!(tail, "line 18\nline 19\nline 20");
        assert_eq!(tail_of(&dir.path().join("missing"), 3), "");
    }

    #[test]
    fn a_binary_that_cannot_start_is_an_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-engine");
        let err = start_engine(&missing, &dir.path().join("data"), "socket", false, Duration::from_secs(1))
            .unwrap_err()
            .to_string();
        assert!(err.contains("could not start the engine") && err.contains("no-such-engine"), "{err}");
    }
}
