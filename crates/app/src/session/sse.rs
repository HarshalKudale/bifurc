//! An open SSE request. Like WebSocket it is an ordinary request type: a name, a saved entity in the engine, a
//! place in the explorer tree, and the same save model as the REST editor (a draft is created by Save, a saved
//! request is written a moment after each edit). What differs is the verb — Connect instead of Send — and that
//! the "response" is a live stream owned by the engine ([`SseStream`]), which is why closing this tab does not
//! close the stream. The name is edited in the window's breadcrumb, which shows this session's name field.
//!
//! Connecting a saved, unedited request sends only its id (`sse.connect {workspaceId, streamId}`); a draft or
//! an edited request sends the target it holds. The engine resolves environment variables either way.

use gpui_kit::component::{
    ActiveTheme as _, IndexPath, Sizable as _, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    input::{Editor, InputEvent, InputState},
    notification::Notification,
    select::{Select, SelectEvent, SelectState},
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div,
};
use serde_json::{Map, Value, json};

use super::{
    Carry, LoadState, SaveMode, SessionEvent, engine_folder,
    sse_stream::{SseStream, StreamEvent},
};
use crate::{
    i18n::{t, t_with},
    app_model::{AppModel, ModelEvent},
    components::{Autosave, KeyValueEvent, KeyValueTable, Syntax, VarEditor, page, request_bar},
    engine::{call, call_in, describe},
    model::{
        RequestKind,
        http::{HTTP_METHODS, header_map, header_pairs},
        stream::StreamView,
    },
};

const DEFAULT_NAME: &str = "New SSE request";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Headers,
    Body,
}

impl Section {
    const ALL: [Section; 2] = [Section::Headers, Section::Body];

    fn label(self) -> &'static str {
        match self {
            Section::Headers => t("session.rest.tab_headers"),
            Section::Body => t("session.rest.tab_body"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Fields {
    name: String,
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: String,
}

pub struct SseSession {
    model: Entity<AppModel>,
    workspace_id: String,
    saved_id: Option<String>,
    folder_id: Option<String>,
    /// The folder the engine has it in. It differs from `folder_id` while a move is waiting to be written.
    saved_folder: Option<String>,
    /// The key the engine knows this stream by: the saved id, or a name made up for a draft.
    stream_id: String,
    entity: Map<String, Value>,
    load: LoadState,
    name: Entity<InputState>,
    method: Entity<SelectState<Vec<&'static str>>>,
    url: VarEditor,
    headers: Entity<KeyValueTable>,
    body: VarEditor,
    section: Section,
    baseline: Fields,
    stream: Entity<SseStream>,
    saving: bool,
    autosave: Autosave,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SessionEvent> for SseSession {}

impl SseSession {
    fn build(
        model: Entity<AppModel>,
        saved_id: Option<String>,
        folder_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let workspace_id = model.read(cx).workspace_id().to_owned();
        let stream_id = saved_id.clone().unwrap_or_else(|| format!("draft-{}", cx.entity_id().as_u64()));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t("session.rest.name_placeholder")));
        let method = cx.new(|cx| SelectState::new(HTTP_METHODS.to_vec(), Some(IndexPath::default()), window, cx));
        let url = VarEditor::line("https://example.test/events", window, cx);
        let headers = cx.new(|cx| KeyValueTable::new(t("session.rest.column_header"), t("session.rest.column_value"), window, cx));
        let body = VarEditor::code(Syntax::Json, t("session.sse.body_placeholder"), window, cx);
        let stream = cx.new(|cx| SseStream::new(window, cx));

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.changed(cx),
            InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit_name(window, cx),
            InputEvent::Focus => {}
        }));
        subscriptions
            .push(cx.subscribe(&method, |this, _, _: &SelectEvent<Vec<&'static str>>, cx| this.changed(cx)));
        subscriptions.push(cx.subscribe_in(
            url.state(),
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.changed(cx),
                InputEvent::PressEnter { .. } => this.connect(window, cx),
                _ => {}
            },
        ));
        subscriptions.push(cx.subscribe(body.state(), |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.changed(cx);
            }
        }));
        subscriptions.push(cx.subscribe(&headers, |this, _, _: &KeyValueEvent, cx| this.changed(cx)));
        subscriptions.push(cx.subscribe(&stream, |this, _, event: &StreamEvent, cx| match event {
            StreamEvent::Clear => this.clear_messages(cx),
        }));
        subscriptions.push(cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| match event {
            ModelEvent::SseStatus(payload) if this.is_mine(payload) => {
                this.stream.update(cx, |s, cx| s.apply_status_event(payload, cx));
                cx.emit(SessionEvent::Changed);
            }
            ModelEvent::SseMessage(payload) if this.is_mine(payload) => {
                this.stream.update(cx, |s, cx| s.apply_message_event(payload, cx));
            }
            ModelEvent::EntityReverted(id) if this.saved_id.as_deref() == Some(id.as_str()) => {
                this.reload(window, cx);
            }
            _ => {}
        }));

        let baseline = Fields {
            name: DEFAULT_NAME.to_owned(),
            method: "GET".to_owned(),
            url: String::new(),
            headers: Vec::new(),
            body: String::new(),
        };
        let mut this = Self {
            model,
            workspace_id,
            saved_id,
            saved_folder: folder_id.clone(),
            folder_id,
            stream_id,
            entity: Map::new(),
            load: LoadState::Ready,
            name,
            method,
            url,
            headers,
            body,
            section: Section::Headers,
            baseline: baseline.clone(),
            stream,
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
        let payload = json!({ "workspaceId": self.workspace_id, "kind": RequestKind::Sse.entity_kind(), "id": id });
        call_in(&client, window, cx, "entity.load", payload, |this, outcome, window, cx| {
            // The stream's status is asked for once, when the tab opens; a re-read does not change it.
            let opening = this.load == LoadState::Loading;
            match outcome.map_err(|e| describe(&e)).and_then(|v| {
                v.get("entity")
                    .and_then(Value::as_object)
                    .cloned()
                    .ok_or_else(|| t("session.rest.gone").to_owned())
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

    /// The engine keeps the stream (and its history) across tabs and windows. Asking for its status when the
    /// tab opens is what makes a live stream still live, with its events, after the tab was closed.
    fn attach(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "streamId": self.stream_id });
        call(&client, cx, "sse.status", payload, |this, outcome, cx| {
            if let Ok(status) = outcome {
                let view = StreamView::from_status(&status);
                this.stream.update(cx, |s, cx| s.replace_view(view, cx));
                cx.emit(SessionEvent::Changed);
            }
        });
    }

    fn is_mine(&self, payload: &Value) -> bool {
        payload.get("streamId").and_then(Value::as_str) == Some(self.stream_id.as_str())
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

    /// The address field, for the key that jumps to it.
    pub fn url_editor(&self) -> &VarEditor {
        &self.url
    }

    pub fn is_saving(&self) -> bool {
        self.saving
    }

    pub fn stream(&self) -> &Entity<SseStream> {
        &self.stream
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
        (fields != self.baseline && fields.url.is_empty()).then(|| t("session.rest.problem_no_url").into())
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
            body: fields.body,
        }
    }

    /// Fills this draft with what was typed into a draft of another type.
    pub fn apply_carry(&mut self, carry: &Carry, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(name) = &carry.name {
            self.name.update(cx, |s, cx| s.set_value(name.clone(), window, cx));
        }
        self.url.set_value(carry.url.clone(), window, cx);
        self.headers.update(cx, |table, cx| table.set_pairs(&carry.headers, window, cx));
        self.body.set_value(carry.body.clone(), window, cx);
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

    fn method(&self, cx: &gpui_kit::App) -> &'static str {
        self.method.read(cx).selected_value().copied().unwrap_or("GET")
    }

    /// The name is trimmed here and in [`fields_of`](Self::fields_of), so that a stray space is not an edit.
    fn fields(&self, cx: &gpui_kit::App) -> Fields {
        Fields {
            name: self.name.read(cx).value().trim().to_owned(),
            method: self.method(cx).to_owned(),
            url: self.url.line_value(cx),
            headers: self.headers.read(cx).pairs(cx),
            body: self.body.value(cx),
        }
    }

    fn fields_of(entity: &Map<String, Value>) -> Fields {
        let text = |key: &str| entity.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        let method = text("method").to_uppercase();
        let method = HTTP_METHODS.iter().find(|m| **m == method).copied().unwrap_or("GET");
        Fields {
            name: text("name").trim().to_owned(),
            method: method.to_owned(),
            url: text("url").trim().to_owned(),
            headers: header_pairs(entity.get("headers").unwrap_or(&Value::Null)),
            body: text("body"),
        }
    }

    fn apply_fields(&mut self, fields: &Fields, window: &mut Window, cx: &mut Context<Self>) {
        let method = HTTP_METHODS.iter().find(|m| **m == fields.method).copied().unwrap_or("GET");
        self.name.update(cx, |s, cx| s.set_value(fields.name.clone(), window, cx));
        self.method.update(cx, |s, cx| s.set_selected_value(&method, window, cx));
        self.url.set_value(fields.url.clone(), window, cx);
        self.headers.update(cx, |table, cx| table.set_pairs(&fields.headers, window, cx));
        self.body.set_value(fields.body.clone(), window, cx);
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
        entity.insert("method".into(), json!(fields.method));
        entity.insert("url".into(), json!(fields.url));
        entity.insert("headers".into(), Value::Object(header_map(&fields.headers)));
        entity.insert("body".into(), json!(fields.body));
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
                window.push_notification(
                    Notification::warning(t("session.rest.save_no_url")).title(t("session.rest.save_no_url_title")),
                    cx,
                );
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
        let kind = RequestKind::Sse.entity_kind();
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
                        // A draft that is already streaming stays under the name it connected with.
                        if !this.stream.read(cx).state().is_live()
                            && let Some(id) = &this.saved_id
                        {
                            this.stream_id = id.clone();
                        }
                        if let Some(id) = this.saved_id.clone() {
                            cx.emit(SessionEvent::Saved { id });
                        }
                    }
                    None => window
                        .push_notification(Notification::error(t("session.rest.engine_no_saved")), cx),
                },
                Ok(_) => {
                    this.entity = this.edited_entity(&fields);
                    this.baseline = fields;
                    this.saved_folder = folder.clone();
                }
                Err(error) => {
                    window.push_notification(
                        Notification::error(describe(&error)).title(t("session.rest.save_failed_title")),
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
        if self.load != LoadState::Ready || self.stream.read(cx).state().is_live() {
            return;
        }
        let fields = self.fields(cx);
        if fields.url.is_empty() {
            window.push_notification(
                Notification::warning(t("session.connection.connect_no_url")).title(t("session.connection.connect_no_url_title")),
                cx,
            );
            return;
        }
        self.stream_id = self.saved_id.clone().unwrap_or_else(|| self.stream_id.clone());
        let payload = match (&self.saved_id, fields == self.baseline) {
            (Some(_), true) => json!({ "workspaceId": self.workspace_id, "streamId": self.stream_id }),
            _ => json!({
                "workspaceId": self.workspace_id,
                "streamId": self.stream_id,
                "url": fields.url,
                "method": fields.method,
                "headers": header_map(&fields.headers),
                "body": fields.body,
            }),
        };
        let client = self.model.read(cx).client().clone();
        call_in(&client, window, cx, "sse.connect", payload, move |_this, outcome, window, cx| {
            let failure = match outcome {
                Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(false) => Some(
                    result
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or(t("session.sse.open_failed"))
                        .to_owned(),
                ),
                Ok(_) => None,
                Err(error) => Some(describe(&error)),
            };
            if let Some(message) = failure {
                window.push_notification(Notification::error(message).title(t("session.connection.connect_failed")), cx);
            }
        });
    }

    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "streamId": self.stream_id });
        call(&client, cx, "sse.disconnect", payload, |_, outcome, cx| {
            if let Err(error) = outcome {
                cx.emit(SessionEvent::Notice(t_with("session.connection.disconnect_failed", &[("error", &describe(&error))])));
            }
        });
    }

    fn clear_messages(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "streamId": self.stream_id });
        self.stream.update(cx, |s, cx| s.clear_messages(cx));
        call(&client, cx, "sse.clearMessages", payload, |_, _, _| {});
    }

    fn render_sections(&self, cx: &mut Context<Self>) -> TabBar {
        TabBar::new("sse-sections")
            .underline()
            .small()
            .selected_index(Section::ALL.iter().position(|s| *s == self.section).unwrap_or(0))
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.section = Section::ALL[*ix];
                cx.notify();
            }))
            .children(Section::ALL.map(|section| Tab::new().label(section.label())))
    }
}

impl Render for SseSession {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        match &self.load {
            LoadState::Loading => return page::loading(t("session.rest.loading")),
            LoadState::Failed(message) => {
                return div()
                    .size_full()
                    .p_4()
                    .bg(theme.background)
                    .child(
                        Alert::error("load-failed", SharedString::from(message.clone()))
                            .title(t("session.rest.open_failed")),
                    )
                    .into_any_element();
            }
            LoadState::Ready => {}
        }

        let live = self.stream.read(cx).state().is_live();
        let row_height = VarEditor::row_height(cx);
        let content = match self.section {
            Section::Headers => self.headers.clone().into_any_element(),
            Section::Body => Editor::new(self.body.state()).bordered(false).h_full().into_any_element(),
        };
        v_flex()
            .size_full()
            .bg(theme.background)
            .child(request_bar(
                Some(
                    div()
                        .w_32()
                        .flex_none()
                        .child(Select::new(&self.method).h(row_height).disabled(live))
                        .into_any_element(),
                ),
                self.url.line_field(cx).disabled(live),
                if live {
                    Button::new("disconnect-stream")
                        .label(t("session.connection.disconnect"))
                        .on_click(cx.listener(|this, _, _, cx| this.disconnect(cx)))
                } else {
                    Button::new("connect-stream")
                        .primary()
                        .label(t("session.connection.connect"))
                        .on_click(cx.listener(|this, _, window, cx| this.connect(window, cx)))
                },
                [],
                cx,
            ))
            .child(div().flex_none().px_3().child(self.render_sections(cx)))
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}
