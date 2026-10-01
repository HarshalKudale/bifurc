//! The workspace as the engine reports it, and the one place the engine's events are received.
//!
//! `AppModel` holds no domain state of its own. It keeps the latest [`ConfigSnapshot`], replaces it when the
//! engine says something changed, and re-broadcasts the engine's socket events to whichever sessions care.
//! Everything that changes the workspace is an engine command; this model then re-reads.

use bifurc_engine_client::{ClientError, EngineClient, Incoming};
use futures::StreamExt as _;
use gpui_kit::{AppContext as _, Context, EventEmitter};
use serde_json::{Value, json};

use crate::{
    engine::{EngineSession, call, describe},
    model::ConfigSnapshot,
};

/// Wire names of the engine events the UI listens to.
///
/// `entity.changed` is only sent when a REST request or mock is *created*; an update, a delete, or any other
/// kind of entity is announced by `sync.entityStatus` alone (the engine sends it after every entity
/// write). Both therefore mean "re-read the workspace".
const EVENTS: [&str; 6] = [
    "event.entity.changed",
    "event.sync.entityStatus",
    "event.settings.changed",
    "event.ws.status",
    "event.ws.message",
    "event.log.entry",
];

#[derive(Debug, Clone, PartialEq)]
pub enum Load {
    Loading,
    Ready,
    Failed(String),
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
    /// A short message for the user: a failed command, a lost connection.
    Notice(String),
}

pub struct AppModel {
    engine: EngineSession,
    config: ConfigSnapshot,
    load: Load,
    refreshing: bool,
    refresh_again: bool,
    connection_lost: bool,
}

impl EventEmitter<ModelEvent> for AppModel {}

impl AppModel {
    pub fn new(engine: EngineSession, cx: &mut Context<Self>) -> Self {
        let mut model = Self {
            engine,
            config: ConfigSnapshot::default(),
            load: Load::Loading,
            refreshing: false,
            refresh_again: false,
            connection_lost: false,
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

    pub fn load(&self) -> &Load {
        &self.load
    }

    pub fn workspace_id(&self) -> &str {
        &self.config.active_workspace_id
    }

    pub fn is_connection_lost(&self) -> bool {
        self.connection_lost
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
                "event.ws.status" => cx.emit(ModelEvent::SocketStatus(event.payload)),
                "event.ws.message" => cx.emit(ModelEvent::SocketMessage(event.payload)),
                "event.log.entry" => cx.emit(ModelEvent::Traffic(event.payload)),
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

    pub fn set_active_workspace(&mut self, id: &str, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        call(&client, cx, "workspace.setActive", json!({ "id": id }), |this, outcome, cx| match outcome {
            Ok(_) => this.refresh(cx),
            Err(error) => cx.emit(ModelEvent::Notice(format!("Couldn't switch workspace. {}", describe(&error)))),
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

    pub fn set_mapping_enabled(&mut self, id: &str, enabled: bool, cx: &mut Context<Self>) {
        let client = self.engine.client.clone();
        let payload = json!({ "workspaceId": self.workspace_id(), "kind": "mappings", "id": id, "enabled": enabled });
        call(&client, cx, "entity.setEnabled", payload, |this, outcome, cx| match outcome {
            Ok(_) => this.refresh(cx),
            Err(error) => cx.emit(ModelEvent::Notice(format!("Couldn't change the mapping. {}", describe(&error)))),
        });
    }
}
