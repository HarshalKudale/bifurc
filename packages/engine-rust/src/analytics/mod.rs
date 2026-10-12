//! Usage analytics, sent to PostHog: what kind of thing the user did, from which client and on which
//! platform. Never a payload, a name, a URL or a path.
//!
//! Whose decision they are depends on where the thing was done ([`route`]):
//! - In a workspace of an organization ([`crate::gitops::workspace_organization`]), it is the organization's,
//!   made in the Bifurc Console. On, what is done in the workspace is reported with the organization's public
//!   id, the console's id of the workspace and the GitHub id of the account it was done as, whatever the
//!   switch and the environment of this device say. Off, nothing done in that workspace is reported at all.
//! - Anywhere else (a workspace that is the user's own, and what belongs to no workspace: the engine starting,
//!   signing in), it is the user's: anonymous, on by default, and off with the switch or the environment. The
//!   only identifier is then a random id made for this data directory, which no organization's event carries.
//!
//! A command a client sends becomes at most one event ([`events::finished`]), sent within [`EVENTS_EVERY`]:
//! - what [`events::describe`] makes of it when it succeeds (`mock created`, `request sent`);
//! - its own name (`request.parseCurl`) when it succeeds and nothing describes it;
//! - `command failed`, with its name and the error code, when it fails.
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
    collections::HashMap,
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
        json::{merged, opt_str, read_json, write_json_pretty},
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
/// How long what `app.json` says about the switch and the organizations is taken as still so. A command is
/// routed by it, and `app.json` is not read for every one.
const ROUTING_TTL: Duration = Duration::from_secs(5);

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
    /// An organization's: sent whatever this device's switch says.
    managed: bool,
}

/// What an organization decided for one of its workspaces on this device.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Managed {
    public_id: String,
    /// The console's id of the workspace.
    workspace_id: String,
    /// The GitHub id of the account the workspace is used as here.
    github_user_id: u64,
    analytics: bool,
}

/// What decides where an event goes: the user's own choice, and the organizations' for their workspaces,
/// keyed by the workspace's id on this device.
#[derive(Debug, Default)]
struct Routing {
    anonymous: bool,
    managed: HashMap<String, Managed>,
}

/// Where the one event of a command goes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Route {
    /// To the anonymous analytics, as this data directory.
    Anonymous,
    /// To the organization's analytics, as the account its workspace is used as.
    Organization(Managed),
    Nowhere,
}

/// A workspace of an organization follows the organization, and only it; everything else follows the user.
fn route(routing: &Routing, workspace: Option<&str>) -> Route {
    match workspace.and_then(|id| routing.managed.get(id)) {
        Some(managed) if managed.analytics => Route::Organization(managed.clone()),
        Some(_) => Route::Nowhere,
        None if routing.anonymous => Route::Anonymous,
        None => Route::Nowhere,
    }
}

/// What a workspace's settings say of its organization, if they name one in full.
fn managed_of(workspace: &Value) -> Option<(String, Managed)> {
    let organization = workspace.get("organization").filter(|organization| organization.is_object())?;
    let text = |key: &str| opt_str(organization, key).filter(|text| !text.is_empty()).map(str::to_string);
    let managed = Managed {
        public_id: text("publicId")?,
        workspace_id: text("workspaceId")?,
        github_user_id: organization.get("githubUserId").and_then(Value::as_u64)?,
        analytics: organization.get("analytics").and_then(Value::as_bool) == Some(true),
    };
    Some((opt_str(workspace, "id")?.to_string(), managed))
}

fn read_routing() -> Routing {
    let settings = load_settings();
    Routing {
        anonymous: !env_opted_out() && settings.telemetry_enabled(),
        managed: settings.workspaces().iter().filter_map(managed_of).collect(),
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

/// What an organization decided about a workspace changed: the next command is routed by the new answer.
pub fn policies_changed() {
    STATE.lock().routing = None;
}

/// What an organization's event says on top of what was done: whose workspace, which one, and which account.
/// The account is a number GitHub gave it; its name is never sent.
fn organization_properties(managed: &Managed) -> Value {
    json!({
        "org_public_id": managed.public_id,
        "workspace_id": managed.workspace_id,
        "github_user_id": managed.github_user_id,
    })
}

/// Whom an organization's event is from: the account, within the organization. Never the install id, so that
/// nothing ties what an account does for an organization to what the same device reports anonymously.
fn organization_distinct_id(managed: &Managed) -> String {
    format!("{}:{}", managed.public_id, managed.github_user_id)
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
/// decided event by event ([`route`]).
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
    if !accepted() {
        return;
    }
    // The environment turned the anonymous analytics off: no id is made for this data directory.
    let install_id = (!env_opted_out()).then(install_id);
    let target = Target { key, host: host(), install_id };
    let routing = routing();
    let reporting = routing.managed.values().filter(|managed| managed.analytics).count();
    if reporting > 0 {
        notice(&format!(
            "usage analytics are on for {reporting} workspace(s) of an organization and managed by its owner in the \
             Bifurc Console: what kind of thing is done in them, and as which GitHub account, never its content"
        ));
    }
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
    let flusher = tokio::spawn(async move {
        if !first.is_empty() {
            match send(&sender, first).await {
                Ok(()) if reports_install => mark_install_reported(),
                Ok(()) => {}
                Err(reason) => notice(&format!("usage analytics could not be sent: {reason}")),
            }
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

/// A command that is about to run, as analytics see it: the event it becomes if it succeeds, and where that
/// event goes.
pub struct Command {
    described: Option<events::Event>,
    route: Route,
}

/// What `action` becomes if it succeeds, and where it goes; ask before the handler takes the payload, and
/// before the command changes which workspaces there are. `None` when nothing of it is sent.
pub fn describe(action: &str, payload: &Value) -> Option<Command> {
    STATE.lock().target.as_ref()?;
    let route = route(&routing(), events::workspace_of(action, payload));
    (route != Route::Nowhere).then(|| Command { described: events::describe(action, payload), route })
}

/// Queue the one event of a finished command, if it has one. `command` is what [`describe`] said before the
/// command ran, and `result` what it answered: its data, or its error code. `action` must be a registered
/// command and `client` a known client name, so that nothing a caller made up is ever sent.
pub fn record_command(command: Option<Command>, action: &str, client: &str, result: Result<&Value, &str>) {
    let Some(Command { described, route }) = command else { return };
    let mut state = STATE.lock();
    let Some(target) = &state.target else { return };
    let (distinct_id, extra) = match &route {
        Route::Anonymous => match &target.install_id {
            Some(install_id) => (install_id.clone(), json!({})),
            None => return,
        },
        Route::Organization(managed) => (organization_distinct_id(managed), organization_properties(managed)),
        Route::Nowhere => return,
    };
    let Some(mut finished) = events::finished(described, action, result) else { return };
    finished.properties.insert("client".into(), json!(client));
    let mut body = event(&distinct_id, &finished.name, merged(&Value::Object(finished.properties), &extra));
    body["timestamp"] = json!(iso_string(now_ms()));
    let managed = matches!(route, Route::Organization(_));
    enqueue(&mut state.queue, Queued { replaces: finished.replaces, event: body, managed });
}

/// Send the queued events. What could not be sent is kept for the next flush.
pub async fn flush() {
    let (target, queue) = {
        let mut state = STATE.lock();
        let Some(target) = state.target.clone() else { return };
        (target, std::mem::take(&mut state.queue))
    };
    // Switched off since they were queued: the anonymous ones are dropped. An organization's are not the
    // user's to drop.
    let anonymous = !opted_out();
    let queue: Vec<Queued> = queue.into_iter().filter(|queued| queued.managed || anonymous).collect();
    if queue.is_empty() {
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
            |key: Option<&str>, n: u32| Queued { replaces: key.map(str::to_string), event: json!(n), managed: false };
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

    fn managed(analytics: bool) -> Managed {
        Managed { public_id: "pub_acme".into(), workspace_id: "ws_team".into(), github_user_id: 7, analytics }
    }

    #[test]
    fn an_organizations_workspace_follows_the_organization_and_nothing_else() {
        let routing = |anonymous: bool| Routing {
            anonymous,
            managed: HashMap::from([("team".to_string(), managed(true)), ("quiet".to_string(), managed(false))]),
        };
        for anonymous in [true, false] {
            // The organization has them on: reported, whatever the user's own switch says.
            assert_eq!(route(&routing(anonymous), Some("team")), Route::Organization(managed(true)));
            // The organization has them off: not reported, not even anonymously.
            assert_eq!(route(&routing(anonymous), Some("quiet")), Route::Nowhere);
        }
        // The user's own workspace, and what belongs to no workspace, follow the user's switch.
        assert_eq!(route(&routing(true), Some("mine")), Route::Anonymous);
        assert_eq!(route(&routing(true), None), Route::Anonymous);
        assert_eq!(route(&routing(false), Some("mine")), Route::Nowhere);
        assert_eq!(route(&routing(false), None), Route::Nowhere);
    }

    #[test]
    fn a_workspace_is_an_organizations_only_when_its_settings_say_so_in_full() {
        let organization = json!({
            "publicId": "pub_acme", "name": "Acme", "workspaceId": "ws_team", "analytics": true, "githubUserId": 7,
        });
        let known = managed_of(&json!({ "id": "team", "name": "Team", "organization": organization })).unwrap();
        assert_eq!(known, ("team".to_string(), managed(true)));

        // A workspace as every version before organizations wrote it, and one the console said nothing of.
        assert!(managed_of(&json!({ "id": "mine", "name": "Mine", "syncConfig": { "repo": "octo/mine" } })).is_none());
        assert!(managed_of(&json!({ "id": "mine", "organization": null })).is_none());
        for missing in ["publicId", "workspaceId", "githubUserId"] {
            let mut partial = organization.clone();
            partial.as_object_mut().unwrap().remove(missing);
            assert!(managed_of(&json!({ "id": "team", "organization": partial })).is_none(), "{missing}");
        }
        let mut off = organization.clone();
        off.as_object_mut().unwrap().remove("analytics");
        assert_eq!(managed_of(&json!({ "id": "team", "organization": off })).unwrap().1, managed(false));
    }

    #[test]
    fn an_organizations_event_names_the_organization_the_workspace_and_the_account_and_not_the_install() {
        let managed = managed(true);
        let properties =
            merged(&json!({ "client": "desktop", "request_type": "rest" }), &organization_properties(&managed));
        let sent = event(&organization_distinct_id(&managed), "request sent", properties);

        assert_eq!(sent["distinct_id"], "pub_acme:7");
        let properties = sent["properties"].as_object().unwrap();
        assert_eq!(properties["org_public_id"], "pub_acme");
        assert_eq!(properties["workspace_id"], "ws_team");
        assert_eq!(properties["github_user_id"], 7);
        assert_eq!(properties["$process_person_profile"], false);
        assert_eq!(properties["$ip"], "0.0.0.0");
        let mut keys: Vec<&str> = properties.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "$geoip_disable", "$ip", "$lib", "$process_person_profile", "arch", "client", "engine_version",
                "github_user_id", "org_public_id", "os", "request_type", "workspace_id",
            ]
        );
    }
}
