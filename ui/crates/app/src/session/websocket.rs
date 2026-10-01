//! An open WebSocket request. It is an ordinary request type: it has a name, a saved entity in the engine, a
//! place in the explorer tree, and Save/Revert like the REST editor beside it. What differs is the verb —
//! Connect instead of Send — and that the "response" is a live stream owned by the engine
//! ([`SocketConnection`]), which is why closing this tab does not close the socket.
//!
//! Connecting a saved, unedited request sends only its id (`ws.connect {workspaceId, socketId}`); a draft or
//! an edited request sends the target it holds. The engine resolves environment variables either way.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    notification::Notification,
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div,
};
use serde_json::{Map, Value, json};

use super::{
    LoadState, SessionEvent,
    connection::{ConnectionEvent, SocketConnection},
};
use crate::{
    app_model::{AppModel, ModelEvent},
    components::{KeyValueEvent, KeyValueTable, VarEditor, page},
    engine::{call, call_in, describe},
    model::{
        RequestKind,
        http::{header_map, header_pairs},
        socket::SocketView,
    },
};

const DEFAULT_NAME: &str = "New WebSocket";

#[derive(Debug, Clone, PartialEq)]
struct Fields {
    name: String,
    url: String,
    headers: Vec<(String, String)>,
}

pub struct WsSession {
    model: Entity<AppModel>,
    workspace_id: String,
    saved_id: Option<String>,
    folder_id: Option<String>,
    /// The key the engine knows this connection by: the saved id, or a name made up for a draft.
    socket_id: String,
    entity: Map<String, Value>,
    load: LoadState,
    name: Entity<InputState>,
    url: VarEditor,
    headers: Entity<KeyValueTable>,
    baseline: Fields,
    connection: Entity<SocketConnection>,
    saving: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SessionEvent> for WsSession {}

impl WsSession {
    fn build(
        model: Entity<AppModel>,
        saved_id: Option<String>,
        folder_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let workspace_id = model.read(cx).workspace_id().to_owned();
        let socket_id = saved_id.clone().unwrap_or_else(|| format!("draft-{}", cx.entity_id().as_u64()));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Connection name"));
        let url = VarEditor::line("wss://example.test/socket", window, cx);
        let headers = cx.new(|cx| KeyValueTable::new("Header", "Value", window, cx));
        let connection = cx.new(|cx| SocketConnection::new(window, cx));

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.changed(cx),
            InputEvent::PressEnter { .. } => this.connect(window, cx),
            _ => {}
        }));
        subscriptions.push(cx.subscribe_in(
            url.state(),
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.changed(cx),
                InputEvent::PressEnter { .. } => this.connect(window, cx),
                _ => {}
            },
        ));
        subscriptions.push(cx.subscribe(&headers, |this, _, _: &KeyValueEvent, cx| this.changed(cx)));
        subscriptions.push(cx.subscribe_in(&connection, window, |this, _, event: &ConnectionEvent, window, cx| {
            match event {
                ConnectionEvent::Send(text) => this.send_message(text.clone(), window, cx),
                ConnectionEvent::Clear => this.clear_messages(cx),
            }
        }));
        subscriptions.push(cx.subscribe(&model, |this, _, event: &ModelEvent, cx| match event {
            ModelEvent::SocketStatus(payload) if this.is_mine(payload) => {
                this.connection.update(cx, |c, cx| c.apply_status_event(payload, cx));
                cx.emit(SessionEvent::Changed);
            }
            ModelEvent::SocketMessage(payload) if this.is_mine(payload) => {
                this.connection.update(cx, |c, cx| c.apply_message_event(payload, cx));
            }
            _ => {}
        }));

        let baseline = Fields { name: DEFAULT_NAME.to_owned(), url: String::new(), headers: Vec::new() };
        let mut this = Self {
            model,
            workspace_id,
            saved_id,
            folder_id,
            socket_id,
            entity: Map::new(),
            load: LoadState::Ready,
            name,
            url,
            headers,
            baseline: baseline.clone(),
            connection,
            saving: false,
            _subscriptions: subscriptions,
        };
        this.apply_fields(&baseline, window, cx);
        this
    }

    pub fn new_draft(
        model: Entity<AppModel>,
        folder_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(model, None, folder_id, window, cx)
    }

    pub fn open_saved(model: Entity<AppModel>, id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::build(model, Some(id.clone()), None, window, cx);
        this.load = LoadState::Loading;
        let client = this.model.read(cx).client().clone();
        let workspace_id = this.workspace_id.clone();
        let payload = json!({ "workspaceId": workspace_id, "kind": RequestKind::WebSocket.entity_kind(), "id": id });
        call_in(&client, window, cx, "entity.load", payload, |this, outcome, window, cx| {
            match outcome.map_err(|e| describe(&e)).and_then(|v| {
                v.get("entity")
                    .and_then(Value::as_object)
                    .cloned()
                    .ok_or_else(|| "The request no longer exists.".to_owned())
            }) {
                Ok(entity) => {
                    this.adopt_entity(entity, window, cx);
                    this.load = LoadState::Ready;
                    this.attach(cx);
                }
                Err(message) => this.load = LoadState::Failed(message),
            }
            cx.emit(SessionEvent::Changed);
            cx.notify();
        });
        this
    }

    /// The engine keeps the connection (and its history) across tabs and windows. Asking for its status when
    /// the tab opens is what makes a live socket still live, with its messages, after the tab was closed.
    fn attach(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "socketId": self.socket_id });
        call(&client, cx, "ws.status", payload, |this, outcome, cx| {
            if let Ok(status) = outcome {
                let view = SocketView::from_status(&status);
                this.connection.update(cx, |c, cx| c.replace_view(view, cx));
                cx.emit(SessionEvent::Changed);
            }
        });
    }

    fn is_mine(&self, payload: &Value) -> bool {
        payload.get("socketId").and_then(Value::as_str) == Some(self.socket_id.as_str())
            && payload.get("workspaceId").and_then(Value::as_str) == Some(self.workspace_id.as_str())
    }

    pub fn saved_id(&self) -> Option<&str> {
        self.saved_id.as_deref()
    }

    pub fn connection(&self) -> &Entity<SocketConnection> {
        &self.connection
    }

    pub fn title(&self, cx: &gpui_kit::App) -> SharedString {
        let name = self.name.read(cx).value();
        if name.is_empty() { DEFAULT_NAME.into() } else { name }
    }

    pub fn is_dirty(&self, cx: &gpui_kit::App) -> bool {
        self.load == LoadState::Ready && self.fields(cx) != self.baseline
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(SessionEvent::Changed);
        cx.notify();
    }

    fn fields(&self, cx: &gpui_kit::App) -> Fields {
        Fields {
            name: self.name.read(cx).value().to_string(),
            url: self.url.line_value(cx),
            headers: self.headers.read(cx).pairs(cx),
        }
    }

    fn fields_of(entity: &Map<String, Value>) -> Fields {
        Fields {
            name: entity.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
            url: entity.get("url").and_then(Value::as_str).unwrap_or_default().trim().to_owned(),
            headers: header_pairs(entity.get("headers").unwrap_or(&Value::Null)),
        }
    }

    fn apply_fields(&mut self, fields: &Fields, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |s, cx| s.set_value(fields.name.clone(), window, cx));
        self.url.set_value(fields.url.clone(), window, cx);
        self.headers.update(cx, |table, cx| table.set_pairs(&fields.headers, window, cx));
    }

    fn adopt_entity(&mut self, entity: Map<String, Value>, window: &mut Window, cx: &mut Context<Self>) {
        let fields = Self::fields_of(&entity);
        self.folder_id = entity.get("folderId").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
        self.apply_fields(&fields, window, cx);
        self.baseline = fields;
        self.entity = entity;
    }

    pub fn revert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let baseline = self.baseline.clone();
        self.apply_fields(&baseline, window, cx);
        cx.emit(SessionEvent::Changed);
        cx.notify();
    }

    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || self.load != LoadState::Ready {
            return;
        }
        let fields = self.fields(cx);
        if fields.url.is_empty() {
            window.push_notification(Notification::warning("Enter a URL before saving.").title("Can't save yet"), cx);
            return;
        }
        let name = if fields.name.trim().is_empty() { DEFAULT_NAME.to_owned() } else { fields.name.trim().to_owned() };
        let mut entity = self.entity.clone();
        entity.insert("name".into(), json!(name));
        entity.insert("url".into(), json!(fields.url));
        entity.insert("headers".into(), Value::Object(header_map(&fields.headers)));
        entity.insert("workspaceId".into(), json!(self.workspace_id));
        if let Some(folder) = &self.folder_id {
            entity.insert("folderId".into(), json!(folder));
        }

        self.saving = true;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        let kind = RequestKind::WebSocket.entity_kind();
        let creating = self.saved_id.is_none();
        let (action, payload) = match &self.saved_id {
            Some(id) => {
                entity.insert("id".into(), json!(id));
                ("entity.update", json!({ "kind": kind, "entity": entity }))
            }
            None => ("entity.create", json!({ "kind": kind, "workspaceId": self.workspace_id, "entity": entity })),
        };
        call_in(&client, window, cx, action, payload, move |this, outcome, window, cx| {
            this.saving = false;
            match outcome {
                Ok(result) => {
                    if creating {
                        match result.get("entity").and_then(Value::as_object).cloned() {
                            Some(saved) => {
                                this.saved_id = saved.get("id").and_then(Value::as_str).map(str::to_owned);
                                this.adopt_entity(saved, window, cx);
                                // A draft that is already connected stays under the name it connected with.
                                if !this.connection.read(cx).state().is_live()
                                    && let Some(id) = &this.saved_id
                                {
                                    this.socket_id = id.clone();
                                }
                                if let Some(id) = this.saved_id.clone() {
                                    cx.emit(SessionEvent::Saved { id });
                                }
                            }
                            None => window.push_notification(
                                Notification::error("The engine did not return the saved request."),
                                cx,
                            ),
                        }
                    } else {
                        let mut saved = this.entity.clone();
                        saved.insert("name".into(), json!(name));
                        saved.insert("url".into(), json!(fields.url));
                        saved.insert("headers".into(), Value::Object(header_map(&fields.headers)));
                        this.adopt_entity(saved, window, cx);
                    }
                    cx.emit(SessionEvent::Changed);
                }
                Err(error) => window
                    .push_notification(Notification::error(describe(&error)).title("Couldn't save the request"), cx),
            }
            cx.notify();
        });
    }

    pub fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.load != LoadState::Ready || self.connection.read(cx).state().is_live() {
            return;
        }
        let fields = self.fields(cx);
        if fields.url.is_empty() {
            window
                .push_notification(Notification::warning("Enter a URL to connect.").title("Nothing to connect to"), cx);
            return;
        }
        self.socket_id = self.saved_id.clone().unwrap_or_else(|| self.socket_id.clone());
        let payload = match (&self.saved_id, fields == self.baseline) {
            (Some(_), true) => json!({ "workspaceId": self.workspace_id, "socketId": self.socket_id }),
            _ => json!({
                "workspaceId": self.workspace_id,
                "socketId": self.socket_id,
                "url": fields.url,
                "headers": header_map(&fields.headers),
            }),
        };
        let client = self.model.read(cx).client().clone();
        call_in(&client, window, cx, "ws.connect", payload, move |_this, outcome, window, cx| {
            let failure = match outcome {
                Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(false) => Some(
                    result
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("The connection could not be opened.")
                        .to_owned(),
                ),
                Ok(_) => None,
                Err(error) => Some(describe(&error)),
            };
            if let Some(message) = failure {
                window.push_notification(Notification::error(message).title("Couldn't connect"), cx);
            }
        });
    }

    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "socketId": self.socket_id });
        call(&client, cx, "ws.disconnect", payload, |_, outcome, cx| {
            if let Err(error) = outcome {
                cx.emit(SessionEvent::Notice(format!("Couldn't disconnect. {}", describe(&error))));
            }
        });
    }

    fn send_message(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "socketId": self.socket_id, "data": text });
        call_in(&client, window, cx, "ws.send", payload, |this, outcome, window, cx| {
            let failure = match outcome {
                Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(false) => {
                    Some(result.get("error").and_then(Value::as_str).unwrap_or("The message was not sent.").to_owned())
                }
                Ok(_) => None,
                Err(error) => Some(describe(&error)),
            };
            match failure {
                Some(message) => {
                    window.push_notification(Notification::error(message).title("Couldn't send the message"), cx)
                }
                None => this.connection.update(cx, |c, cx| c.clear_composer(window, cx)),
            }
        });
    }

    fn clear_messages(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "socketId": self.socket_id });
        self.connection.update(cx, |c, cx| c.clear_messages(cx));
        call(&client, cx, "ws.clearMessages", payload, |_, _, _| {});
    }
}

impl Render for WsSession {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        match &self.load {
            LoadState::Loading => return page::loading("Loading request…"),
            LoadState::Failed(message) => {
                return div()
                    .size_full()
                    .p_4()
                    .bg(theme.background)
                    .child(
                        Alert::error("load-failed", SharedString::from(message.clone()))
                            .title("Couldn't open the request"),
                    )
                    .into_any_element();
            }
            LoadState::Ready => {}
        }

        let dirty = self.is_dirty(cx);
        let state = self.connection.read(cx).state();
        v_flex()
            .size_full()
            .bg(theme.background)
            .gap_2()
            .p_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.name).small().appearance(false)))
                    .child(
                        Button::new("revert-socket")
                            .ghost()
                            .label("Revert")
                            .disabled(!dirty)
                            .on_click(cx.listener(|this, _, window, cx| this.revert(window, cx))),
                    )
                    .child(
                        Button::new("save-socket")
                            .label("Save")
                            .loading(self.saving)
                            .disabled(!dirty && self.saved_id.is_some())
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(self.url.line_field(cx).disabled(state.is_live())))
                    .child(if state.is_live() {
                        Button::new("disconnect-socket")
                            .label("Disconnect")
                            .on_click(cx.listener(|this, _, _, cx| this.disconnect(cx)))
                    } else {
                        Button::new("connect-socket")
                            .primary()
                            .label("Connect")
                            .on_click(cx.listener(|this, _, window, cx| this.connect(window, cx)))
                    }),
            )
            .child(page::section_label("Headers", cx))
            .child(div().flex_1().min_h_0().child(self.headers.clone()))
            .into_any_element()
    }
}
