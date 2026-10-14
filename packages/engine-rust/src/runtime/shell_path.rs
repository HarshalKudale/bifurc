//! Puts `bifurc` on the `PATH` of a macOS install.
//!
//! A DMG is a copy dragged into Applications: no installer runs, so the app does it itself, each time the engine
//! starts. The launcher (`bin/bifurc`) is linked as `/usr/local/bin/bifurc` when that folder lets the user write
//! to it, and otherwise as `~/.local/bin/bifurc` with that folder added to the shell profiles.
//! Only what Bifurc made is ever replaced.

use std::{
    fs, io,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

const SYSTEM_BIN: &str = "/usr/local/bin";
const PROFILE_MARK: &str = "# Added by Bifurc";
const PROFILE_LINE: &str = "export PATH=\"$HOME/.local/bin:$PATH\"";

/// Makes `bifurc` reachable from a new terminal. Quiet: a failure only means the command is not on `PATH`.
pub fn ensure(app_dir: &Path) {
    let launcher = app_dir.join("bin").join("bifurc");
    if !launcher.is_file() {
        return;
    }
    if link(&launcher, &Path::new(SYSTEM_BIN).join("bifurc")).is_ok() {
        return;
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
    let bin = home.join(".local").join("bin");
    let changed = fs::create_dir_all(&bin).and_then(|()| link(&launcher, &bin.join("bifurc")));
    if matches!(changed, Ok(true)) {
        // A login shell reads `.zprofile` (zsh, the default) or `.bash_profile`; Terminal opens login shells.
        add_to_profile(&home.join(".zprofile"), true);
        add_to_profile(&home.join(".bash_profile"), false);
    }
}

/// Makes `link` point at `target`. `true` when that changed something. Anything that is not a link to a Bifurc
/// (or to nothing) is somebody else's and is left alone.
fn link(target: &Path, link: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(link) {
        Ok(meta) if !meta.file_type().is_symlink() => return Err(io::Error::other("something else is there")),
        Ok(_) => {
            let old = fs::read_link(link)?;
            if old == target {
                return Ok(false);
            }
            if old.exists() && !old.to_string_lossy().contains("Bifurc.app") {
                return Err(io::Error::other("another command is linked there"));
            }
            fs::remove_file(link)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    symlink(target, link).map(|()| true)
}

/// Adds the line that puts `~/.local/bin` on `PATH` to `profile`, once. `create`: make the file if it is missing.
fn add_to_profile(profile: &Path, create: bool) {
    let text = match fs::read_to_string(profile) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound && create => String::new(),
        Err(_) => return,
    };
    if text.contains(PROFILE_MARK) {
        return;
    }
    let separator = if text.is_empty() || text.ends_with('\n') { "" } else { "\n" };
    let _ = fs::write(profile, format!("{text}{separator}\n{PROFILE_MARK}\n{PROFILE_LINE}\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_is_made_once_and_foreign_files_stay() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Bifurc.app").join("bifurc");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "").unwrap();
        let at = dir.path().join("bifurc");
        assert!(link(&target, &at).unwrap());
        assert!(!link(&target, &at).unwrap());
        fs::remove_file(&at).unwrap();
        fs::write(&at, "mine").unwrap();
        assert!(link(&target, &at).is_err());
        assert_eq!(fs::read_to_string(&at).unwrap(), "mine");
    }

    #[test]
    fn the_profile_line_is_added_once() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join(".zprofile");
        fs::write(&profile, "export A=1").unwrap();
        add_to_profile(&profile, true);
        add_to_profile(&profile, true);
        let text = fs::read_to_string(&profile).unwrap();
        assert_eq!(text.matches(PROFILE_MARK).count(), 1);
        assert!(text.starts_with("export A=1\n"));
        add_to_profile(&dir.path().join(".bash_profile"), false);
        assert!(!dir.path().join(".bash_profile").exists());
    }
}
