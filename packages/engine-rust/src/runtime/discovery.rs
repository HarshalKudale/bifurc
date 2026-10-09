//! How clients find a running engine: one engine serves one data directory and records where it listens in
//! `<dataDir>/runtime.json`.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRuntimeInfo {
    pub pid: u32,
    /// `socket` | `ws` | `stdio`
    pub transport: String,
    pub address: String,
    pub data_dir: String,
    #[serde(default)]
    pub protocol_version: String,
    #[serde(default)]
    pub engine_version: String,
    #[serde(default)]
    pub started_at: i64,
}

pub const RUNTIME_FILE: &str = "runtime.json";

/// An absolute path with `.` and `..` resolved, without touching the filesystem.
pub fn resolve_path(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn data_dir_key(data_dir: &Path) -> String {
    let resolved = resolve_path(data_dir).to_string_lossy().into_owned();
    let normalized = if cfg!(windows) { resolved.to_lowercase() } else { resolved };
    hex::encode(Sha256::digest(normalized.as_bytes()))[..16].to_string()
}

/// The socket address derived from a data directory, so every client computes the same one.
pub fn engine_socket_path(data_dir: &Path) -> String {
    let key = data_dir_key(data_dir);
    if cfg!(windows) {
        return format!(r"\\.\pipe\bifurc-{key}");
    }
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()).map(PathBuf::from);
    runtime_dir.unwrap_or_else(std::env::temp_dir).join(format!("bifurc-{key}.sock")).to_string_lossy().into_owned()
}

pub fn runtime_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join(RUNTIME_FILE)
}

pub fn write_runtime_info(info: &EngineRuntimeInfo) -> std::io::Result<()> {
    let dir = Path::new(&info.data_dir);
    std::fs::create_dir_all(dir)?;
    let file = runtime_file_path(dir);
    std::fs::write(&file, serde_json::to_string_pretty(info)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn read_runtime_info(data_dir: &Path) -> Option<EngineRuntimeInfo> {
    let text = std::fs::read_to_string(runtime_file_path(data_dir)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Remove `runtime.json` if it is this process's.
pub fn remove_runtime_info(data_dir: &Path) {
    if read_runtime_info(data_dir).is_some_and(|info| info.pid == std::process::id()) {
        let _ = std::fs::remove_file(runtime_file_path(data_dir));
    }
}

#[cfg(unix)]
pub fn is_process_alive(pid: u32) -> bool {
    // SAFETY: signal 0 delivers nothing; it only reports whether the process exists.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

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

/// Whether something listens at a `socket` address. A pid alone does not prove an engine: a `runtime.json`
/// that outlived its engine (a crash, a reboot) names a pid the system may have given to another process.
#[cfg(unix)]
pub fn is_socket_listening(address: &str) -> bool {
    std::os::unix::net::UnixStream::connect(address).is_ok()
}

#[cfg(windows)]
pub fn is_socket_listening(address: &str) -> bool {
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, GetLastError},
        System::Pipes::WaitNamedPipeW,
    };
    let name: Vec<u16> = address.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `name` is a NUL-terminated wide string that outlives the call.
    // A pipe whose instances are all busy times out; only a pipe no one serves is not found.
    unsafe { WaitNamedPipeW(name.as_ptr(), 1) != 0 || GetLastError() != ERROR_FILE_NOT_FOUND }
}

/// The engine serving `data_dir`, if its process is still alive and, over a socket, still listening.
pub fn find_running_engine(data_dir: &Path) -> Option<EngineRuntimeInfo> {
    read_runtime_info(data_dir)
        .filter(|info| is_process_alive(info.pid))
        .filter(|info| info.transport != "socket" || is_socket_listening(&info.address))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_is_stable_for_equivalent_paths() {
        let base = std::env::temp_dir().join("bifurc-key-test");
        let same = base.join("sub").join("..");
        assert_eq!(engine_socket_path(&base), engine_socket_path(&same));
        assert!(engine_socket_path(&base).contains("bifurc-"));
    }

    #[test]
    fn this_process_is_alive() {
        assert!(is_process_alive(std::process::id()));
    }

    #[test]
    fn a_live_pid_with_no_one_on_its_socket_is_not_a_running_engine() {
        let dir = tempfile::tempdir().unwrap();
        let mut info = EngineRuntimeInfo {
            pid: std::process::id(),
            transport: "socket".to_string(),
            address: engine_socket_path(dir.path()),
            data_dir: dir.path().to_string_lossy().into_owned(),
            protocol_version: String::new(),
            engine_version: String::new(),
            started_at: 0,
        };
        write_runtime_info(&info).unwrap();
        assert!(find_running_engine(dir.path()).is_none());
        info.transport = "stdio".to_string();
        write_runtime_info(&info).unwrap();
        assert!(find_running_engine(dir.path()).is_some());
    }
}
