//! Knowing that a newer Bifurc exists, and how this install gets it.
//!
//! The engine reads the update manifest ([`manifest`]) once a day and keeps it in `update.json` in the data
//! directory, so an interface has the answer as soon as it asks. What the manifest means depends on how this
//! copy was installed ([`channel`]). Every Windows install but the portable zip is the Store's package (the
//! Store, Microsoft's installer for it and winget all install it), so the Store updates them all and the
//! action opens Bifurc's page there. A package manager updates its own installs. On macOS and Linux the
//! action is `install`: `update.apply` runs `install.sh` from the website in a process of its own, which
//! installs the new version over this one, then stops the engine and opens Bifurc again;
//! `event.update.progress` follows it. Anything else is offered a download.
//!
//! `update.status` says what there is and what to do about it, and `event.update.changed` says when that
//! changed. The request for the manifest carries the engine's version and nothing else.
//!
//! Off with `"updateCheckEnabled": false` in `app.json` or `BIFURC_UPDATE_CHECK=0`; asking with `update.check`
//! still works then. Nothing is asked on its own before the terms and the privacy policy were accepted, or by
//! a build that is not an install.

pub mod channel;
pub mod manifest;

use std::{path::PathBuf, sync::LazyLock, time::Duration};

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

/// The manifest, and the `stable.json` asset of the latest release it is served from, for when the website does not answer.
const MANIFEST_URLS: [&str; 2] = [
    "https://bifurc.app/updates/v1/stable.json",
    "https://github.com/HarshalKudale/bifurc-releases/releases/latest/download/stable.json",
];
/// Bifurc's page in the Store app, which has the Update button for an install that has one to take.
const STORE_PAGE_URL: &str = "ms-windows-store://pdp/?productid=9NCB4FDZZQPV";
/// What `update.apply` runs: the same script a person pastes into a terminal.
const INSTALL_SCRIPT_URL: &str = "https://bifurc.app/install.sh";
/// Runs the install script, and once it has installed the new version stops this engine (which closes the
/// interfaces) and opens the interface again. A failed script leaves the running Bifurc as it was. The script
/// is saved before it runs, not piped: a pipe's status is the shell's, which succeeds on a download that failed.
const REINSTALL: &str = r#"
progress() { printf '{"state":"%s","version":"%s"%s}\n' "$1" "$BIFURC_VERSION" "$2" > "$BIFURC_UPDATE_PROGRESS"; }
progress installing ""
script="$BIFURC_UPDATE_LOG.sh"
if { curl -fsSL -o "$script" "$BIFURC_INSTALL_SCRIPT" && sh "$script"; } > "$BIFURC_UPDATE_LOG" 2>&1; then
  rm -f "$script"
  progress done ""
  "$BIFURC_APP_DIR/bin/bifurc" engine stop --no-start > /dev/null 2>&1
  sleep 1
  "$BIFURC_APP_DIR/bifurc-ui" > /dev/null 2>&1 &
else
  rm -f "$script"
  progress failed ",\"error\":\"the install script failed; what it said is in $BIFURC_UPDATE_LOG\""
fi
"#;
const USER_AGENT: &str = concat!("bifurc-engine/", env!("CARGO_PKG_VERSION"));
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// A manifest is a few kilobytes; anything much larger is not one.
const MAX_MANIFEST_BYTES: usize = 512 * 1024;
/// The first look is left until the engine has started what it was started for.
const FIRST_LOOK_AFTER: Duration = Duration::from_secs(30);
const LOOK_EVERY: Duration = Duration::from_secs(60 * 60);
const CHECK_EVERY_MS: i64 = 24 * 60 * 60 * 1000;
/// Where the reinstall writes how far it is, and what the install script said, in the data directory.
const PROGRESS_FILE: &str = "update-progress.json";
const LOG_FILE: &str = "update.log";
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

/// What a person does to get the update: open the Store, let Bifurc reinstall itself, run a package
/// manager's command, or download a file.
fn action(channel: Channel, available: &Value) -> Value {
    if available.is_null() {
        return Value::Null;
    }
    if matches!(channel, Channel::Store | Channel::Installer | Channel::Winget) {
        return json!({ "kind": "store", "url": STORE_PAGE_URL });
    }
    if channel.script_settings().is_some() {
        return json!({ "kind": "install" });
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
        "action": action(channel, &available),
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
    status_from(&stored(), ENGINE_VERSION, channel::detect(), enabled(), checking, error)
}

fn progress_path() -> PathBuf {
    data_dir().join(PROGRESS_FILE)
}

/// Reinstall Bifurc with `install.sh`, in a process of its own that outlives this engine, which it stops once
/// the new version is in. What it does is read back from its progress file and announced as
/// `event.update.progress`. The version comes from the manifest this engine stored, never from the caller.
pub fn apply() -> Result<Value, EngineError> {
    let status = status();
    let Some(settings) = channel::detect().script_settings() else {
        return Err(EngineError::bad_request("This install is not updated by Bifurc itself."));
    };
    if status["action"]["kind"] != "install" {
        return Err(EngineError::bad_request("There is no update this install can apply."));
    }
    let version = str_of(&status["available"], "version").to_string();
    let exe = std::env::current_exe().map_err(|e| EngineError::engine(e.to_string()))?;
    let app_dir = exe.parent().ok_or_else(|| EngineError::engine("The engine has no folder."))?;
    let mut command = std::process::Command::new("/bin/sh");
    command
        .args(["-c", REINSTALL])
        .envs(settings.iter().copied())
        .env("BIFURC_VERSION", &version)
        .env("BIFURC_INSTALL_SCRIPT", INSTALL_SCRIPT_URL)
        .env("BIFURC_APP_DIR", app_dir)
        .env("BIFURC_DATA_DIR", data_dir())
        .env("BIFURC_UPDATE_PROGRESS", progress_path())
        .env("BIFURC_UPDATE_LOG", data_dir().join(LOG_FILE))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // It outlives this engine, which it stops.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let _ = std::fs::remove_file(progress_path());
    command.spawn().map_err(|e| {
        EngineError::engine(format!("The update could not be started: {e}"))
            .keyed("updates.updater_failed", json!({ "detail": e.to_string() }))
    })?;
    bus().emit("update.progress", json!({ "state": "starting", "version": version }));
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
                "channels": { "store": "0.6.0" },
                "releases": [{
                    "version": "0.6.0",
                    "critical": critical,
                    "packages": { "nsis-x64": { "url": "https://github.com/HarshalKudale/bifurc-releases/releases/download/v0.6.0/setup.exe" } }
                }]
            }
        })
    }

    fn status_of(stored: &Value, current: &str, channel: Channel) -> Value {
        status_from(stored, current, channel, true, false, None)
    }

    #[test]
    fn every_windows_install_but_the_zip_is_updated_from_the_store() {
        let stored = stored_with(None, false);
        let store = json!({ "kind": "store", "url": STORE_PAGE_URL });
        for channel in [Channel::Store, Channel::Installer, Channel::Winget] {
            assert_eq!(status_of(&stored, "0.5.0", channel)["action"], store, "{channel:?}");
        }
        assert_eq!(status_of(&stored, "0.5.0", Channel::Portable)["action"]["kind"], "download");
    }

    #[test]
    fn macos_and_linux_installs_are_reinstalled_by_the_script() {
        let stored = stored_with(None, false);
        for channel in [Channel::Dmg, Channel::Tarball, Channel::Deb] {
            assert_eq!(status_of(&stored, "0.5.0", channel)["action"], json!({ "kind": "install" }), "{channel:?}");
        }
        // The script makes no AppImage, and a package manager updates its own.
        assert_eq!(status_of(&stored, "0.5.0", Channel::AppImage)["action"]["kind"], "download");
        assert_eq!(status_of(&stored, "0.5.0", Channel::Aur)["action"]["kind"], "command");
    }

    #[test]
    fn an_install_from_the_store_waits_for_the_store_to_have_the_release() {
        let mut stored = stored_with(None, false);
        stored["manifest"]["channels"]["store"] = json!("0.5.0");
        for channel in [Channel::Store, Channel::Installer, Channel::Winget] {
            assert_eq!(status_of(&stored, "0.5.0", channel)["available"], Value::Null, "{channel:?}");
        }
        assert_eq!(status_of(&stored, "0.5.0", Channel::Dmg)["available"]["version"], "0.6.0");
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
            json!({ "kind": "store", "url": STORE_PAGE_URL })
        );
        assert_eq!(
            status_of(&stored, "0.5.0", Channel::Aur)["action"],
            json!({ "kind": "command", "command": "yay -Syu bifurc-bin" })
        );
        // No file is named for an AppImage here, so the download page it is.
        assert_eq!(status_of(&stored, "0.5.0", Channel::AppImage)["action"]["url"], manifest::DOWNLOAD_PAGE);
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
