//! Finding the engine: the default data directory and `<dataDir>/runtime.json`, where the engine for a data
//! directory advertises where it listens. Agrees with the engine's own `runtime/discovery` and
//! `workspace/paths`.

use std::{
    env,
    path::{Path, PathBuf},
};

use serde_json::Value;

/// The file the engine writes into its data directory.
pub const RUNTIME_FILE: &str = "runtime.json";

/// A running engine as its `runtime.json` describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeInfo {
    pub pid: u32,
    /// `socket`, `ws` or `stdio`.
    pub transport: String,
    pub address: String,
    pub data_dir: String,
    pub engine_version: String,
    /// The file's content as written, for `--json` output.
    pub raw: Value,
}

impl RuntimeInfo {
    /// Read the fields the CLI uses from a parsed `runtime.json`; `None` unless it has a pid and an address.
    pub fn from_json(raw: Value) -> Option<Self> {
        let pid = u32::try_from(raw.get("pid")?.as_u64()?).ok()?;
        let address = raw.get("address")?.as_str()?.to_string();
        let text = |key: &str| raw.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        Some(Self {
            pid,
            address,
            transport: text("transport"),
            data_dir: text("dataDir"),
            engine_version: text("engineVersion"),
            raw,
        })
    }

    /// `pid 42, socket \\.\pipe\bifurc-…`
    pub fn describe(&self) -> String {
        format!("pid {}, {} {}", self.pid, self.transport, self.address)
    }
}

/// The data directory the desktop app, the CLI and the engine all default to.
pub fn platform_default_data_dir() -> PathBuf {
    let home = || env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    let dir = |name: &str| env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        dir("LOCALAPPDATA").unwrap_or_else(|| home().join("AppData").join("Local")).join("Bifurc")
    } else if cfg!(target_os = "macos") {
        home().join("Library").join("Application Support").join("Bifurc")
    } else {
        dir("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config")).join("bifurc")
    }
}

/// The path of `runtime.json` in `data_dir`.
pub fn runtime_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join(RUNTIME_FILE)
}

/// The engine advertised in `data_dir`, whether or not its process is still alive.
pub fn read_runtime_info(data_dir: &Path) -> Option<RuntimeInfo> {
    let text = std::fs::read_to_string(runtime_file_path(data_dir)).ok()?;
    RuntimeInfo::from_json(serde_json::from_str(&text).ok()?)
}

/// The engine serving `data_dir`, if its process is still alive.
pub fn find_running_engine(data_dir: &Path) -> Option<RuntimeInfo> {
    read_runtime_info(data_dir).filter(|info| is_process_alive(info.pid))
}

#[cfg(unix)]
pub fn is_process_alive(pid: u32) -> bool {
    // SAFETY: signal 0 delivers nothing; it only reports whether the process exists.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Whether a process with this pid exists (one owned by someone else counts as alive).
#[cfg(windows)]
pub fn is_process_alive(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_ACCESS_DENIED, GetLastError},
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    const STILL_ACTIVE: u32 = 259;
    // SAFETY: the handle is checked before use and closed before returning.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != 0 && code == STILL_ACTIVE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_runtime(dir: &Path, value: &Value) {
        std::fs::write(runtime_file_path(dir), value.to_string()).unwrap();
    }

    #[test]
    fn reads_what_the_engine_writes() {
        let dir = tempfile::tempdir().unwrap();
        write_runtime(
            dir.path(),
            &json!({"pid": 47820, "transport": "socket", "address": r"\\.\pipe\bifurc-a70f", "dataDir": "C:\\x",
                    "protocolVersion": "1.0.0", "engineVersion": "0.4.0", "startedAt": 1}),
        );
        let info = read_runtime_info(dir.path()).unwrap();
        assert_eq!(info.pid, 47820);
        assert_eq!(info.transport, "socket");
        assert_eq!(info.address, r"\\.\pipe\bifurc-a70f");
        assert_eq!(info.engine_version, "0.4.0");
        assert_eq!(info.describe(), r"pid 47820, socket \\.\pipe\bifurc-a70f");
        assert_eq!(info.raw["startedAt"], 1);
    }

    #[test]
    fn a_missing_garbled_or_incomplete_file_is_no_engine() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_runtime_info(dir.path()).is_none());
        std::fs::write(runtime_file_path(dir.path()), "not json").unwrap();
        assert!(read_runtime_info(dir.path()).is_none());
        write_runtime(dir.path(), &json!({"pid": 1}));
        assert!(read_runtime_info(dir.path()).is_none());
        write_runtime(dir.path(), &json!({"pid": "1", "address": "x"}));
        assert!(read_runtime_info(dir.path()).is_none());
    }

    #[test]
    fn an_engine_whose_process_is_gone_is_not_running() {
        let dir = tempfile::tempdir().unwrap();
        write_runtime(dir.path(), &json!({"pid": std::process::id(), "transport": "socket", "address": "a"}));
        assert!(find_running_engine(dir.path()).is_some());
        // Pids are far below this on every supported platform.
        write_runtime(dir.path(), &json!({"pid": 4_000_000_000u64, "transport": "socket", "address": "a"}));
        assert!(read_runtime_info(dir.path()).is_some());
        assert!(find_running_engine(dir.path()).is_none());
    }

    #[test]
    fn this_process_is_alive() {
        assert!(is_process_alive(std::process::id()));
    }

    #[test]
    fn the_default_data_dir_is_the_platforms_app_directory() {
        let dir = platform_default_data_dir();
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name, if cfg!(any(windows, target_os = "macos")) { "Bifurc" } else { "bifurc" });
    }
}
