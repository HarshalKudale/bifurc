//! Anonymous usage analytics, sent to PostHog: what kind of thing the user did, from which client and on which
//! platform. Never a payload, a name, a URL or a path: the only identifier is a random id made for this data
//! directory.
//!
//! A command a client sends becomes at most one event ([`events::finished`]), sent within [`EVENTS_EVERY`]:
//! - what [`events::describe`] makes of it when it succeeds (`mock created`, `request sent`);
//! - its own name (`request.parseCurl`) when it succeeds and nothing describes it;
//! - `command failed`, with its name and the error code, when it fails.
//!
//! Nothing is sent before the terms of service and the privacy policy were accepted on this data directory:
//! accepting them is what allows analytics, so they begin with the acceptance ([`resume`]) and are on from
//! then until the user turns them off.
//!
//! Off when the build has no project key and when `BIFURC_TELEMETRY=0` or `DO_NOT_TRACK=1` is set. With
//! `"telemetryEnabled": false` in `app.json` one event is still sent, once per data directory: `app installed`,
//! so that an install is known of; nothing the user does is. The key comes from `BIFURC_POSTHOG_KEY` when the engine runs,
//! else from when it was built.

pub mod events;

use std::{sync::LazyLock, time::Duration};

use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::{
    ENGINE_VERSION,
    runner::report_html::iso_string,
    runtime::notice,
    utils::{
        id::now_ms,
        json::{merged, read_json, write_json_pretty},
    },
    workspace::{paths::data_dir, settings::load_settings},
};

const DEFAULT_HOST: &str = "https://us.i.posthog.com";
const EVENTS_EVERY: Duration = Duration::from_secs(60);
/// What an engine that cannot reach PostHog keeps; the oldest events go first.
const MAX_QUEUED: usize = 500;
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the last flush may hold up a stopping engine.
const SHUTDOWN_FLUSH: Duration = Duration::from_secs(2);

/// Where events go, and as whom.
#[derive(Clone)]
struct Target {
    key: String,
    host: String,
    install_id: String,
}

/// An event waiting to be sent.
struct Queued {
    replaces: Option<String>,
    event: Value,
}

#[derive(Default)]
struct State {
    /// `None` while analytics are off.
    target: Option<Target>,
    /// The transport of the engine that is running, from [`start`] until [`stop`].
    transport: Option<String>,
    queue: Vec<Queued>,
    flusher: Option<tokio::task::JoinHandle<()>>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Mutex::default);

fn non_empty(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn project_key() -> Option<String> {
    non_empty(std::env::var("BIFURC_POSTHOG_KEY").ok())
        .or_else(|| non_empty(option_env!("BIFURC_POSTHOG_KEY").map(str::to_string)))
}

fn host() -> String {
    non_empty(std::env::var("BIFURC_POSTHOG_HOST").ok())
        .or_else(|| non_empty(option_env!("BIFURC_POSTHOG_HOST").map(str::to_string)))
        .unwrap_or_else(|| DEFAULT_HOST.to_string())
}

fn env_opts_out(telemetry: Option<&str>, do_not_track: Option<&str>) -> bool {
    let off = |value: &str| matches!(value.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no");
    telemetry.is_some_and(off) || do_not_track.is_some_and(|value| !value.trim().is_empty() && !off(value))
}

/// The environment says to send nothing at all.
fn env_opted_out() -> bool {
    env_opts_out(std::env::var("BIFURC_TELEMETRY").ok().as_deref(), std::env::var("DO_NOT_TRACK").ok().as_deref())
}

/// The user turned usage analytics off: only the install itself is reported.
fn switched_off() -> bool {
    !load_settings().telemetry_enabled()
}

fn opted_out() -> bool {
    env_opted_out() || switched_off()
}

/// Whether what `settings` holds allows analytics: the terms and the policy, both accepted.
fn allowed_by(terms_accepted: bool, privacy_accepted: bool) -> bool {
    terms_accepted && privacy_accepted
}

/// The user accepted the terms of service and the privacy policy on this data directory.
fn accepted() -> bool {
    let settings = load_settings();
    allowed_by(settings.terms_accepted(), settings.privacy_accepted())
}

fn analytics_path() -> std::path::PathBuf {
    data_dir().join("analytics.json")
}

/// Whether `stored` (what `analytics.json` holds) says the install was already reported.
fn install_reported(stored: Option<&Value>) -> bool {
    stored.and_then(|v| v.get("installReported")).and_then(Value::as_bool) == Some(true)
}

/// Remember that `app installed` reached PostHog, so it is sent once per data directory.
fn mark_install_reported() {
    let path = analytics_path();
    let stored = read_json(&path).filter(Value::is_object).unwrap_or_else(|| json!({}));
    let _ = write_json_pretty(&path, &merged(&stored, &json!({ "installReported": true })));
}

/// The random id of this data directory, made on first use. It is derived from nothing.
fn install_id() -> String {
    let path = analytics_path();
    let stored = read_json(&path);
    if let Some(id) = stored.as_ref().and_then(|v| v.get("installId")).and_then(Value::as_str).filter(|id| !id.is_empty())
    {
        return id.to_string();
    }
    let id = uuid::Uuid::new_v4().to_string();
    let _ = write_json_pretty(&path, &json!({ "installId": id }));
    id
}

fn event(target: &Target, name: &str, properties: Value) -> Value {
    let common = json!({
        "$lib": "bifurc-engine",
        // No person profile and no location lookup: the events stay anonymous.
        "$process_person_profile": false,
        "$geoip_disable": true,
        // PostHog stores the connecting IP as `$ip` unless the event supplies one; supplying a placeholder keeps the
        // real address out of the event.
        "$ip": "0.0.0.0",
        "engine_version": ENGINE_VERSION,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    });
    json!({ "event": name, "distinct_id": target.install_id, "properties": merged(&common, &properties) })
}

async fn send(target: &Target, batch: Vec<Value>) -> Result<(), String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder().timeout(SEND_TIMEOUT).build().map_err(|e| e.to_string())?;
    let body = json!({ "api_key": target.key, "batch": batch });
    let response = client
        .post(format!("{}/batch/", target.host.trim_end_matches('/')))
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| format!("{e:?}"))?;
    if response.status().is_success() { Ok(()) } else { Err(format!("{} answered {}", target.host, response.status())) }
}

/// Turn analytics on for this engine, if the build has a project key, the terms and the policy were accepted
/// and the user has not opted out. Before they are accepted nothing is sent; [`resume`] begins then.
pub fn start(transport: &str) {
    STATE.lock().transport = Some(transport.to_string());
    begin();
}

/// The terms or the policy were just accepted: begin, if that was what analytics were waiting for.
pub fn resume() {
    begin();
}

fn begin() {
    let transport = {
        let state = STATE.lock();
        // No engine is running, or analytics already are.
        if state.target.is_some() || state.flusher.is_some() {
            return;
        }
        let Some(transport) = state.transport.clone() else { return };
        transport
    };
    let Some(key) = project_key() else { return };
    if env_opted_out() || !accepted() {
        return;
    }
    let target = Target { key, host: host(), install_id: install_id() };
    let installed =
        (!install_reported(read_json(&analytics_path()).as_ref())).then(|| event(&target, "app installed", json!({})));
    // Switched off in the settings: that the app was installed here is still reported, once, and nothing else.
    if switched_off() {
        if let Some(installed) = installed {
            notice(
                "usage analytics are off; that Bifurc was installed on this device is reported once — set \
                 BIFURC_TELEMETRY=0 to send nothing at all",
            );
            tokio::spawn(async move {
                if send(&target, vec![installed]).await.is_ok() {
                    mark_install_reported();
                }
            });
        }
        return;
    }
    notice(
        "anonymous usage analytics are on (what kind of thing you do, never its content) — set \
         \"telemetryEnabled\": false in app.json to report only the install, or BIFURC_TELEMETRY=0 to send nothing",
    );
    let started = event(&target, "engine started", json!({ "transport": transport }));
    let first = target.clone();
    let flusher = tokio::spawn(async move {
        let reports_install = installed.is_some();
        let batch: Vec<Value> = installed.into_iter().chain([started]).collect();
        match send(&first, batch).await {
            Ok(()) if reports_install => mark_install_reported(),
            Ok(()) => {}
            Err(reason) => notice(&format!("usage analytics could not be sent: {reason}")),
        }
        let mut tick = tokio::time::interval(EVENTS_EVERY);
        tick.tick().await;
        loop {
            tick.tick().await;
            flush().await;
        }
    });
    let mut state = STATE.lock();
    state.target = Some(target);
    state.flusher = Some(flusher);
}

fn enqueue(queue: &mut Vec<Queued>, item: Queued) {
    if let Some(key) = &item.replaces {
        queue.retain(|queued| queued.replaces.as_ref() != Some(key));
    }
    if queue.len() >= MAX_QUEUED {
        queue.remove(0);
    }
    queue.push(item);
}

/// What `action` becomes if it succeeds; ask before the handler takes the payload.
pub fn describe(action: &str, payload: &Value) -> Option<events::Event> {
    STATE.lock().target.as_ref()?;
    events::describe(action, payload)
}

/// Queue the one event of a finished command, if it has one. `described` is what [`describe`] said before the
/// command ran, and `result` what it answered: its data, or its error code. `action` must be a registered
/// command and `client` a known client name, so that nothing a caller made up is ever sent.
pub fn record_command(described: Option<events::Event>, action: &str, client: &str, result: Result<&Value, &str>) {
    let mut state = STATE.lock();
    let Some(target) = &state.target else { return };
    let Some(mut finished) = events::finished(described, action, result) else { return };
    finished.properties.insert("client".into(), json!(client));
    let mut body = event(target, &finished.name, Value::Object(finished.properties));
    body["timestamp"] = json!(iso_string(now_ms()));
    enqueue(&mut state.queue, Queued { replaces: finished.replaces, event: body });
}

/// Send the queued events. What could not be sent is kept for the next flush.
pub async fn flush() {
    let (target, queue) = {
        let mut state = STATE.lock();
        let Some(target) = state.target.clone() else { return };
        (target, std::mem::take(&mut state.queue))
    };
    if queue.is_empty() {
        return;
    }
    if opted_out() {
        // Switched off while running: nothing more is counted until the engine starts again.
        STATE.lock().target = None;
        return;
    }
    let batch: Vec<Value> = queue.iter().map(|queued| queued.event.clone()).collect();
    if send(&target, batch).await.is_err() {
        let mut state = STATE.lock();
        let newer = std::mem::replace(&mut state.queue, queue);
        for item in newer {
            enqueue(&mut state.queue, item);
        }
    }
}

/// Send what is left and turn analytics off.
pub async fn stop() {
    let flusher = STATE.lock().flusher.take();
    if let Some(flusher) = flusher {
        flusher.abort();
    }
    let _ = tokio::time::timeout(SHUTDOWN_FLUSH, flush()).await;
    let mut state = STATE.lock();
    state.target = None;
    state.transport = None;
    state.queue.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_can_opt_out() {
        assert!(!env_opts_out(None, None));
        assert!(env_opts_out(Some("0"), None));
        assert!(env_opts_out(Some("Off"), None));
        assert!(!env_opts_out(Some("1"), None));
        assert!(env_opts_out(None, Some("1")));
        assert!(!env_opts_out(None, Some("0")));
        assert!(!env_opts_out(None, Some("")));
    }

    #[test]
    fn nothing_is_allowed_until_the_terms_and_the_policy_are_both_accepted() {
        assert!(!allowed_by(false, false));
        assert!(!allowed_by(true, false));
        assert!(!allowed_by(false, true));
        assert!(allowed_by(true, true));
    }

    #[test]
    fn the_install_is_reported_once() {
        assert!(!install_reported(None));
        assert!(!install_reported(Some(&json!({ "installId": "install-1" }))));
        assert!(install_reported(Some(&json!({ "installId": "install-1", "installReported": true }))));
    }

    #[test]
    fn an_event_carries_the_install_and_nothing_about_the_person() {
        let target = Target { key: "phc_test".into(), host: DEFAULT_HOST.into(), install_id: "install-1".into() };
        let sent = event(&target, "request.parseCurl", json!({ "client": "cli" }));

        assert_eq!(sent["event"], "request.parseCurl");
        assert_eq!(sent["distinct_id"], "install-1");
        let properties = sent["properties"].as_object().unwrap();
        let mut keys: Vec<&str> = properties.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["$geoip_disable", "$ip", "$lib", "$process_person_profile", "arch", "client", "engine_version", "os"]
        );
        assert_eq!(properties["$process_person_profile"], false);
        assert_eq!(properties["$ip"], "0.0.0.0");
    }

    #[test]
    fn a_later_save_replaces_the_one_still_waiting() {
        let item = |key: Option<&str>, n: u32| Queued { replaces: key.map(str::to_string), event: json!(n) };
        let mut queue = Vec::new();
        enqueue(&mut queue, item(Some("mocks/a"), 1));
        enqueue(&mut queue, item(None, 2));
        enqueue(&mut queue, item(Some("mocks/b"), 3));
        enqueue(&mut queue, item(Some("mocks/a"), 4));
        let waiting: Vec<&Value> = queue.iter().map(|queued| &queued.event).collect();
        assert_eq!(waiting, [&json!(2), &json!(3), &json!(4)]);

        for n in 0..(MAX_QUEUED as u32 + 5) {
            enqueue(&mut queue, item(None, n));
        }
        assert_eq!(queue.len(), MAX_QUEUED);
    }
}
