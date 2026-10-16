//! Where the data directory is when no one says: the same answer for the desktop app, the CLI and the engine.

use std::{env, path::PathBuf};

/// The environment variable that overrides the default data directory.
pub const DATA_DIR_ENV: &str = "BIFURC_DATA_DIR";

fn home_dir() -> PathBuf {
    env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default()
}

fn env_dir(name: &str) -> Option<PathBuf> {
    env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The data directory of this platform: `%LOCALAPPDATA%\Bifurc`, `~/Library/Application Support/Bifurc`, or
/// `$XDG_CONFIG_HOME/bifurc` (`~/.config/bifurc`).
pub fn platform_default_data_dir() -> PathBuf {
    if cfg!(windows) {
        env_dir("LOCALAPPDATA").unwrap_or_else(|| home_dir().join("AppData").join("Local")).join("Bifurc")
    } else if cfg!(target_os = "macos") {
        home_dir().join("Library").join("Application Support").join("Bifurc")
    } else {
        env_dir("XDG_CONFIG_HOME").unwrap_or_else(|| home_dir().join(".config")).join("bifurc")
    }
}

/// `BIFURC_DATA_DIR` if set and not empty, else [`platform_default_data_dir`].
pub fn default_data_dir() -> PathBuf {
    env_dir(DATA_DIR_ENV).unwrap_or_else(platform_default_data_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Tests that set environment variables take this lock.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn the_platform_default_is_the_apps_directory() {
        let name = platform_default_data_dir().file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name, if cfg!(any(windows, target_os = "macos")) { "Bifurc" } else { "bifurc" });
    }

    #[test]
    fn the_environment_overrides_the_default_unless_it_is_empty() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: serialised by ENV_LOCK; no other test in this crate touches the environment.
        unsafe { env::set_var(DATA_DIR_ENV, "somewhere") };
        let set = default_data_dir();
        unsafe { env::set_var(DATA_DIR_ENV, "") };
        let empty = default_data_dir();
        unsafe { env::remove_var(DATA_DIR_ENV) };
        assert_eq!(set, PathBuf::from("somewhere"));
        assert_eq!(empty, platform_default_data_dir());
        assert_eq!(default_data_dir(), platform_default_data_dir());
    }
}
