//! How this copy of Bifurc was installed, which decides how it is updated.
//!
//! Every package is the same folder in a different wrapper, so the wrapper says which it is: `cargo xtask
//! package` writes `resources/install.json` (`{"package": "nsis"}`) into each one. Two wrappers are shared by
//! more than one channel, and those are told apart here: the installer is what winget runs, and
//! it says so with `/CHANNEL=winget`, which the installer keeps in `install-channel` beside the app; the zip is
//! what Scoop unpacks, under a folder of its own. A folder with no `install.json` is a build, not an install.

use std::path::Path;

use serde_json::Value;

use crate::utils::json::{opt_str, read_json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Store,
    Installer,
    Winget,
    Scoop,
    Portable,
    AppImage,
    Deb,
    Aur,
    Tarball,
    Dmg,
    /// Not an install: a build run from where it was built.
    Unknown,
}

const ALL: [Channel; 11] = [
    Channel::Store,
    Channel::Installer,
    Channel::Winget,
    Channel::Scoop,
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
            Channel::Installer => "installer",
            Channel::Winget => "winget",
            Channel::Scoop => "scoop",
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

    /// The name of this channel in the manifest's `channels`, for the channels that publish a release some
    /// time after it is made. The others have a release as soon as it is on the releases page.
    pub fn manifest_key(self) -> Option<&'static str> {
        match self {
            Channel::Store | Channel::Winget | Channel::Scoop | Channel::Aur => Some(self.as_str()),
            _ => None,
        }
    }

    /// The name, in a release's `packages`, of the file this install is replaced with.
    pub fn package_key(self) -> Option<String> {
        let arm = std::env::consts::ARCH == "aarch64";
        let package = self.updater_package()?;
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

    /// The kind of package `bifurc-updater` installs over this install, for the installs that sit in a folder
    /// the person owns. A package manager that runs the installer (winget) or unpacks the zip (Scoop) leaves the
    /// same files in the same places, so the same package replaces them. The Store's files are the Store's and
    /// the AUR's are root's; a dmg's `Bifurc.app` is in /Applications or ~/Applications, which the person can write.
    pub fn updater_package(self) -> Option<&'static str> {
        match self {
            Channel::Installer | Channel::Winget => Some("nsis"),
            Channel::Scoop | Channel::Portable => Some("zip"),
            Channel::AppImage => Some("appimage"),
            Channel::Deb => Some("deb"),
            Channel::Tarball => Some("tarball"),
            Channel::Dmg => Some("dmg"),
            _ => None,
        }
    }

    /// The command that updates an install a package manager owns.
    pub fn update_command(self) -> Option<&'static str> {
        match self {
            Channel::Winget => Some("winget upgrade --id HarshalKudale.Bifurc"),
            Channel::Scoop => Some("scoop update bifurc"),
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
        Some("nsis") => std::fs::read_to_string(app_dir.join("install-channel"))
            .ok()
            .and_then(|name| Channel::named(&name))
            .filter(|channel| *channel == Channel::Winget)
            .unwrap_or(Channel::Installer),
        Some("zip") if path.contains("/scoop/apps/") => Channel::Scoop,
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
            ("nsis", Channel::Installer),
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
    fn the_installer_remembers_the_package_manager_that_ran_it() {
        let (_root, dir) = app("nsis", &["app"]);
        std::fs::write(dir.join("install-channel"), "winget\r\n").unwrap();
        assert_eq!(channel_of(&dir), Channel::Winget);
        // Only a package manager that runs the installer is believed.
        std::fs::write(dir.join("install-channel"), "store").unwrap();
        assert_eq!(channel_of(&dir), Channel::Installer);
    }

    #[test]
    fn a_zip_under_scoop_is_scoops() {
        let (_root, dir) = app("zip", &["scoop", "apps", "bifurc", "current"]);
        assert_eq!(channel_of(&dir), Channel::Scoop);
    }

    #[test]
    fn channels_are_named_both_ways() {
        for channel in ALL {
            assert_eq!(Channel::named(channel.as_str()), Some(channel));
        }
        assert_eq!(Channel::named("nope"), None);
    }
}
