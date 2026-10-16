//! Finding the engine binary `bifurc-engine[.exe]` that `engine start` launches.
//!
//! In order: the `BIFURC_ENGINE` environment variable; a binary next to this executable; one in the folder above
//! when this executable is in a `bin/` (an install, whose `bin/` holds the commands that go on `PATH`); then,
//! walking up from the executable's directory, `target/release/` and `target/debug/` of each ancestor (a development build).

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

use crate::error::{CliError, Result};

pub use bifurc_client::{ENGINE_ENV, engine_file_name};

/// The engine binary, given the value of `BIFURC_ENGINE` and the path of the running executable.
pub fn locate_engine_from(explicit: Option<OsString>, current_exe: Option<&Path>) -> Result<PathBuf> {
    let name = engine_file_name();
    if let Some(explicit) = explicit.filter(|v| !v.is_empty()) {
        let path = PathBuf::from(explicit);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(CliError::msg(format!("{ENGINE_ENV} is set to {}, which is not a file.", path.display())))
        };
    }
    if let Some(dir) = current_exe.and_then(Path::parent) {
        let beside = dir.join(&name);
        if beside.is_file() {
            return Ok(beside);
        }
        if dir.file_name().is_some_and(|folder| folder == "bin") {
            if let Some(above) = dir.parent().map(|root| root.join(&name)).filter(|above| above.is_file()) {
                return Ok(above);
            }
        }
        for ancestor in dir.ancestors() {
            for profile in ["release", "debug"] {
                let candidate = ancestor.join("target").join(profile).join(&name);
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
    }
    Err(CliError::msg(format!(
        "Could not find the engine ({name}). Looked next to this executable, above its bin/ and in target/release and target/debug \
         above it. Build it with `cargo build -p bifurc-engine`, or set {ENGINE_ENV} to its path."
    )))
}

/// The engine binary for this process: `BIFURC_ENGINE`, then the surroundings of the running executable.
pub fn locate_engine() -> Result<PathBuf> {
    // Resolved, because macOS reports the PATH symlink an install's `bin/bifurc` was called by, not the install.
    let exe = env::current_exe().ok().map(|exe| exe.canonicalize().unwrap_or(exe));
    locate_engine_from(env::var_os(ENGINE_ENV), exe.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, sync::Mutex};

    /// Tests that set environment variables take this lock.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn the_environment_variable_wins_over_everything() {
        let dir = tempfile::tempdir().unwrap();
        let explicit = dir.path().join("custom-engine");
        touch(&explicit);
        let exe = dir.path().join("bin").join("bifurc");
        touch(&dir.path().join("bin").join(engine_file_name()));
        let found = locate_engine_from(Some(explicit.clone().into()), Some(&exe)).unwrap();
        assert_eq!(found, explicit);
    }

    #[test]
    fn a_variable_pointing_nowhere_is_an_error_not_a_fallthrough() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("bifurc");
        touch(&dir.path().join(engine_file_name()));
        let err = locate_engine_from(Some(dir.path().join("missing").into()), Some(&exe)).unwrap_err();
        assert!(err.to_string().contains("BIFURC_ENGINE is set to"));
    }

    #[test]
    fn an_empty_variable_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("bifurc");
        let beside = dir.path().join(engine_file_name());
        touch(&beside);
        assert_eq!(locate_engine_from(Some(OsString::new()), Some(&exe)).unwrap(), beside);
    }

    #[test]
    fn a_binary_beside_the_executable_wins_over_a_build_directory() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("a").join("bifurc");
        let beside = dir.path().join("a").join(engine_file_name());
        touch(&beside);
        touch(&dir.path().join("target").join("release").join(engine_file_name()));
        assert_eq!(locate_engine_from(None, Some(&exe)).unwrap(), beside);
    }

    #[test]
    fn an_installs_bin_folder_finds_the_engine_above_it() {
        let dir = tempfile::tempdir().unwrap();
        let above = dir.path().join(engine_file_name());
        touch(&above);
        let exe = dir.path().join("bin").join("bifurc-mcp");
        assert_eq!(locate_engine_from(None, Some(&exe)).unwrap(), above);
        // Only a `bin/`: any other folder is not an install's.
        let exe = dir.path().join("tools").join("bifurc-mcp");
        assert!(locate_engine_from(None, Some(&exe)).is_err());
    }

    #[test]
    fn walking_up_prefers_release_to_debug() {
        let dir = tempfile::tempdir().unwrap();
        let release = dir.path().join("target").join("release").join(engine_file_name());
        touch(&release);
        touch(&dir.path().join("target").join("debug").join(engine_file_name()));
        let exe = dir.path().join("deep").join("er").join("bifurc");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        assert_eq!(locate_engine_from(None, Some(&exe)).unwrap(), release);
    }

    #[test]
    fn walking_up_falls_back_to_debug() {
        let dir = tempfile::tempdir().unwrap();
        let debug = dir.path().join("target").join("debug").join(engine_file_name());
        touch(&debug);
        let exe = dir.path().join("target").join("debug").join("deps").join("bifurc");
        assert_eq!(locate_engine_from(None, Some(&exe)).unwrap(), debug);
    }

    #[test]
    fn a_nearer_build_directory_wins_over_a_farther_one() {
        let dir = tempfile::tempdir().unwrap();
        let far = dir.path().join("target").join("release").join(engine_file_name());
        let near = dir.path().join("sub").join("target").join("debug").join(engine_file_name());
        touch(&far);
        touch(&near);
        let exe = dir.path().join("sub").join("x").join("bifurc");
        assert_eq!(locate_engine_from(None, Some(&exe)).unwrap(), near);
    }

    #[test]
    fn nothing_found_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("bifurc");
        let err = locate_engine_from(None, Some(&exe)).unwrap_err().to_string();
        assert!(err.contains("Could not find the engine") && err.contains("BIFURC_ENGINE"), "{err}");
        assert!(locate_engine_from(None, None).is_err());
    }

    #[test]
    fn locate_engine_reads_the_environment_variable() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let explicit = dir.path().join("engine-from-env");
        touch(&explicit);
        // SAFETY: serialised by ENV_LOCK; no other test in this crate touches the environment.
        unsafe { env::set_var(ENGINE_ENV, &explicit) };
        let found = locate_engine();
        unsafe { env::remove_var(ENGINE_ENV) };
        assert_eq!(found.unwrap(), explicit);
    }
}
