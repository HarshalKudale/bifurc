//! Usage analytics, sent to PostHog: what kind of thing the user did, from which client and on which
//! platform. Never a payload, a name, a URL or a path.
//!
//! Whose decision they are: the user's, with one app-level switch. They are anonymous, on by default, and off
//! with `"telemetryEnabled": false` in `app.json` or the environment. The only identifier is a random id made
//! for this data directory. Every event of something done in a workspace ([`events::workspace_of`]) carries
//! that workspace's own id as `workspace_id`, whatever the workspace is (local only, a git repository, synced,
//! imported by its owner or by a collaborator); an event that belongs to no workspace carries none.
//!
//! An `organization` an older version left in a workspace of `app.json` is ignored.
//!
//! A command a client sends becomes at most one event ([`events::finished`]), sent within [`EVENTS_EVERY`]:
//! - what [`events::describe`] makes of it when it succeeds (`mock created`, `request sent`);
//! - its own name (`request.parseCurl`) when it succeeds and nothing describes it;
//! - `command failed`, with its name and the error code, when it fails.
//!
//! Two more kinds of event are no command's, and always anonymous:
//! - `engine failure`, with a fixed word for what failed in the background ([`failed`]);
//! - a crash report a Bifurc process left in the data directory (`bifurc_protocol::crash`), read here because
//!   the engine is the one process that may send anything: a panic as PostHog's `$exception`, with the file
//!   and line of Bifurc's code and never the message, and `app failure` for what failed before an engine was
//!   there to be told ([`collect_crashes`]).
//!
//! Nothing is sent before the terms of service and the privacy policy were accepted on this data directory:
//! accepting them is what allows analytics, so they begin with the acceptance ([`resume`]).
//!
//! Nothing at all is sent by a build with no project key. The anonymous analytics are off when
//! `BIFURC_TELEMETRY=0` or `DO_NOT_TRACK=1` is set. With `"telemetryEnabled": false` in `app.json` one
//! anonymous event is still sent, once per data directory: `app installed`, so that an install is known of;
//! nothing the user does is. The key comes from `BIFURC_POSTHOG_KEY` when the engine runs, else from when it
//! was built.

pub mod events;

use std::{
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::{
    ENGINE_VERSION,
    runner::report_html::iso_string,
    runtime::notice,
    utils::{
        id::now_ms,
        json::{merged, read_json, truthy, write_json_pretty},
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
/// How long what `app.json` says about the switch is taken as still so. A command is
/// routed by it, and `app.json` is not read for every one.
const ROUTING_TTL: Duration = Duration::from_secs(5);
/// How many crash reports stay in the data directory for the user to look at or attach to an issue.
const KEPT_CRASHES: usize = 20;

/// Where events go.
#[derive(Clone)]
struct Target {
    key: String,
    host: String,
    /// Whom the anonymous events are from. `None` when the environment turned them off: the id is then never
    /// made.
    install_id: Option<String>,
}

/// An event waiting to be sent.
struct Queued {
    replaces: Option<String>,
    event: Value,
}

/// What decides where an event goes: the user's own choice, and nothing else.
#[derive(Debug, Default)]
struct Routing {
    anonymous: bool,
}

/// Where the one event of a command goes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Route {
    /// To the anonymous analytics, as this data directory.
    Anonymous,
    Nowhere,
}

/// Every workspace, and what belongs to none, follows the user's switch.
fn route(routing: &Routing) -> Route {
    if routing.anonymous { Route::Anonymous } else { Route::Nowhere }
}

fn read_routing() -> Routing {
    let settings = load_settings();
    Routing {
        anonymous: !env_opted_out() && settings.telemetry_enabled(),
    }
}

#[derive(Default)]
struct State {
    /// `None` until analytics begin, and in a build with no project key.
    target: Option<Target>,
    /// The transport of the engine that is running, from [`start`] until [`stop`].
    transport: Option<String>,
    queue: Vec<Queued>,
    flusher: Option<tokio::task::JoinHandle<()>>,
    routing: Option<(Instant, Arc<Routing>)>,
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

fn routing() -> Arc<Routing> {
    if let Some((read_at, routing)) = &STATE.lock().routing {
        if read_at.elapsed() < ROUTING_TTL {
            return routing.clone();
        }
    }
    let fresh = Arc::new(read_routing());
    STATE.lock().routing = Some((Instant::now(), fresh.clone()));
    fresh
}

/// What an event done in a workspace says on top of what was done: the workspace's own id on this device, which
/// is a random id and no name. Nothing about the account or any organization.
fn workspace_properties(workspace_id: &str) -> Value {
    json!({ "workspace_id": workspace_id })
}

fn event(distinct_id: &str, name: &str, properties: Value) -> Value {
    let common = json!({
        "$lib": "bifurc-engine",
        // No person profile and no location lookup.
        "$process_person_profile": false,
        "$geoip_disable": true,
        // PostHog stores the connecting IP as `$ip` unless the event supplies one; supplying a placeholder keeps the
        // real address out of the event.
        "$ip": "0.0.0.0",
        "engine_version": ENGINE_VERSION,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    });
    json!({ "event": name, "distinct_id": distinct_id, "properties": merged(&common, &properties) })
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

/// Turn analytics on for this engine, if the build has a project key and the terms and the policy were
/// accepted. Before they are accepted nothing is sent; [`resume`] begins then. What is sent from then on is
/// decided by the one switch ([`route`]).
pub fn start(transport: &str) {
    STATE.lock().transport = Some(transport.to_string());
    begin();
    // With nothing to send to, the reports are still kept few.
    collect_crashes();
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
    if !accepted() {
        return;
    }
    // The environment turned the anonymous analytics off: no id is made for this data directory.
    let install_id = (!env_opted_out()).then(install_id);
    let target = Target { key, host: host(), install_id };
    let routing = routing();
    let mut first: Vec<Value> = Vec::new();
    let mut reports_install = false;
    if let Some(install_id) = &target.install_id {
        if !install_reported(read_json(&analytics_path()).as_ref()) {
            first.push(event(install_id, "app installed", json!({})));
            reports_install = true;
        }
        if routing.anonymous {
            notice(
                "anonymous usage analytics are on (what kind of thing you do, never its content) — set \
                 \"telemetryEnabled\": false in app.json to report only the install, or BIFURC_TELEMETRY=0 to send nothing",
            );
            first.push(event(install_id, "engine started", json!({ "transport": transport })));
        } else if reports_install {
            // Switched off in the settings: that the app was installed here is still reported, once.
            notice(
                "anonymous usage analytics are off; that Bifurc was installed on this device is reported once — set \
                 BIFURC_TELEMETRY=0 to not report that either",
            );
        }
    }
    let sender = target.clone();
    STATE.lock().target = Some(target);
    let flusher = tokio::spawn(async move {
        if !first.is_empty() {
            match send(&sender, first).await {
                Ok(()) if reports_install => mark_install_reported(),
                Ok(()) => {}
                Err(reason) => notice(&format!("usage analytics could not be sent: {reason}")),
            }
        }
        // What crashed before this engine started is not left waiting for the first tick.
        flush().await;
        let mut tick = tokio::time::interval(EVENTS_EVERY);
        tick.tick().await;
        loop {
            tick.tick().await;
            flush().await;
        }
    });
    STATE.lock().flusher = Some(flusher);
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

/// A command that is about to run, as analytics see it: the event it becomes if it succeeds, and where that
/// event goes.
pub struct Command {
    described: Option<events::Event>,
    route: Route,
    /// The workspace it is done in, when the payload names one.
    workspace: Option<String>,
}

/// What `action` becomes if it succeeds, and where it goes; ask before the handler takes the payload, and
/// before the command changes which workspaces there are. `None` when nothing of it is sent.
pub fn describe(action: &str, payload: &Value) -> Option<Command> {
    STATE.lock().target.as_ref()?;
    let route = route(&routing());
    (route != Route::Nowhere).then(|| Command {
        described: events::describe(action, payload),
        route,
        workspace: events::workspace_of(action, payload).map(str::to_string),
    })
}

/// Queue the one event of a finished command, if it has one. `command` is what [`describe`] said before the
/// command ran, and `result` what it answered: its data, or its error code. `action` must be a registered
/// command and `client` a known client name, so that nothing a caller made up is ever sent.
pub fn record_command(command: Option<Command>, action: &str, client: &str, result: Result<&Value, &str>) {
    let Some(Command { described, route, workspace }) = command else { return };
    let mut state = STATE.lock();
    let Some(target) = &state.target else { return };
    let distinct_id = match &route {
        Route::Anonymous => match &target.install_id {
            Some(install_id) => install_id.clone(),
            None => return,
        },
        Route::Nowhere => return,
    };
    // A command that makes a workspace only knows its id from what it answered.
    let made = result.ok().and_then(|data| events::made_workspace(action, data));
    let extra = match workspace.as_deref().or(made) {
        Some(id) => workspace_properties(id),
        None => json!({}),
    };
    let Some(mut finished) = events::finished(described, action, result) else { return };
    finished.properties.insert("client".into(), json!(client));
    let mut body = event(&distinct_id, &finished.name, merged(&Value::Object(finished.properties), &extra));
    body["timestamp"] = json!(iso_string(now_ms()));
    enqueue(&mut state.queue, Queued { replaces: finished.replaces, event: body });
}

/// Queue an event that is no command's and no workspace's: anonymous, so only if the user's own analytics are
/// on. `at` is when it happened.
fn record(found: events::Event, at: i64) {
    if route(&routing()) != Route::Anonymous {
        return;
    }
    let mut state = STATE.lock();
    let Some(install_id) = state.target.as_ref().and_then(|target| target.install_id.clone()) else { return };
    let mut body = event(&install_id, &found.name, Value::Object(found.properties));
    body["timestamp"] = json!(iso_string(at));
    enqueue(&mut state.queue, Queued { replaces: found.replaces, event: body });
}

/// Something failed in the engine with no command to answer for it. `what` is a fixed word, never the
/// error's text.
pub fn failed(what: &'static str) {
    record(events::failure(what), now_ms());
}

/// Queue the crash reports the Bifurc processes left in the data directory since the last look, and keep
/// only the newest [`KEPT_CRASHES`] files. A report is looked at once, whether or not it may be sent.
fn collect_crashes() {
    let Ok(dir) = std::fs::read_dir(data_dir().join(bifurc_protocol::crash::DIR)) else { return };
    let mut files: Vec<std::path::PathBuf> =
        dir.flatten().map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|ext| ext == "json")).collect();
    // A file's name begins with when it was written.
    files.sort();
    let kept = files.split_off(files.len().saturating_sub(KEPT_CRASHES));
    for old in files {
        let _ = std::fs::remove_file(old);
    }
    if STATE.lock().target.is_none() {
        return;
    }
    for path in kept {
        let Some(report) = read_json(&path).filter(|report| !truthy(report, "collected")) else { continue };
        let _ = write_json_pretty(&path, &merged(&report, &json!({ "collected": true })));
        if let Some(found) = events::reported(&report) {
            record(found, report.get("at").and_then(Value::as_i64).unwrap_or_else(now_ms));
        }
    }
}

/// Send the queued events. What could not be sent is kept for the next flush.
pub async fn flush() {
    collect_crashes();
    let (target, queue) = {
        let mut state = STATE.lock();
        let Some(target) = state.target.clone() else { return };
        (target, std::mem::take(&mut state.queue))
    };
    // Switched off since they were queued: they are dropped.
    if opted_out() || queue.is_empty() {
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
    state.routing = None;
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
        let sent = event("install-1", "request.parseCurl", json!({ "client": "cli" }));

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
        let item =
            |key: Option<&str>, n: u32| Queued { replaces: key.map(str::to_string), event: json!(n) };
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

    /// A scratch data root with `app.json` holding `settings`, and analytics running as install `install-1` the
    /// way `begin` leaves them. Hand back the root: dropping it releases the store for the next test.
    fn running(settings: Value) -> crate::testing::ScratchRoot {
        let root = crate::testing::scratch_data_root();
        let mut stored = load_settings();
        for (key, value) in settings.as_object().unwrap() {
            stored.set(key, value.clone());
        }
        crate::workspace::settings::save_settings(&stored).unwrap();
        let mut state = STATE.lock();
        state.target = Some(Target { key: "key".into(), host: "http://localhost".into(), install_id: Some("install-1".into()) });
        state.queue.clear();
        state.routing = None;
        root
    }

    fn stop_running() -> Vec<Value> {
        let mut state = STATE.lock();
        state.target = None;
        state.routing = None;
        std::mem::take(&mut state.queue).into_iter().map(|queued| queued.event).collect()
    }

    /// The events a command leaves behind once it answered `result`.
    fn sent_by(action: &str, payload: Value, result: Result<&Value, &str>) -> Vec<Value> {
        let command = describe(action, &payload);
        record_command(command, action, "desktop", result);
        std::mem::take(&mut STATE.lock().queue).into_iter().map(|queued| queued.event).collect()
    }

    #[test]
    fn every_event_done_in_a_workspace_carries_the_workspace_id_and_the_rest_do_not() {
        let _root = running(json!({ "telemetryEnabled": true }));
        let done = json!({});
        // Local only, local git, synced, imported by its owner or by a collaborator: all the same to analytics.
        for workspace in ["local-only", "local-git", "synced", "owner-import", "collaborator-import"] {
            let sent = sent_by("request.parseCurl", json!({ "workspaceId": workspace }), Ok(&done));
            assert_eq!(sent.len(), 1, "{workspace}");
            let properties = sent[0]["properties"].as_object().unwrap();
            assert_eq!(properties["workspace_id"], workspace);
            assert_eq!(sent[0]["distinct_id"], "install-1");
            assert!(!properties.keys().any(|key| key.contains("org") || key.contains("github")), "{workspace}");
        }
        // A command that belongs to no workspace has none to name.
        let sent = sent_by("request.parseCurl", json!({}), Ok(&done));
        assert!(sent[0]["properties"].get("workspace_id").is_none());
        // Making a workspace names the new one, known only from the answer.
        let added = sent_by("workspace.add", json!({ "name": "n" }), Ok(&json!({ "id": "fresh", "name": "n" })));
        assert_eq!((added[0]["event"].as_str(), added[0]["properties"]["workspace_id"].as_str()), (Some("workspace created"), Some("fresh")));
        let imported = sent_by(
            "workspace.import",
            json!({ "accountId": "a", "repo": "octo/team" }),
            Ok(&json!({ "ok": true, "workspace": { "id": "brought" } })),
        );
        assert_eq!(imported[0]["properties"]["workspace_id"], "brought");
        // An import that brought nothing has no workspace to name.
        let refused = sent_by("workspace.import", json!({ "accountId": "a", "repo": "octo/x" }), Ok(&json!({ "ok": false, "error": "no" })));
        assert!(refused.iter().all(|sent| sent["properties"].get("workspace_id").is_none()));
        // What is no command's and no workspace's stays as it was.
        failed("listener_bind");
        let background = stop_running();
        assert_eq!(background.len(), 1);
        assert!(background[0]["properties"].get("workspace_id").is_none());
    }

    #[test]
    fn with_the_switch_off_nothing_is_sent_whatever_an_old_app_json_says_of_organizations() {
        let organization = json!({
            "publicId": "pub_acme", "name": "Acme", "workspaceId": "ws_team", "analytics": true, "githubUserId": 7,
        });
        let workspaces = json!([{ "id": "team", "name": "Team", "accountId": "a", "organization": organization }]);

        // On: the old `organization` is ignored, the event is anonymous and names the workspace by its own id.
        let root = running(json!({ "telemetryEnabled": true, "workspaces": workspaces }));
        assert_eq!(load_settings().workspace("team").unwrap()["organization"]["publicId"], "pub_acme");
        let sent = sent_by("request.parseCurl", json!({ "workspaceId": "team" }), Ok(&json!({})));
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["distinct_id"], "install-1");
        let properties = sent[0]["properties"].as_object().unwrap();
        assert_eq!(properties["workspace_id"], "team");
        for gone in ["org_public_id", "github_user_id"] {
            assert!(!properties.contains_key(gone), "{gone}");
        }
        stop_running();
        drop(root);

        // Off: the organization's `analytics: true` no longer overrides the switch.
        let _root = running(json!({ "telemetryEnabled": false, "workspaces": workspaces }));
        assert!(describe("request.parseCurl", &json!({ "workspaceId": "team" })).is_none());
        assert!(sent_by("request.parseCurl", json!({ "workspaceId": "team" }), Ok(&json!({}))).is_empty());
        failed("listener_bind");
        assert!(stop_running().is_empty());
    }
}
