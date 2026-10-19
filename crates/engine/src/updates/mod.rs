//! Knowing that a newer Bifurc exists, and how this install gets it.
//!
//! The engine reads the update manifest ([`manifest`]) once a day and keeps it in `update.json` in the data
//! directory, so an interface has the answer as soon as it asks. What the manifest means depends on how this
//! copy was installed ([`channel`]): the Store updates a Store install, a package manager its own, and the
//! others are replaced by a file from the releases page.
//!
//! This module never downloads or installs a release: `update.status` says what there is and what to do about it,
//! and `event.update.changed` says when that changed. Where `bifurc-updater` sits beside the engine (every
//! package but the Store's) the action is `updater`, and `update.apply` starts it; it downloads the file,
//! checks its hash, closes Bifurc, installs and opens it again, and `event.update.progress` follows it. The
//! request for the manifest carries the engine's version and nothing else.
//!
//! Off with `"updateCheckEnabled": false` in `app.json` or `BIFURC_UPDATE_CHECK=0`; asking with `update.check`
//! still works then. Nothing is asked on its own before the terms and the privacy policy were accepted, or by
//! a build that is not an install.

pub mod channel;
pub mod manifest;

use std::{env::consts::EXE_SUFFIX, path::PathBuf, sync::LazyLock, time::Duration};

use parking_lot::Mutex;
use serde_json::{Value, json};

use self::channel::Channel;
use crate::{
    ENGINE_VERSION,
    kernel::{CommandRegistry, bus},
    protocol::EngineError,
    utils::{
        id::now_ms,
        json::{i64_of, merged, opt_str, read_json, str_of, write_json_pretty},
    },
    workspace::{paths::data_dir, settings::load_settings},
};

/// The manifest, and the same file where it is published from, for when the website does not answer.
const MANIFEST_URLS: [&str; 2] = [
    "https://bifurc.app/updates/v1/stable.json",
    "https://raw.githubusercontent.com/HarshalKudale/bifurc-releases/main/updates/v1/stable.json",
];
const STORE_UPDATES_URL: &str = "ms-windows-store://downloadsandupdates";
const USER_AGENT: &str = concat!("bifurc-engine/", env!("CARGO_PKG_VERSION"));
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// A manifest is a few kilobytes; anything much larger is not one.
const MAX_MANIFEST_BYTES: usize = 512 * 1024;
/// The first look is left until the engine has started what it was started for.
const FIRST_LOOK_AFTER: Duration = Duration::from_secs(30);
const LOOK_EVERY: Duration = Duration::from_secs(60 * 60);
const CHECK_EVERY_MS: i64 = 24 * 60 * 60 * 1000;
/// Where `bifurc-updater` writes how far it is, in the data directory.
const PROGRESS_FILE: &str = "update-progress.json";
/// How long the engine follows an update before it stops looking.
const LISTEN_FOR: Duration = Duration::from_secs(30 * 60);

#[derive(Default)]
struct State {
    checking: bool,
    /// Why the last check failed; cleared by one that succeeds.
    error: Option<EngineError>,
    task: Option<tokio::task::JoinHandle<()>>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Mutex::default);

fn cache_path() -> PathBuf {
    data_dir().join("update.json")
}

/// What `update.json` holds: `{checkedAt, manifest, skipped}`.
fn stored() -> Value {
    read_json(&cache_path()).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

fn env_switched_off(value: Option<&str>) -> bool {
    value.is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no"))
}

/// Whether the engine looks for updates on its own.
fn enabled() -> bool {
    !env_switched_off(std::env::var("BIFURC_UPDATE_CHECK").ok().as_deref()) && load_settings().update_check_enabled()
}

/// The updater packaged beside this engine: only the installs that can be replaced in place have one.
fn updater_path() -> Option<PathBuf> {
    let updater = std::env::current_exe().ok()?.parent()?.join(format!("bifurc-updater{}", EXE_SUFFIX));
    updater.is_file().then_some(updater)
}

fn is_sha256(text: Option<&str>) -> bool {
    text.is_some_and(|text| text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

/// What a person does to get the update: let the updater do it, open the Store, run a package manager's
/// command, or download a file. `self_update` is whether an updater sits beside this engine; the updater is
/// used only for a file the manifest gives a checksum for.
fn action(channel: Channel, available: &Value, self_update: bool) -> Value {
    if available.is_null() {
        return Value::Null;
    }
    if channel == Channel::Store {
        return json!({ "kind": "store", "url": STORE_UPDATES_URL });
    }
    if self_update && channel.updater_package().is_some() && is_sha256(opt_str(available, "sha256")) {
        return json!({ "kind": "updater" });
    }
    if let Some(command) = channel.update_command() {
        return json!({ "kind": "command", "command": command });
    }
    json!({ "kind": "download", "url": opt_str(available, "downloadUrl").unwrap_or(manifest::DOWNLOAD_PAGE) })
}

/// The reply of `update.status` and the payload of `event.update.changed`, from what `update.json` holds.
fn status_from(
    stored: &Value,
    current: &str,
    channel: Channel,
    enabled: bool,
    checking: bool,
    error: Option<EngineError>,
    self_update: bool,
) -> Value {
    let manifest = stored.get("manifest").unwrap_or(&Value::Null);
    let mut available = manifest::plan(manifest, current, channel);
    if let Some(update) = available.as_object_mut() {
        // A security release is not one a person is left to forget about.
        let critical = update.get("critical").and_then(Value::as_bool).unwrap_or(false);
        let skipped = !critical && update.get("version").and_then(Value::as_str) == opt_str(stored, "skipped");
        update.insert("skipped".to_string(), json!(skipped));
    }
    let mut status = json!({
        "currentVersion": current,
        "channel": channel.as_str(),
        "enabled": enabled,
        "checking": checking,
        "checkedAt": stored.get("checkedAt").cloned().unwrap_or(Value::Null),
        "unsupported": manifest::unsupported(manifest, current),
        "action": action(channel, &available, self_update),
        "available": available,
    });
    crate::kernel::streams::put_error(status.as_object_mut().expect("an object"), error.as_ref());
    status
}

pub fn status() -> Value {
    let (checking, error) = {
        let state = STATE.lock();
        (state.checking, state.error.clone())
    };
    status_from(&stored(), ENGINE_VERSION, channel::detect(), enabled(), checking, error, updater_path().is_some())
}

fn progress_path() -> PathBuf {
    data_dir().join(PROGRESS_FILE)
}

/// Hand the update to `bifurc-updater`, which downloads it, stops this engine, installs it and starts the
/// interface again. What the updater does is read back from its progress file and announced as
/// `event.update.progress`. The file to fetch comes from the manifest this engine stored, never from the caller.
pub fn apply() -> Result<Value, EngineError> {
    let status = status();
    let (Some(package), Some(updater)) = (channel::detect().updater_package(), updater_path()) else {
        return Err(EngineError::bad_request("This install is not updated by an updater."));
    };
    if status["action"]["kind"] != "updater" {
        return Err(EngineError::bad_request("There is no update this install can apply."));
    }
    let available = &status["available"];
    let exe = std::env::current_exe().map_err(|e| EngineError::engine(e.to_string()))?;
    let app_dir = exe.parent().ok_or_else(|| EngineError::engine("The engine has no folder."))?;
    let mut command = std::process::Command::new(updater);
    command
        .args(["apply", "--package", package])
        .args(["--url", str_of(available, "downloadUrl")])
        .args(["--sha256", str_of(available, "sha256")])
        .args(["--version", str_of(available, "next")])
        .arg("--app-dir")
        .arg(app_dir)
        .arg("--launch")
        .arg(app_dir.join(format!("bifurc-ui{EXE_SUFFIX}")))
        .arg("--data-dir")
        .arg(data_dir())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(image) = std::env::var_os("APPIMAGE").filter(|image| !image.is_empty()) {
        command.arg("--target").arg(image);
    }
    // It outlives this engine, which it stops.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let _ = std::fs::remove_file(progress_path());
    command.spawn().map_err(|e| {
        EngineError::engine(format!("The updater could not be started: {e}"))
            .keyed("updates.updater_failed", json!({ "detail": e.to_string() }))
    })?;
    bus().emit("update.progress", json!({ "state": "starting", "version": str_of(available, "next") }));
    tokio::spawn(async {
        let mut last = Value::Null;
        for _ in 0..LISTEN_FOR.as_secs() * 2 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let Some(now) = read_json(&progress_path()).filter(Value::is_object) else { continue };
            if now != last {
                bus().emit("update.progress", now.clone());
                last = now;
            }
            if matches!(opt_str(&last, "state"), Some("done" | "failed")) {
                break;
            }
        }
    });
    Ok(json!({ "started": true }))
}

async fn fetch_from(client: &reqwest::Client, url: &str) -> Result<Value, EngineError> {
    let response = client.get(url).send().await.map_err(|e| {
        EngineError::upstream(format!("{url} could not be reached: {e}"))
            .keyed("updates.unreachable", json!({ "url": url, "detail": e.to_string() }))
    })?;
    if !response.status().is_success() {
        let status = response.status();
        return Err(EngineError::upstream(format!("{url} answered {status}"))
            .keyed("updates.bad_status", json!({ "url": url, "status": status.to_string() })));
    }
    let body = response.bytes().await.map_err(|e| {
        EngineError::upstream(format!("{url} could not be read: {e}"))
            .keyed("updates.unreadable", json!({ "url": url, "detail": e.to_string() }))
    })?;
    if body.len() > MAX_MANIFEST_BYTES {
        return Err(EngineError::upstream(format!("{url} is too large to be an update manifest"))
            .keyed("updates.too_large", json!({ "url": url })));
    }
    serde_json::from_slice::<Value>(&body)
        .ok()
        .filter(|manifest| manifest.get("releases").is_some_and(Value::is_array))
        .ok_or_else(|| {
            EngineError::upstream(format!("{url} is not an update manifest"))
                .keyed("updates.not_manifest", json!({ "url": url }))
        })
}

/// Where the manifest is read from, in the order tried. `BIFURC_UPDATE_URL` names another address.
fn manifest_urls() -> Vec<String> {
    match std::env::var("BIFURC_UPDATE_URL").ok().filter(|url| !url.trim().is_empty()) {
        Some(url) => vec![url],
        None => MANIFEST_URLS.iter().map(|url| (*url).to_string()).collect(),
    }
}

/// The manifest, from the first of `urls` that has it.
async fn fetch(urls: Vec<String>) -> Result<Value, EngineError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder().timeout(FETCH_TIMEOUT).user_agent(USER_AGENT).build().map_err(|e| {
        EngineError::engine(e.to_string()).keyed("updates.client_failed", json!({ "detail": e.to_string() }))
    })?;
    let mut failure =
        EngineError::engine("There is no address to look for updates at.").keyed("updates.no_address", json!({}));
    for url in urls {
        match fetch_from(&client, &url).await {
            Ok(manifest) => return Ok(manifest),
            Err(reason) => failure = reason,
        }
    }
    Err(failure)
}

/// Read the manifest now, whatever the settings say, and report what it means. A check that is already
/// running is not joined by a second one.
pub async fn check() -> Value {
    check_at(manifest_urls()).await
}

async fn check_at(urls: Vec<String>) -> Value {
    {
        let mut state = STATE.lock();
        if state.checking {
            drop(state);
            return status();
        }
        state.checking = true;
    }
    bus().emit("update.changed", status());
    let outcome = fetch(urls).await;
    if let Ok(manifest) = &outcome {
        let _ = write_json_pretty(
            &cache_path(),
            &merged(&stored(), &json!({ "checkedAt": now_ms(), "manifest": manifest })),
        );
    }
    {
        let mut state = STATE.lock();
        state.checking = false;
        state.error = outcome.err();
    }
    let status = status();
    bus().emit("update.changed", status.clone());
    status
}

/// Stop announcing `version`; a newer one is announced again.
fn skip(version: &str) -> std::io::Result<()> {
    write_json_pretty(&cache_path(), &merged(&stored(), &json!({ "skipped": version })))?;
    bus().emit("update.changed", status());
    Ok(())
}

/// Whether it is time for the engine to look on its own.
fn due() -> bool {
    let settings = load_settings();
    enabled()
        && settings.terms_accepted()
        && settings.privacy_accepted()
        && channel::detect() != Channel::Unknown
        && now_ms() - i64_of(&stored(), "checkedAt", 0) >= CHECK_EVERY_MS
}

/// Look for updates once a day for as long as the engine runs.
pub fn start() {
    let task = tokio::spawn(async {
        tokio::time::sleep(FIRST_LOOK_AFTER).await;
        loop {
            if due() {
                check().await;
            }
            tokio::time::sleep(LOOK_EVERY).await;
        }
    });
    if let Some(earlier) = STATE.lock().task.replace(task) {
        earlier.abort();
    }
}

pub fn stop() {
    if let Some(task) = STATE.lock().task.take() {
        task.abort();
    }
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("update.status", |_, _| Ok(status()));
    registry.register("update.check", |_, _| async { Ok(check().await) });
    registry.register("update.apply", |_, _| async { apply() });
    registry.register_sync("update.skip", |params, _| {
        skip(str_of(&params, "version"))?;
        Ok(status())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored_with(skipped: Option<&str>, critical: bool) -> Value {
        json!({
            "checkedAt": 5,
            "skipped": skipped,
            "manifest": {
                "minimumSupported": "0.4.0",
                "channels": { "store": "0.6.0", "scoop": "0.6.0" },
                "releases": [{
                    "version": "0.6.0",
                    "critical": critical,
                    "packages": { "nsis-x64": { "url": "https://github.com/HarshalKudale/bifurc-releases/releases/download/v0.6.0/setup.exe" } }
                }]
            }
        })
    }

    fn status_of(stored: &Value, current: &str, channel: Channel) -> Value {
        status_from(stored, current, channel, true, false, None, false)
    }

    #[test]
    fn an_install_with_an_updater_is_updated_by_it_when_the_file_has_a_checksum() {
        let file = |name: &str, hash: Option<&str>| json!({ "url": format!("https://github.com/HarshalKudale/bifurc-releases/releases/download/v0.6.0/{name}"), "sha256": hash });
        let mut stored = stored_with(None, false);
        stored["manifest"]["releases"][0]["packages"] = json!({
            "nsis-x64": file("setup.exe", Some(&"a".repeat(64))),
            "zip-x64": file("app.zip", None),
        });
        let action = |channel, self_update| {
            status_from(&stored, "0.5.0", channel, true, false, None, self_update)["action"].clone()
        };
        if std::env::consts::ARCH != "aarch64" {
            let updater = json!({ "kind": "updater" });
            assert_eq!(action(Channel::Installer, true), updater);
            // winget runs the same installer, so the same file replaces its install.
            assert_eq!(action(Channel::Winget, true), updater);
            assert_eq!(action(Channel::Installer, false)["kind"], "download");
            // A file with no checksum is not run.
            assert_eq!(action(Channel::Portable, true)["kind"], "download");
        }
        // The Store's files are the Store's, whatever sits beside the engine.
        assert_eq!(action(Channel::Store, true)["kind"], "store");
        assert_eq!(action(Channel::Aur, true)["kind"], "command");
    }

    #[test]
    fn nothing_is_known_before_the_first_check() {
        let status = status_of(&json!({}), "0.5.0", Channel::Installer);
        assert_eq!(status["available"], Value::Null);
        assert_eq!(status["action"], Value::Null);
        assert_eq!(status["checkedAt"], Value::Null);
        assert_eq!(status["unsupported"], false);
    }

    #[test]
    fn each_channel_is_told_its_own_way_to_update() {
        let stored = stored_with(None, false);
        assert_eq!(
            status_of(&stored, "0.5.0", Channel::Store)["action"],
            json!({ "kind": "store", "url": STORE_UPDATES_URL })
        );
        assert_eq!(
            status_of(&stored, "0.5.0", Channel::Scoop)["action"],
            json!({ "kind": "command", "command": "scoop update bifurc" })
        );
        if std::env::consts::ARCH != "aarch64" {
            let installer = status_of(&stored, "0.5.0", Channel::Installer);
            assert!(installer["action"]["url"].as_str().unwrap().ends_with("/v0.6.0/setup.exe"));
        }
        // No file is named for a .deb here, so the download page it is.
        assert_eq!(status_of(&stored, "0.5.0", Channel::Deb)["action"]["url"], manifest::DOWNLOAD_PAGE);
        assert_eq!(status_of(&stored, "0.6.0", Channel::Installer)["action"], Value::Null);
    }

    #[test]
    fn a_skipped_version_stays_skipped_unless_it_is_a_security_release() {
        let skipped = |stored: &Value| status_of(stored, "0.5.0", Channel::Installer)["available"]["skipped"].clone();
        assert_eq!(skipped(&stored_with(None, false)), false);
        assert_eq!(skipped(&stored_with(Some("0.6.0"), false)), true);
        assert_eq!(skipped(&stored_with(Some("0.5.5"), false)), false);
        assert_eq!(skipped(&stored_with(Some("0.6.0"), true)), false);
    }

    #[test]
    fn an_old_version_is_reported_as_unsupported() {
        assert_eq!(status_of(&stored_with(None, false), "0.3.0", Channel::Installer)["unsupported"], true);
    }

    /// Answers every request on a local port with `body`, and returns the address.
    async fn serve(body: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}/stable.json", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut request = [0u8; 2048];
                let _ = stream.read(&mut request).await;
                let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(format!("{head}{body}").as_bytes()).await;
            }
        });
        address
    }

    #[tokio::test]
    async fn a_check_keeps_the_manifest_and_a_failed_one_keeps_what_was_known() {
        let _root = crate::testing::scratch_data_root();
        let manifest = serve(r#"{"releases":[{"version":"99.0.0","summary":"Later."}]}"#).await;
        let other = serve("<html>not a manifest</html>").await;

        // The first address has no manifest, so the second is asked.
        let status = check_at(vec![other.clone(), manifest]).await;
        assert_eq!(status["error"], Value::Null);
        assert_eq!(status["checking"], false);
        assert!(status["checkedAt"].as_i64().unwrap() > 0);
        assert_eq!(status["available"]["version"], "99.0.0");
        assert_eq!(status["available"]["skipped"], false);
        assert_eq!(stored()["manifest"]["releases"][0]["summary"], "Later.");

        skip("99.0.0").unwrap();
        assert_eq!(super::status()["available"]["skipped"], true);

        let failed = check_at(vec![other]).await;
        assert!(failed["error"].as_str().unwrap().ends_with("is not an update manifest"), "{failed}");
        assert_eq!(failed["errorKey"], "updates.not_manifest");
        assert!(failed["errorParams"]["url"].as_str().unwrap().starts_with("http://"), "{failed}");
        assert_eq!(failed["available"]["version"], "99.0.0");
        STATE.lock().error = None;
    }

    #[test]
    fn the_environment_switches_the_checks_off() {
        assert!(env_switched_off(Some("0")));
        assert!(env_switched_off(Some(" Off ")));
        assert!(!env_switched_off(Some("1")));
        assert!(!env_switched_off(None));
    }
}
