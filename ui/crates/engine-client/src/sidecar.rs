//! Finding the engine, or starting it as a sidecar, so the UI and the `bifurc` CLI share one engine.
//!
//! The rendezvous is the engine's own: one engine per data directory, and it advertises itself in
//! `<dataDir>/runtime.json` (`packages/engine/src/runtime/discovery.ts`). The UI therefore never picks a
//! socket address; it reads the file the engine wrote. If a live engine is already there — started by the
//! CLI, or by another UI window — the UI attaches to it and leaves it running on exit. Otherwise it starts
//! `bifurc-engine --transport socket` and owns that process's lifetime.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;

use crate::{
    connection::{EngineClient, Hello},
    error::{ClientError, Result},
};

const RUNTIME_FILE: &str = "runtime.json";
const ENGINE_ENTRY_RELATIVE: &str = "packages/engine/dist/runtime/hostCli.js";
const START_TIMEOUT: Duration = Duration::from_secs(20);

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

/// Where the engine's entry script is, searched in the order a developer and then a package would need.
pub fn locate_engine_entry() -> Option<PathBuf> {
    if let Some(explicit) = env::var_os("BIFURC_ENGINE_ENTRY").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(explicit));
    }
    let mut starts: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let packaged = dir.join("engine").join("runtime").join("hostCli.js");
        if packaged.is_file() {
            return Some(packaged);
        }
        starts.push(dir.to_path_buf());
    }
    if let Ok(cwd) = env::current_dir() {
        starts.push(cwd);
    }
    starts
        .iter()
        .flat_map(|start| start.ancestors())
        .map(|dir| dir.join(ENGINE_ENTRY_RELATIVE))
        .find(|candidate| candidate.is_file())
}

#[derive(Debug, Clone)]
pub struct SidecarConfig {
    pub data_dir: PathBuf,
    /// The `node` executable. `BIFURC_NODE` if set, otherwise whatever `node` resolves to on `PATH`.
    pub node: PathBuf,
    /// `None` searches with [`locate_engine_entry`].
    pub engine_entry: Option<PathBuf>,
}

impl Default for SidecarConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            node: env::var_os("BIFURC_NODE")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("node")),
            engine_entry: None,
        }
    }
}

/// An engine this process is connected to, and whether it also started it.
pub struct Sidecar {
    pub info: RuntimeInfo,
    pub data_dir: PathBuf,
    child: Option<Child>,
}

impl Sidecar {
    /// `true` when this process started the engine and so is responsible for stopping it.
    pub fn is_owned(&self) -> bool {
        self.child.is_some()
    }

    /// Stops an engine this process started: asks it to shut down, then makes sure it is gone. An engine
    /// that was already running is left alone — it belongs to whoever started it.
    pub fn release(&mut self, client: &EngineClient) {
        let Some(mut child) = self.child.take() else { return };
        let _ = client.call_blocking("engine.shutdown", serde_json::json!({}));
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Sidecar {
    /// Stops the engine whoever started it, for a restart: asks it to shut down and waits until it is gone.
    /// An engine this process started is killed if it will not leave; one that belongs to another client is
    /// only asked, and `false` says it is still there.
    pub fn stop(&mut self, client: &EngineClient) -> bool {
        let _ = client.call_blocking("engine.shutdown", serde_json::json!({}));
        let deadline = Instant::now() + Duration::from_secs(8);
        if let Some(mut child) = self.child.take() {
            while Instant::now() < deadline {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return true;
                }
                thread::sleep(Duration::from_millis(100));
            }
            let _ = child.kill();
            let _ = child.wait();
            return true;
        }
        while Instant::now() < deadline {
            // The engine removes runtime.json as it stops.
            if read_runtime_info(&self.data_dir).is_none_or(|info| info.pid != self.info.pid) {
                return true;
            }
            thread::sleep(Duration::from_millis(100));
        }
        false
    }
}

/// Connects to the engine for `config.data_dir`, starting one if none is running.
pub fn attach_or_start(config: &SidecarConfig, client_version: &str) -> Result<(EngineClient, Hello, Sidecar)> {
    if let Some(info) = read_runtime_info(&config.data_dir).filter(|i| i.transport == "socket")
        && let Ok((client, hello)) = EngineClient::connect_attempts(&info.address, client_version, 1)
    {
        let sidecar = Sidecar { info, data_dir: config.data_dir.clone(), child: None };
        return Ok((client, hello, sidecar));
    }
    start(config, client_version)
}

fn start(config: &SidecarConfig, client_version: &str) -> Result<(EngineClient, Hello, Sidecar)> {
    let entry = match &config.engine_entry {
        Some(entry) => entry.clone(),
        None => locate_engine_entry().ok_or_else(|| {
            ClientError::Protocol(format!(
                "could not find the engine (looked for {ENGINE_ENTRY_RELATIVE} above the executable and the working directory). \
                 Build it with `npm run build:packages`, or set BIFURC_ENGINE_ENTRY to its hostCli.js."
            ))
        })?,
    };

    fs::create_dir_all(&config.data_dir)?;
    let log_path = config.data_dir.join("engine.log");
    let log = fs::OpenOptions::new().create(true).append(true).open(&log_path)?;

    let mut command = Command::new(&config.node);
    command
        .arg(&entry)
        .arg("--data-dir")
        .arg(&config.data_dir)
        .arg("--transport")
        .arg("socket")
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    hide_console(&mut command);
    let mut child = command.spawn().map_err(|e| {
        ClientError::Protocol(format!(
            "could not start `{}` for the engine: {e}. Install Node.js, or set BIFURC_NODE to the executable.",
            config.node.display()
        ))
    })?;

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
            let sidecar = Sidecar { info, data_dir: config.data_dir.clone(), child: Some(child) };
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

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_: &mut Command) {}

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
            r#"{"pid":47820,"transport":"socket","address":"\\\\.\\pipe\\bifurc-a70f","dataDir":"C:\\x","protocolVersion":"1.0.0","engineVersion":"0.3.3","startedAt":1}"#,
        )
        .unwrap();
        let info = read_runtime_info(dir.path()).unwrap();
        assert_eq!(info.pid, 47820);
        assert_eq!(info.transport, "socket");
        assert_eq!(info.address, r"\\.\pipe\bifurc-a70f");
        assert_eq!(info.engine_version, "0.3.3");
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
        let config = SidecarConfig {
            data_dir: dir.path().to_path_buf(),
            node: PathBuf::from("node"),
            engine_entry: Some(dir.path().join("missing").join("hostCli.js")),
        };
        // `node` may not exist on the machine running the tests; either way the failure names the cause.
        let err = start(&config, "test").err().expect("no engine at that path");
        let text = err.to_string();
        assert!(text.contains("engine") && text.contains("exited") || text.contains("BIFURC_NODE"), "{text}");
    }
}
