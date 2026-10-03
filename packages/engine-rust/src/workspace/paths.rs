//! The data root: where `app.json`, the workspaces (`data/<wsId>`) and the blob store live. Set once at
//! startup; one engine serves one data root.

use std::path::{Path, PathBuf};

use parking_lot::RwLock;

use crate::runtime::discovery::resolve_path;

static DATA_ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Set (and create) the data root. Every store function reads it.
pub fn set_data_root(dir: &Path) -> std::io::Result<()> {
    let resolved = resolve_path(dir);
    std::fs::create_dir_all(&resolved)?;
    *DATA_ROOT.write() = Some(resolved);
    Ok(())
}

/// The data root. Panics if no one called [`set_data_root`]: a store call before startup is a bug.
pub fn data_dir() -> PathBuf {
    DATA_ROOT.read().clone().expect("Data root not initialised — call set_data_root() at startup")
}

pub fn is_data_root_set() -> bool {
    DATA_ROOT.read().is_some()
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default()
}

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The data directory the desktop app, the CLI and the engine all default to.
pub fn platform_default_data_dir() -> PathBuf {
    if cfg!(windows) {
        env_dir("LOCALAPPDATA").unwrap_or_else(|| home_dir().join("AppData").join("Local")).join("Bifurc")
    } else if cfg!(target_os = "macos") {
        home_dir().join("Library").join("Application Support").join("Bifurc")
    } else {
        env_dir("XDG_CONFIG_HOME").unwrap_or_else(|| home_dir().join(".config")).join("bifurc")
    }
}
