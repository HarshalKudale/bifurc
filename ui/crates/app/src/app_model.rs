//! The workspace as the engine reports it, and the one place the engine's events are received.
//!
//! `AppModel` holds no domain state of its own. It keeps the latest [`ConfigSnapshot`], the workspace's git
//! status and the proxy's status, replaces them when the engine says something changed, and re-broadcasts
//! the engine's socket events to whichever views care. Everything that changes the workspace is an engine
//! command; this model then re-reads.

use bifurc_engine_client::{ClientError, EngineClient, Incoming};
use futures::StreamExt as _;
use gpui_kit::{AppContext as _, Context, EventEmitter};
use serde_json::{Value, json};

use crate::{
    engine::{EngineSession, call, describe, failure},
    model::{ConfigSnapshot, git::GitStatus},
};

/// Wire names of the engine events the UI listens to.
///
/// `entity.changed` is only sent when a REST request or mock is *created*; an update, a delete, or any other
/// kind of entity is announced by `sync.entityStatus` alone (the engine sends it after every entity
/// write). Both therefore mean "re-read the workspace".
const EVENTS: [&str; 10] = [
    "event.entity.changed",
    "event.sync.entityStatus",
    "event.sync.status",
    "event.settings.changed",
    "event.server.error",
    "event.ws.status",
    "event.ws.message",
    "event.log.entry",
    "event.runner.progress",
    "event.runner.finished",
];

#[derive(Debug, Clone, PartialEq)]
pub enum Load {
    Loading,
    Ready,
    Failed(String),
}

/// The forward proxy, as `server.status` reports it. (The reverse proxy that serves `.localhost` names is a
/// separate listener on `settings.port`; the engine has no status command for it.)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProxyStatus {
    pub running: bool,
    pub port: u16,
    pub error: Option<String>,
    /// A start or stop is in flight.
    pub busy: bool,
}

#[derive(Debug, Clone)]
pub enum ModelEvent {
    /// A new snapshot is in place.
    ConfigChanged,
    /// The active workspace is not the one it was; anything open belongs to the previous one.
    WorkspaceSwitched,
    /// `event.ws.status`, as the engine sent it.
    SocketStatus(Value),
    /// `event.ws.message`, as the engine sent it.
    SocketMessage(Value),
    /// `event.log.entry`: proxied traffic, as a batch of entries.
    Traffic(Value),
    /// `event.runner.progress`: one request of a folder run has finished.
    RunnerProgress(Value),
    /// `event.runner.finished`: a folder run ended, with its report.
    RunnerFinished(Value),
    /// The entity with this id was put back to its last synced version; an editor showing it must re-read it.
    EntityReverted(String),
    /// A short message for the user: a failed command, a lost connection.
    Notice(String),
}

pub struct AppModel {
    engine: EngineSession,
    config: ConfigSnapshot,
    git: GitStatus,
    proxy: ProxyStatus,
    load: Load,
    refreshing: bool,
    refresh_again: bool,
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
            proxy: ProxyStatus::default(),
            load: Load::Loading,
            refreshing: false,
            refresh_again: false,
            connection_lost: false,
            publishing: false,
        };
        model.listen(cx);
        model.refresh(cx);
        model
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

    pub fn proxy(&self) -> &ProxyStatus {
        &self.proxy
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

    fn listen(&mut self, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        let subscribing = cx.background_spawn(async move { client.subscribe(&EVENTS).await });
        cx.spawn(async move |this, cx| {
            let mut incoming = match subscribing.await {
                Ok(incoming) => incoming,
                Err(error) => {
                    this.update(cx, |_, cx| {
                        cx.emit(ModelEvent::Notice(format!("Live updates are unavailable: {}", describe(&error))));
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
            Incoming::Closed => {
                self.connection_lost = true;
                cx.emit(ModelEvent::Notice("The connection to the engine was lost.".to_owned()));
                cx.notify();
            }
            Incoming::Event(event) => match event.event.as_str() {
                "event.entity.changed" | "event.sync.entityStatus" | "event.settings.changed" => self.refresh(cx),
                "event.sync.status" => self.refresh_git(cx),
                "event.server.error" => self.refresh_proxy(cx),
                "event.ws.status" => cx.emit(ModelEvent::SocketStatus(event.payload)),
                "event.ws.message" => cx.emit(ModelEvent::SocketMessage(event.payload)),
                "event.log.entry" => cx.emit(ModelEvent::Traffic(event.payload)),
                "event.runner.progress" => cx.emit(ModelEvent::RunnerProgress(event.payload)),
                "event.runner.finished" => cx.emit(ModelEvent::RunnerFinished(event.payload)),
                _ => {}
            },
        }
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
                self.config = config;
                self.load = Load::Ready;
                cx.emit(ModelEvent::ConfigChanged);
                if !previous_workspace.is_empty() && previous_workspace != self.config.active_workspace_id {
                    cx.emit(ModelEvent::WorkspaceSwitched);
                }
                self.refresh_git(cx);
                self.refresh_proxy(cx);
            }
            Err(message) => {
                // Keep showing the last good snapshot; only a first load has nothing to fall back on.
                if self.load == Load::Loading {
                    self.load = Load::Failed(message.clone());
                }
                cx.emit(ModelEvent::Notice(format!("Couldn't read the workspace. {message}")));
            }
        }
        cx.notify();
        if std::mem::take(&mut self.refresh_again) {
            self.refresh(cx);
        }
    }

    /// Re-reads which files differ from the last commit. A failure keeps the previous status: the footer
    /// would rather be a moment stale than empty.
    pub fn refresh_git(&mut self, cx: &mut Context<Self>) {
        if self.workspace_id().is_empty() {
            return;
        }
        let client = self.engine.client.clone();
        let workspace_id = self.workspace_id().to_owned();
        let payload = json!({ "workspaceId": workspace_id });
        call(&client, cx, "sync.getEntityStatus", payload, move |this, outcome, cx| {
            // An answer for a workspace that is no longer the active one describes the wrong repository.
            if let Ok(value) = outcome
                && this.workspace_id() == workspace_id
            {
                this.git = GitStatus::from_value(&value);
                cx.notify();
            }
        });
    }

    pub fn refresh_proxy(&mut self, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "server.status", json!({}), |this, outcome, cx| {
            if let Ok(value) = outcome {
                this.proxy = ProxyStatus {
                    running: value.get("running").and_then(Value::as_bool).unwrap_or(false),
                    port: value.get("port").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()).unwrap_or(0),
                    error: value.get("error").and_then(Value::as_str).map(str::to_owned),
                    busy: false,
                };
                cx.notify();
            }
        });
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
                cx.emit(ModelEvent::Notice(format!("Couldn't change the forward proxy. {}", describe(&error))));
            }
            // Whatever the command said, the status is the authority on what is running now.
            this.proxy.busy = false;
            this.refresh_proxy(cx);
        });
    }

    /// Commits `paths` (and pushes, when the workspace has a remote).
    pub fn publish(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        if self.publishing {
            return;
        }
        self.publishing = true;
        cx.notify();
        let client = self.engine.client.clone();
        // With changed files, publish them (the engine commits, and pushes when there is a remote). With none,
        // what is left to do is push the commits the remote has not seen.
        let (action, payload) = if paths.is_empty() {
            ("sync.push", json!({ "workspaceId": self.workspace_id() }))
        } else {
            ("entity.publish", json!({ "workspaceId": self.workspace_id(), "paths": paths }))
        };
        call(&client, cx, action, payload, |this, outcome, cx| {
            this.publishing = false;
            let failure = match outcome {
                Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(false) => {
                    Some(result.get("error").and_then(Value::as_str).unwrap_or("The engine refused.").to_owned())
                }
                Ok(_) => None,
                Err(error) => Some(describe(&error)),
            };
            if let Some(message) = failure {
                cx.emit(ModelEvent::Notice(format!("Couldn't publish. {message}")));
            }
            this.refresh_git(cx);
            cx.notify();
        });
    }

    /// Commits the file of the entity with `id` (and pushes, when the workspace has a remote).
    pub fn sync_entity(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.git.entity_path(id).map(str::to_owned) {
            Some(path) => self.publish(vec![path], cx),
            None => cx.emit(ModelEvent::Notice("Couldn't sync. It has no file in the workspace yet.".to_owned())),
        }
    }

    /// Puts the file of the entity with `id` back to the last synced version, discarding what was saved since.
    pub fn revert_entity(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.publishing {
            return;
        }
        let Some(path) = self.git.entity_path(id).map(str::to_owned) else {
            cx.emit(ModelEvent::Notice("Couldn't revert. It has no file in the workspace yet.".to_owned()));
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
                Some(message) => cx.emit(ModelEvent::Notice(format!("Couldn't revert. {message}"))),
                None => cx.emit(ModelEvent::EntityReverted(id)),
            }
            this.refresh(cx);
            cx.notify();
        });
    }

    pub fn set_active_workspace(&mut self, id: &str, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "workspace.setActive", json!({ "id": id }), |this, outcome, cx| match outcome {
            Ok(_) => this.refresh(cx),
            Err(error) => cx.emit(ModelEvent::Notice(format!("Couldn't switch workspace. {}", describe(&error)))),
        });
    }

    /// Creates a workspace and switches to it.
    pub fn add_workspace(&mut self, name: &str, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "workspace.add", json!({ "name": name }), |this, outcome, cx| match outcome {
            Ok(created) => match created.get("id").and_then(Value::as_str) {
                Some(id) => this.set_active_workspace(&id.to_owned(), cx),
                None => this.refresh(cx),
            },
            Err(error) => cx.emit(ModelEvent::Notice(format!("Couldn't create the workspace. {}", describe(&error)))),
        });
    }

    pub fn rename_workspace(&mut self, id: &str, name: &str, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "workspace.rename", json!({ "id": id, "name": name }), |this, outcome, cx| match outcome {
            Ok(_) => this.refresh(cx),
            Err(error) => cx.emit(ModelEvent::Notice(format!("Couldn't rename the workspace. {}", describe(&error)))),
        });
    }

    pub fn delete_workspace(&mut self, id: &str, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "workspace.delete", json!({ "id": id }), |this, outcome, cx| match outcome {
            Ok(_) => this.refresh(cx),
            Err(error) => cx.emit(ModelEvent::Notice(format!("Couldn't delete the workspace. {}", describe(&error)))),
        });
    }

    /// `None` selects no environment.
    pub fn set_active_environment(&mut self, id: Option<&str>, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "env.setActive", json!({ "id": id }), |this, outcome, cx| match outcome {
            Ok(_) => this.refresh(cx),
            Err(error) => cx.emit(ModelEvent::Notice(format!("Couldn't change environment. {}", describe(&error)))),
        });
    }

    /// Turns a mapping, mock or proxy rule on or off. `kind` is the command's own name for it: `mappings`,
    /// `mocks` or `rules`.
    pub fn set_enabled(&mut self, kind: &'static str, id: &str, enabled: bool, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        let payload = json!({ "workspaceId": self.workspace_id(), "kind": kind, "id": id, "enabled": enabled });
        call(&client, cx, "entity.setEnabled", payload, |this, outcome, cx| match outcome {
            Ok(_) => this.refresh(cx),
            Err(error) => cx.emit(ModelEvent::Notice(format!("Couldn't change it. {}", describe(&error)))),
        });
    }

    /// Changes engine-wide settings: ports, TLS, tray behavior. `patch` holds only what changes.
    pub fn update_settings(&mut self, patch: Value, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "settings.update", patch, |this, outcome, cx| match outcome {
            Ok(_) => this.refresh(cx),
            Err(error) => cx.emit(ModelEvent::Notice(format!("Couldn't change the setting. {}", describe(&error)))),
        });
    }

    /// A message for the user from a view that has no window in hand.
    pub fn notice(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        cx.emit(ModelEvent::Notice(text.into()));
    }
}
