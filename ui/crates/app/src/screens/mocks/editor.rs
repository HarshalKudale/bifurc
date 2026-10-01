//! An open mock rule: what it matches, and what it answers with.
//!
//! Like an open request, this holds an *edit* of an entity the engine owns. The controls are read when
//! needed, compared with a baseline to know whether there is anything to save, and written back over the
//! engine's copy, so fields this editor does not show (the captured request a mock was made from) survive.
//!
//! A draft is created by Save. Once it exists there is no Save step: an edit is written a moment after it
//! is made. The mock's name is edited in the window's breadcrumb, which shows this editor's name field.
//!
//! The editor is one entity shown in two dock panels: [`MockEditor::render_rule`] and
//! [`MockEditor::render_response`].

use gpui_kit::component::{
    ActiveTheme as _, IndexPath, Sizable as _, WindowExt as _,
    alert::Alert,
    checkbox::Checkbox,
    h_flex,
    input::{Editor, Input, InputEvent, InputState},
    notification::Notification,
    select::{Select, SelectEvent, SelectState},
    switch::Switch,
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, SharedString,
    Styled as _, Subscription, Window, div,
};
use serde_json::{Map, Value, json};

use crate::{
    app_model::{AppModel, ModelEvent},
    components::{Autosave, KeyValueEvent, KeyValueTable, Syntax, VarEditor, page},
    engine::{call_in, describe},
    model::http::{HTTP_METHODS, header_map, header_pairs},
    session::{LoadState, SaveMode},
};

const DEFAULT_NAME: &str = "New mock";
/// The engine's value for "any method"; the editor shows it as a word.
const ANY_METHOD: &str = "*";
const ANY_METHOD_LABEL: &str = "ANY";
const STREAMING: [(&str, &str); 3] = [("none", "All at once"), ("sse", "Server-sent events"), ("chunked", "Chunked")];

#[derive(Debug, Clone)]
pub enum MockEvent {
    /// Something the tab strip shows may have changed.
    Changed,
    /// A draft was saved and now has this id.
    Saved { id: String },
}

#[derive(Debug, Clone, PartialEq)]
struct Fields {
    name: String,
    method: String,
    pattern: String,
    use_regex: bool,
    enabled: bool,
    status: String,
    mock_status: bool,
    headers: Vec<(String, String)>,
    body: String,
    mock_body: bool,
    delay: String,
    mock_delay: bool,
    streaming: String,
    chunk_delay: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Body,
    Headers,
    Timing,
}

const SECTIONS: [Section; 3] = [Section::Body, Section::Headers, Section::Timing];

pub struct MockEditor {
    model: Entity<AppModel>,
    workspace_id: String,
    saved_id: Option<String>,
    folder_id: Option<String>,
    /// The entity as the engine last returned it; the base that edited fields are written over.
    entity: Map<String, Value>,
    load: LoadState,
    name: Entity<InputState>,
    method: Entity<SelectState<Vec<&'static str>>>,
    pattern: Entity<InputState>,
    use_regex: bool,
    enabled: bool,
    status: Entity<InputState>,
    mock_status: bool,
    headers: Entity<KeyValueTable>,
    body: VarEditor,
    mock_body: bool,
    /// A body stored as base64 is binary; it is kept as it is rather than edited as text.
    binary_body: bool,
    delay: Entity<InputState>,
    mock_delay: bool,
    streaming: Entity<SelectState<Vec<&'static str>>>,
    chunk_delay: Entity<InputState>,
    section: Section,
    baseline: Fields,
    saving: bool,
    autosave: Autosave,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<MockEvent> for MockEditor {}

fn method_options() -> Vec<&'static str> {
    std::iter::once(ANY_METHOD_LABEL).chain(HTTP_METHODS).collect()
}

impl MockEditor {
    fn build(
        model: Entity<AppModel>,
        saved_id: Option<String>,
        folder_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let workspace_id = model.read(cx).workspace_id().to_owned();
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Mock name").default_value(DEFAULT_NAME));
        let method = cx.new(|cx| SelectState::new(method_options(), Some(IndexPath::default()), window, cx));
        let pattern = cx.new(|cx| InputState::new(window, cx).placeholder("/api/users or a full URL"));
        let status = cx.new(|cx| InputState::new(window, cx).placeholder("200").default_value("200"));
        let delay = cx.new(|cx| InputState::new(window, cx).placeholder("0").default_value("0"));
        let chunk_delay = cx.new(|cx| InputState::new(window, cx).placeholder("100").default_value("100"));
        let streaming = cx.new(|cx| {
            SelectState::new(STREAMING.map(|(_, label)| label).to_vec(), Some(IndexPath::default()), window, cx)
        });
        let headers = cx.new(|cx| KeyValueTable::new("Header", "Value", window, cx));
        let body = VarEditor::code(Syntax::Json, "Response body", window, cx);

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.changed(cx),
            InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit_name(window, cx),
            InputEvent::Focus => {}
        }));
        subscriptions.push(cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| {
            if let ModelEvent::EntityReverted(id) = event
                && this.saved_id.as_deref() == Some(id.as_str())
            {
                this.reload(window, cx);
            }
        }));
        for input in [&pattern, &status, &delay, &chunk_delay] {
            subscriptions.push(cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.changed(cx);
                }
            }));
        }
        for select in [&method, &streaming] {
            subscriptions
                .push(cx.subscribe(select, |this, _, _: &SelectEvent<Vec<&'static str>>, cx| this.changed(cx)));
        }
        subscriptions.push(cx.subscribe(&headers, |this, _, _: &KeyValueEvent, cx| {
            this.follow_content_type(cx);
            this.changed(cx);
        }));
        subscriptions.push(cx.subscribe(body.state(), |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.changed(cx);
            }
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
            pattern,
            use_regex: false,
            enabled: true,
            status,
            mock_status: true,
            headers,
            body,
            mock_body: true,
            binary_body: false,
            delay,
            mock_delay: true,
            streaming,
            chunk_delay,
            section: Section::Body,
            baseline: Fields {
                name: String::new(),
                method: String::new(),
                pattern: String::new(),
                use_regex: false,
                enabled: true,
                status: String::new(),
                mock_status: true,
                headers: Vec::new(),
                body: String::new(),
                mock_body: true,
                delay: String::new(),
                mock_delay: true,
                streaming: String::new(),
                chunk_delay: String::new(),
            },
            saving: false,
            autosave: Autosave::new(window),
            _subscriptions: subscriptions,
        };
        this.baseline = this.fields(cx);
        this
    }

    /// A mock that does not exist yet. Saving creates it, in `folder_id` if given.
    pub fn new_draft(
        model: Entity<AppModel>,
        folder_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(model, None, folder_id, window, cx)
    }

    /// A saved mock, loaded by id.
    pub fn open_saved(model: Entity<AppModel>, id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::build(model, Some(id), None, window, cx);
        this.load = LoadState::Loading;
        this.reload(window, cx);
        this
    }

    /// Reads the saved mock from the engine into the editor, replacing what the editor holds.
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.saved_id.clone() else { return };
        // An edit waiting to be written belongs to the copy that is being replaced.
        self.autosave.cancel();
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "kind": "mocks", "id": id });
        call_in(&client, window, cx, "entity.load", payload, |this, outcome, window, cx| {
            match outcome.map_err(|e| describe(&e)).and_then(|v| {
                v.get("entity")
                    .and_then(Value::as_object)
                    .cloned()
                    .ok_or_else(|| "The mock no longer exists.".to_owned())
            }) {
                Ok(entity) => {
                    this.adopt_entity(entity, window, cx);
                    this.load = LoadState::Ready;
                }
                Err(message) => this.load = LoadState::Failed(message),
            }
            this.changed(cx);
        });
    }

    pub fn saved_id(&self) -> Option<&str> {
        self.saved_id.as_deref()
    }

    /// The field the mock's name is typed in. The shell shows it in the breadcrumb.
    pub fn name_input(&self) -> &Entity<InputState> {
        &self.name
    }

    pub fn is_saving(&self) -> bool {
        self.saving
    }

    pub fn title(&self, cx: &gpui_kit::App) -> SharedString {
        let name = self.name.read(cx).value();
        if name.is_empty() { DEFAULT_NAME.into() } else { name }
    }

    /// The method as a tab leads with it.
    pub fn badge(&self, cx: &gpui_kit::App) -> &'static str {
        self.method.read(cx).selected_value().copied().unwrap_or(ANY_METHOD_LABEL)
    }

    pub fn is_dirty(&self, cx: &gpui_kit::App) -> bool {
        self.load == LoadState::Ready && self.fields(cx) != self.baseline
    }

    /// Why the edit cannot be written as it stands, for a saved mock that has one.
    pub fn problem(&self, cx: &gpui_kit::App) -> Option<SharedString> {
        if self.saved_id.is_none() || self.load != LoadState::Ready {
            return None;
        }
        let fields = self.fields(cx);
        if fields == self.baseline {
            return None;
        }
        self.edited_entity(&fields).err().map(|reason| format!("Not saved. {reason}").into())
    }

    /// A saved mock is written a moment after each change; a draft waits for Save.
    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(MockEvent::Changed);
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

    /// Writes an edit that is still waiting, now: the tab is about to close.
    pub fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saved_id.is_some() {
            self.autosave.cancel();
            self.save(SaveMode::Auto, window, cx);
        }
    }

    fn streaming_value(&self, cx: &gpui_kit::App) -> &'static str {
        let label = self.streaming.read(cx).selected_value().copied().unwrap_or(STREAMING[0].1);
        STREAMING.iter().find(|(_, l)| *l == label).map_or(STREAMING[0].0, |(value, _)| value)
    }

    fn fields(&self, cx: &gpui_kit::App) -> Fields {
        let text = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        Fields {
            name: text(&self.name),
            method: self.badge(cx).to_owned(),
            pattern: text(&self.pattern),
            use_regex: self.use_regex,
            enabled: self.enabled,
            status: text(&self.status),
            mock_status: self.mock_status,
            headers: self.headers.read(cx).pairs(cx),
            body: self.body.value(cx),
            mock_body: self.mock_body,
            delay: text(&self.delay),
            mock_delay: self.mock_delay,
            streaming: self.streaming_value(cx).to_owned(),
            chunk_delay: text(&self.chunk_delay),
        }
    }

    /// The body is highlighted for the `Content-Type` the mock answers with.
    fn follow_content_type(&mut self, cx: &mut Context<Self>) {
        let content_type = self
            .headers
            .read(cx)
            .pairs(cx)
            .into_iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value)
            .unwrap_or_default();
        self.body.set_syntax(Syntax::for_content_type(&content_type), cx);
    }

    fn adopt_entity(&mut self, entity: Map<String, Value>, window: &mut Window, cx: &mut Context<Self>) {
        let text = |key: &str| entity.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        let flag = |key: &str, default: bool| entity.get(key).and_then(Value::as_bool).unwrap_or(default);
        let number = |key: &str, default: u64| entity.get(key).and_then(Value::as_u64).unwrap_or(default).to_string();
        self.folder_id = entity.get("folderId").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);

        let method = text("method").to_uppercase();
        let method = method_options().into_iter().find(|m| *m == method).unwrap_or(ANY_METHOD_LABEL);
        let streaming = text("streamingMode");
        let streaming = STREAMING.iter().find(|(value, _)| *value == streaming).map_or(STREAMING[0].1, |(_, l)| l);

        self.name.update(cx, |s, cx| s.set_value(text("name"), window, cx));
        self.method.update(cx, |s, cx| s.set_selected_value(&method, window, cx));
        self.pattern.update(cx, |s, cx| s.set_value(text("urlPattern"), window, cx));
        self.use_regex = flag("useRegex", false);
        self.enabled = flag("enabled", true);
        self.status.update(cx, |s, cx| s.set_value(number("responseStatus", 200), window, cx));
        self.mock_status = flag("responseStatusMocked", true);
        let headers = header_pairs(entity.get("responseHeaders").unwrap_or(&Value::Null));
        self.headers.update(cx, |table, cx| table.set_pairs(&headers, window, cx));
        self.binary_body = entity.get("responseBodyEncoding").and_then(Value::as_str) == Some("base64");
        self.body.set_value(text("responseBody"), window, cx);
        self.follow_content_type(cx);
        self.mock_body = flag("responseBodyMocked", true);
        self.delay.update(cx, |s, cx| s.set_value(number("responseDelay", 0), window, cx));
        self.mock_delay = flag("responseDelayMocked", true);
        self.streaming.update(cx, |s, cx| s.set_selected_value(&streaming, window, cx));
        self.chunk_delay.update(cx, |s, cx| s.set_value(number("streamingChunkDelay", 100), window, cx));

        self.entity = entity;
        self.baseline = self.fields(cx);
    }

    /// The loaded entity with the editor's fields written over it, or what stops it from being saved.
    fn edited_entity(&self, fields: &Fields) -> Result<Map<String, Value>, &'static str> {
        if fields.pattern.is_empty() {
            return Err("Enter the URL pattern this mock answers.");
        }
        let status: u16 = fields
            .status
            .parse()
            .ok()
            .filter(|s| (100..=599).contains(s))
            .ok_or("The status must be a number from 100 to 599.")?;
        let delay: u64 = fields.delay.parse().map_err(|_| "The delay must be a whole number of milliseconds.")?;
        let chunk_delay: u64 =
            fields.chunk_delay.parse().map_err(|_| "The chunk delay must be a whole number of milliseconds.")?;

        let mut entity = self.entity.clone();
        entity.insert("name".into(), json!(if fields.name.is_empty() { DEFAULT_NAME } else { fields.name.as_str() }));
        entity.insert(
            "method".into(),
            json!(if fields.method == ANY_METHOD_LABEL { ANY_METHOD } else { fields.method.as_str() }),
        );
        entity.insert("urlPattern".into(), json!(fields.pattern));
        entity.insert("useRegex".into(), json!(fields.use_regex));
        entity.insert("enabled".into(), json!(fields.enabled));
        entity.insert("responseStatus".into(), json!(status));
        entity.insert("responseStatusMocked".into(), json!(fields.mock_status));
        entity.insert("responseHeaders".into(), Value::Object(header_map(&fields.headers)));
        // Every header listed here is one the mock sets; a header left to the real server is simply not listed.
        entity.insert(
            "mockedResponseHeaders".into(),
            Value::Array(fields.headers.iter().map(|(name, _)| json!(name)).collect()),
        );
        if !self.binary_body {
            entity.insert("responseBody".into(), json!(fields.body));
        }
        entity.insert("responseBodyMocked".into(), json!(fields.mock_body));
        entity.insert("responseDelay".into(), json!(delay));
        entity.insert("responseDelayMocked".into(), json!(fields.mock_delay));
        entity.insert("streamingMode".into(), json!(fields.streaming));
        entity.insert("streamingChunkDelay".into(), json!(chunk_delay));
        Ok(entity)
    }

    /// Creates the mock if it is a draft, or writes the edit over the saved one. A write that is asked
    /// for while another is in flight is not lost: the one in flight looks for a newer edit when it lands.
    pub fn save(&mut self, mode: SaveMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || self.load != LoadState::Ready {
            return;
        }
        let fields = self.fields(cx);
        let creating = self.saved_id.is_none();
        if !creating && fields == self.baseline {
            return;
        }
        let mut entity = match self.edited_entity(&fields) {
            Ok(entity) => entity,
            Err(reason) => {
                if mode == SaveMode::Explicit {
                    window.push_notification(Notification::warning(reason).title("Can't save yet"), cx);
                }
                return;
            }
        };
        entity.insert("workspaceId".into(), json!(self.workspace_id));
        entity.entry("protocol").or_insert(json!("rest"));
        entity.entry("capturedHeaders").or_insert(json!({}));
        entity.entry("capturedBody").or_insert(json!(""));
        if let Some(folder) = &self.folder_id {
            entity.insert("folderId".into(), json!(folder));
        }

        self.saving = true;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        if let Some(id) = &self.saved_id {
            entity.insert("id".into(), json!(id));
        }
        let sent = entity.clone();
        let (action, payload) = match &self.saved_id {
            Some(_) => ("entity.update", json!({ "kind": "mocks", "entity": entity })),
            None => ("entity.create", json!({ "kind": "mocks", "workspaceId": self.workspace_id, "entity": entity })),
        };
        call_in(&client, window, cx, action, payload, move |this, outcome, window, cx| {
            this.saving = false;
            // The controls are left alone: they may hold what was typed while this was in flight. What was
            // sent becomes the baseline, so that newer text still reads as an edit.
            match outcome {
                Ok(result) if creating => match result.get("entity").and_then(Value::as_object).cloned() {
                    Some(saved) => {
                        this.saved_id = saved.get("id").and_then(Value::as_str).map(str::to_owned);
                        this.entity = saved;
                        this.baseline = fields;
                        if let Some(id) = this.saved_id.clone() {
                            cx.emit(MockEvent::Saved { id });
                        }
                    }
                    None => {
                        window.push_notification(Notification::error("The engine did not return the saved mock."), cx)
                    }
                },
                Ok(_) => {
                    this.entity = sent;
                    this.baseline = fields;
                }
                Err(error) => {
                    window.push_notification(Notification::error(describe(&error)).title("Couldn't save the mock"), cx);
                    cx.emit(MockEvent::Changed);
                    cx.notify();
                    return;
                }
            }
            // An edit made since this write was sent is the next write. (`changed` schedules it.)
            this.changed(cx);
        });
    }

    fn placeholder(&self) -> Option<AnyElement> {
        match &self.load {
            LoadState::Loading => Some(page::loading("Loading mock…")),
            LoadState::Failed(message) => Some(
                div()
                    .size_full()
                    .p_4()
                    .child(
                        Alert::error("load-failed", SharedString::from(message.clone()))
                            .title("Couldn't open the mock"),
                    )
                    .into_any_element(),
            ),
            LoadState::Ready => None,
        }
    }

    /// What the mock matches: the method and URL pattern, and whether it is on.
    pub fn render_rule(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some(placeholder) = self.placeholder() {
            return placeholder;
        }
        let theme = cx.theme().clone();
        let row_height = VarEditor::row_height(cx);
        v_flex()
            .size_full()
            .bg(theme.background)
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .child(div().w_32().flex_none().child(Select::new(&self.method).h(row_height)))
                    .child(div().flex_1().min_w_0().child(Input::new(&self.pattern).h(row_height))),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_6()
                    .px_3()
                    .pb_3()
                    .child(Switch::new("mock-enabled").checked(self.enabled).label("Enabled").small().on_change(
                        cx.listener(|this, checked: &bool, _, cx| {
                            this.enabled = *checked;
                            this.changed(cx);
                        }),
                    ))
                    .child(
                        Checkbox::new("mock-regex")
                            .checked(self.use_regex)
                            .small()
                            .label("Pattern is a regular expression")
                            .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                this.use_regex = *checked;
                                this.changed(cx);
                            })),
                    ),
            )
            .child(div().px_3().text_sm().text_color(theme.muted_foreground).child(
                "A request through the proxy with this method whose URL contains the pattern is answered by this mock.",
            ))
            .into_any_element()
    }

    fn pass_through(
        id: &'static str,
        mocked: bool,
        what: &'static str,
        cx: &mut Context<Self>,
        set: fn(&mut Self, bool),
    ) -> impl IntoElement {
        Switch::new(id)
            .checked(mocked)
            .small()
            .label(format!("Replace the {what}"))
            .tooltip(format!("Off: the real server's {what} is passed through"))
            .on_change(cx.listener(move |this, checked: &bool, _, cx| {
                set(this, *checked);
                this.changed(cx);
            }))
    }

    /// What the mock answers with: status, body, headers and timing.
    pub fn render_response(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some(placeholder) = self.placeholder() {
            return placeholder;
        }
        let theme = cx.theme().clone();
        let headers = self.headers.read(cx).pairs(cx).len();
        let content = match self.section {
            Section::Body => v_flex()
                .size_full()
                .child(h_flex().flex_none().px_3().py_2().child(Self::pass_through(
                    "mock-body",
                    self.mock_body,
                    "body",
                    cx,
                    |this, on| this.mock_body = on,
                )))
                .child(div().flex_1().min_h_0().child(if self.binary_body {
                    page::empty("Binary body", "This mock answers with data that isn't text. It is kept as it is.")
                } else {
                    Editor::new(self.body.state()).bordered(false).h_full().into_any_element()
                }))
                .into_any_element(),
            Section::Headers => self.headers.clone().into_any_element(),
            Section::Timing => v_flex()
                .size_full()
                .p_3()
                .gap_4()
                .child(v_flex().gap_2().child(page::section_label("Delay before answering (ms)", cx)).child(
                    h_flex().gap_4().child(div().w_32().child(Input::new(&self.delay).small())).child(
                        Self::pass_through("mock-delay", self.mock_delay, "delay", cx, |this, on| this.mock_delay = on),
                    ),
                ))
                .child(
                    v_flex().gap_2().child(page::section_label("How the body is sent", cx)).child(
                        h_flex()
                            .gap_4()
                            .child(div().w_56().child(Select::new(&self.streaming).small()))
                            .child(
                                div().text_sm().text_color(theme.muted_foreground).child("Pause between chunks (ms)"),
                            )
                            .child(div().w_24().child(Input::new(&self.chunk_delay).small())),
                    ),
                )
                .into_any_element(),
        };

        v_flex()
            .size_full()
            .bg(theme.background)
            .child(
                h_flex()
                    .flex_none()
                    .gap_4()
                    .px_3()
                    .pt_2()
                    .child(div().text_sm().text_color(theme.muted_foreground).child("Status"))
                    .child(div().w_24().child(Input::new(&self.status).small()))
                    .child(Self::pass_through("mock-status", self.mock_status, "status", cx, |this, on| {
                        this.mock_status = on
                    })),
            )
            .child(
                div().flex_none().px_3().child(
                    TabBar::new("mock-sections")
                        .underline()
                        .small()
                        .selected_index(SECTIONS.iter().position(|s| *s == self.section).unwrap_or(0))
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.section = SECTIONS[*ix];
                            cx.notify();
                        }))
                        .child(Tab::new().label("Body"))
                        .child(Tab::new().label(if headers == 0 {
                            "Headers".to_owned()
                        } else {
                            format!("Headers ({headers})")
                        }))
                        .child(Tab::new().label("Timing")),
                ),
            )
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}
