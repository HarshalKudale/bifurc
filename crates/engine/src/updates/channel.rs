//! How this copy of Bifurc was installed, which decides how it is updated.
//!
//! Every package is the same folder in a different wrapper, so the wrapper says which it is: `cargo xtask
//! package` writes `resources/install.json` (`{"package": "msix"}`) into each one. The tarball is also what
//! the AUR package installs, under `/usr`. A folder with no `install.json` is a build, not an install.

use std::path::Path;

use serde_json::Value;

use crate::utils::json::{opt_str, read_json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Store,
    Winget,
    Portable,
    AppImage,
    Deb,
    Aur,
    Tarball,
    Dmg,
    /// Not an install: a build run from where it was built.
    Unknown,
}

const ALL: [Channel; 9] = [
    Channel::Store,
    Channel::Winget,
    Channel::Portable,
    Channel::AppImage,
    Channel::Deb,
    Channel::Aur,
    Channel::Tarball,
    Channel::Dmg,
    Channel::Unknown,
];

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Store => "store",
            Channel::Winget => "winget",
            Channel::Portable => "portable",
            Channel::AppImage => "appimage",
            Channel::Deb => "deb",
            Channel::Aur => "aur",
            Channel::Tarball => "tarball",
            Channel::Dmg => "dmg",
            Channel::Unknown => "unknown",
        }
    }

    fn named(name: &str) -> Option<Self> {
        ALL.into_iter().find(|channel| channel.as_str() == name.trim().to_ascii_lowercase())
    }

    /// The name in the manifest's `channels` of the channel that delivers this install's updates, for the
    /// channels that publish a release some time after it is made. The others have a release as soon as it is
    /// on the releases page. A winget install is updated from the Store, so it follows the Store.
    pub fn manifest_key(self) -> Option<&'static str> {
        match self {
            Channel::Store | Channel::Winget => Some("store"),
            Channel::Aur => Some(self.as_str()),
            _ => None,
        }
    }

    /// The name, in a release's `packages`, of the file this install is replaced with.
    pub fn package_key(self) -> Option<String> {
        let arm = std::env::consts::ARCH == "aarch64";
        let package = self.package()?;
        let arch = match package {
            "appimage" | "tarball" => {
                if arm {
                    "aarch64"
                } else {
                    "x86_64"
                }
            }
            "deb" => {
                if arm {
                    "arm64"
                } else {
                    "amd64"
                }
            }
            _ => {
                if arm {
                    "arm64"
                } else {
                    "x64"
                }
            }
        };
        Some(format!("{package}-{arch}"))
    }

    /// The kind of release file this install is, whose link a download is offered with.
    pub fn package(self) -> Option<&'static str> {
        match self {
            Channel::Portable => Some("zip"),
            Channel::AppImage => Some("appimage"),
            Channel::Deb => Some("deb"),
            Channel::Tarball => Some("tarball"),
            Channel::Dmg => Some("dmg"),
            _ => None,
        }
    }

    /// Whether `install.sh` reinstalls this install, and the settings it is run with so it puts the same kind
    /// back: a tarball stays a tarball on a system that has apt, and a .deb asks for root through polkit, since
    /// no terminal is there to type a password in. The AppImage is not one the script makes.
    pub fn script_settings(self) -> Option<&'static [(&'static str, &'static str)]> {
        match self {
            Channel::Dmg => Some(&[]),
            Channel::Tarball => Some(&[("BIFURC_METHOD", "tarball")]),
            Channel::Deb => Some(&[("BIFURC_METHOD", "deb"), ("BIFURC_ELEVATE", "pkexec")]),
            _ => None,
        }
    }

    /// Whether this is the Store's package, which `update.apply` updates with winget.
    pub fn uses_winget(self) -> bool {
        matches!(self, Channel::Store | Channel::Winget)
    }

    /// The command that updates an install a package manager owns.
    pub fn update_command(self) -> Option<&'static str> {
        match self {
            Channel::Aur => Some("yay -Syu bifurc-bin"),
            _ => None,
        }
    }
}

/// The channel of the app in `app_dir`: the folder the engine's executable is in.
pub fn channel_of(app_dir: &Path) -> Channel {
    let marker = read_json(&app_dir.join("resources").join("install.json")).unwrap_or(Value::Null);
    let path = app_dir.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    match opt_str(&marker, "package") {
        Some("msix") => Channel::Store,
        Some("zip") => Channel::Portable,
        Some("appimage") => Channel::AppImage,
        Some("deb") => Channel::Deb,
        Some("tarball") if path.starts_with("/usr/") => Channel::Aur,
        Some("tarball") => Channel::Tarball,
        Some("dmg") => Channel::Dmg,
        _ => Channel::Unknown,
    }
}

/// This engine's channel. `BIFURC_INSTALL_CHANNEL` names one outright, which is how a build is made to act
/// like an install.
pub fn detect() -> Channel {
    if let Some(named) = std::env::var("BIFURC_INSTALL_CHANNEL").ok().and_then(|name| Channel::named(&name)) {
        return named;
    }
    match std::env::current_exe().ok().as_deref().and_then(Path::parent) {
        Some(dir) => channel_of(dir),
        None => Channel::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(package: &str, parents: &[&str]) -> (tempfile::TempDir, std::path::PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let dir = parents.iter().fold(root.path().to_path_buf(), |dir, part| dir.join(part));
        std::fs::create_dir_all(dir.join("resources")).unwrap();
        std::fs::write(dir.join("resources").join("install.json"), format!(r#"{{"package":"{package}"}}"#)).unwrap();
        (root, dir)
    }

    #[test]
    fn a_folder_without_a_marker_is_not_an_install() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(channel_of(root.path()), Channel::Unknown);
    }

    #[test]
    fn each_wrapper_is_its_channel() {
        for (package, channel) in [
            ("msix", Channel::Store),
            ("zip", Channel::Portable),
            ("appimage", Channel::AppImage),
            ("deb", Channel::Deb),
            ("tarball", Channel::Tarball),
            ("dmg", Channel::Dmg),
        ] {
            let (_root, dir) = app(package, &["app"]);
            assert_eq!(channel_of(&dir), channel, "{package}");
        }
    }

    #[test]
    fn channels_are_named_both_ways() {
        for channel in ALL {
            assert_eq!(Channel::named(channel.as_str()), Some(channel));
        }
        assert_eq!(Channel::named("nope"), None);
    }
}
