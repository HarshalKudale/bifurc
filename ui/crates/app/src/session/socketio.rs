//! An open Socket.IO request. Like WebSocket it is an ordinary request type: a name, a saved entity in the
//! engine, a place in the explorer tree, and the same save model as the REST editor (a draft is created by
//! Save, a saved request is written a moment after each edit). What differs is the verb — Connect instead of
//! Send — and that the "response" is a live connection owned by the engine ([`SocketIoConnection`]), which is
//! why closing this tab does not close it. The name is edited in the window's breadcrumb, which shows this
//! session's name field.
//!
//! Besides the address and the headers a request holds what a Socket.IO client is given: the namespace to
//! join, the path the server listens on and the `auth` payload sent with the connection.
//!
//! Connecting a saved, unedited request sends only its id (`socketio.connect {workspaceId, connectionId}`); a
//! draft or an edited request sends the target it holds. The engine resolves environment variables either way.

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, InputEvent, InputState},
    notification::Notification,
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div,
};
use bifurc_engine_client::ClientError;
use serde_json::{Map, Value, json};

use super::{
    Carry, LoadState, SaveMode, SessionEvent, engine_folder,
    socketio_connection::{SocketIoConnection, SocketIoEvent},
};
use crate::{
    app_model::{AppModel, ModelEvent},
    components::{Autosave, KeyValueEvent, KeyValueTable, Syntax, VarEditor, page},
    engine::{call, call_in, describe},
    model::{
        RequestKind,
        http::{header_map, header_pairs},
        socketio::SocketIoView,
    },
};

const DEFAULT_NAME: &str = "New Socket.IO request";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Headers,
    Auth,
    Settings,
}

impl Section {
    const ALL: [Section; 3] = [Section::Headers, Section::Auth, Section::Settings];

    fn label(self) -> &'static str {
        match self {
            Section::Headers => "Headers",
            Section::Auth => "Auth",
            Section::Settings => "Settings",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Fields {
    name: String,
    url: String,
    headers: Vec<(String, String)>,
    namespace: String,
    path: String,
    auth: String,
}

pub struct SocketIoSession {
    model: Entity<AppModel>,
    workspace_id: String,
    saved_id: Option<String>,
    folder_id: Option<String>,
    /// The folder the engine has it in. It differs from `folder_id` while a move is waiting to be written.
    saved_folder: Option<String>,
    /// The key the engine knows this connection by: the saved id, or a name made up for a draft.
    connection_id: String,
    entity: Map<String, Value>,
    load: LoadState,
    name: Entity<InputState>,
    url: VarEditor,
    headers: Entity<KeyValueTable>,
    namespace: VarEditor,
    path: VarEditor,
    auth: VarEditor,
    section: Section,
    baseline: Fields,
    connection: Entity<SocketIoConnection>,
    saving: bool,
    autosave: Autosave,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SessionEvent> for SocketIoSession {}

impl SocketIoSession {
    fn build(
        model: Entity<AppModel>,
        saved_id: Option<String>,
        folder_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let workspace_id = model.read(cx).workspace_id().to_owned();
        let connection_id = saved_id.clone().unwrap_or_else(|| format!("draft-{}", cx.entity_id().as_u64()));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Request name"));
        let url = VarEditor::line("https://example.test", window, cx);
        let headers = cx.new(|cx| KeyValueTable::new("Header", "Value", window, cx));
        let namespace = VarEditor::line("/  (the main namespace)", window, cx);
        let path = VarEditor::line("/socket.io", window, cx);
        let auth = VarEditor::code(Syntax::Json, "{ \"token\": \"…\" }  (sent when the connection opens)", window, cx);
        let connection = cx.new(|cx| SocketIoConnection::new(window, cx));

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.changed(cx),
            InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit_name(window, cx),
            InputEvent::Focus => {}
        }));
        // Enter in any of the one-row fields connects, as it does in the address.
        for field in [&url, &namespace, &path] {
            subscriptions.push(cx.subscribe_in(
                field.state(),
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => this.changed(cx),
                    InputEvent::PressEnter { .. } => this.connect(window, cx),
                    _ => {}
                },
            ));
        }
        subscriptions.push(cx.subscribe(auth.state(), |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.changed(cx);
            }
        }));
        subscriptions.push(cx.subscribe(&headers, |this, _, _: &KeyValueEvent, cx| this.changed(cx)));
        subscriptions.push(cx.subscribe_in(&connection, window, |this, _, event: &SocketIoEvent, window, cx| {
            match event {
                SocketIoEvent::Emit { event, args, ack } => this.emit(event.clone(), args.clone(), *ack, window, cx),
                SocketIoEvent::Clear => this.clear_messages(cx),
            }
        }));
        subscriptions.push(cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| match event {
            ModelEvent::SocketIoStatus(payload) if this.is_mine(payload) => {
                this.connection.update(cx, |c, cx| c.apply_status_event(payload, cx));
                cx.emit(SessionEvent::Changed);
                // The address and the Connect button follow the connection's state.
                cx.notify();
            }
            ModelEvent::SocketIoMessage(payload) if this.is_mine(payload) => {
                this.connection.update(cx, |c, cx| c.apply_message_event(payload, cx));
            }
            ModelEvent::EntityReverted(id) if this.saved_id.as_deref() == Some(id.as_str()) => {
                this.reload(window, cx);
            }
            _ => {}
        }));

        let baseline = Fields {
            name: DEFAULT_NAME.to_owned(),
            url: String::new(),
            headers: Vec::new(),
            namespace: String::new(),
            path: String::new(),
            auth: String::new(),
        };
        let mut this = Self {
            model,
            workspace_id,
            saved_id,
            saved_folder: folder_id.clone(),
            folder_id,
            connection_id,
            entity: Map::new(),
            load: LoadState::Ready,
            name,
            url,
            headers,
            namespace,
            path,
            auth,
            section: Section::Headers,
            baseline: baseline.clone(),
            connection,
            saving: false,
            autosave: Autosave::new(window),
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
        let mut this = Self::build(model, Some(id), None, window, cx);
        this.load = LoadState::Loading;
        this.reload(window, cx);
        this
    }

    /// Reads the saved request from the engine into the editor, replacing what the editor holds.
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.saved_id.clone() else { return };
        // An edit waiting to be written belongs to the copy that is being replaced.
        self.autosave.cancel();
        let client = self.model.read(cx).client().clone();
        let payload =
            json!({ "workspaceId": self.workspace_id, "kind": RequestKind::SocketIo.entity_kind(), "id": id });
        call_in(&client, window, cx, "entity.load", payload, |this, outcome, window, cx| {
            // The connection's status is asked for once, when the tab opens; a re-read does not change it.
            let opening = this.load == LoadState::Loading;
            match outcome.map_err(|e| describe(&e)).and_then(|v| {
                v.get("entity")
                    .and_then(Value::as_object)
                    .cloned()
                    .ok_or_else(|| "The request no longer exists.".to_owned())
            }) {
                Ok(entity) => {
                    this.adopt_entity(entity, window, cx);
                    this.load = LoadState::Ready;
                    if opening {
                        this.attach(cx);
                    }
                }
                Err(message) => this.load = LoadState::Failed(message),
            }
            cx.emit(SessionEvent::Changed);
            cx.notify();
        });
    }

    /// The engine keeps the connection (and its history) across tabs and windows. Asking for its status when
    /// the tab opens is what makes a live connection still live, with its messages, after the tab was closed.
    fn attach(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "connectionId": self.connection_id });
        call(&client, cx, "socketio.status", payload, |this, outcome, cx| {
            if let Ok(status) = outcome {
                let view = SocketIoView::from_status(&status);
                this.connection.update(cx, |c, cx| c.replace_view(view, cx));
                cx.emit(SessionEvent::Changed);
                cx.notify();
            }
        });
    }

    fn is_mine(&self, payload: &Value) -> bool {
        payload.get("connectionId").and_then(Value::as_str) == Some(self.connection_id.as_str())
            && payload.get("workspaceId").and_then(Value::as_str) == Some(self.workspace_id.as_str())
    }

    pub fn saved_id(&self) -> Option<&str> {
        self.saved_id.as_deref()
    }

    pub fn folder_id(&self) -> Option<&str> {
        self.folder_id.as_deref()
    }

    /// The field the request's name is typed in. The shell shows it in the breadcrumb.
    pub fn name_input(&self) -> &Entity<InputState> {
        &self.name
    }

    pub fn is_saving(&self) -> bool {
        self.saving
    }

    pub fn connection(&self) -> &Entity<SocketIoConnection> {
        &self.connection
    }

    pub fn title(&self, cx: &gpui_kit::App) -> SharedString {
        let name = self.name.read(cx).value();
        if name.is_empty() { DEFAULT_NAME.into() } else { name }
    }

    pub fn is_dirty(&self, cx: &gpui_kit::App) -> bool {
        self.load == LoadState::Ready && (self.fields(cx) != self.baseline || self.folder_id != self.saved_folder)
    }

    /// Why the edit cannot be written as it stands, for a saved request that has one.
    pub fn problem(&self, cx: &gpui_kit::App) -> Option<SharedString> {
        if self.saved_id.is_none() || self.load != LoadState::Ready {
            return None;
        }
        let fields = self.fields(cx);
        (fields != self.baseline && fields.url.is_empty()).then(|| "Not saved. Enter a URL.".into())
    }

    /// A saved request is written a moment after each change; a draft waits for Save.
    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(SessionEvent::Changed);
        cx.notify();
        if self.saved_id.is_some() && self.load == LoadState::Ready {
            self.autosave.schedule(cx, |this, window, cx| this.save(SaveMode::Auto, window, cx));
        }
    }

    /// The name was confirmed, by Enter or by leaving the field. Naming a draft is what creates it, when it
    /// has what it needs to be created.
    fn commit_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saved_id.is_none() && self.fields(cx).name != self.baseline.name {
            self.save(SaveMode::Auto, window, cx);
        }
    }

    /// Puts it in another folder, or at the top level with `None`. A saved one is moved by its next write;
    /// a draft is created there.
    pub fn set_folder(&mut self, folder: Option<String>, cx: &mut Context<Self>) {
        if self.folder_id != folder {
            self.folder_id = folder;
            self.changed(cx);
        }
    }

    /// Writes an edit that is still waiting, now: the tab is about to close.
    pub fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saved_id.is_some() {
            self.autosave.cancel();
            self.save(SaveMode::Auto, window, cx);
        }
    }

    /// What this draft keeps if it becomes a request of another type.
    pub fn carry(&self, cx: &gpui_kit::App) -> Carry {
        let fields = self.fields(cx);
        Carry {
            name: (!fields.name.is_empty() && fields.name != DEFAULT_NAME).then_some(fields.name),
            url: fields.url,
            headers: fields.headers,
            body: String::new(),
        }
    }

    /// Fills this draft with what was typed into a draft of another type.
    pub fn apply_carry(&mut self, carry: &Carry, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(name) = &carry.name {
            self.name.update(cx, |s, cx| s.set_value(name.clone(), window, cx));
        }
        self.url.set_value(carry.url.clone(), window, cx);
        self.headers.update(cx, |table, cx| table.set_pairs(&carry.headers, window, cx));
        self.changed(cx);
    }

    /// Fills this draft with a request the engine built, from a cURL command. It stays a draft, in the
    /// folder it was started in, and what was filled in is its first edit.
    pub fn adopt_import(&mut self, entity: Map<String, Value>, window: &mut Window, cx: &mut Context<Self>) {
        let (folder, baseline) = (self.folder_id.clone(), self.baseline.clone());
        self.adopt_entity(entity, window, cx);
        (self.folder_id, self.saved_folder) = (folder.clone(), folder);
        self.baseline = baseline;
        self.changed(cx);
    }

    /// The name is trimmed here and in [`fields_of`](Self::fields_of), so that a stray space is not an edit.
    fn fields(&self, cx: &gpui_kit::App) -> Fields {
        Fields {
            name: self.name.read(cx).value().trim().to_owned(),
            url: self.url.line_value(cx),
            headers: self.headers.read(cx).pairs(cx),
            namespace: self.namespace.line_value(cx),
            path: self.path.line_value(cx),
            auth: self.auth.value(cx),
        }
    }

    fn fields_of(entity: &Map<String, Value>) -> Fields {
        let text = |key: &str| entity.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        Fields {
            name: text("name").trim().to_owned(),
            url: text("url").trim().to_owned(),
            headers: header_pairs(entity.get("headers").unwrap_or(&Value::Null)),
            namespace: text("namespace").trim().to_owned(),
            path: text("path").trim().to_owned(),
            auth: text("auth"),
        }
    }

    fn apply_fields(&mut self, fields: &Fields, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |s, cx| s.set_value(fields.name.clone(), window, cx));
        self.url.set_value(fields.url.clone(), window, cx);
        self.headers.update(cx, |table, cx| table.set_pairs(&fields.headers, window, cx));
        self.namespace.set_value(fields.namespace.clone(), window, cx);
        self.path.set_value(fields.path.clone(), window, cx);
        self.auth.set_value(fields.auth.clone(), window, cx);
    }

    fn adopt_entity(&mut self, entity: Map<String, Value>, window: &mut Window, cx: &mut Context<Self>) {
        let fields = Self::fields_of(&entity);
        self.folder_id = entity.get("folderId").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
        self.saved_folder = self.folder_id.clone();
        self.apply_fields(&fields, window, cx);
        self.baseline = fields;
        self.entity = entity;
    }

    /// The loaded entity with the editor's fields written over it.
    fn edited_entity(&self, fields: &Fields) -> Map<String, Value> {
        let mut entity = self.entity.clone();
        entity.insert("name".into(), json!(if fields.name.is_empty() { DEFAULT_NAME } else { fields.name.as_str() }));
        entity.insert("url".into(), json!(fields.url));
        entity.insert("headers".into(), Value::Object(header_map(&fields.headers)));
        entity.insert("namespace".into(), json!(fields.namespace));
        entity.insert("path".into(), json!(fields.path));
        entity.insert("auth".into(), json!(fields.auth));
        entity
    }

    /// Creates the request if it is a draft, or writes the edit over the saved one. A write that is asked
    /// for while another is in flight is not lost: the one in flight looks for a newer edit when it lands.
    pub fn save(&mut self, mode: SaveMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || self.load != LoadState::Ready {
            return;
        }
        let fields = self.fields(cx);
        let creating = self.saved_id.is_none();
        if !creating && fields == self.baseline && self.folder_id == self.saved_folder {
            return;
        }
        if fields.url.is_empty() {
            if mode == SaveMode::Explicit {
                window
                    .push_notification(Notification::warning("Enter a URL before saving.").title("Can't save yet"), cx);
            }
            return;
        }
        let mut entity = self.edited_entity(&fields);
        entity.insert("workspaceId".into(), json!(self.workspace_id));
        entity.insert("folderId".into(), json!(self.folder_id));
        let folder = self.folder_id.clone();

        self.saving = true;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        let kind = RequestKind::SocketIo.entity_kind();
        let (action, payload) = match &self.saved_id {
            Some(id) => {
                entity.insert("id".into(), json!(id));
                ("entity.update", json!({ "kind": kind, "entity": entity }))
            }
            None => ("entity.create", json!({ "kind": kind, "workspaceId": self.workspace_id, "entity": entity })),
        };
        call_in(&client, window, cx, action, payload, move |this, outcome, window, cx| {
            this.saving = false;
            // The controls are left alone: they may hold what was typed while this was in flight. What was
            // sent becomes the baseline, so that newer text still reads as an edit.
            match outcome {
                Ok(result) if creating => match result.get("entity").and_then(Value::as_object).cloned() {
                    Some(saved) => {
                        this.saved_id = saved.get("id").and_then(Value::as_str).map(str::to_owned);
                        let placed = engine_folder(&saved);
                        this.entity = saved;
                        this.baseline = fields;
                        if this.folder_id == folder {
                            this.folder_id = placed.clone();
                        }
                        this.saved_folder = placed;
                        // A draft that is already connected stays under the name it connected with.
                        if !this.connection.read(cx).state().is_live()
                            && let Some(id) = &this.saved_id
                        {
                            this.connection_id = id.clone();
                        }
                        if let Some(id) = this.saved_id.clone() {
                            cx.emit(SessionEvent::Saved { id });
                        }
                    }
                    None => window
                        .push_notification(Notification::error("The engine did not return the saved request."), cx),
                },
                Ok(_) => {
                    this.entity = this.edited_entity(&fields);
                    this.baseline = fields;
                    this.saved_folder = folder.clone();
                }
                Err(error) => {
                    window.push_notification(
                        Notification::error(describe(&error)).title("Couldn't save the request"),
                        cx,
                    );
                    cx.emit(SessionEvent::Changed);
                    cx.notify();
                    return;
                }
            }
            // An edit made since this write was sent is the next write. (`changed` schedules it.)
            this.changed(cx);
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
        self.connection_id = self.saved_id.clone().unwrap_or_else(|| self.connection_id.clone());
        let payload = match (&self.saved_id, fields == self.baseline) {
            (Some(_), true) => json!({ "workspaceId": self.workspace_id, "connectionId": self.connection_id }),
            _ => json!({
                "workspaceId": self.workspace_id,
                "connectionId": self.connection_id,
                "url": fields.url,
                "headers": header_map(&fields.headers),
                "namespace": fields.namespace,
                "path": fields.path,
                "auth": fields.auth,
            }),
        };
        let client = self.model.read(cx).client().clone();
        call_in(&client, window, cx, "socketio.connect", payload, move |_this, outcome, window, cx| {
            if let Some(message) = refusal(outcome, "The connection could not be opened.") {
                window.push_notification(Notification::error(message).title("Couldn't connect"), cx);
            }
        });
    }

    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "connectionId": self.connection_id });
        call(&client, cx, "socketio.disconnect", payload, |_, outcome, cx| {
            if let Err(error) = outcome {
                cx.emit(SessionEvent::Notice(format!("Couldn't disconnect. {}", describe(&error))));
            }
        });
    }

    /// Emits an event over the open connection. What was composed is kept: an event is often emitted again.
    fn emit(&mut self, event: String, args: String, ack: bool, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        let payload = json!({
            "workspaceId": self.workspace_id, "connectionId": self.connection_id, "event": event, "args": args,
            "ack": ack,
        });
        call_in(&client, window, cx, "socketio.emit", payload, |_this, outcome, window, cx| {
            if let Some(message) = refusal(outcome, "The event was not emitted.") {
                window.push_notification(Notification::error(message).title("Couldn't emit the event"), cx);
            }
        });
    }

    fn clear_messages(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "connectionId": self.connection_id });
        self.connection.update(cx, |c, cx| c.clear_messages(cx));
        call(&client, cx, "socketio.clearMessages", payload, |_, _, _| {});
    }

    fn render_sections(&self, cx: &mut Context<Self>) -> TabBar {
        TabBar::new("socketio-sections")
            .segmented()
            .small()
            .selected_index(Section::ALL.iter().position(|s| *s == self.section).unwrap_or(0))
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.section = Section::ALL[*ix];
                cx.notify();
            }))
            .children(Section::ALL.map(|section| Tab::new().label(section.label())))
    }

    /// The namespace and the server's path: one labelled one-row field each.
    fn render_settings(&self, live: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let field = |label: &'static str, hint: &'static str, editor: &VarEditor, cx: &mut Context<Self>| {
            v_flex()
                .gap_1()
                .child(page::section_label(label, cx))
                .child(editor.line_field(cx).disabled(live))
                .child(div().text_xs().text_color(theme.muted_foreground).child(hint))
        };
        v_flex()
            .p_3()
            .gap_4()
            .child(field(
                "Namespace",
                "Left empty, the path of the URL names it (https://host/admin joins /admin); otherwise the main namespace.",
                &self.namespace,
                cx,
            ))
            .child(field(
                "Path",
                "Where the server listens. Left empty, /socket.io.",
                &self.path,
                cx,
            ))
            .into_any_element()
    }
}

/// Why a command the engine answers with `{ ok, error }` did not do what was asked, if it did not.
fn refusal(outcome: Result<Value, ClientError>, fallback: &str) -> Option<String> {
    match outcome {
        Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(false) => {
            Some(result.get("error").and_then(Value::as_str).unwrap_or(fallback).to_owned())
        }
        Ok(_) => None,
        Err(error) => Some(describe(&error)),
    }
}

impl Render for SocketIoSession {
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

        let live = self.connection.read(cx).state().is_live();
        let row_height = VarEditor::row_height(cx);
        let content = match self.section {
            Section::Headers => self.headers.clone().into_any_element(),
            Section::Auth => Editor::new(self.auth.state()).bordered(false).h_full().into_any_element(),
            Section::Settings => self.render_settings(live, cx),
        };
        v_flex()
            .size_full()
            .bg(theme.background)
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .child(div().flex_1().min_w_0().child(self.url.line_field(cx).disabled(live)))
                    .child(if live {
                        Button::new("disconnect-socketio")
                            .h(row_height)
                            .label("Disconnect")
                            .on_click(cx.listener(|this, _, _, cx| this.disconnect(cx)))
                    } else {
                        Button::new("connect-socketio")
                            .primary()
                            .h(row_height)
                            .label("Connect")
                            .on_click(cx.listener(|this, _, window, cx| this.connect(window, cx)))
                    }),
            )
            .child(div().flex_none().px_3().child(self.render_sections(cx)))
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}
