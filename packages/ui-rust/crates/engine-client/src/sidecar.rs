//! Finding the engine, or starting it as a sidecar, so the UI and the `bifurc` CLI share one engine.
//!
//! The rendezvous is the engine's own: one engine per data directory, and it advertises itself in
//! `<dataDir>/runtime.json` (`packages/engine/src/runtime/discovery.ts`). The UI therefore never picks a
//! socket address; it reads the file the engine wrote. If a live engine is already there — started by the
//! CLI, or by another UI window — the UI attaches to it. Otherwise it starts `bifurc-engine --transport
//! socket`, detached. Either way the engine's lifetime is its own: it shows a tray icon, keeps running when
//! its clients close, and stops when asked to (or, with no tray icon, when its last interface closes).

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;

use crate::{
    connection::{EngineClient, Hello},
    error::{ClientError, Result},
};

const RUNTIME_FILE: &str = "runtime.json";
const START_TIMEOUT: Duration = Duration::from_secs(20);
const RESTART_TIMEOUT: Duration = Duration::from_secs(30);

/// The contents of `runtime.json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub pid: u32,
    pub transport: String,
    pub address: String,
    pub data_dir: String,
    #[serde(default)]
    pub engine_version: String,
}

/// The data directory the Electron app, the CLI and the engine all default to
/// (`platformDefaultDataDir` in `packages/engine/src/workspace/paths.ts`). `BIFURC_DATA_DIR` overrides it, as
/// it does for the engine.
pub fn default_data_dir() -> PathBuf {
    if let Some(dir) = env::var_os("BIFURC_DATA_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = || env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    if cfg!(windows) {
        let base = env::var_os("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join("AppData").join("Local"));
        base.join("Bifurc")
    } else if cfg!(target_os = "macos") {
        home().join("Library").join("Application Support").join("Bifurc")
    } else {
        env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".config"))
            .join("bifurc")
    }
}

pub fn read_runtime_info(data_dir: &Path) -> Option<RuntimeInfo> {
    let text = fs::read_to_string(data_dir.join(RUNTIME_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// The native engine (`bifurc-engine`), which is the engine the UI starts: `BIFURC_ENGINE` if set, then the one
/// an installed app ships beside its executable, then the newest build in a `target/{release,debug}` above the
/// executable or the working directory (a checkout).
pub fn locate_engine_binary() -> Option<PathBuf> {
    if let Some(explicit) = env::var_os("BIFURC_ENGINE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(explicit));
    }
    let name = format!("bifurc-engine{}", env::consts::EXE_SUFFIX);
    let exe = env::current_exe().ok();
    if let Some(packaged) = exe.as_deref().and_then(Path::parent).map(|dir| dir.join(&name))
        && packaged.is_file()
    {
        return Some(packaged);
    }
    let starts: Vec<PathBuf> = exe.iter().filter_map(|e| e.parent()).map(Path::to_path_buf).chain(env::current_dir().ok()).collect();
    for dir in starts.iter().flat_map(|start| start.ancestors()) {
        let newest = ["release", "debug"]
            .iter()
            .map(|profile| dir.join("target").join(profile).join(&name))
            .filter_map(|candidate| Some((fs::metadata(&candidate).ok()?.modified().ok()?, candidate)))
            .max_by_key(|(modified, _)| *modified);
        if let Some((_, candidate)) = newest {
            return Some(candidate);
        }
    }
    None
}

#[derive(Debug, Clone)]
pub struct SidecarConfig {
    pub data_dir: PathBuf,
    /// The engine executable, from [`locate_engine_binary`].
    pub engine_binary: Option<PathBuf>,
    /// The engine's tray helper (`bifurc-tray`), handed to an engine this process starts. `None` leaves the
    /// engine to find one itself.
    pub tray: Option<PathBuf>,
    /// The interface the engine's tray icon opens: the executable of the client that starts the engine.
    pub ui: Option<PathBuf>,
}

impl Default for SidecarConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            engine_binary: locate_engine_binary(),
            tray: None,
            ui: None,
        }
    }
}

/// The engine this process is connected to, and whether this process is the one that started it.
///
/// Starting it is all the ownership there is. The engine runs on its own once started: it shows its own tray
/// icon and decides itself when to stop, so a client closing never stops it and never has to.
pub struct Sidecar {
    pub info: RuntimeInfo,
    pub data_dir: PathBuf,
    started_here: bool,
}

impl Sidecar {
    /// `true` when this process started the engine rather than joining one that was already running.
    pub fn started_here(&self) -> bool {
        self.started_here
    }
}

/// Connects to the engine for `config.data_dir`, starting one if none is running.
pub fn attach_or_start(config: &SidecarConfig, client_version: &str) -> Result<(EngineClient, Hello, Sidecar)> {
    if let Some(info) = read_runtime_info(&config.data_dir).filter(|i| i.transport == "socket")
        && let Ok((client, hello)) = EngineClient::connect_attempts(&info.address, client_version, 1)
    {
        let sidecar = Sidecar { info, data_dir: config.data_dir.clone(), started_here: false };
        return Ok((client, hello, sidecar));
    }
    start(config, client_version)
}

/// Connects to the engine that replaces the one with `old_pid`, once it has announced itself. For a client
/// that heard the engine say it is restarting.
pub fn reconnect(data_dir: &Path, old_pid: u32, client_version: &str) -> Result<(EngineClient, Hello, Sidecar)> {
    let deadline = Instant::now() + RESTART_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(info) = read_runtime_info(data_dir).filter(|i| i.pid != old_pid && i.transport == "socket")
            && let Ok((client, hello)) = EngineClient::connect_attempts(&info.address, client_version, 1)
        {
            let sidecar = Sidecar { info, data_dir: data_dir.to_path_buf(), started_here: false };
            return Ok((client, hello, sidecar));
        }
        thread::sleep(Duration::from_millis(150));
    }
    Err(ClientError::Protocol(format!(
        "the engine did not come back within {}s. See {}.",
        RESTART_TIMEOUT.as_secs(),
        data_dir.join("engine.log").display()
    )))
}

fn start(config: &SidecarConfig, client_version: &str) -> Result<(EngineClient, Hello, Sidecar)> {
    let Some(binary) = &config.engine_binary else {
        return Err(ClientError::Protocol(
            "could not find the engine: no bifurc-engine beside the executable or in a target/ above it.              Build it with `cargo build -p bifurc-engine` at the repository root, or set BIFURC_ENGINE to its path."
                .into(),
        ));
    };
    let mut command = Command::new(binary);

    fs::create_dir_all(&config.data_dir)?;
    let log_path = config.data_dir.join("engine.log");
    let log = fs::OpenOptions::new().create(true).append(true).open(&log_path)?;

    command
        .arg("--data-dir")
        .arg(&config.data_dir)
        .arg("--transport")
        .arg("socket")
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    if let Some(tray) = &config.tray {
        command.arg("--tray").arg(tray);
    }
    if let Some(ui) = &config.ui {
        command.arg("--ui").arg(ui);
    }
    detach(&mut command);
    let mut child = command
        .spawn()
        .map_err(|e| ClientError::Protocol(format!("could not start the engine `{}`: {e}.", binary.display())))?;

    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(ClientError::Protocol(format!(
                "the engine exited ({status}) before it was ready.\n{}",
                log_tail(&log_path)
            )));
        }
        if let Some(info) =
            read_runtime_info(&config.data_dir).filter(|i| i.pid == child.id() && i.transport == "socket")
            && let Ok((client, hello)) = EngineClient::connect_attempts(&info.address, client_version, 1)
        {
            let sidecar = Sidecar { info, data_dir: config.data_dir.clone(), started_here: true };
            return Ok((client, hello, sidecar));
        }
        thread::sleep(Duration::from_millis(150));
    }
    let _ = child.kill();
    let _ = child.wait();
    Err(ClientError::Protocol(format!(
        "the engine did not become ready within {}s. See {}.\n{}",
        START_TIMEOUT.as_secs(),
        log_path.display(),
        log_tail(&log_path)
    )))
}

/// The engine outlives the client that starts it, so it gets no console and no share in this process's
/// signals: closing the client, or the terminal it was run from, leaves the engine running.
#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(any(windows, unix)))]
fn detach(_: &mut Command) {}

fn log_tail(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.trim_end().lines().collect();
    lines[lines.len().saturating_sub(12)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_info_reads_what_the_engine_writes() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(RUNTIME_FILE),
            r#"{"pid":47820,"transport":"socket","address":"\\\\.\\pipe\\bifurc-a70f","dataDir":"C:\\x","protocolVersion":"1.0.0","engineVersion":"0.4.0","startedAt":1}"#,
        )
        .unwrap();
        let info = read_runtime_info(dir.path()).unwrap();
        assert_eq!(info.pid, 47820);
        assert_eq!(info.transport, "socket");
        assert_eq!(info.address, r"\\.\pipe\bifurc-a70f");
        assert_eq!(info.engine_version, "0.4.0");
    }

    #[test]
    fn a_missing_or_garbled_runtime_file_is_no_engine() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_runtime_info(dir.path()).is_none());
        fs::write(dir.path().join(RUNTIME_FILE), "not json").unwrap();
        assert!(read_runtime_info(dir.path()).is_none());
    }


    #[test]
    fn an_engine_that_cannot_be_found_says_how_to_point_at_one() {
        let dir = tempfile::tempdir().unwrap();
        let config = SidecarConfig { data_dir: dir.path().to_path_buf(), engine_binary: None, tray: None, ui: None };
        let text = start(&config, "test").err().expect("no engine to start").to_string();
        assert!(text.contains("BIFURC_ENGINE"), "{text}");
    }
}
