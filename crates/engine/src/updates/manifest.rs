//! The update manifest, and what it means for one install: which version it can have, and by which steps.
//!
//! The manifest is published at `https://bifurc.app/updates/v1/stable.json`:
//!
//! ```json
//! {
//!   "schema": 1,
//!   "minimumSupported": "0.4.0",
//!   "channels": { "store": "0.4.0", "winget": "0.5.0" },
//!   "releases": [
//!     { "version": "0.5.0", "date": "2026-10-20", "minDirectFrom": "0.4.0", "critical": false,
//!       "summary": "…", "notesUrl": "https://bifurc.app/changelog",
//!       "packages": { "zip-x64": { "url": "https://github.com/…/Bifurc-0.5.0-win-x64.zip", "sha256": "…", "size": 1 } } }
//!   ]
//! }
//! ```
//!
//! `v1` in the address is the contract: a field is only ever added, and a field this engine does not know is
//! ignored. `channels` names the version a store or a package manager has, which is behind the newest release
//! until that store has accepted it; an install from one of them is offered nothing newer. `minDirectFrom` is
//! the oldest version that may be replaced by that release in one step: an older install is taken through the
//! releases in between ([`plan`]).

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
    min_direct_from: Version,
    raw: &'a Value,
}

fn releases(manifest: &Value) -> Vec<Release<'_>> {
    let mut found: Vec<Release> = array_of(manifest, "releases")
        .iter()
        .filter_map(|raw| {
            Some(Release {
                version: parse_version(str_of(raw, "version"))?,
                // A release that does not say may replace any version.
                min_direct_from: opt_str(raw, "minDirectFrom").and_then(parse_version).unwrap_or((0, 0, 0)),
                raw,
            })
        })
        .collect();
    found.sort_by_key(|release| release.version);
    found
}

/// The versions `current` goes through to reach the newest of `releases`, in order: each step is the newest
/// release the version before it may be replaced by. Empty when nothing is newer. When no release accepts the
/// version it stands at, the path ends with the oldest newer one: the nearest thing to a step there is.
fn path(current: Version, releases: &[Release]) -> Vec<Version> {
    let Some(target) = releases.last().map(|release| release.version).filter(|target| *target > current) else {
        return Vec::new();
    };
    let mut steps = Vec::new();
    let mut at = current;
    while at < target {
        let reachable = releases.iter().rev().find(|release| release.version > at && release.min_direct_from <= at);
        let next = match reachable {
            Some(release) => release.version,
            None => match releases.iter().find(|release| release.version > at) {
                Some(release) => release.version,
                None => break,
            },
        };
        steps.push(next);
        at = next;
    }
    steps
}

fn text(version: Version) -> String {
    format!("{}.{}.{}", version.0, version.1, version.2)
}

/// What the manifest offers the install at `current` from `channel`: `null` when it is up to date.
///
/// ```json
/// { "version": "0.6.0", "date": "…", "summary": "…", "notesUrl": "…", "critical": false,
///   "path": ["0.5.0", "0.6.0"], "next": "0.5.0", "downloadUrl": "https://github.com/…" }
/// ```
///
/// `version` is where the install ends up, `next` the step to take now, and `downloadUrl` the file of that
/// step for this install, when the manifest names one and it is where downloads are published, with its
/// `sha256` when the manifest gives that too.
pub fn plan(manifest: &Value, current: &str, channel: Channel) -> Value {
    let Some(current) = parse_version(current) else { return Value::Null };
    let cap = channel.manifest_key().and_then(|key| opt_str(manifest.get("channels")?, key)).and_then(parse_version);
    let mut known = releases(manifest);
    if let Some(cap) = cap {
        known.retain(|release| release.version <= cap);
    }
    let steps = path(current, &known);
    let (Some(next), Some(target)) = (steps.first(), steps.last()) else { return Value::Null };
    let release = |version: &Version| known.iter().find(|release| release.version == *version).map(|r| r.raw);
    let (Some(next_raw), Some(target_raw)) = (release(next), release(target)) else { return Value::Null };
    let package = channel.package_key().and_then(|key| next_raw.get("packages")?.get(key));
    let download = package.and_then(|package| opt_str(package, "url")).and_then(trusted).map(str::to_string);
    let sha256 = download.as_ref().and(package).and_then(|package| opt_str(package, "sha256")).map(str::to_string);
    // A security release anywhere on the way makes the whole update one that should not wait.
    let critical =
        known.iter().any(|r| r.version > current && r.version <= *target && bool_of(r.raw, "critical", false));
    json!({
        "version": text(*target),
        "date": opt_str(target_raw, "date"),
        "summary": opt_str(target_raw, "summary"),
        "notesUrl": opt_str(target_raw, "notesUrl").and_then(trusted),
        "critical": critical,
        "path": steps.iter().map(|step| text(*step)).collect::<Vec<_>>(),
        "next": text(*next),
        "downloadUrl": download,
        "sha256": sha256,
    })
}

/// Whether `current` is older than the oldest version the manifest says is still supported.
pub fn unsupported(manifest: &Value, current: &str) -> bool {
    match (parse_version(current), opt_str(manifest, "minimumSupported").and_then(parse_version)) {
        (Some(current), Some(minimum)) => current < minimum,
        _ => false,
    }
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
            "minimumSupported": "0.4.0",
            "channels": { "store": "0.5.0" },
            "releases": [
                { "version": "0.6.0", "minDirectFrom": "0.5.0", "summary": "six", "notesUrl": "https://bifurc.app/changelog",
                  "packages": file("v0.6.0/Bifurc-0.6.0-win.zip") },
                { "version": "0.5.0", "minDirectFrom": "0.4.0", "critical": true,
                  "packages": file("v0.5.0/Bifurc-0.5.0-win.zip") },
                { "version": "0.4.0", "minDirectFrom": "0.3.0" },
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
        assert_eq!(plan["path"], json!(["0.6.0"]));
        assert_eq!(plan["critical"], false);
        assert!(plan["downloadUrl"].as_str().unwrap().ends_with("Bifurc-0.6.0-win.zip"));
    }

    #[test]
    fn an_old_install_is_taken_through_the_releases_in_between() {
        let plan = plan(&manifest(), "0.3.0", Channel::Portable);
        assert_eq!(plan["version"], "0.6.0");
        assert_eq!(plan["path"], json!(["0.4.0", "0.5.0", "0.6.0"]));
        assert_eq!(plan["next"], "0.4.0");
        // The first step has no file in this manifest, and a security release is on the way.
        assert_eq!(plan["downloadUrl"], Value::Null);
        assert_eq!(plan["critical"], true);
    }

    #[test]
    fn an_install_older_than_every_step_is_still_given_one() {
        assert_eq!(plan(&manifest(), "0.1.0", Channel::Portable)["next"], "0.4.0");
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

    #[test]
    fn a_version_below_the_minimum_is_unsupported() {
        assert!(unsupported(&manifest(), "0.3.9"));
        assert!(!unsupported(&manifest(), "0.4.0"));
        assert!(!unsupported(&json!({}), "0.1.0"));
    }
}
