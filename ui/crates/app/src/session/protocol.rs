//! An open GraphQL, gRPC or SOAP request.
//!
//! The three share one editor because they share one shape: a name, a target, a few short fields, one or
//! two documents, and a table of headers. What differs is which stored field each control edits, and that is
//! a table ([`fields_of`]) rather than three copies of the same view.
//!
//! These protocols are sent by id (`graphql.send`, `grpc.send`, `soap.send`): the engine reads the stored
//! request, resolves its variables and runs its scripts. So sending an edited request saves it first.
//!
//! A draft is created by Save. Once it exists there is no Save step: an edit is written a moment after it
//! is made. The request's name is edited in the window's breadcrumb, which shows this session's name field.

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
use serde_json::{Map, Value, json};

use super::{Carry, LoadState, SaveMode, SessionEvent, exchange::Exchange};
use crate::{
    app_model::{AppModel, ModelEvent},
    components::{Autosave, KeyValueEvent, KeyValueTable, Syntax, VarEditor, page},
    engine::{call_in, describe},
    model::{
        RequestKind,
        http::{BodyEncoding, header_map, header_pairs},
        snippet::RequestParts,
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    /// One row of text.
    Line,
    /// A document, edited in a code editor and shown as a section.
    Document(Syntax),
    /// A string map, edited as a name/value table and shown as a section.
    Pairs,
}

#[derive(Clone, Copy)]
struct FieldSpec {
    /// The entity field this control edits.
    key: &'static str,
    label: &'static str,
    placeholder: &'static str,
    kind: FieldKind,
}

const fn field(key: &'static str, label: &'static str, placeholder: &'static str, kind: FieldKind) -> FieldSpec {
    FieldSpec { key, label, placeholder, kind }
}

const GRAPHQL_FIELDS: &[FieldSpec] = &[
    field("endpointUrl", "Endpoint", "https://api.example.test/graphql", FieldKind::Line),
    field("operationName", "Operation", "Operation name (optional)", FieldKind::Line),
    field("query", "Query", "query { ... }", FieldKind::Document(Syntax::GraphQl)),
    field("variables", "Variables", "{ }", FieldKind::Document(Syntax::Json)),
    field("headers", "Headers", "", FieldKind::Pairs),
];

const GRPC_FIELDS: &[FieldSpec] = &[
    field("serverAddress", "Server", "localhost:50051", FieldKind::Line),
    field("serviceName", "Service", "package.Service", FieldKind::Line),
    field("methodName", "Method", "MethodName", FieldKind::Line),
    field("requestBody", "Message", "{ }", FieldKind::Document(Syntax::Json)),
    field("metadata", "Metadata", "", FieldKind::Pairs),
];

const SOAP_FIELDS: &[FieldSpec] = &[
    field("endpointUrl", "Endpoint", "https://example.test/service", FieldKind::Line),
    field("soapAction", "SOAPAction", "urn:Action (optional)", FieldKind::Line),
    field("body", "Envelope", "<soap:Envelope>…</soap:Envelope>", FieldKind::Document(Syntax::Markup)),
    field("headers", "Headers", "", FieldKind::Pairs),
];

/// The controls of each protocol's editor. The first `Line` field is the target, shown beside Send.
fn fields_of(kind: RequestKind) -> &'static [FieldSpec] {
    match kind {
        RequestKind::GraphQl => GRAPHQL_FIELDS,
        RequestKind::Grpc => GRPC_FIELDS,
        RequestKind::Soap => SOAP_FIELDS,
        RequestKind::Rest | RequestKind::WebSocket => &[],
    }
}

/// The name a request of this kind has until it is given another.
fn default_name(kind: RequestKind) -> String {
    format!("New {} request", kind.label())
}

/// The field a draft's body is carried to and from when its type changes. GraphQL has none.
fn body_key(kind: RequestKind) -> Option<&'static str> {
    match kind {
        RequestKind::Soap => Some("body"),
        RequestKind::Grpc => Some("requestBody"),
        RequestKind::GraphQl | RequestKind::Rest | RequestKind::WebSocket => None,
    }
}

/// What a new request of each protocol starts with, beyond empty fields.
fn defaults_of(kind: RequestKind) -> Map<String, Value> {
    let value = match kind {
        RequestKind::GraphQl => {
            json!({ "protocol": "graphql", "query": "", "variables": "", "operationName": "", "headers": {} })
        }
        RequestKind::Grpc => json!({
            "protocol": "grpc", "requestBody": "{}", "metadata": {}, "useReflection": true, "streamingType": "unary",
        }),
        RequestKind::Soap => json!({ "protocol": "soap", "soapAction": "", "headers": {}, "body": "" }),
        RequestKind::Rest | RequestKind::WebSocket => json!({}),
    };
    value.as_object().cloned().unwrap_or_default()
}

enum Control {
    Text(VarEditor),
    Pairs(Entity<KeyValueTable>),
}

#[derive(Debug, Clone, PartialEq)]
enum FieldValue {
    Text(String),
    Pairs(Vec<(String, String)>),
}

pub struct ProtocolSession {
    model: Entity<AppModel>,
    kind: RequestKind,
    workspace_id: String,
    saved_id: Option<String>,
    folder_id: Option<String>,
    entity: Map<String, Value>,
    load: LoadState,
    name: Entity<InputState>,
    controls: Vec<Control>,
    /// Index into the fields that are shown as sections (documents and tables).
    section: usize,
    baseline: (String, Vec<FieldValue>),
    exchange: Entity<Exchange>,
    sending: bool,
    saving: bool,
    /// Set when Send was asked for with unsaved edits: the save that follows sends when it lands.
    send_after_save: bool,
    autosave: Autosave,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SessionEvent> for ProtocolSession {}

impl ProtocolSession {
    fn build(
        model: Entity<AppModel>,
        kind: RequestKind,
        saved_id: Option<String>,
        folder_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        debug_assert!(kind.send_command().is_some(), "{} is not an HTTP-like request type", kind.label());
        let workspace_id = model.read(cx).workspace_id().to_owned();
        let name =
            cx.new(|cx| InputState::new(window, cx).placeholder("Request name").default_value(default_name(kind)));
        let exchange = cx.new(|cx| Exchange::new(window, cx));

        let mut subscriptions =
            vec![cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.changed(cx),
                InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit_name(window, cx),
                InputEvent::Focus => {}
            })];
        subscriptions.push(cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| {
            if let ModelEvent::EntityReverted(id) = event
                && this.saved_id.as_deref() == Some(id.as_str())
            {
                this.reload(window, cx);
            }
        }));
        let controls: Vec<Control> = fields_of(kind)
            .iter()
            .map(|spec| match spec.kind {
                FieldKind::Line => {
                    let editor = VarEditor::line(spec.placeholder, window, cx);
                    subscriptions.push(cx.subscribe_in(
                        editor.state(),
                        window,
                        |this, _, event: &InputEvent, window, cx| match event {
                            InputEvent::Change => this.changed(cx),
                            InputEvent::PressEnter { .. } => this.send(window, cx),
                            _ => {}
                        },
                    ));
                    Control::Text(editor)
                }
                FieldKind::Document(syntax) => {
                    let editor = VarEditor::code(syntax, spec.placeholder, window, cx);
                    subscriptions.push(cx.subscribe(editor.state(), |this, _, event: &InputEvent, cx| {
                        if matches!(event, InputEvent::Change) {
                            this.changed(cx);
                        }
                    }));
                    Control::Text(editor)
                }
                FieldKind::Pairs => {
                    let table = cx.new(|cx| KeyValueTable::new("Name", "Value", window, cx));
                    subscriptions.push(cx.subscribe(&table, |this, _, _: &KeyValueEvent, cx| this.changed(cx)));
                    Control::Pairs(table)
                }
            })
            .collect();

        let mut this = Self {
            model,
            kind,
            workspace_id,
            saved_id,
            folder_id,
            entity: defaults_of(kind),
            load: LoadState::Ready,
            name,
            controls,
            section: 0,
            baseline: (String::new(), Vec::new()),
            exchange,
            sending: false,
            saving: false,
            send_after_save: false,
            autosave: Autosave::new(window),
            _subscriptions: subscriptions,
        };
        this.baseline = this.values(cx);
        this
    }

    pub fn new_draft(
        model: Entity<AppModel>,
        kind: RequestKind,
        folder_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::build(model, kind, None, folder_id, window, cx);
        let entity = this.entity.clone();
        this.fill_controls(&entity, window, cx);
        this.baseline = this.values(cx);
        this
    }

    pub fn open_saved(
        model: Entity<AppModel>,
        kind: RequestKind,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::build(model, kind, Some(id), None, window, cx);
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
        let payload = json!({ "workspaceId": self.workspace_id, "kind": self.kind.entity_kind(), "id": id });
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
                }
                Err(message) => this.load = LoadState::Failed(message),
            }
            cx.emit(SessionEvent::Changed);
            cx.notify();
        });
    }

    pub fn kind(&self) -> RequestKind {
        self.kind
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

    pub fn exchange(&self) -> &Entity<Exchange> {
        &self.exchange
    }

    pub fn title(&self, cx: &gpui_kit::App) -> SharedString {
        let name = self.name.read(cx).value();
        if name.is_empty() { format!("{} request", self.kind.label()).into() } else { name }
    }

    pub fn is_dirty(&self, cx: &gpui_kit::App) -> bool {
        self.load == LoadState::Ready && self.values(cx) != self.baseline
    }

    /// The request as an HTTP call, for a snippet. gRPC is not HTTP in a way a snippet could show.
    pub fn request_parts(&self, cx: &gpui_kit::App) -> Option<RequestParts> {
        let text = |key: &str| self.text_of(key, cx).unwrap_or_default();
        let mut headers = self.pairs_of("headers", cx).unwrap_or_default();
        match self.kind {
            RequestKind::GraphQl => {
                let variables: Value = serde_json::from_str(&text("variables")).unwrap_or(Value::Null);
                let mut body = json!({ "query": text("query") });
                if !variables.is_null() {
                    body["variables"] = variables;
                }
                if !text("operationName").is_empty() {
                    body["operationName"] = json!(text("operationName"));
                }
                if !headers.iter().any(|(name, _)| name.eq_ignore_ascii_case("content-type")) {
                    headers.push(("Content-Type".into(), "application/json".into()));
                }
                Some(RequestParts { method: "POST".into(), url: text("endpointUrl"), headers, body: body.to_string() })
            }
            RequestKind::Soap => {
                if !headers.iter().any(|(name, _)| name.eq_ignore_ascii_case("content-type")) {
                    headers.push(("Content-Type".into(), "text/xml; charset=utf-8".into()));
                }
                if !text("soapAction").is_empty() {
                    headers.push(("SOAPAction".into(), text("soapAction")));
                }
                Some(RequestParts { method: "POST".into(), url: text("endpointUrl"), headers, body: text("body") })
            }
            _ => None,
        }
    }

    fn control(&self, key: &str) -> Option<&Control> {
        fields_of(self.kind).iter().position(|spec| spec.key == key).and_then(|ix| self.controls.get(ix))
    }

    fn text_of(&self, key: &str, cx: &gpui_kit::App) -> Option<String> {
        match self.control(key)? {
            Control::Text(editor) => Some(editor.value(cx)),
            Control::Pairs(_) => None,
        }
    }

    fn pairs_of(&self, key: &str, cx: &gpui_kit::App) -> Option<Vec<(String, String)>> {
        match self.control(key)? {
            Control::Pairs(table) => Some(table.read(cx).pairs(cx)),
            Control::Text(_) => None,
        }
    }

    /// Why the edit cannot be written as it stands, for a saved request that has one.
    pub fn problem(&self, cx: &gpui_kit::App) -> Option<SharedString> {
        if self.saved_id.is_none() || self.load != LoadState::Ready {
            return None;
        }
        let values = self.values(cx);
        (values != self.baseline && Self::target_is_empty(&values))
            .then(|| format!("Not saved. Enter the {}.", self.target_label()).into())
    }

    /// The first field is the target; a request without one cannot be written.
    fn target_is_empty(values: &(String, Vec<FieldValue>)) -> bool {
        matches!(values.1.first(), Some(FieldValue::Text(text)) if text.is_empty())
    }

    fn target_label(&self) -> String {
        fields_of(self.kind)[0].label.to_lowercase()
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
        if self.saved_id.is_none() && self.values(cx).0 != self.baseline.0 {
            self.save(SaveMode::Auto, window, cx);
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
        let (name, values) = self.values(cx);
        let mut carry =
            Carry { name: (!name.is_empty() && name != default_name(self.kind)).then_some(name), ..Carry::default() };
        let body_key = body_key(self.kind);
        let mut has_headers = false;
        for (ix, (spec, value)) in fields_of(self.kind).iter().zip(values).enumerate() {
            match value {
                FieldValue::Text(text) if ix == 0 => carry.url = text,
                FieldValue::Text(text) if Some(spec.key) == body_key => carry.body = text,
                FieldValue::Pairs(pairs) if !has_headers => {
                    carry.headers = pairs;
                    has_headers = true;
                }
                _ => {}
            }
        }
        carry
    }

    /// Fills this draft with what was typed into a draft of another type.
    pub fn apply_carry(&mut self, carry: &Carry, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(name) = &carry.name {
            self.name.update(cx, |s, cx| s.set_value(name.clone(), window, cx));
        }
        let body_key = body_key(self.kind);
        let mut has_headers = false;
        for (ix, (spec, control)) in fields_of(self.kind).iter().zip(&self.controls).enumerate() {
            match control {
                Control::Text(editor) if ix == 0 => editor.set_value(carry.url.clone(), window, cx),
                // A document is not blanked by a draft that had no such document.
                Control::Text(editor) if Some(spec.key) == body_key && !carry.body.is_empty() => {
                    editor.set_value(carry.body.clone(), window, cx)
                }
                Control::Pairs(table) if !has_headers => {
                    has_headers = true;
                    table.update(cx, |table, cx| table.set_pairs(&carry.headers, window, cx));
                }
                _ => {}
            }
        }
        self.changed(cx);
    }

    fn values(&self, cx: &gpui_kit::App) -> (String, Vec<FieldValue>) {
        let fields = fields_of(self.kind)
            .iter()
            .zip(&self.controls)
            .map(|(spec, control)| match control {
                Control::Text(editor) if spec.kind == FieldKind::Line => FieldValue::Text(editor.line_value(cx)),
                Control::Text(editor) => FieldValue::Text(editor.value(cx)),
                Control::Pairs(table) => FieldValue::Pairs(table.read(cx).pairs(cx)),
            })
            .collect();
        (self.name.read(cx).value().trim().to_owned(), fields)
    }

    fn fill_controls(&mut self, entity: &Map<String, Value>, window: &mut Window, cx: &mut Context<Self>) {
        for (spec, control) in fields_of(self.kind).iter().zip(&self.controls) {
            match control {
                Control::Text(editor) => {
                    let text = entity.get(spec.key).and_then(Value::as_str).unwrap_or_default().to_owned();
                    editor.set_value(text, window, cx);
                }
                Control::Pairs(table) => {
                    let pairs = header_pairs(entity.get(spec.key).unwrap_or(&Value::Null));
                    table.update(cx, |table, cx| table.set_pairs(&pairs, window, cx));
                }
            }
        }
    }

    fn adopt_entity(&mut self, entity: Map<String, Value>, window: &mut Window, cx: &mut Context<Self>) {
        self.folder_id = entity.get("folderId").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
        if let Some(name) = entity.get("name").and_then(Value::as_str) {
            self.name.update(cx, |state, cx| state.set_value(name.to_owned(), window, cx));
        }
        self.fill_controls(&entity, window, cx);
        self.entity = entity;
        self.baseline = self.values(cx);
    }

    /// The loaded entity with the editor's fields written over it.
    fn edited_entity(&self, (name, values): &(String, Vec<FieldValue>)) -> Map<String, Value> {
        let mut entity = self.entity.clone();
        entity.insert("name".into(), json!(if name.is_empty() { default_name(self.kind) } else { name.clone() }));
        for (spec, value) in fields_of(self.kind).iter().zip(values) {
            entity.insert(
                spec.key.into(),
                match value {
                    FieldValue::Text(text) => json!(text),
                    FieldValue::Pairs(pairs) => Value::Object(header_map(pairs)),
                },
            );
        }
        entity
    }

    /// Creates the request if it is a draft, or writes the edit over the saved one. A write that is asked
    /// for while another is in flight is not lost: the one in flight looks for a newer edit when it lands.
    pub fn save(&mut self, mode: SaveMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || self.load != LoadState::Ready {
            return;
        }
        let values = self.values(cx);
        let creating = self.saved_id.is_none();
        if !creating && values == self.baseline {
            return;
        }
        if Self::target_is_empty(&values) {
            self.send_after_save = false;
            if mode == SaveMode::Explicit {
                window.push_notification(
                    Notification::warning(format!("Enter the {} first.", self.target_label())).title("Can't save yet"),
                    cx,
                );
            }
            return;
        }
        let mut entity = self.edited_entity(&values);
        entity.insert("workspaceId".into(), json!(self.workspace_id));
        if let Some(folder) = &self.folder_id {
            entity.insert("folderId".into(), json!(folder));
        }

        self.saving = true;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        let kind = self.kind.entity_kind();
        let edited = entity.clone();
        let (action, payload) = match &self.saved_id {
            Some(id) => {
                entity.insert("id".into(), json!(id));
                ("entity.update", json!({ "kind": kind, "entity": entity }))
            }
            None => ("entity.create", json!({ "kind": kind, "workspaceId": self.workspace_id, "entity": entity })),
        };
        call_in(&client, window, cx, action, payload, move |this, outcome, window, cx| {
            this.saving = false;
            let send_next = std::mem::take(&mut this.send_after_save);
            // The controls are left alone: they may hold what was typed while this was in flight. What was
            // sent becomes the baseline, so that newer text still reads as an edit.
            let saved = match outcome {
                Ok(result) if creating => {
                    let saved = result.get("entity").and_then(Value::as_object).cloned();
                    match &saved {
                        Some(saved) => {
                            this.saved_id = saved.get("id").and_then(Value::as_str).map(str::to_owned);
                            this.entity = saved.clone();
                            this.baseline = values;
                            if let Some(id) = this.saved_id.clone() {
                                cx.emit(SessionEvent::Saved { id });
                            }
                        }
                        None => window
                            .push_notification(Notification::error("The engine did not return the saved request."), cx),
                    }
                    saved.is_some()
                }
                Ok(_) => {
                    this.entity = edited;
                    this.baseline = values;
                    true
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
            };
            // An edit made since this write was sent is the next write. (`changed` schedules it.)
            this.changed(cx);
            if send_next && saved {
                this.send(window, cx);
            }
        });
    }

    /// Sends the stored request. Unsaved edits are saved first, because the engine sends what it has stored.
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self.kind.send_command() else { return };
        if self.sending || self.load != LoadState::Ready {
            return;
        }
        // The write in flight may not hold the latest edit; the send waits for it and looks again.
        if self.saving {
            self.send_after_save = true;
            return;
        }
        let Some(id) = self.saved_id.clone().filter(|_| !self.is_dirty(cx)) else {
            self.send_after_save = true;
            self.save(SaveMode::Explicit, window, cx);
            return;
        };
        self.sending = true;
        self.exchange.update(cx, |exchange, cx| exchange.begin(cx));
        self.changed(cx);
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "id": id });
        call_in(&client, window, cx, action, payload, move |this, outcome, window, cx| {
            this.sending = false;
            match outcome {
                Ok(result) => {
                    this.exchange.update(cx, |exchange, cx| exchange.finish(&result, BodyEncoding::Text, window, cx));
                }
                Err(error) => {
                    let message = describe(&error);
                    this.exchange.update(cx, |exchange, cx| exchange.fail(message, cx));
                }
            }
            this.changed(cx);
        });
    }

    /// The fields shown as sections, with their index among all fields.
    fn sections(&self) -> Vec<(usize, &'static FieldSpec)> {
        fields_of(self.kind).iter().enumerate().filter(|(_, spec)| spec.kind != FieldKind::Line).collect()
    }
}

impl Render for ProtocolSession {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        match &self.load {
            LoadState::Loading => return page::loading("Loading request…"),
            LoadState::Failed(message) => {
                return div()
                    .size_full()
                    .p_4()
                    .child(
                        Alert::error("load-failed", SharedString::from(message.clone()))
                            .title("Couldn't open the request"),
                    )
                    .into_any_element();
            }
            LoadState::Ready => {}
        }

        let specs = fields_of(self.kind);
        let line = |ix: usize| match &self.controls[ix] {
            Control::Text(editor) => editor.line_field(cx).into_any_element(),
            Control::Pairs(table) => table.clone().into_any_element(),
        };
        // Every line field after the target sits in one row, each under its label.
        let extra_lines: Vec<usize> = specs
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, spec)| spec.kind == FieldKind::Line)
            .map(|(ix, _)| ix)
            .collect();
        let sections = self.sections();
        let section = self.section.min(sections.len().saturating_sub(1));
        let content = sections.get(section).map(|(ix, _)| match &self.controls[*ix] {
            Control::Text(editor) => Editor::new(editor.state()).bordered(false).h_full().into_any_element(),
            Control::Pairs(table) => table.clone().into_any_element(),
        });

        v_flex()
            .size_full()
            .bg(theme.background)
            .child(
                h_flex().flex_none().gap_2().px_3().py_2().child(div().flex_1().min_w_0().child(line(0))).child(
                    Button::new("send-request")
                        .primary()
                        .h(VarEditor::row_height(cx))
                        .label("Send")
                        .loading(self.sending)
                        .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                ),
            )
            .child(h_flex().flex_none().gap_3().px_3().pb_2().items_start().children(extra_lines.into_iter().map(
                |ix| {
                    v_flex().flex_1().min_w_0().gap_1().child(page::section_label(specs[ix].label, cx)).child(line(ix))
                },
            )))
            .child(
                div().flex_none().px_3().child(
                    TabBar::new("protocol-sections")
                        .underline()
                        .small()
                        .selected_index(section)
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.section = *ix;
                            cx.notify();
                        }))
                        .children(sections.iter().map(|(_, spec)| Tab::new().label(spec.label))),
                ),
            )
            .child(div().flex_1().min_h_0().children(content))
            .into_any_element()
    }
}
