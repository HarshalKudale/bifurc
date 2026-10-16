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

pub use bifurc_client::platform_default_data_dir;
