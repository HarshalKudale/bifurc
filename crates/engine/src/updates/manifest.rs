//! The update manifest, and what it means for one install: the newest version it can have, and where from.
//!
//! The manifest is published at `https://bifurc.app/updates/v1/stable.json`:
//!
//! ```json
//! {
//!   "schema": 1,
//!   "channels": { "store": "0.4.0", "winget": "0.5.0" },
//!   "releases": [
//!     { "version": "0.5.0", "date": "2026-10-20", "critical": false,
//!       "summary": "…", "notesUrl": "https://bifurc.app/changelog",
//!       "packages": { "zip-x64": { "url": "https://github.com/…/Bifurc-0.5.0-win-x64.zip", "sha256": "…", "size": 1 } } }
//!   ]
//! }
//! ```
//!
//! `v1` in the address is the contract: a field is only ever added, and a field this engine does not know is
//! ignored. `channels` names the version a store or a package manager has, which is behind the newest release
//! until that store has accepted it; an install from one of them is offered nothing newer. Any version is
//! replaced by the newest in one step: the new version migrates the data it finds (`workspace::migrations`).

use serde_json::{Value, json};

use super::channel::Channel;
use crate::utils::json::{array_of, bool_of, opt_str, str_of};

/// Where a download may come from, and where a link may lead: the releases repository and the website.
const TRUSTED_PREFIXES: [&str; 2] = ["https://github.com/HarshalKudale/bifurc-releases/", "https://bifurc.app/"];
/// Where a person gets Bifurc when the manifest names no file for their install.
pub const DOWNLOAD_PAGE: &str = "https://bifurc.app/download";

pub type Version = (u64, u64, u64);

/// `major.minor.patch`, with an optional `v` in front. A pre-release suffix is not a version an install is
/// offered, so it does not parse.
pub fn parse_version(text: &str) -> Option<Version> {
    let mut parts = text.trim().trim_start_matches('v').split('.').map(|part| part.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

fn trusted(url: &str) -> Option<&str> {
    TRUSTED_PREFIXES.iter().any(|prefix| url.starts_with(prefix)).then_some(url)
}

struct Release<'a> {
    version: Version,
    raw: &'a Value,
}

fn releases(manifest: &Value) -> Vec<Release<'_>> {
    let mut found: Vec<Release> = array_of(manifest, "releases")
        .iter()
        .filter_map(|raw| Some(Release { version: parse_version(str_of(raw, "version"))?, raw }))
        .collect();
    found.sort_by_key(|release| release.version);
    found
}

fn text(version: Version) -> String {
    format!("{}.{}.{}", version.0, version.1, version.2)
}

/// What the manifest offers the install at `current` from `channel`: `null` when it is up to date.
///
/// ```json
/// { "version": "0.6.0", "date": "…", "summary": "…", "notesUrl": "…", "critical": false,
///   "downloadUrl": "https://github.com/…" }
/// ```
///
/// `version` is the newest release this install can have, and `downloadUrl` its file for this install, when the
/// manifest names one and it is where downloads are published, with its `sha256` when the manifest gives that too.
pub fn plan(manifest: &Value, current: &str, channel: Channel) -> Value {
    let Some(current) = parse_version(current) else { return Value::Null };
    let cap = channel.manifest_key().and_then(|key| opt_str(manifest.get("channels")?, key)).and_then(parse_version);
    let mut known = releases(manifest);
    if let Some(cap) = cap {
        known.retain(|release| release.version <= cap);
    }
    let Some(newest) = known.last().filter(|release| release.version > current) else { return Value::Null };
    let package = channel.package_key().and_then(|key| newest.raw.get("packages")?.get(key));
    let download = package.and_then(|package| opt_str(package, "url")).and_then(trusted).map(str::to_string);
    let sha256 = download.as_ref().and(package).and_then(|package| opt_str(package, "sha256")).map(str::to_string);
    // A security release anywhere since this version makes the whole update one that should not wait.
    let critical = known.iter().any(|r| r.version > current && bool_of(r.raw, "critical", false));
    json!({
        "version": text(newest.version),
        "date": opt_str(newest.raw, "date"),
        "summary": opt_str(newest.raw, "summary"),
        "notesUrl": opt_str(newest.raw, "notesUrl").and_then(trusted),
        "critical": critical,
        "downloadUrl": download,
        "sha256": sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name a portable zip's file has in `packages` on this machine.
    fn zip() -> String {
        Channel::Portable.package_key().unwrap()
    }

    fn manifest() -> Value {
        let file = |name: &str| json!({ zip(): { "url": format!("https://github.com/HarshalKudale/bifurc-releases/releases/download/{name}") } });
        json!({
            "schema": 1,
            "channels": { "store": "0.5.0" },
            "releases": [
                { "version": "0.6.0", "summary": "six", "notesUrl": "https://bifurc.app/changelog",
                  "packages": file("v0.6.0/Bifurc-0.6.0-win.zip") },
                { "version": "0.5.0", "critical": true,
                  "packages": file("v0.5.0/Bifurc-0.5.0-win.zip") },
                { "version": "0.4.0" },
                { "version": "0.7.0-beta.1" }
            ]
        })
    }

    #[test]
    fn versions_parse_strictly() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.10.0"), Some((0, 10, 0)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3-beta.1"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
    }

    #[test]
    fn an_install_on_the_newest_version_is_offered_nothing() {
        assert_eq!(plan(&manifest(), "0.6.0", Channel::Portable), Value::Null);
        assert_eq!(plan(&manifest(), "0.9.0", Channel::Portable), Value::Null);
    }

    #[test]
    fn a_recent_install_goes_straight_to_the_newest() {
        let plan = plan(&manifest(), "0.5.0", Channel::Portable);
        assert_eq!(plan["version"], "0.6.0");
        assert_eq!(plan["critical"], false);
        assert!(plan["downloadUrl"].as_str().unwrap().ends_with("Bifurc-0.6.0-win.zip"));
    }

    #[test]
    fn an_old_install_goes_straight_to_the_newest_too() {
        let plan = plan(&manifest(), "0.1.0", Channel::Portable);
        assert_eq!(plan["version"], "0.6.0");
        assert!(plan["downloadUrl"].as_str().unwrap().ends_with("Bifurc-0.6.0-win.zip"));
        // A security release came out since this version.
        assert_eq!(plan["critical"], true);
    }

    #[test]
    fn a_store_install_is_offered_what_the_store_has() {
        assert_eq!(plan(&manifest(), "0.4.0", Channel::Store)["version"], "0.5.0");
        assert_eq!(plan(&manifest(), "0.5.0", Channel::Store), Value::Null);
    }

    #[test]
    fn a_download_from_anywhere_else_is_not_offered() {
        let mut manifest = manifest();
        manifest["releases"][0]["packages"][zip()]["url"] = json!("https://example.com/Bifurc-0.6.0-win.zip");
        manifest["releases"][0]["notesUrl"] = json!("http://bifurc.app/changelog");
        let plan = plan(&manifest, "0.5.0", Channel::Portable);
        assert_eq!(plan["downloadUrl"], Value::Null);
        assert_eq!(plan["notesUrl"], Value::Null);
    }
}
