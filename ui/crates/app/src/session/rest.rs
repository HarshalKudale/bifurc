//! An open REST request: the editor, its save/send commands and its response.
//!
//! The saved request lives in the engine; this entity holds an *edit* of it. Fields are read from the
//! controls when needed (the controls are the single source of truth for what is typed), compared to a
//! baseline taken at the last load or save to know if there is anything to save, and written back as one
//! `entity.update`. Fields the editor does not show — scripts, ids, timestamps — are carried through
//! unchanged from the entity the engine returned, so saving here cannot erase what another client set.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IndexPath, Sizable as _, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    notification::Notification,
    select::{Select, SelectEvent, SelectState},
    spinner::Spinner,
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div,
};
use serde_json::{Map, Value, json};

use super::{LoadState, SessionEvent, exchange::Exchange};
use crate::{
    app_model::AppModel,
    engine::{call_in, describe},
    model::{
        RequestKind,
        http::{BodyEncoding, HTTP_METHODS, format_headers, parse_headers},
    },
};

const DEFAULT_NAME: &str = "New request";

/// What the editor shows, as plain values, normalised so that trivially different text (trailing
/// whitespace on a header line) does not count as an edit.
#[derive(Debug, Clone, PartialEq)]
struct Fields {
    name: String,
    method: String,
    url: String,
    headers: String,
    body: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Headers,
    Body,
}

pub struct RestSession {
    model: Entity<AppModel>,
    workspace_id: String,
    saved_id: Option<String>,
    folder_id: Option<String>,
    /// The entity as the engine last returned it; the base that edited fields are written over.
    entity: Map<String, Value>,
    load: LoadState,
    name: Entity<InputState>,
    method: Entity<SelectState<Vec<&'static str>>>,
    url: Entity<InputState>,
    headers: Entity<TextareaState>,
    body: Entity<TextareaState>,
    section: Section,
    baseline: Fields,
    exchange: Entity<Exchange>,
    sending: bool,
    saving: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SessionEvent> for RestSession {}

impl RestSession {
    fn build(
        model: Entity<AppModel>,
        saved_id: Option<String>,
        folder_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let workspace_id = model.read(cx).workspace_id().to_owned();
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Request name"));
        let method = cx.new(|cx| SelectState::new(HTTP_METHODS.to_vec(), Some(IndexPath::default()), window, cx));
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("https://api.example.test/resource"));
        let headers = cx.new(|cx| TextareaState::new(window, cx).placeholder("Header-Name: value"));
        let body = cx.new(|cx| TextareaState::new(window, cx).placeholder("Request body"));
        let exchange = cx.new(|cx| Exchange::new(window, cx));

        let mut subscriptions = Vec::new();
        for input in [&name, &url] {
            subscriptions.push(cx.subscribe_in(input, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => cx.emit(SessionEvent::Changed),
                InputEvent::PressEnter { .. } => this.send(window, cx),
                _ => {}
            }));
        }
        for area in [&headers, &body] {
            subscriptions.push(cx.subscribe_in(area, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.emit(SessionEvent::Changed);
                }
            }));
        }
        subscriptions.push(cx.subscribe_in(&method, window, |_, _, _: &SelectEvent<Vec<&'static str>>, _, cx| {
            cx.emit(SessionEvent::Changed);
        }));

        let mut this = Self {
            model,
            workspace_id,
            saved_id,
            folder_id,
            entity: Map::new(),
            load: LoadState::Ready,
            name,
            method,
            url,
            headers,
            body,
            section: Section::Headers,
            baseline: Fields {
                name: DEFAULT_NAME.to_owned(),
                method: "GET".to_owned(),
                url: String::new(),
                headers: String::new(),
                body: String::new(),
            },
            exchange,
            sending: false,
            saving: false,
            _subscriptions: subscriptions,
        };
        this.apply_fields(&this.baseline.clone(), window, cx);
        this
    }

    /// A request that does not exist yet. Saving creates it, in `folder_id` if given.
    pub fn new_draft(
        model: Entity<AppModel>,
        folder_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(model, None, folder_id, window, cx)
    }

    /// A saved request, loaded by id.
    pub fn open_saved(model: Entity<AppModel>, id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::build(model, Some(id.clone()), None, window, cx);
        this.load = LoadState::Loading;
        let client = this.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": this.workspace_id, "kind": RequestKind::Rest.entity_kind(), "id": id });
        call_in(&client, window, cx, "entity.load", payload, |this, outcome, window, cx| {
            match outcome.map_err(|e| describe(&e)).and_then(|v| entity_of(&v)) {
                Ok(entity) => {
                    this.adopt_entity(entity, window, cx);
                    this.load = LoadState::Ready;
                }
                Err(message) => this.load = LoadState::Failed(message),
            }
            cx.emit(SessionEvent::Changed);
            cx.notify();
        });
        this
    }

    pub fn saved_id(&self) -> Option<&str> {
        self.saved_id.as_deref()
    }

    pub fn exchange(&self) -> &Entity<Exchange> {
        &self.exchange
    }

    pub fn title(&self, cx: &gpui_kit::App) -> SharedString {
        let name = self.name.read(cx).value();
        if name.is_empty() { DEFAULT_NAME.into() } else { name }
    }

    pub fn is_dirty(&self, cx: &gpui_kit::App) -> bool {
        self.load == LoadState::Ready && self.fields(cx) != self.baseline
    }

    fn fields(&self, cx: &gpui_kit::App) -> Fields {
        Fields {
            name: self.name.read(cx).value().to_string(),
            method: self.method.read(cx).selected_value().copied().unwrap_or("GET").to_owned(),
            url: self.url.read(cx).value().trim().to_owned(),
            headers: format_headers(&Value::Object(parse_headers(&self.headers.read(cx).value()))),
            body: self.body.read(cx).value().to_string(),
        }
    }

    fn fields_of(entity: &Map<String, Value>) -> Fields {
        let text = |key: &str| entity.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        Fields {
            name: text("name"),
            method: entity.get("method").and_then(Value::as_str).unwrap_or("GET").to_uppercase(),
            url: text("url").trim().to_owned(),
            headers: format_headers(entity.get("headers").unwrap_or(&Value::Null)),
            body: text("body"),
        }
    }

    fn apply_fields(&mut self, fields: &Fields, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |s, cx| s.set_value(fields.name.clone(), window, cx));
        self.url.update(cx, |s, cx| s.set_value(fields.url.clone(), window, cx));
        self.headers.update(cx, |s, cx| s.set_value(fields.headers.clone(), window, cx));
        self.body.update(cx, |s, cx| s.set_value(fields.body.clone(), window, cx));
        let method = HTTP_METHODS.iter().find(|m| **m == fields.method).copied().unwrap_or("GET");
        self.method.update(cx, |s, cx| s.set_selected_value(&method, window, cx));
    }

    fn adopt_entity(&mut self, entity: Map<String, Value>, window: &mut Window, cx: &mut Context<Self>) {
        let fields = Self::fields_of(&entity);
        self.folder_id = entity.get("folderId").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
        self.apply_fields(&fields, window, cx);
        self.baseline = fields;
        self.entity = entity;
    }

    /// Puts the editor back to what was last saved.
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
        let mut entity = self.entity.clone();
        entity.insert(
            "name".into(),
            json!(if fields.name.trim().is_empty() { DEFAULT_NAME } else { fields.name.trim() }),
        );
        entity.insert("method".into(), json!(fields.method));
        entity.insert("url".into(), json!(fields.url));
        entity.insert("headers".into(), Value::Object(parse_headers(&fields.headers)));
        entity.insert("body".into(), json!(fields.body));
        entity.insert("protocol".into(), json!("rest"));
        entity.insert("workspaceId".into(), json!(self.workspace_id));
        if let Some(folder) = &self.folder_id {
            entity.insert("folderId".into(), json!(folder));
        }

        self.saving = true;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        let kind = RequestKind::Rest.entity_kind();
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
                        let saved = this.edited_entity(&fields);
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

    /// The loaded entity with the saved fields written over it: what the engine now holds.
    fn edited_entity(&self, fields: &Fields) -> Map<String, Value> {
        let mut entity = self.entity.clone();
        entity.insert("name".into(), json!(fields.name.trim()));
        entity.insert("method".into(), json!(fields.method));
        entity.insert("url".into(), json!(fields.url));
        entity.insert("headers".into(), Value::Object(parse_headers(&fields.headers)));
        entity.insert("body".into(), json!(fields.body));
        entity
    }

    /// Sends the request. A saved, unedited request is sent by id; anything else is sent as the draft the
    /// editor holds.
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sending || self.load != LoadState::Ready {
            return;
        }
        let fields = self.fields(cx);
        if fields.url.is_empty() {
            window.push_notification(Notification::warning("Enter a URL to send.").title("Nothing to send"), cx);
            return;
        }
        let payload = match (&self.saved_id, self.fields(cx) == self.baseline) {
            (Some(id), true) => json!({ "workspaceId": self.workspace_id, "id": id }),
            _ => {
                let script = |key: &str| self.entity.get(key).and_then(Value::as_str).map(str::to_owned);
                let mut scripts = Map::new();
                for (field, key) in [("pre", "preScript"), ("post", "postScript"), ("test", "testScript")] {
                    if let Some(text) = script(key).filter(|s| !s.is_empty()) {
                        scripts.insert(field.into(), json!(text));
                    }
                }
                json!({
                    "workspaceId": self.workspace_id,
                    "request": {
                        "method": fields.method,
                        "url": fields.url,
                        "headers": parse_headers(&fields.headers),
                        "body": fields.body,
                    },
                    "scripts": scripts,
                })
            }
        };

        self.sending = true;
        self.exchange.update(cx, |exchange, cx| exchange.begin(cx));
        cx.emit(SessionEvent::Changed);
        cx.notify();
        let summary = format!("{} {}", fields.method, fields.url);
        let client = self.model.read(cx).client().clone();
        call_in(&client, window, cx, "request.send", payload, move |this, outcome, window, cx| {
            this.sending = false;
            match outcome {
                Ok(result) => {
                    let status = this
                        .exchange
                        .update(cx, |exchange, cx| exchange.finish(&result, BodyEncoding::Base64, window, cx));
                    let took = result.get("durationMs").and_then(Value::as_u64).unwrap_or(0);
                    cx.emit(SessionEvent::Activity(match status {
                        Some(status) => format!("{summary} → {status} in {took} ms"),
                        None => format!("{summary} → done in {took} ms"),
                    }));
                }
                Err(error) => {
                    let message = describe(&error);
                    this.exchange.update(cx, |exchange, cx| exchange.fail(message.clone(), cx));
                    cx.emit(SessionEvent::Activity(format!("{summary} failed: {message}")));
                }
            }
            cx.emit(SessionEvent::Changed);
            cx.notify();
        });
    }

    fn render_sections(&self, cx: &mut Context<Self>) -> TabBar {
        TabBar::new("request-sections")
            .underline()
            .small()
            .selected_index(match self.section {
                Section::Headers => 0,
                Section::Body => 1,
            })
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.section = if *ix == 0 { Section::Headers } else { Section::Body };
                cx.notify();
            }))
            .child(Tab::new().label("Headers"))
            .child(Tab::new().label("Body"))
    }
}

fn entity_of(result: &Value) -> Result<Map<String, Value>, String> {
    result.get("entity").and_then(Value::as_object).cloned().ok_or_else(|| "The request no longer exists.".to_owned())
}

impl Render for RestSession {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        match &self.load {
            LoadState::Loading => {
                return v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .bg(theme.background)
                    .child(Spinner::new())
                    .child("Loading request…")
                    .into_any_element();
            }
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
        v_flex()
            .size_full()
            .bg(theme.background)
            .gap_2()
            .p_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.name).appearance(false)))
                    .child(
                        Button::new("revert-request")
                            .ghost()
                            .label("Revert")
                            .disabled(!dirty)
                            .on_click(cx.listener(|this, _, window, cx| this.revert(window, cx))),
                    )
                    .child(
                        Button::new("save-request")
                            .label("Save")
                            .loading(self.saving)
                            .disabled(!dirty && self.saved_id.is_some())
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().w_32().flex_none().child(Select::new(&self.method)))
                    .child(div().flex_1().child(Input::new(&self.url)))
                    .child(
                        Button::new("send-request")
                            .primary()
                            .label("Send")
                            .loading(self.sending)
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            )
            .child(self.render_sections(cx))
            .child(div().flex_1().min_h_0().child(match self.section {
                Section::Headers => Textarea::new(&self.headers).h_full(),
                Section::Body => Textarea::new(&self.body).h_full(),
            }))
            .into_any_element()
    }
}
