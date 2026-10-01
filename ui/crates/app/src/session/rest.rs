//! An open REST request: the editor, its save/send commands and its response.
//!
//! The saved request lives in the engine; this entity holds an *edit* of it. Fields are read from the
//! controls when needed (the controls are the single source of truth for what is typed), compared to a
//! baseline taken at the last load or save to know if there is anything to save, and written back as one
//! `entity.update`. Fields the editor does not show — ids, timestamps — are carried through unchanged from
//! the entity the engine returned, so saving here cannot erase what another client set.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IndexPath, Sizable as _, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, Input, InputEvent, InputState},
    notification::Notification,
    select::{Select, SelectEvent, SelectState},
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div,
};
use serde_json::{Map, Value, json};

use super::{LoadState, SessionEvent, exchange::Exchange};
use crate::{
    app_model::AppModel,
    components::{KeyValueEvent, KeyValueTable, Syntax, VarEditor, page},
    engine::{call_in, describe},
    model::{
        RequestKind,
        http::{
            BodyEncoding, BodyMode, HTTP_METHODS, decode_form, encode_form, header_map, header_pairs, join_query,
            pretty_json, split_query,
        },
        snippet::RequestParts,
    },
};

const DEFAULT_NAME: &str = "New request";

/// What the editor holds, as plain values. Two of these are compared to know whether there is an edit.
#[derive(Debug, Clone, PartialEq)]
struct Fields {
    name: String,
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: String,
    pre_script: String,
    post_script: String,
    test_script: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Params,
    Headers,
    Body,
    Scripts,
}

const SECTIONS: [Section; 4] = [Section::Params, Section::Headers, Section::Body, Section::Scripts];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    Pre,
    Post,
    Test,
}

const SCRIPTS: [Script; 3] = [Script::Pre, Script::Post, Script::Test];

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
    url: VarEditor,
    params: Entity<KeyValueTable>,
    headers: Entity<KeyValueTable>,
    body_mode: BodyMode,
    body: VarEditor,
    form: Entity<KeyValueTable>,
    pre_script: VarEditor,
    post_script: VarEditor,
    test_script: VarEditor,
    section: Section,
    script: Script,
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
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Request name").default_value(DEFAULT_NAME));
        let method = cx.new(|cx| SelectState::new(HTTP_METHODS.to_vec(), Some(IndexPath::default()), window, cx));
        let url = VarEditor::line("https://api.example.test/resource", window, cx);
        let params = cx.new(|cx| KeyValueTable::new("Parameter", "Value", window, cx));
        let headers = cx.new(|cx| KeyValueTable::new("Header", "Value", window, cx));
        let form = cx.new(|cx| KeyValueTable::new("Field", "Value", window, cx));
        let body = VarEditor::code(Syntax::Json, "Request body", window, cx);
        let pre_script = VarEditor::code(Syntax::JavaScript, "// Runs before the request is sent", window, cx);
        let post_script = VarEditor::code(Syntax::JavaScript, "// Runs after the response arrives", window, cx);
        let test_script = VarEditor::code(Syntax::JavaScript, "// test(\"status is 200\", () => { ... })", window, cx);
        let exchange = cx.new(|cx| Exchange::new(window, cx));

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.changed(cx),
            InputEvent::PressEnter { .. } => this.send(window, cx),
            _ => {}
        }));
        subscriptions.push(cx.subscribe_in(
            url.state(),
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.sync_params_from_url(window, cx);
                    this.changed(cx);
                }
                InputEvent::PressEnter { .. } => this.send(window, cx),
                _ => {}
            },
        ));
        subscriptions.push(cx.subscribe_in(&params, window, |this, _, _: &KeyValueEvent, window, cx| {
            this.sync_url_from_params(window, cx);
            this.changed(cx);
        }));
        for table in [&headers, &form] {
            subscriptions.push(cx.subscribe(table, |this, _, _: &KeyValueEvent, cx| this.changed(cx)));
        }
        for editor in [&body, &pre_script, &post_script, &test_script] {
            subscriptions.push(cx.subscribe(editor.state(), |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.changed(cx);
                }
            }));
        }
        subscriptions.push(cx.subscribe(&method, |this, _, _: &SelectEvent<Vec<&'static str>>, cx| this.changed(cx)));

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
            params,
            headers,
            body_mode: BodyMode::None,
            body,
            form,
            pre_script,
            post_script,
            test_script,
            section: Section::Params,
            script: Script::Pre,
            baseline: Fields {
                name: String::new(),
                method: String::new(),
                url: String::new(),
                headers: Vec::new(),
                body: String::new(),
                pre_script: String::new(),
                post_script: String::new(),
                test_script: String::new(),
            },
            exchange,
            sending: false,
            saving: false,
            _subscriptions: subscriptions,
        };
        this.baseline = this.fields(cx);
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

    pub fn method(&self, cx: &gpui_kit::App) -> &'static str {
        self.method.read(cx).selected_value().copied().unwrap_or("GET")
    }

    pub fn is_dirty(&self, cx: &gpui_kit::App) -> bool {
        self.load == LoadState::Ready && self.fields(cx) != self.baseline
    }

    /// The request as the editor holds it, for writing out as a code snippet.
    pub fn request_parts(&self, cx: &gpui_kit::App) -> RequestParts {
        let fields = self.fields(cx);
        RequestParts { method: fields.method, url: fields.url, headers: fields.headers, body: fields.body }
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(SessionEvent::Changed);
        cx.notify();
    }

    fn fields(&self, cx: &gpui_kit::App) -> Fields {
        let body = match self.body_mode {
            BodyMode::None => String::new(),
            BodyMode::Form => encode_form(&self.form.read(cx).pairs(cx)),
            _ => self.body.value(cx),
        };
        Fields {
            name: self.name.read(cx).value().trim().to_owned(),
            method: self.method(cx).to_owned(),
            url: self.url.line_value(cx),
            headers: self.headers.read(cx).pairs(cx),
            body,
            pre_script: self.pre_script.value(cx),
            post_script: self.post_script.value(cx),
            test_script: self.test_script.value(cx),
        }
    }

    /// The params table mirrors the URL's query. Each side only writes the other when they disagree, which
    /// is what stops the two from answering each other forever.
    fn sync_params_from_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (_, query) = split_query(&self.url.line_value(cx));
        if self.params.read(cx).pairs(cx) != query {
            self.params.update(cx, |table, cx| table.set_pairs(&query, window, cx));
        }
    }

    fn sync_url_from_params(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.url.line_value(cx);
        let (base, query) = split_query(&current);
        let pairs = self.params.read(cx).pairs(cx);
        if pairs != query {
            self.url.set_value(join_query(&base, &pairs), window, cx);
        }
    }

    fn content_type(&self, cx: &gpui_kit::App) -> Option<String> {
        self.headers
            .read(cx)
            .pairs(cx)
            .into_iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value)
    }

    fn body_syntax(mode: BodyMode) -> Syntax {
        match mode {
            BodyMode::Json => Syntax::Json,
            BodyMode::Xml | BodyMode::Html => Syntax::Markup,
            BodyMode::None | BodyMode::Text | BodyMode::Form => Syntax::Plain,
        }
    }

    /// Changes how the body is edited. The stored request records the mode only as its `Content-Type`, so
    /// that header follows the choice; and a body moving between text and form fields is converted so what
    /// was typed is kept.
    fn set_body_mode(&mut self, mode: BodyMode, window: &mut Window, cx: &mut Context<Self>) {
        if mode == self.body_mode {
            return;
        }
        match (self.body_mode, mode) {
            (BodyMode::Form, _) => {
                let text = encode_form(&self.form.read(cx).pairs(cx));
                self.body.set_value(text, window, cx);
            }
            (_, BodyMode::Form) => {
                let pairs = decode_form(&self.body.value(cx));
                self.form.update(cx, |table, cx| table.set_pairs(&pairs, window, cx));
            }
            _ => {}
        }
        self.body_mode = mode;
        self.body.set_syntax(Self::body_syntax(mode), cx);

        let mut headers = self.headers.read(cx).pairs(cx);
        headers.retain(|(name, _)| !name.eq_ignore_ascii_case("content-type"));
        if let Some(content_type) = mode.content_type() {
            headers.push(("Content-Type".to_owned(), content_type.to_owned()));
        }
        self.headers.update(cx, |table, cx| table.set_pairs(&headers, window, cx));
        self.changed(cx);
    }

    fn format_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match pretty_json(&self.body.value(cx)) {
            Some(pretty) => self.body.set_value(pretty, window, cx),
            None => window.push_notification(
                Notification::warning("The body isn't valid JSON, so it was left as it is.").title("Can't format"),
                cx,
            ),
        }
    }

    fn adopt_entity(&mut self, entity: Map<String, Value>, window: &mut Window, cx: &mut Context<Self>) {
        let text = |key: &str| entity.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        self.folder_id = entity.get("folderId").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);

        let headers = header_pairs(entity.get("headers").unwrap_or(&Value::Null));
        let body = text("body");
        let url = text("url");
        let method = entity.get("method").and_then(Value::as_str).unwrap_or("GET").to_uppercase();
        let method = HTTP_METHODS.iter().find(|m| **m == method).copied().unwrap_or("GET");

        self.name.update(cx, |s, cx| s.set_value(text("name"), window, cx));
        self.method.update(cx, |s, cx| s.set_selected_value(&method, window, cx));
        self.url.set_value(url.trim().to_owned(), window, cx);
        let (_, query) = split_query(url.trim());
        self.params.update(cx, |table, cx| table.set_pairs(&query, window, cx));
        self.headers.update(cx, |table, cx| table.set_pairs(&headers, window, cx));

        self.body_mode = BodyMode::detect(self.content_type(cx).as_deref(), &body);
        self.body.set_syntax(Self::body_syntax(self.body_mode), cx);
        if self.body_mode == BodyMode::Form {
            let pairs = decode_form(&body);
            self.form.update(cx, |table, cx| table.set_pairs(&pairs, window, cx));
        }
        self.body.set_value(body, window, cx);
        self.pre_script.set_value(text("preScript"), window, cx);
        self.post_script.set_value(text("postScript"), window, cx);
        self.test_script.set_value(text("testScript"), window, cx);

        self.entity = entity;
        // The baseline is read back from the controls, so that what they normalise (a trimmed name, a form
        // body re-encoded) is not mistaken for an edit.
        self.baseline = self.fields(cx);
    }

    /// Puts the editor back to what was last saved.
    pub fn revert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut entity = self.entity.clone();
        if self.saved_id.is_none() {
            entity.insert("name".into(), json!(DEFAULT_NAME));
        }
        self.adopt_entity(entity, window, cx);
        self.changed(cx);
    }

    /// The loaded entity with the editor's fields written over it.
    fn edited_entity(&self, fields: &Fields) -> Map<String, Value> {
        let mut entity = self.entity.clone();
        entity.insert("name".into(), json!(if fields.name.is_empty() { DEFAULT_NAME } else { fields.name.as_str() }));
        entity.insert("method".into(), json!(fields.method));
        entity.insert("url".into(), json!(fields.url));
        entity.insert("headers".into(), Value::Object(header_map(&fields.headers)));
        entity.insert("body".into(), json!(fields.body));
        entity.insert("preScript".into(), json!(fields.pre_script));
        entity.insert("postScript".into(), json!(fields.post_script));
        entity.insert("testScript".into(), json!(fields.test_script));
        entity
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
        let mut entity = self.edited_entity(&fields);
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
        let payload = match (&self.saved_id, fields == self.baseline) {
            (Some(id), true) => json!({ "workspaceId": self.workspace_id, "id": id }),
            _ => {
                let mut scripts = Map::new();
                for (field, text) in
                    [("pre", &fields.pre_script), ("post", &fields.post_script), ("test", &fields.test_script)]
                {
                    if !text.trim().is_empty() {
                        scripts.insert(field.into(), json!(text));
                    }
                }
                json!({
                    "workspaceId": self.workspace_id,
                    "request": {
                        "method": fields.method,
                        "url": fields.url,
                        "headers": header_map(&fields.headers),
                        "body": fields.body,
                    },
                    "scripts": scripts,
                })
            }
        };

        self.sending = true;
        self.exchange.update(cx, |exchange, cx| exchange.begin(cx));
        self.changed(cx);
        let client = self.model.read(cx).client().clone();
        call_in(&client, window, cx, "request.send", payload, move |this, outcome, window, cx| {
            this.sending = false;
            match outcome {
                Ok(result) => {
                    this.exchange.update(cx, |exchange, cx| exchange.finish(&result, BodyEncoding::Base64, window, cx));
                }
                Err(error) => {
                    let message = describe(&error);
                    this.exchange.update(cx, |exchange, cx| exchange.fail(message, cx));
                }
            }
            this.changed(cx);
        });
    }

    fn render_sections(&self, cx: &mut Context<Self>) -> TabBar {
        let count = |n: usize, label: &str| if n == 0 { label.to_owned() } else { format!("{label} ({n})") };
        let params = self.params.read(cx).pairs(cx).len();
        let headers = self.headers.read(cx).pairs(cx).len();
        let scripts = [&self.pre_script, &self.post_script, &self.test_script]
            .iter()
            .filter(|editor| !editor.value(cx).trim().is_empty())
            .count();
        TabBar::new("request-sections")
            .underline()
            .small()
            .selected_index(SECTIONS.iter().position(|s| *s == self.section).unwrap_or(0))
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.section = SECTIONS[*ix];
                cx.notify();
            }))
            .child(Tab::new().label(count(params, "Params")))
            .child(Tab::new().label(count(headers, "Headers")))
            .child(Tab::new().label(if self.body_mode == BodyMode::None {
                "Body".to_owned()
            } else {
                format!("Body · {}", self.body_mode.label())
            }))
            .child(Tab::new().label(count(scripts, "Scripts")))
    }

    fn render_body(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let modes = TabBar::new("body-modes")
            .segmented()
            .small()
            .selected_index(BodyMode::ALL.iter().position(|m| *m == self.body_mode).unwrap_or(0))
            .on_click(cx.listener(|this, ix: &usize, window, cx| this.set_body_mode(BodyMode::ALL[*ix], window, cx)))
            .children(BodyMode::ALL.map(|mode| Tab::new().label(mode.label())));
        let content = match self.body_mode {
            BodyMode::None => page::empty("No body", "This request is sent without a body."),
            BodyMode::Form => self.form.clone().into_any_element(),
            _ => Editor::new(self.body.state()).bordered(false).h_full().into_any_element(),
        };
        v_flex()
            .size_full()
            .child(h_flex().flex_none().px_3().py_2().gap_2().child(modes).child(div().flex_1()).when(
                self.body_mode == BodyMode::Json,
                |this| {
                    this.child(
                        Button::new("format-body")
                            .ghost()
                            .small()
                            .label("Format")
                            .on_click(cx.listener(|this, _, window, cx| this.format_body(window, cx))),
                    )
                },
            ))
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }

    fn render_scripts(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let editor = match self.script {
            Script::Pre => &self.pre_script,
            Script::Post => &self.post_script,
            Script::Test => &self.test_script,
        };
        v_flex()
            .size_full()
            .child(
                h_flex().flex_none().px_3().py_2().child(
                    TabBar::new("script-kinds")
                        .segmented()
                        .small()
                        .selected_index(SCRIPTS.iter().position(|s| *s == self.script).unwrap_or(0))
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.script = SCRIPTS[*ix];
                            cx.notify();
                        }))
                        .child(Tab::new().label("Pre-request"))
                        .child(Tab::new().label("Post-response"))
                        .child(Tab::new().label("Tests")),
                ),
            )
            .child(div().flex_1().min_h_0().child(Editor::new(editor.state()).bordered(false).h_full()))
            .into_any_element()
    }
}

fn entity_of(result: &Value) -> Result<Map<String, Value>, String> {
    result.get("entity").and_then(Value::as_object).cloned().ok_or_else(|| "The request no longer exists.".to_owned())
}

impl Render for RestSession {
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

        let dirty = self.is_dirty(cx);
        let row_height = VarEditor::row_height(cx);
        let content = match self.section {
            Section::Params => self.params.clone().into_any_element(),
            Section::Headers => self.headers.clone().into_any_element(),
            Section::Body => self.render_body(cx),
            Section::Scripts => self.render_scripts(cx),
        };
        v_flex()
            .size_full()
            .bg(theme.background)
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .px_3()
                    .pt_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.name).small().appearance(false)))
                    .child(
                        Button::new("revert-request")
                            .ghost()
                            .small()
                            .label("Revert")
                            .disabled(!dirty)
                            .on_click(cx.listener(|this, _, window, cx| this.revert(window, cx))),
                    )
                    .child(
                        Button::new("save-request")
                            .small()
                            .label("Save")
                            .loading(self.saving)
                            .disabled(!dirty && self.saved_id.is_some())
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .child(div().w_32().flex_none().child(Select::new(&self.method).h(row_height)))
                    .child(div().flex_1().min_w_0().child(self.url.line_field(cx)))
                    .child(
                        Button::new("send-request")
                            .primary()
                            .h(row_height)
                            .label("Send")
                            .loading(self.sending)
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            )
            .child(div().flex_none().px_3().child(self.render_sections(cx)))
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}
