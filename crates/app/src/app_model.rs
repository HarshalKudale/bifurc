//! The workspace as the engine reports it, and the one place the engine's events are received.
//!
//! `AppModel` holds no domain state of its own. It keeps the latest [`ConfigSnapshot`], the workspace's git
//! status and the proxy's status, replaces them when the engine says something changed, and re-broadcasts
//! the engine's socket events to whichever views care. Everything that changes the workspace is an engine
//! command; this model then re-reads.

use std::time::Duration;

use bifurc_engine_client::{ClientError, EngineClient, Incoming, reconnect};
use futures::StreamExt as _;
use gpui_kit::{AppContext as _, Context, EventEmitter, Global, WeakEntity};
use serde_json::{Value, json};

use crate::{
    engine::{EngineSession, call, describe, error_text, failure},
    i18n::{t, t_with},
    model::{
        ConfigSnapshot,
        agent::{self, Approval, FaultRule, Recording, Replay},
        claims::{Claim, Claims},
        config::AccountInfo,
        git::GitStatus,
        update::UpdateStatus,
        vault::VaultInfo,
    },
};

/// The model of the open window, for code that has only an `App` (the engine helpers claiming what is saved).
pub struct ModelHandle(pub WeakEntity<AppModel>);

impl Global for ModelHandle {}

/// Wire names of the engine events the UI listens to.
///
/// `entity.changed` says what happened to one entity: `created`, or `updated` / `deleted` with the fields a
/// listing shows, which [`ConfigSnapshot::apply_change`] patches in without a fetch. `sync.entityStatus`
/// follows every write with the git state, and says `configChanged: false` when `entity.changed` already
/// described the whole change; without that field (an older engine, or a pull, sync or import) it means
/// "re-read the workspace".
const EVENTS: [&str; 40] = [
    "event.request.stream",
    "event.flow.progress",
    "event.flow.finished",
    "event.flow.loadtest.progress",
    "event.flow.loadtest.finished",
    "event.account.changed",
    "event.companions.changed",
    "event.update.changed",
    "event.engine.stopping",
    "event.ui.activate",
    "event.entity.changed",
    "event.sync.entityStatus",
    "event.history.changed",
    "event.vault.changed",
    "event.cookies.changed",
    "event.faults.changed",
    "event.devservice.changed",
    "event.devservice.output",
    "event.mail.changed",
    "event.webhook.changed",
    "event.claims.changed",
    "event.cassette.changed",
    "event.approval.requested",
    "event.approval.decided",
    "event.sync.status",
    "event.settings.changed",
    "event.server.error",
    "event.ws.status",
    "event.ws.message",
    "event.grpc.stream.status",
    "event.grpc.stream.message",
    "event.sse.status",
    "event.sse.message",
    "event.socketio.status",
    "event.socketio.message",
    "event.mcp.status",
    "event.mcp.message",
    "event.log.entry",
    "event.runner.progress",
    "event.runner.finished",
];

/// How long a refresh waits for the rest of a burst of changes.
const REFRESH_DELAY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq)]
pub enum Load {
    Loading,
    Ready,
    Failed(String),
}

/// One of the engine's two proxy listeners, as `server.status` reports it: the forward proxy, or (asked with
/// `target: "reverse"`) the reverse proxy that serves `.localhost` names on `settings.port`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProxyStatus {
    pub running: bool,
    pub port: u16,
    pub error: Option<String>,
    /// A start or stop is in flight.
    pub busy: bool,
}

impl ProxyStatus {
    fn from_value(value: &Value) -> Self {
        Self {
            running: value.get("running").and_then(Value::as_bool).unwrap_or(false),
            port: value.get("port").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()).unwrap_or(0),
            error: error_text(value),
            busy: false,
        }
    }
}

/// What a read-only workspace says when it is asked to sync.
pub const READ_ONLY_NOTICE: &str = "app.model.read_only_notice";

/// What the engine says about this machine and its accounts (`setup.getState`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SetupState {
    /// The first answer has arrived.
    pub known: bool,
    /// Without git a workspace is plain files: no history, no audit log, no revert, no sync.
    pub git_installed: bool,
    /// This build can sign in to GitHub.
    pub sign_in_available: bool,
    /// A workspace is used by everyone its repository is shared with, not by its owner alone.
    pub team_sync: bool,
    pub accounts: Vec<AccountInfo>,
    /// Where the terms of service are published.
    pub terms_of_service_url: String,
    /// The terms of service were accepted on this data directory.
    pub terms_accepted: bool,
    /// Where the privacy policy is published.
    pub privacy_policy_url: String,
    /// The privacy policy was accepted on this data directory.
    pub privacy_accepted: bool,
    /// The first-run setup was walked through to its end.
    pub onboarded: bool,
    /// The first run's servers step was passed: the engine starts the two proxies by itself.
    pub servers_confirmed: bool,
}

impl SetupState {
    fn from_value(value: &Value) -> Self {
        let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
        Self {
            known: true,
            git_installed: flag("gitInstalled"),
            sign_in_available: flag("signInAvailable"),
            team_sync: flag("teamSync"),
            terms_of_service_url: value.get("termsOfServiceUrl").and_then(Value::as_str).unwrap_or_default().to_owned(),
            terms_accepted: flag("termsAccepted"),
            privacy_policy_url: value.get("privacyPolicyUrl").and_then(Value::as_str).unwrap_or_default().to_owned(),
            privacy_accepted: flag("privacyAccepted"),
            onboarded: flag("onboarded"),
            servers_confirmed: flag("serversConfirmed"),
            accounts: value
                .get("accounts")
                .and_then(Value::as_array)
                .map(|accounts| accounts.iter().filter_map(AccountInfo::from_value).collect())
                .unwrap_or_default(),
        }
    }
}

/// Something that was deleted here and is still listed: its deletion waits to be confirmed or reverted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deleted {
    /// `kind` as `entity.delete` takes it: `requests`, `mocks`, `mappings`, ...
    Entity { kind: &'static str, id: String },
    /// `kind` as `folder.delete` takes it: `request`, `mock`, ...
    Folder { kind: &'static str, id: String },
}

/// What a sync commits: the named files, or everything in the workspace that was never synced.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncScope {
    Paths(Vec<String>),
    All,
}

#[derive(Debug, Clone)]
pub enum ModelEvent {
    /// A new snapshot is in place.
    ConfigChanged,
    /// One entity's listing in the snapshot was patched from an `entity.changed` event. Nothing appeared or
    /// vanished except that entity's own entry, so a view that only reacts to the set of things (open tabs,
    /// validation, reloads) has nothing to do; one that renders the listing re-renders on its own.
    ConfigPatched,
    /// The active workspace is not the one it was; anything open belongs to the previous one.
    WorkspaceSwitched,
    /// `event.request.stream`: text of a REST response that is still arriving.
    RequestStream(Value),
    /// `event.ws.status`, as the engine sent it.
    SocketStatus(Value),
    /// `event.ws.message`, as the engine sent it.
    SocketMessage(Value),
    /// `event.grpc.stream.status`: a live gRPC call opened or ended.
    GrpcStatus(Value),
    /// `event.grpc.stream.message`: one message of a live gRPC call.
    GrpcMessage(Value),
    /// `event.sse.status`: an SSE stream opened, ended or failed.
    SseStatus(Value),
    /// `event.sse.message`: one event of an SSE stream.
    SseMessage(Value),
    /// `event.socketio.status`: a Socket.IO connection opened, ended or failed.
    SocketIoStatus(Value),
    /// `event.socketio.message`: one event or acknowledgement of a Socket.IO connection.
    SocketIoMessage(Value),
    /// `event.mcp.status`: an MCP connection opened, ended or failed, or its server declared something new.
    McpStatus(Value),
    /// `event.mcp.message`: one line of an MCP connection's log, new or answered.
    McpMessage(Value),
    /// `event.log.entry`: proxied traffic, as a batch of entries.
    Traffic(Value),
    /// `event.runner.progress`: one request of a folder run has finished.
    RunnerProgress(Value),
    /// `event.runner.finished`: a folder run ended, with its report.
    RunnerFinished(Value),
    /// `event.flow.progress`: the nodes and connections of a flow's run that changed.
    FlowProgress(Value),
    /// `event.flow.finished`: a flow's run ended, with its report.
    FlowFinished(Value),
    /// `event.flow.loadtest.progress`: the instances of a flow's load test that changed, with its counts.
    FlowLoadProgress(Value),
    /// `event.flow.loadtest.finished`: a flow's load test ended, with its report.
    FlowLoadFinished(Value),
    /// The saved request with this id got a response from a send in the app, so it has a new sample.
    ResponseReceived(String),
    /// The entity with this id was put back to its last synced version; an editor showing it must re-read it.
    EntityReverted(String),
    /// `event.sync.status`: the workspace's remote sync started, finished or failed at something.
    SyncStatus,
    /// `event.history.changed`: the active workspace has a commit it did not have, or lost one.
    HistoryChanged,
    /// A sync that will commit is about to run: the user may give its commits a message first.
    AskSyncMessage(SyncScope),
    /// `event.account.changed`: a sign-in finished or failed, or an account was signed out.
    AccountChanged(Value),
    /// `event.companions.changed`: a browser extension connected or disconnected.
    CompanionsChanged,
    /// `event.cookies.changed`: a response set or removed cookies, or the jar was cleared or reconfigured.
    CookiesChanged,
    /// `event.devservice.changed`: a dev server was defined, started, stopped or removed, or exited by itself.
    DevServiceChanged(Value),
    /// `event.devservice.output`: new lines a dev server printed, in order and complete.
    DevServiceOutput(Value),
    /// `event.mail.changed`: the mail sink received a mail (`received`) or was emptied (`cleared`).
    MailChanged(Value),
    /// `event.webhook.changed`: a webhook listener started or stopped; the payload is every listener, as `webhook.list` has them.
    WebhookChanged(Value),
    /// `event.claims.changed`: someone claimed, released or took over an entity; the model already holds the new list.
    ClaimsChanged,
    /// `event.approval.requested`: an agent's call is waiting for a person to approve or deny it.
    ApprovalRequested(Approval),
    /// The engine was stopped and started again: anything that lived only in the old one is gone.
    EngineRestarted,
    /// A short message for the user: a failed command, a lost connection.
    Notice(String),
}

pub struct AppModel {
    engine: EngineSession,
    config: ConfigSnapshot,
    git: GitStatus,
    /// The workspace's vaults: names and states, never values.
    vaults: Vec<VaultInfo>,
    proxy: ProxyStatus,
    reverse_proxy: ProxyStatus,
    /// What agents have started that changes this app's traffic or waits on the user.
    faults: Vec<FaultRule>,
    recording: Option<Recording>,
    replaying: Option<Replay>,
    approvals: Vec<Approval>,
    /// The entities of the workspace somebody is editing; see [`crate::model::claims`].
    claims: Claims,
    setup: SetupState,
    /// What the engine knows about newer versions.
    update: UpdateStatus,
    /// The engine is being stopped and started again; its connection closing is expected.
    restarting_engine: bool,
    load: Load,
    refreshing: bool,
    refresh_again: bool,
    /// A fetch is waiting out [`REFRESH_DELAY`].
    refresh_scheduled: bool,
    git_refreshing: bool,
    git_again: bool,
    connection_lost: bool,
    /// A publish or sync is in flight.
    publishing: bool,
}

impl EventEmitter<ModelEvent> for AppModel {}

impl AppModel {
    pub fn new(engine: EngineSession, cx: &mut Context<Self>) -> Self {
        let mut model = Self {
            engine,
            config: ConfigSnapshot::default(),
            git: GitStatus::default(),
            vaults: Vec::new(),
            proxy: ProxyStatus::default(),
            reverse_proxy: ProxyStatus::default(),
            faults: Vec::new(),
            recording: None,
            replaying: None,
            approvals: Vec::new(),
            claims: Claims::default(),
            setup: SetupState::default(),
            update: UpdateStatus::default(),
            restarting_engine: false,
            load: Load::Loading,
            refreshing: false,
            refresh_again: false,
            refresh_scheduled: false,
            git_refreshing: false,
            git_again: false,
            connection_lost: false,
            publishing: false,
        };
        let handle = ModelHandle(cx.weak_entity());
        cx.set_global(handle);
        model.listen(cx);
        model.refresh(cx);
        model.refresh_setup(cx);
        model.refresh_update(cx);
        model.refresh_agent_activity(cx);
        model
    }

    pub fn update(&self) -> &UpdateStatus {
        &self.update
    }

    /// Re-reads what the engine last found about newer versions. It asks nobody: the engine looks once a day.
    pub fn refresh_update(&mut self, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "update.status", json!({}), |this, outcome, cx| {
            if let Ok(value) = outcome {
                this.update = UpdateStatus::from_value(&value);
                cx.notify();
            }
        });
    }

    /// Has the engine read the list of releases now.
    pub fn check_for_update(&mut self, cx: &mut Context<Self>) {
        if self.update.checking {
            return;
        }
        self.update.checking = true;
        cx.notify();
        let client = self.engine.client.clone();
        call(&client, cx, "update.check", json!({}), |this, outcome, cx| {
            match outcome {
                Ok(value) => this.update = UpdateStatus::from_value(&value),
                Err(error) => {
                    this.update.checking = false;
                    cx.emit(ModelEvent::Notice(t_with(
                        "app.model.check_updates_failed",
                        &[("error", &describe(&error))],
                    )));
                }
            }
            cx.notify();
        });
    }

    /// Stops announcing `version`; a newer one is announced again.
    pub fn skip_update(&mut self, version: &str, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "update.skip", json!({ "version": version }), |this, outcome, cx| match outcome {
            Ok(value) => {
                this.update = UpdateStatus::from_value(&value);
                cx.notify();
            }
            Err(error) => {
                cx.emit(ModelEvent::Notice(t_with("app.model.skip_version_failed", &[("error", &describe(&error))])))
            }
        });
    }

    pub fn setup(&self) -> &SetupState {
        &self.setup
    }

    /// Re-reads whether git is installed and which accounts are signed in.
    pub fn refresh_setup(&mut self, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "setup.getState", json!({}), |this, outcome, cx| {
            if let Ok(value) = outcome {
                this.setup = SetupState::from_value(&value);
                cx.notify();
            }
        });
    }

    pub fn engine(&self) -> &EngineSession {
        &self.engine
    }

    pub fn client(&self) -> &EngineClient {
        &self.engine.client
    }

    pub fn config(&self) -> &ConfigSnapshot {
        &self.config
    }

    pub fn git(&self) -> &GitStatus {
        &self.git
    }

    pub fn vaults(&self) -> &[VaultInfo] {
        &self.vaults
    }

    pub fn proxy(&self) -> &ProxyStatus {
        &self.proxy
    }

    pub fn reverse_proxy(&self) -> &ProxyStatus {
        &self.reverse_proxy
    }

    pub fn faults(&self) -> &[FaultRule] {
        &self.faults
    }

    pub fn recording(&self) -> Option<&Recording> {
        self.recording.as_ref()
    }

    pub fn replaying(&self) -> Option<&Replay> {
        self.replaying.as_ref()
    }

    pub fn approvals(&self) -> &[Approval] {
        &self.approvals
    }

    /// Reads what agents have going. A failure keeps what is shown: the events keep it current after.
    fn refresh_agent_activity(&mut self, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "fault.list", json!({}), |this, outcome, cx| {
            if let Ok(value) = outcome {
                this.faults = FaultRule::list_from_value(&value);
                cx.notify();
            }
        });
        call(&client, cx, "cassette.recordStatus", json!({}), |this, outcome, cx| {
            if let Ok(value) = outcome {
                this.recording = agent::recording_from_value(&value);
                cx.notify();
            }
        });
        call(&client, cx, "cassette.replayStatus", json!({}), |this, outcome, cx| {
            if let Ok(value) = outcome {
                this.replaying = agent::replay_from_value(&value);
                cx.notify();
            }
        });
        self.refresh_approvals(cx);
    }

    fn refresh_approvals(&mut self, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "approvals.list", json!({}), |this, outcome, cx| {
            if let Ok(value) = outcome {
                this.approvals = Approval::list_from_value(&value);
                cx.notify();
            }
        });
    }

    /// Runs `action` for the footer's controls. The state follows from the event the engine sends; only a
    /// failure is reported here.
    fn run_for_agents(&mut self, action: &'static str, payload: Value, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, action, payload, |_, outcome, cx| {
            if let Some(error) = failure(&outcome) {
                cx.emit(ModelEvent::Notice(error));
            }
        });
    }

    /// Removes the fault rule `id`, or all of them.
    pub fn clear_faults(&mut self, id: Option<&str>, cx: &mut Context<Self>) {
        self.run_for_agents("fault.clear", id.map_or_else(|| json!({}), |id| json!({ "id": id })), cx);
    }

    pub fn stop_recording(&mut self, cx: &mut Context<Self>) {
        self.run_for_agents("cassette.recordStop", json!({}), cx);
    }

    pub fn stop_replay(&mut self, cx: &mut Context<Self>) {
        self.run_for_agents("cassette.replayStop", json!({}), cx);
    }

    /// Lets the held call go on, or fails it as blocked. `interactive` is how the engine knows a person decided.
    pub fn decide_approval(&mut self, id: &str, approve: bool, cx: &mut Context<Self>) {
        let decision = if approve { "approve" } else { "deny" };
        self.run_for_agents("approvals.decide", json!({ "id": id, "decision": decision, "interactive": true }), cx);
    }

    pub fn is_restarting_engine(&self) -> bool {
        self.restarting_engine
    }

    pub fn load(&self) -> &Load {
        &self.load
    }

    pub fn workspace_id(&self) -> &str {
        &self.config.active_workspace_id
    }

    pub fn is_connection_lost(&self) -> bool {
        self.connection_lost
    }

    pub fn is_publishing(&self) -> bool {
        self.publishing
    }

    /// The workspace in front is read-only: its account may read its repository and not push to it, so what
    /// is changed here is saved here and nothing of it is synced.
    pub fn is_read_only(&self) -> bool {
        self.config.active_workspace().and_then(|workspace| workspace.sync.as_ref()).is_some_and(|sync| sync.read_only)
    }

    /// Says why nothing is synced, when the workspace is read-only. Returns whether it is.
    fn refuse_read_only(&mut self, cx: &mut Context<Self>) -> bool {
        if self.is_read_only() {
            cx.emit(ModelEvent::Notice(t(READ_ONLY_NOTICE).to_owned()));
        }
        self.is_read_only()
    }

    fn listen(&mut self, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        let subscribing = cx.background_spawn(async move { client.subscribe(&EVENTS).await });
        cx.spawn(async move |this, cx| {
            let mut incoming = match subscribing.await {
                Ok(incoming) => incoming,
                Err(error) => {
                    this.update(cx, |_, cx| {
                        cx.emit(ModelEvent::Notice(t_with(
                            "app.model.live_updates_unavailable",
                            &[("error", &describe(&error))],
                        )));
                    })
                    .ok();
                    return;
                }
            };
            while let Some(item) = incoming.next().await {
                if this.update(cx, |this, cx| this.on_incoming(item, cx)).is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    fn on_incoming(&mut self, incoming: Incoming, cx: &mut Context<Self>) {
        match incoming {
            // A restart closes the old connection on purpose; the new one is in place when it is done.
            Incoming::Closed if self.restarting_engine => {}
            Incoming::Closed => {
                self.connection_lost = true;
                cx.emit(ModelEvent::Notice(t("app.engine.connection_lost").to_owned()));
                cx.notify();
            }
            Incoming::Event(event) => match event.event.as_str() {
                // The engine is stopping on purpose. With a successor on its way this app waits for it; with
                // none, the engine is gone and every interface goes with it.
                "event.engine.stopping" => {
                    if event.payload.get("restart").and_then(Value::as_bool) == Some(true) {
                        self.await_successor(cx);
                    } else {
                        cx.quit();
                    }
                }
                // The engine's tray icon was asked for the interface, and this is it.
                "event.ui.activate" => {
                    cx.activate(true);
                    for window in cx.windows() {
                        window.update(cx, |_, window, _| window.activate_window()).ok();
                    }
                }
                "event.entity.changed" => {
                    if self.config.apply_change(&event.payload) {
                        // A fetch that is already on its way may carry the state from before this change.
                        self.refresh_again |= self.refreshing;
                        cx.emit(ModelEvent::ConfigPatched);
                        cx.notify();
                    } else {
                        self.refresh_soon(cx);
                    }
                }
                "event.sync.entityStatus" => {
                    let in_front = event.payload.get("wsId").and_then(Value::as_str) == Some(self.workspace_id());
                    if in_front && event.payload.get("configChanged").and_then(Value::as_bool) == Some(false) {
                        self.refresh_git(cx);
                    } else {
                        self.refresh_soon(cx);
                    }
                }
                "event.settings.changed" => self.refresh_soon(cx),
                "event.vault.changed" => {
                    self.refresh_vaults(cx);
                    self.refresh_git(cx);
                }
                "event.sync.status" => {
                    self.refresh_git(cx);
                    cx.emit(ModelEvent::SyncStatus);
                }
                "event.history.changed"
                    if event.payload.get("workspaceId").and_then(Value::as_str) == Some(self.workspace_id()) =>
                {
                    cx.emit(ModelEvent::HistoryChanged);
                }
                "event.account.changed" => {
                    self.refresh_setup(cx);
                    self.refresh(cx);
                    cx.emit(ModelEvent::AccountChanged(event.payload));
                }
                "event.companions.changed" => cx.emit(ModelEvent::CompanionsChanged),
                "event.cookies.changed" => cx.emit(ModelEvent::CookiesChanged),
                "event.devservice.changed" => cx.emit(ModelEvent::DevServiceChanged(event.payload)),
                "event.devservice.output" => cx.emit(ModelEvent::DevServiceOutput(event.payload)),
                "event.mail.changed" => cx.emit(ModelEvent::MailChanged(event.payload)),
                "event.webhook.changed" => cx.emit(ModelEvent::WebhookChanged(event.payload)),
                "event.claims.changed" => self.apply_claims(&event.payload, cx),
                "event.faults.changed" => {
                    self.faults = FaultRule::list_from_value(&event.payload);
                    cx.notify();
                }
                "event.cassette.changed" => {
                    self.recording = event.payload.get("record").and_then(agent::recording_from_value);
                    self.replaying = event.payload.get("replay").and_then(agent::replay_from_value);
                    cx.notify();
                }
                // The decided item does not carry the pending list, so both events read it again.
                "event.approval.requested" => {
                    self.refresh_approvals(cx);
                    cx.emit(ModelEvent::ApprovalRequested(Approval::from_value(&event.payload)));
                }
                "event.approval.decided" => self.refresh_approvals(cx),
                "event.update.changed" => {
                    self.update = UpdateStatus::from_value(&event.payload);
                    cx.notify();
                }
                "event.server.error" => self.refresh_proxy(cx),
                "event.request.stream" => cx.emit(ModelEvent::RequestStream(event.payload)),
                "event.ws.status" => cx.emit(ModelEvent::SocketStatus(event.payload)),
                "event.ws.message" => cx.emit(ModelEvent::SocketMessage(event.payload)),
                "event.grpc.stream.status" => cx.emit(ModelEvent::GrpcStatus(event.payload)),
                "event.grpc.stream.message" => cx.emit(ModelEvent::GrpcMessage(event.payload)),
                "event.sse.status" => cx.emit(ModelEvent::SseStatus(event.payload)),
                "event.sse.message" => cx.emit(ModelEvent::SseMessage(event.payload)),
                "event.socketio.status" => cx.emit(ModelEvent::SocketIoStatus(event.payload)),
                "event.socketio.message" => cx.emit(ModelEvent::SocketIoMessage(event.payload)),
                "event.mcp.status" => cx.emit(ModelEvent::McpStatus(event.payload)),
                "event.mcp.message" => cx.emit(ModelEvent::McpMessage(event.payload)),
                "event.log.entry" => cx.emit(ModelEvent::Traffic(event.payload)),
                "event.runner.progress" => cx.emit(ModelEvent::RunnerProgress(event.payload)),
                "event.runner.finished" => cx.emit(ModelEvent::RunnerFinished(event.payload)),
                "event.flow.progress" => cx.emit(ModelEvent::FlowProgress(event.payload)),
                "event.flow.finished" => cx.emit(ModelEvent::FlowFinished(event.payload)),
                "event.flow.loadtest.progress" => cx.emit(ModelEvent::FlowLoadProgress(event.payload)),
                "event.flow.loadtest.finished" => cx.emit(ModelEvent::FlowLoadFinished(event.payload)),
                _ => {}
            },
        }
    }

    /// [`refresh`](Self::refresh) in a moment: writes arrive in bursts (autosave, a folder run, an import),
    /// and everything that arrives within [`REFRESH_DELAY`] is read once.
    fn refresh_soon(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.refresh_scheduled, true) {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REFRESH_DELAY).await;
            this.update(cx, |this, cx| {
                this.refresh_scheduled = false;
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Re-reads the workspace. Changes arrive in bursts (a save is several writes), so a refresh asked for
    /// while one is in flight collapses into a single follow-up.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            self.refresh_again = true;
            return;
        }
        self.refreshing = true;
        let client = self.engine.client.clone();
        call(&client, cx, "config.get", json!({}), |this, outcome, cx| this.apply_config(outcome, cx));
    }

    fn apply_config(&mut self, outcome: Result<Value, ClientError>, cx: &mut Context<Self>) {
        self.refreshing = false;
        let previous_workspace = self.config.active_workspace_id.clone();
        match outcome.map_err(|e| describe(&e)).and_then(|value| ConfigSnapshot::from_value(&value)) {
            Ok(config) => {
                // Vaults belong to the workspace and the proxies to the settings; an entity write moves neither.
                let rest_changed = self.load != Load::Ready
                    || previous_workspace != config.active_workspace_id
                    || self.config.settings != config.settings;
                self.config = config;
                self.load = Load::Ready;
                cx.emit(ModelEvent::ConfigChanged);
                if !previous_workspace.is_empty() && previous_workspace != self.config.active_workspace_id {
                    cx.emit(ModelEvent::WorkspaceSwitched);
                }
                self.refresh_git(cx);
                if rest_changed {
                    self.refresh_vaults(cx);
                    self.refresh_proxy(cx);
                    self.refresh_claims(cx);
                }
            }
            Err(message) => {
                // Keep showing the last good snapshot; only a first load has nothing to fall back on.
                if self.load == Load::Loading {
                    self.load = Load::Failed(message.clone());
                }
                cx.emit(ModelEvent::Notice(t_with("app.model.read_workspace_failed", &[("error", &message)])));
            }
        }
        cx.notify();
        if std::mem::take(&mut self.refresh_again) {
            self.refresh(cx);
        }
    }

    pub fn claims(&self) -> &Claims {
        &self.claims
    }

    /// Re-reads the workspace's claims (`claims.list`); a failure keeps the list there is.
    pub fn refresh_claims(&mut self, cx: &mut Context<Self>) {
        if self.workspace_id().is_empty() {
            return;
        }
        let client = self.engine.client.clone();
        let payload = json!({ "workspaceId": self.workspace_id() });
        call(&client, cx, "claims.list", payload, |this, outcome, cx| {
            if let Ok(value) = outcome {
                this.apply_claims(&value, cx);
            }
        });
    }

    /// Replaces the claims with those of `claims.list` / `claims.changed`, when they are this workspace's.
    fn apply_claims(&mut self, payload: &Value, cx: &mut Context<Self>) {
        let ours = payload.get("workspaceId").and_then(Value::as_str).is_none_or(|ws| ws == self.workspace_id());
        let claims = Claims::from_value(payload);
        if ours && claims != self.claims {
            self.claims = claims;
            cx.emit(ModelEvent::ClaimsChanged);
            cx.notify();
        }
    }

    /// A save was refused with `claims.locked`: the editor learns who holds the entity, whatever the list said.
    pub fn note_locked(&mut self, params: &Value, cx: &mut Context<Self>) {
        if let Some(claim) = Claim::from_refusal(params)
            && self.claims.of(&claim.id) != Some(&claim)
        {
            self.claims.insert(claim);
            cx.emit(ModelEvent::ClaimsChanged);
            cx.notify();
        }
    }

    /// Takes the entity over from whoever holds it (`claims.set` with `force`). The list follows from the event.
    pub fn take_over(&mut self, claim: &Claim, cx: &mut Context<Self>) {
        let payload = json!({ "workspaceId": self.workspace_id(), "id": claim.id, "kind": claim.kind, "force": true });
        self.run_for_agents("claims.set", payload, cx);
    }

    /// Lets go of the app's claims on `ids`, as their tabs close.
    pub fn release_claims(&mut self, ids: &[String], cx: &mut Context<Self>) {
        let mine: Vec<&String> = ids.iter().filter(|id| self.claims.held_by_me(id)).collect();
        if !mine.is_empty() {
            let payload = json!({ "workspaceId": self.workspace_id(), "ids": mine });
            self.run_for_agents("claims.release", payload, cx);
        }
    }

    /// Re-reads the workspace's vaults. A failure keeps the list there is.
    pub fn refresh_vaults(&mut self, cx: &mut Context<Self>) {
        if self.workspace_id().is_empty() {
            return;
        }
        let client = self.engine.client.clone();
        let workspace_id = self.workspace_id().to_owned();
        let payload = json!({ "workspaceId": workspace_id });
        call(&client, cx, "vault.list", payload, move |this, outcome, cx| {
            if let Ok(value) = outcome
                && this.workspace_id() == workspace_id
            {
                let vaults = VaultInfo::list_from_value(&value);
                if vaults != this.vaults {
                    this.vaults = vaults;
                    cx.notify();
                }
            }
        });
    }

    /// Re-reads which files differ from the last commit. A failure keeps the previous status: the footer
    /// would rather be a moment stale than empty. A read asked for while one is in flight collapses into one
    /// follow-up, and nothing is announced when the answer is what was already shown.
    pub fn refresh_git(&mut self, cx: &mut Context<Self>) {
        if self.workspace_id().is_empty() {
            return;
        }
        if std::mem::replace(&mut self.git_refreshing, true) {
            self.git_again = true;
            return;
        }
        let client = self.engine.client.clone();
        let workspace_id = self.workspace_id().to_owned();
        let payload = json!({ "workspaceId": workspace_id });
        call(&client, cx, "sync.getEntityStatus", payload, move |this, outcome, cx| {
            this.git_refreshing = false;
            // An answer for a workspace that is no longer the active one describes the wrong repository.
            if let Ok(value) = outcome
                && this.workspace_id() == workspace_id
            {
                let git = GitStatus::from_value(&value);
                if git != this.git {
                    this.git = git;
                    cx.notify();
                }
            }
            if std::mem::take(&mut this.git_again) {
                this.refresh_git(cx);
            }
        });
    }

    pub fn refresh_proxy(&mut self, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "server.status", json!({}), |this, outcome, cx| {
            if let Ok(value) = outcome {
                this.proxy = ProxyStatus::from_value(&value);
                cx.notify();
            }
        });
        call(&client, cx, "server.status", json!({ "target": "reverse" }), |this, outcome, cx| {
            if let Ok(value) = outcome {
                this.reverse_proxy = ProxyStatus::from_value(&value);
                cx.notify();
            }
        });
    }

    /// Starts the reverse proxy if it is stopped, stops it if it is running.
    pub fn toggle_reverse_proxy(&mut self, cx: &mut Context<Self>) {
        if self.reverse_proxy.busy {
            return;
        }
        self.reverse_proxy.busy = true;
        cx.notify();
        let action = if self.reverse_proxy.running { "server.stop" } else { "server.start" };
        let client = self.engine.client.clone();
        call(&client, cx, action, json!({ "target": "reverse" }), |this, outcome, cx| {
            if let Err(error) = outcome {
                cx.emit(ModelEvent::Notice(t_with("app.model.reverse_proxy_failed", &[("error", &describe(&error))])));
            }
            this.reverse_proxy.busy = false;
            this.refresh_proxy(cx);
        });
    }

    /// Asks the engine to restart itself, then reconnects to its successor. Everything the old one held in
    /// memory (open WebSocket connections, captured traffic) is gone; what is saved in the workspace is not.
    pub fn restart_engine(&mut self, cx: &mut Context<Self>) {
        if self.restarting_engine {
            return;
        }
        let client = self.engine.client.clone();
        call(&client, cx, "engine.shutdown", json!({ "restart": true }), |this, outcome, cx| match outcome {
            // The engine announces the restart to every client, this one included; the announcement is what
            // starts the wait. A reply lost to the closing connection is the same answer.
            Ok(_) | Err(ClientError::Closed) => this.await_successor(cx),
            Err(error) => {
                cx.emit(ModelEvent::Notice(t_with("app.model.restart_engine_failed", &[("error", &describe(&error))])))
            }
        });
    }

    /// The engine said it is restarting: waits for the one that replaces it and moves this app onto it.
    fn await_successor(&mut self, cx: &mut Context<Self>) {
        if self.restarting_engine {
            return;
        }
        self.restarting_engine = true;
        cx.notify();
        let data_dir = self.engine.data_dir.clone();
        let old_pid = self.engine.info.pid;
        let restarting = cx.background_spawn(async move {
            reconnect(&data_dir, old_pid, env!("CARGO_PKG_VERSION")).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let outcome = restarting.await;
            this.update(cx, |this, cx| {
                this.restarting_engine = false;
                match outcome {
                    Ok((client, hello, sidecar)) => {
                        let engine = EngineSession::new(client, hello, sidecar);
                        cx.set_global(engine.clone());
                        this.engine = engine;
                        this.connection_lost = false;
                        this.listen(cx);
                        this.refresh(cx);
                        this.refresh_setup(cx);
                        cx.emit(ModelEvent::EngineRestarted);
                        this.refresh_claims(cx);
                        this.refresh_update(cx);
                        this.refresh_agent_activity(cx);
                    }
                    Err(message) => {
                        // The old connection is closed whatever happened, so nothing is live now.
                        this.connection_lost = true;
                        cx.emit(ModelEvent::Notice(t_with("app.model.restart_engine_failed", &[("error", &message)])));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Starts the proxy if it is stopped, stops it if it is running.
    pub fn toggle_proxy(&mut self, cx: &mut Context<Self>) {
        if self.proxy.busy {
            return;
        }
        let action = if self.proxy.running { "server.stop" } else { "server.start" };
        self.run_proxy_command(action, cx);
    }

    pub fn restart_proxy(&mut self, cx: &mut Context<Self>) {
        if !self.proxy.busy {
            self.run_proxy_command("server.restart", cx);
        }
    }

    fn run_proxy_command(&mut self, action: &'static str, cx: &mut Context<Self>) {
        self.proxy.busy = true;
        cx.notify();
        let client = self.engine.client.clone();
        call(&client, cx, action, json!({}), |this, outcome, cx| {
            if let Err(error) = outcome {
                cx.emit(ModelEvent::Notice(t_with("app.model.forward_proxy_failed", &[("error", &describe(&error))])));
            }
            // Whatever the command said, the status is the authority on what is running now.
            this.proxy.busy = false;
            this.refresh_proxy(cx);
        });
    }

    /// Commits `paths` (and pushes, when the workspace has a remote), once the user has had the chance to
    /// give the commit a message.
    pub fn publish(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        if self.publishing || self.refuse_read_only(cx) {
            return;
        }
        if paths.is_empty() {
            self.publish_paths(paths, None, cx);
        } else {
            cx.emit(ModelEvent::AskSyncMessage(SyncScope::Paths(paths)));
        }
    }

    /// Runs the sync the user was asked about. An empty `message` leaves the engine to word the commits.
    pub fn sync_with_message(&mut self, scope: SyncScope, message: &str, cx: &mut Context<Self>) {
        if self.publishing || self.refuse_read_only(cx) {
            return;
        }
        let message = Some(message.trim().to_owned()).filter(|message| !message.is_empty());
        match scope {
            SyncScope::Paths(paths) => self.publish_paths(paths, message, cx),
            SyncScope::All => self.sync_everything(message, cx),
        }
    }

    fn publish_paths(&mut self, paths: Vec<String>, message: Option<String>, cx: &mut Context<Self>) {
        self.publishing = true;
        cx.notify();
        let client = self.engine.client.clone();
        // With changed files, publish them (the engine commits, and pushes when there is a remote). With none,
        // what is left to do is push the commits the remote has not seen.
        let (action, payload) = if paths.is_empty() {
            ("sync.push", json!({ "workspaceId": self.workspace_id() }))
        } else {
            let mut payload = json!({ "workspaceId": self.workspace_id(), "paths": paths });
            if let Some(message) = message {
                payload["message"] = json!(message);
            }
            ("entity.publish", payload)
        };
        call(&client, cx, action, payload, |this, outcome, cx| {
            this.publishing = false;
            let failure = match outcome {
                Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(false) => {
                    Some(error_text(&result).unwrap_or_else(|| t("app.engine.refused").to_owned()))
                }
                Ok(_) => None,
                Err(error) => Some(describe(&error)),
            };
            if let Some(message) = failure {
                cx.emit(ModelEvent::Notice(t_with("app.model.publish_failed", &[("error", &message)])));
            }
            this.refresh_git(cx);
            cx.notify();
        });
    }

    /// Syncs everything in the workspace that was never synced: the engine commits each new or changed item
    /// in a commit of its own, then pushes when there is a remote. Progress arrives as `event.sync.status`.
    pub fn sync_all(&mut self, cx: &mut Context<Self>) {
        if self.publishing || self.refuse_read_only(cx) {
            return;
        }
        // With nothing to commit there is no commit to word: what is left is the push.
        if self.git.rollup(&[]).changed.is_empty() {
            self.sync_everything(None, cx);
        } else {
            cx.emit(ModelEvent::AskSyncMessage(SyncScope::All));
        }
    }

    fn sync_everything(&mut self, message: Option<String>, cx: &mut Context<Self>) {
        self.publishing = true;
        cx.notify();
        let client = self.engine.client.clone();
        let mut payload = json!({ "workspaceId": self.workspace_id() });
        if let Some(message) = message {
            payload["message"] = json!(message);
        }
        call(&client, cx, "sync.all", payload, |this, outcome, cx| {
            this.publishing = false;
            let message = match failure(&outcome) {
                Some(message) => t_with("app.model.sync_failed", &[("error", &message)]),
                None => {
                    let result = outcome.unwrap_or(Value::Null);
                    let count = |key: &str| result.get(key).and_then(Value::as_u64).unwrap_or(0);
                    let pushed = result.get("pushed").and_then(Value::as_bool) == Some(true);
                    let message = match (count("synced"), pushed) {
                        (0, true) => t("app.model.sync.pushed_waiting").to_owned(),
                        (0, false) => t("app.model.sync.up_to_date").to_owned(),
                        (1, true) => t("app.model.sync.synced_one_pushed").to_owned(),
                        (1, false) => t("app.model.sync.synced_one").to_owned(),
                        (synced, true) => t_with("app.model.sync.synced_other_pushed", &[("count", &synced)]),
                        (synced, false) => t_with("app.model.sync.synced_other", &[("count", &synced)]),
                    };
                    let deletions = match count("pendingDeletions") {
                        0 => None,
                        1 => Some(t("app.model.sync.deletions_one").to_owned()),
                        waiting => Some(t_with("app.model.sync.deletions_other", &[("count", &waiting)])),
                    };
                    match deletions {
                        Some(deletions) => {
                            t_with("app.model.sync.with_deletions", &[("synced", &message), ("deletions", &deletions)])
                        }
                        None => message,
                    }
                }
            };
            cx.emit(ModelEvent::Notice(message));
            this.refresh(cx);
            cx.notify();
        });
    }

    /// Commits the file of the entity with `id` (and pushes, when the workspace has a remote).
    pub fn sync_entity(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.git.entity_path(id).map(str::to_owned) {
            Some(path) => self.publish(vec![path], cx),
            None => cx.emit(ModelEvent::Notice(t("app.model.sync_no_file").to_owned())),
        }
    }

    /// Puts the file of the entity with `id` back to the last synced version, discarding what was saved since.
    pub fn revert_entity(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.publishing {
            return;
        }
        let Some(path) = self.git.entity_path(id).map(str::to_owned) else {
            cx.emit(ModelEvent::Notice(t("app.model.revert_no_file").to_owned()));
            return;
        };
        self.publishing = true;
        cx.notify();
        let client = self.engine.client.clone();
        let payload = json!({ "workspaceId": self.workspace_id(), "relPath": path });
        let id = id.to_owned();
        call(&client, cx, "git.discard", payload, move |this, outcome, cx| {
            this.publishing = false;
            match failure(&outcome) {
                Some(message) => cx.emit(ModelEvent::Notice(t_with("app.model.revert_failed", &[("error", &message)]))),
                None => cx.emit(ModelEvent::EntityReverted(id)),
            }
            this.refresh(cx);
            cx.notify();
        });
    }

    /// Syncs a deletion: commits it (and pushes, when the workspace has a remote), after which what was
    /// deleted is gone for good.
    pub fn confirm_delete(&mut self, target: Deleted, cx: &mut Context<Self>) {
        self.resolve_delete(target, false, cx);
    }

    /// Brings back what was deleted, as it was when it was last synced.
    pub fn revert_delete(&mut self, target: Deleted, cx: &mut Context<Self>) {
        self.resolve_delete(target, true, cx);
    }

    fn resolve_delete(&mut self, target: Deleted, revert: bool, cx: &mut Context<Self>) {
        // Confirming a delete syncs it; taking one back does not.
        if self.publishing || (!revert && self.refuse_read_only(cx)) {
            return;
        }
        self.publishing = true;
        cx.notify();
        let (action, kind, id) = match (&target, revert) {
            (Deleted::Entity { kind, id }, true) => ("entity.revertDelete", *kind, id.clone()),
            (Deleted::Entity { kind, id }, false) => ("entity.confirmDelete", *kind, id.clone()),
            (Deleted::Folder { kind, id }, true) => ("folder.revertDelete", *kind, id.clone()),
            (Deleted::Folder { kind, id }, false) => ("folder.confirmDelete", *kind, id.clone()),
        };
        let client = self.engine.client.clone();
        let payload = json!({ "workspaceId": self.workspace_id(), "kind": kind, "id": id });
        call(&client, cx, action, payload, move |this, outcome, cx| {
            this.publishing = false;
            let failed = if revert { "app.model.revert_delete_failed" } else { "app.model.confirm_delete_failed" };
            match failure(&outcome) {
                Some(message) => cx.emit(ModelEvent::Notice(t_with(failed, &[("error", &message)]))),
                None if revert => cx.emit(ModelEvent::EntityReverted(id)),
                None => {}
            }
            this.refresh(cx);
            cx.notify();
        });
    }

    /// Puts each of `paths` back to its last synced version, one after the other. A file that was never synced
    /// is removed. Reports the first failure once all of them have been tried.
    pub fn revert_paths(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        if self.publishing || paths.is_empty() {
            return;
        }
        self.publishing = true;
        cx.notify();
        self.discard_next(paths, None, cx);
    }

    fn discard_next(&mut self, mut paths: Vec<String>, first_failure: Option<String>, cx: &mut Context<Self>) {
        let Some(path) = paths.pop() else {
            self.publishing = false;
            if let Some(message) = first_failure {
                cx.emit(ModelEvent::Notice(t_with("app.model.revert_failed", &[("error", &message)])));
            }
            self.refresh(cx);
            cx.notify();
            return;
        };
        let client = self.engine.client.clone();
        let payload = json!({ "workspaceId": self.workspace_id(), "relPath": path });
        call(&client, cx, "git.discard", payload, move |this, outcome, cx| {
            let first_failure = first_failure.or_else(|| failure(&outcome));
            this.discard_next(paths, first_failure, cx);
        });
    }

    pub fn set_active_workspace(&mut self, id: &str, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "workspace.setActive", json!({ "id": id }), |this, outcome, cx| match outcome {
            Ok(_) => this.refresh(cx),
            Err(error) => cx
                .emit(ModelEvent::Notice(t_with("app.model.switch_workspace_failed", &[("error", &describe(&error))]))),
        });
    }

    pub fn rename_workspace(&mut self, id: &str, name: &str, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "workspace.rename", json!({ "id": id, "name": name }), |this, outcome, cx| match outcome {
            Ok(_) => this.refresh(cx),
            Err(error) => cx
                .emit(ModelEvent::Notice(t_with("app.model.rename_workspace_failed", &[("error", &describe(&error))]))),
        });
    }

    pub fn delete_workspace(&mut self, id: &str, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "workspace.delete", json!({ "id": id }), |this, outcome, cx| match outcome {
            Ok(_) => this.refresh(cx),
            Err(error) => cx
                .emit(ModelEvent::Notice(t_with("app.model.delete_workspace_failed", &[("error", &describe(&error))]))),
        });
    }

    /// `None` selects no environment.
    pub fn set_active_environment(&mut self, id: Option<&str>, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "env.setActive", json!({ "id": id }), |this, outcome, cx| match outcome {
            Ok(_) => this.refresh(cx),
            Err(error) => {
                cx.emit(ModelEvent::Notice(t_with("app.model.environment_failed", &[("error", &describe(&error))])))
            }
        });
    }

    /// Turns a mapping, mock or proxy rule on or off. `kind` is the command's own name for it: `mappings`,
    /// `mocks` or `rules`.
    pub fn set_enabled(&mut self, kind: &'static str, id: &str, enabled: bool, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        let payload = json!({ "workspaceId": self.workspace_id(), "kind": kind, "id": id, "enabled": enabled });
        call(&client, cx, "entity.setEnabled", payload, |this, outcome, cx| match outcome {
            Ok(_) => this.refresh(cx),
            Err(error) => {
                cx.emit(ModelEvent::Notice(t_with("app.model.change_failed", &[("error", &describe(&error))])))
            }
        });
    }

    /// Changes engine-wide settings: ports, TLS, tray behavior. `patch` holds only what changes.
    pub fn update_settings(&mut self, patch: Value, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "settings.update", patch, |this, outcome, cx| match outcome {
            Ok(_) => this.refresh(cx),
            Err(error) => {
                cx.emit(ModelEvent::Notice(t_with("app.model.setting_failed", &[("error", &describe(&error))])))
            }
        });
    }

    /// A message for the user from a view that has no window in hand.
    pub fn notice(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        cx.emit(ModelEvent::Notice(text.into()));
    }
}
