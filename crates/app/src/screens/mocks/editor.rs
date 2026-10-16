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

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use gpui_kit::component::{
    ActiveTheme as _, IndexPath, Sizable as _, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Editor, Input, InputEvent, InputState},
    notification::Notification,
    select::{Select, SelectEvent, SelectState},
    switch::Switch,
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, SharedString,
    Styled as _, Subscription, Window, div,
};
use serde_json::{Map, Value, json};

use crate::{
    app_model::{AppModel, ModelEvent},
    components::{
        Autosave, KeyValueEvent, KeyValueTable, Syntax, VarEditor,
        binary::{BinaryBody, save_to_file, suggested_file_name},
        page,
    },
    engine::{call_in, describe},
    i18n::{t, t_with},
    model::{
        http::{BodyEncoding, HTTP_METHODS, HttpResponse, header_map, header_pairs, pretty_json},
        media::{self, BodyClass},
    },
    session::{LoadState, SaveMode, engine_folder},
};

const DEFAULT_NAME: &str = "New mock";
/// The types a mock with no real response behind it can answer with. They are all text.
const CONTENT_TYPES: [&str; 7] = [
    "application/json",
    "text/plain",
    "text/html",
    "application/xml",
    "text/css",
    "application/javascript",
    "text/csv",
];
/// Headers that describe how the real server framed its body. A mock never sets them.
const FRAMING_HEADERS: [&str; 5] =
    ["connection", "content-length", "transfer-encoding", "keep-alive", "content-encoding"];
/// The engine's value for "any method"; the editor shows it as a word.
const ANY_METHOD: &str = "*";
const ANY_METHOD_LABEL: &str = "ANY";
const STREAMING: [(&str, &str); 3] = [
    ("none", "mocks.editor.streaming.all_at_once"),
    ("sse", "mocks.editor.streaming.sse"),
    ("chunked", "mocks.editor.streaming.chunked"),
];

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
    /// The body as text. Empty when the body is data.
    body: String,
    /// The body is data, not text: the mock answers with an image or another binary type.
    is_data: bool,
    /// The data, in base64. `None` when the body is text, or no file has been chosen yet.
    data: Option<Arc<str>>,
    mock_body: bool,
    delay: String,
    mock_delay: bool,
    streaming: String,
    chunk_delay: String,
    /// There is no real response behind this mock: it answers on its own, with the type it was given.
    standalone: bool,
    content_type: String,
}

/// The response controls as they stand, to tell whether anyone has edited them since they were filled in.
#[derive(Debug, Clone, PartialEq, Default)]
struct ResponseState {
    status: String,
    body: String,
    headers: Vec<(String, String)>,
    content_type: String,
    standalone: bool,
}

/// `request.replay` can only be sent for a URL that names its host, and only for a method that reads.
fn probe_url(method: &str, pattern: &str, use_regex: bool) -> Option<String> {
    let reads = matches!(method, ANY_METHOD_LABEL | "GET" | "HEAD");
    let absolute = pattern.starts_with("http://") || pattern.starts_with("https://");
    (reads && absolute && !use_regex && !pattern.contains("{{")).then(|| pattern.to_owned())
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
    /// The folder the engine has it in. It differs from `folder_id` while a move is waiting to be written.
    saved_folder: Option<String>,
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
    /// See `content_type`.
    locked_content_type: String,
    /// The body when it is an image or other data: its bytes in base64, as the engine stores them. Which of
    /// this and the text editor is the body follows the `Content-Type` the mock answers with.
    data: Option<Arc<str>>,
    /// `data`, decoded once for showing.
    data_view: Option<BinaryBody>,
    /// What choosing or saving a file came to, when there is something to say.
    data_note: Option<SharedString>,
    delay: Entity<InputState>,
    mock_delay: bool,
    streaming: Entity<SelectState<Vec<&'static str>>>,
    chunk_delay: Entity<InputState>,
    /// See [`Fields::standalone`]. Set when the real server has nothing at the URL, or could not be asked.
    standalone: bool,
    content_type_select: Entity<SelectState<Vec<&'static str>>>,
    /// The real response's headers as they were when it was fetched. A header in the table that still
    /// matches one of these passes through; one that was changed or added is what the mock sets.
    probed_headers: Vec<(String, String)>,
    /// The method and URL the real response was last asked for.
    probe_key: Option<(String, String)>,
    probing: bool,
    probe_note: Option<SharedString>,
    /// The response controls as they were last filled in; while they still match, a new answer may replace them.
    pristine: ResponseState,
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
        let name = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t("mocks.editor.name_placeholder")).default_value(DEFAULT_NAME)
        });
        let method = cx.new(|cx| SelectState::new(method_options(), Some(IndexPath::default()), window, cx));
        let pattern = cx.new(|cx| InputState::new(window, cx).placeholder(t("mocks.editor.pattern_placeholder")));
        let status = cx.new(|cx| InputState::new(window, cx).placeholder("200").default_value("200"));
        let delay = cx.new(|cx| InputState::new(window, cx).placeholder("0").default_value("0"));
        let chunk_delay = cx.new(|cx| InputState::new(window, cx).placeholder("100").default_value("100"));
        let streaming = cx.new(|cx| {
            SelectState::new(STREAMING.map(|(_, key)| t(key)).to_vec(), Some(IndexPath::default()), window, cx)
        });
        let content_type_select =
            cx.new(|cx| SelectState::new(CONTENT_TYPES.to_vec(), Some(IndexPath::default()), window, cx));
        let headers =
            cx.new(|cx| KeyValueTable::new(t("mocks.editor.table.header"), t("mocks.editor.table.value"), window, cx));
        let body = VarEditor::code(Syntax::Json, t("mocks.editor.body_label"), window, cx);

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
        for input in [&status, &delay, &chunk_delay] {
            subscriptions.push(cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.changed(cx);
                }
            }));
        }
        // Entering the URL is what asks the real server for its response.
        subscriptions.push(cx.subscribe_in(&pattern, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.changed(cx),
            InputEvent::PressEnter { .. } | InputEvent::Blur => this.maybe_probe(window, cx),
            InputEvent::Focus => {}
        }));
        subscriptions.push(cx.subscribe_in(
            &method,
            window,
            |this, _, _: &SelectEvent<Vec<&'static str>>, window, cx| {
                this.changed(cx);
                this.maybe_probe(window, cx);
            },
        ));
        subscriptions
            .push(cx.subscribe(&streaming, |this, _, _: &SelectEvent<Vec<&'static str>>, cx| this.changed(cx)));
        subscriptions.push(cx.subscribe(
            &content_type_select,
            |this, _, event: &SelectEvent<Vec<&'static str>>, cx| {
                let SelectEvent::Confirm(Some(value)) = event else { return };
                if this.standalone && this.locked_content_type != *value {
                    this.locked_content_type = (*value).to_owned();
                    this.follow_content_type(cx);
                    this.changed(cx);
                }
            },
        ));
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
            saved_folder: folder_id.clone(),
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
            locked_content_type: String::new(),
            data: None,
            data_view: None,
            data_note: None,
            delay,
            mock_delay: true,
            streaming,
            chunk_delay,
            standalone: false,
            content_type_select,
            probed_headers: Vec::new(),
            probe_key: None,
            probing: false,
            probe_note: None,
            pristine: ResponseState::default(),
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
                is_data: false,
                data: None,
                mock_body: true,
                delay: String::new(),
                mock_delay: true,
                streaming: String::new(),
                chunk_delay: String::new(),
                standalone: false,
                content_type: String::new(),
            },
            saving: false,
            autosave: Autosave::new(window),
            _subscriptions: subscriptions,
        };
        this.baseline = this.fields(cx);
        this.pristine = this.response_state(cx);
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
                    .ok_or_else(|| t("mocks.editor.error.gone").to_owned())
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

    pub fn folder_id(&self) -> Option<&str> {
        self.folder_id.as_deref()
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
        self.load == LoadState::Ready && (self.fields(cx) != self.baseline || self.folder_id != self.saved_folder)
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
        self.edited_entity(&fields).err().map(|reason| t_with("mocks.editor.not_saved", &[("reason", &reason)]).into())
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

    fn streaming_value(&self, cx: &gpui_kit::App) -> &'static str {
        let label = self.streaming.read(cx).selected_value().copied().unwrap_or(t(STREAMING[0].1));
        STREAMING.iter().find(|(_, key)| t(key) == label).map_or(STREAMING[0].0, |(value, _)| value)
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
            headers: self
                .headers
                .read(cx)
                .pairs(cx)
                .into_iter()
                .filter(|(name, _)| !name.trim().eq_ignore_ascii_case("content-type"))
                .collect(),
            body: if self.is_data(cx) { String::new() } else { self.body.value(cx) },
            is_data: self.is_data(cx),
            data: self.data.clone().filter(|_| self.is_data(cx)),
            // Data (an image, a PDF) cannot be mocked: the real response goes through.
            mock_body: self.mock_body && !self.is_data(cx),
            delay: text(&self.delay),
            mock_delay: self.mock_delay,
            streaming: self.streaming_value(cx).to_owned(),
            chunk_delay: text(&self.chunk_delay),
            standalone: self.standalone,
            content_type: self.locked_content_type.clone(),
        }
    }

    fn response_state(&self, cx: &gpui_kit::App) -> ResponseState {
        ResponseState {
            status: self.status.read(cx).value().trim().to_owned(),
            body: self.body.value(cx),
            headers: self.headers.read(cx).pairs(cx),
            content_type: self.locked_content_type.clone(),
            standalone: self.standalone,
        }
    }

    /// Shows the chosen type in the type list, when it is one of them.
    fn sync_content_type_select(&self, window: &mut Window, cx: &mut Context<Self>) {
        let content_type = media::essence(&self.locked_content_type);
        if let Some(listed) = CONTENT_TYPES.iter().find(|t| **t == content_type) {
            self.content_type_select.update(cx, |s, cx| s.set_selected_value(listed, window, cx));
        }
    }

    /// Turns the mock into the whole response (or back into a change to the real server's).
    fn set_standalone(&mut self, standalone: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.standalone = standalone;
        if standalone {
            if self.locked_content_type.is_empty() || media::classify(&self.locked_content_type) != BodyClass::Text {
                self.locked_content_type = CONTENT_TYPES[0].to_owned();
            }
            // Whatever data the real response had cannot be what the mock answers with.
            self.data = None;
            self.data_view = None;
            self.sync_content_type_select(window, cx);
        }
        self.follow_content_type(cx);
        self.changed(cx);
    }

    /// Asks the real server what it answers at this URL, to start the mock from. Only a draft whose response
    /// has not been touched is filled in: what someone typed is never replaced.
    fn maybe_probe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saved_id.is_some() || self.load != LoadState::Ready {
            return;
        }
        let pattern = self.pattern.read(cx).value().trim().to_owned();
        if pattern.is_empty() {
            return;
        }
        let method = self.badge(cx);
        let key = (method.to_owned(), pattern.clone());
        if self.probe_key.as_ref() == Some(&key) || self.response_state(cx) != self.pristine {
            return;
        }
        self.probe_key = Some(key.clone());
        let Some(url) = probe_url(method, &pattern, self.use_regex) else {
            self.probing = false;
            self.apply_standalone(t("mocks.editor.probe.no_url").into(), window, cx);
            return;
        };
        self.probing = true;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        let method = if method == ANY_METHOD_LABEL { "GET" } else { method };
        // A mock started from a command asks with the command's headers, which is how the real server knows the caller.
        let headers = self.entity.get("capturedHeaders").filter(|headers| headers.is_object()).cloned();
        let payload =
            json!({ "method": method, "url": url, "headers": headers.unwrap_or_else(|| json!({})), "body": "" });
        call_in(&client, window, cx, "request.replay", payload, move |this, outcome, window, cx| {
            if this.probe_key.as_ref() != Some(&key) {
                return;
            }
            this.probing = false;
            if this.response_state(cx) != this.pristine {
                cx.notify();
                return;
            }
            let response = outcome.as_ref().ok().and_then(|v| HttpResponse::from_value(v, BodyEncoding::Base64));
            match (outcome, response) {
                (Err(error), _) => this.apply_standalone(
                    t_with("mocks.editor.probe.unreachable", &[("error", &describe(&error))]).into(),
                    window,
                    cx,
                ),
                (Ok(_), Some(response)) if matches!(response.status, 404 | 410) => this.apply_standalone(
                    t_with("mocks.editor.probe.not_found", &[("status", &response.status)]).into(),
                    window,
                    cx,
                ),
                (Ok(value), Some(response)) => {
                    let bytes = value
                        .get("body")
                        .and_then(Value::as_str)
                        .and_then(|body| STANDARD.decode(body.trim()).ok())
                        .unwrap_or_default();
                    this.apply_probe(&response, bytes, window, cx);
                }
                (Ok(_), None) => this.apply_standalone(t("mocks.editor.probe.unreadable").into(), window, cx),
            }
        });
    }

    /// Nothing real to start from: the response is empty and the mock is all of it.
    fn apply_standalone(&mut self, note: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        self.standalone = true;
        self.probed_headers.clear();
        self.locked_content_type = CONTENT_TYPES[0].to_owned();
        self.data = None;
        self.data_view = None;
        self.data_note = None;
        self.status.update(cx, |s, cx| s.set_value("200", window, cx));
        self.headers.update(cx, |table, cx| table.set_pairs(&[], window, cx));
        self.body.set_value(String::new(), window, cx);
        (self.mock_status, self.mock_body, self.mock_delay) = (true, true, true);
        self.probe_note = Some(note);
        self.sync_content_type_select(window, cx);
        self.follow_content_type(cx);
        self.pristine = self.response_state(cx);
        self.changed(cx);
    }

    /// Fills the mock with the real response, every part of it left to the real server until it is switched on.
    fn apply_probe(&mut self, response: &HttpResponse, bytes: Vec<u8>, window: &mut Window, cx: &mut Context<Self>) {
        self.standalone = false;
        let content_type = response.content_type().unwrap_or_default().to_owned();
        let headers: Vec<(String, String)> = response
            .headers
            .iter()
            .filter(|(name, _)| {
                !name.eq_ignore_ascii_case("content-type")
                    && !FRAMING_HEADERS.iter().any(|h| name.eq_ignore_ascii_case(h))
            })
            .cloned()
            .collect();
        self.locked_content_type = content_type.clone();
        self.probed_headers = headers.clone();
        self.headers.update(cx, |table, cx| table.set_pairs(&headers, window, cx));
        self.status.update(cx, |s, cx| s.set_value(response.status.to_string(), window, cx));
        self.delay.update(cx, |s, cx| s.set_value("0", window, cx));
        (self.mock_status, self.mock_body, self.mock_delay) = (false, false, false);
        self.data_note = None;
        if media::classify(&content_type) == BodyClass::Text {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let text = if content_type.to_ascii_lowercase().contains("json") {
                pretty_json(&text).unwrap_or(text)
            } else {
                text
            };
            self.data = None;
            self.body.set_value(text, window, cx);
        } else {
            // Only the type is kept: the real server's body goes through as it is.
            self.data = None;
            self.body.set_value(String::new(), window, cx);
        }
        self.show_data(cx);
        self.follow_content_type(cx);
        let kind = media::essence(&content_type);
        let started = if kind.is_empty() {
            t_with("mocks.editor.probe.started", &[("status", &response.status)])
        } else {
            t_with("mocks.editor.probe.started_with_type", &[("status", &response.status), ("type", &kind)])
        };
        self.probe_note = Some(started.into());
        self.pristine = self.response_state(cx);
        self.changed(cx);
    }

    /// The `Content-Type` of the real server's response, kept as the mock was made from it. It is locked: the
    /// header table does not show it, a mock never overrides it, and it decides whether the body can be mocked.
    fn content_type(&self, _cx: &gpui_kit::App) -> String {
        self.locked_content_type.clone()
    }

    /// The body is an image or other data rather than text: the response type says so, or the stored body
    /// is bytes and nothing says what they are.
    fn is_data(&self, cx: &gpui_kit::App) -> bool {
        let content_type = self.content_type(cx);
        !self.standalone
            && (media::classify(&content_type) != BodyClass::Text || (content_type.is_empty() && self.data.is_some()))
    }

    /// The body is highlighted for the `Content-Type` the mock answers with.
    fn follow_content_type(&mut self, cx: &mut Context<Self>) {
        let content_type = self.content_type(cx);
        self.body.set_syntax(Syntax::for_content_type(&content_type), cx);
        // The preview is drawn for the type the bytes are said to be.
        if self.data_view.as_ref().is_some_and(|view| view.content_type() != content_type) {
            self.show_data(cx);
        }
    }

    /// Decodes `data` for showing. Done when the data or its type changes, not while drawing.
    fn show_data(&mut self, cx: &gpui_kit::App) {
        let content_type = self.content_type(cx);
        self.data_view = self
            .data
            .as_deref()
            .and_then(|data| STANDARD.decode(data.trim()).ok())
            .map(|bytes| BinaryBody::new(bytes, content_type));
    }

    fn save_data(&mut self, cx: &mut Context<Self>) {
        let Some(view) = &self.data_view else { return };
        let name = suggested_file_name(&self.pattern.read(cx).value(), "mock-body", view.content_type());
        save_to_file(view.bytes().to_vec(), name, cx, |this, outcome, cx| {
            this.data_note = Some(match outcome {
                Ok(path) => t_with("mocks.editor.saved_to", &[("path", &path.display())]).into(),
                Err(error) => t_with("mocks.editor.save_file_failed", &[("error", &error)]).into(),
            });
            cx.notify();
        });
    }

    fn adopt_entity(&mut self, entity: Map<String, Value>, window: &mut Window, cx: &mut Context<Self>) {
        let text = |key: &str| entity.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        let flag = |key: &str, default: bool| entity.get(key).and_then(Value::as_bool).unwrap_or(default);
        let number = |key: &str, default: u64| entity.get(key).and_then(Value::as_u64).unwrap_or(default).to_string();
        self.folder_id = entity.get("folderId").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
        self.saved_folder = self.folder_id.clone();

        let method = text("method").to_uppercase();
        let method = method_options().into_iter().find(|m| *m == method).unwrap_or(ANY_METHOD_LABEL);
        let streaming = text("streamingMode");
        let streaming =
            STREAMING.iter().find(|(value, _)| *value == streaming).map_or(t(STREAMING[0].1), |(_, key)| t(key));

        self.name.update(cx, |s, cx| s.set_value(text("name"), window, cx));
        self.method.update(cx, |s, cx| s.set_selected_value(&method, window, cx));
        self.pattern.update(cx, |s, cx| s.set_value(text("urlPattern"), window, cx));
        self.use_regex = flag("useRegex", false);
        self.enabled = flag("enabled", true);
        self.status.update(cx, |s, cx| s.set_value(number("responseStatus", 200), window, cx));
        self.mock_status = flag("responseStatusMocked", true);
        let (locked, headers): (Vec<_>, Vec<_>) = header_pairs(entity.get("responseHeaders").unwrap_or(&Value::Null))
            .into_iter()
            .partition(|(name, _)| name.eq_ignore_ascii_case("content-type"));
        self.locked_content_type = locked.into_iter().next().map(|(_, value)| value).unwrap_or_default();
        self.headers.update(cx, |table, cx| table.set_pairs(&headers, window, cx));
        let stored_as_bytes = entity.get("responseBodyEncoding").and_then(Value::as_str) == Some("base64");
        let class = media::classify(&self.content_type(cx));
        let body = text("responseBody");
        self.data_note = None;
        if stored_as_bytes && class == BodyClass::Text && !self.content_type(cx).is_empty() {
            // Bytes under a text type are text: they are shown, and saved again, as text.
            let decoded = STANDARD.decode(body.trim()).map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
            self.data = None;
            self.body.set_value(decoded.unwrap_or(body), window, cx);
        } else if stored_as_bytes {
            self.data = Some(body.into());
            self.body.set_value(String::new(), window, cx);
        } else {
            self.data = None;
            // A text body under a data type cannot be what the mock answers with; it is not carried over.
            self.body.set_value(if class == BodyClass::Text { body } else { String::new() }, window, cx);
        }
        self.show_data(cx);
        self.follow_content_type(cx);
        self.mock_body = flag("responseBodyMocked", true);
        self.delay.update(cx, |s, cx| s.set_value(number("responseDelay", 0), window, cx));
        self.mock_delay = flag("responseDelayMocked", true);
        self.streaming.update(cx, |s, cx| s.set_selected_value(&streaming, window, cx));
        self.chunk_delay.update(cx, |s, cx| s.set_value(number("streamingChunkDelay", 100), window, cx));
        self.standalone = flag("standalone", false);
        self.probed_headers = header_pairs(entity.get("probedHeaders").unwrap_or(&Value::Null));
        self.probe_note = None;
        self.sync_content_type_select(window, cx);

        self.entity = entity;
        self.baseline = self.fields(cx);
        self.pristine = self.response_state(cx);
    }

    /// Fills this draft with a mock the engine built, from a cURL command. It stays a draft, in the folder it
    /// was started in, and what was filled in is its first edit. The command says what is called, not what
    /// answers, so the real server is asked as it is when a URL is typed.
    pub fn adopt_import(&mut self, entity: Map<String, Value>, window: &mut Window, cx: &mut Context<Self>) {
        let (folder, baseline) = (self.folder_id.clone(), self.baseline.clone());
        self.adopt_entity(entity, window, cx);
        (self.folder_id, self.saved_folder) = (folder.clone(), folder);
        self.baseline = baseline;
        self.changed(cx);
        self.maybe_probe(window, cx);
    }

    /// The loaded entity with the editor's fields written over it, or what stops it from being saved.
    fn edited_entity(&self, fields: &Fields) -> Result<Map<String, Value>, &'static str> {
        if fields.pattern.is_empty() {
            return Err(t("mocks.editor.error.pattern_required"));
        }
        let status: u16 =
            fields.status.parse().ok().filter(|s| (100..=599).contains(s)).ok_or(t("mocks.editor.error.status"))?;
        let delay: u64 = fields.delay.parse().map_err(|_| t("mocks.editor.error.delay"))?;
        let chunk_delay: u64 = fields.chunk_delay.parse().map_err(|_| t("mocks.editor.error.chunk_delay"))?;

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
        entity.insert("standalone".into(), json!(fields.standalone));
        // With no real response behind it the mock is every part of the answer.
        entity.insert("responseStatusMocked".into(), json!(fields.mock_status || fields.standalone));
        let mut response_headers = header_map(&fields.headers);
        if !fields.content_type.is_empty() {
            response_headers.insert("Content-Type".into(), json!(fields.content_type));
        }
        entity.insert("responseHeaders".into(), Value::Object(response_headers));
        entity.insert("probedHeaders".into(), Value::Object(header_map(&self.probed_headers)));
        // A header the mock sets is one that differs from what the real server sent, or that it did not send.
        // The rest are left to the real server.
        let unchanged = |name: &str, value: &str| {
            self.probed_headers.iter().any(|(n, v)| n.eq_ignore_ascii_case(name) && v == value)
        };
        entity.insert(
            "mockedResponseHeaders".into(),
            Value::Array(
                fields
                    .headers
                    .iter()
                    .filter(|(name, value)| fields.standalone || !unchanged(name, value))
                    .map(|(name, _)| json!(name))
                    .collect(),
            ),
        );
        if fields.is_data {
            // Bytes are stored as they are. A mock that replaces the body needs some to replace it with.
            match &fields.data {
                Some(data) => entity.insert("responseBody".into(), json!(data.as_ref())),
                None if fields.mock_body => {
                    return Err(t("mocks.editor.error.body_file"));
                }
                None => entity.insert("responseBody".into(), json!("")),
            };
            entity.insert("responseBodyEncoding".into(), json!("base64"));
        } else {
            entity.insert("responseBody".into(), json!(fields.body));
            entity.remove("responseBodyEncoding");
        }
        entity.insert("responseBodyMocked".into(), json!(fields.mock_body || fields.standalone));
        entity.insert("responseDelay".into(), json!(delay));
        entity.insert("responseDelayMocked".into(), json!(fields.mock_delay || fields.standalone));
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
        if !creating && fields == self.baseline && self.folder_id == self.saved_folder {
            return;
        }
        let mut entity = match self.edited_entity(&fields) {
            Ok(entity) => entity,
            Err(reason) => {
                if mode == SaveMode::Explicit {
                    window.push_notification(Notification::warning(reason).title(t("mocks.editor.cant_save_yet")), cx);
                }
                return;
            }
        };
        entity.insert("workspaceId".into(), json!(self.workspace_id));
        entity.entry("protocol").or_insert(json!("rest"));
        entity.entry("capturedHeaders").or_insert(json!({}));
        entity.entry("capturedBody").or_insert(json!(""));
        entity.insert("folderId".into(), json!(self.folder_id));
        let folder = self.folder_id.clone();

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
                        let placed = engine_folder(&saved);
                        this.entity = saved;
                        this.baseline = fields;
                        if this.folder_id == folder {
                            this.folder_id = placed.clone();
                        }
                        this.saved_folder = placed;
                        if let Some(id) = this.saved_id.clone() {
                            cx.emit(MockEvent::Saved { id });
                        }
                    }
                    None => window.push_notification(Notification::error(t("mocks.editor.error.no_saved_mock")), cx),
                },
                Ok(_) => {
                    this.entity = sent;
                    this.baseline = fields;
                    this.saved_folder = folder.clone();
                }
                Err(error) => {
                    window.push_notification(
                        Notification::error(describe(&error)).title(t("mocks.editor.error.save_title")),
                        cx,
                    );
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
            LoadState::Loading => Some(page::loading(t("mocks.editor.loading"))),
            LoadState::Failed(message) => Some(
                div()
                    .size_full()
                    .p_4()
                    .child(
                        Alert::error("load-failed", SharedString::from(message.clone()))
                            .title(t("mocks.editor.error.open_title")),
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
                    .child(div().flex_1().min_w_0().child(VarEditor::row_input(&self.pattern, cx))),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_6()
                    .px_3()
                    .pb_3()
                    .child(
                        Switch::new("mock-enabled")
                            .checked(self.enabled)
                            .label(t("mocks.editor.enabled"))
                            .small()
                            .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                this.enabled = *checked;
                                this.changed(cx);
                            })),
                    )
                    .child(
                        Checkbox::new("mock-regex")
                            .checked(self.use_regex)
                            .small()
                            .label(t("mocks.editor.regex"))
                            .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                this.use_regex = *checked;
                                this.changed(cx);
                            })),
                    )
                    .child(
                        Switch::new("mock-standalone")
                            .checked(self.standalone)
                            .label(t("mocks.editor.standalone"))
                            .small()
                            .on_change(cx.listener(|this, checked: &bool, window, cx| {
                                this.set_standalone(*checked, window, cx);
                            })),
                    ),
            )
            .child(div().px_3().text_sm().text_color(theme.muted_foreground).child(t("mocks.editor.rule_help")))
            .children(
                self.probing
                    .then(|| t("mocks.editor.probe.fetching").into())
                    .or_else(|| self.probe_note.clone())
                    .map(|note: SharedString| div().px_3().pt_1().text_sm().text_color(theme.info).child(note)),
            )
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
            .label(t_with("mocks.editor.pass_through.label", &[("what", &what)]))
            .tooltip(t_with("mocks.editor.pass_through.tooltip", &[("what", &what)]))
            .on_change(cx.listener(move |this, checked: &bool, _, cx| {
                set(this, *checked);
                this.changed(cx);
            }))
    }

    /// The body, as the response type allows it to be: text in an editor, or for an image or other data a
    /// preview of what the real server sent (data is never replaced). The type is the locked `Content-Type`.
    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let is_data = self.is_data(cx);
        let content_type = media::essence(&self.content_type(cx));
        let shown_type = if content_type.is_empty() { t("mocks.editor.unknown_type") } else { content_type.as_str() };
        let unsupported: SharedString = t_with("mocks.editor.unsupported_type", &[("type", &shown_type)]).into();
        let kind: AnyElement = if self.standalone {
            div()
                .w_56()
                .child(Select::new(&self.content_type_select).small().placeholder(content_type))
                .into_any_element()
        } else {
            Tag::secondary()
                .small()
                .child(if content_type.is_empty() {
                    t("mocks.editor.no_content_type").to_owned()
                } else {
                    content_type
                })
                .into_any_element()
        };
        let toolbar = h_flex()
            .flex_none()
            .px_3()
            .py_2()
            .gap_3()
            .children((!is_data && !self.standalone).then(|| {
                Self::pass_through("mock-body", self.mock_body, t("mocks.editor.part.body"), cx, |this, on| {
                    this.mock_body = on
                })
            }))
            .child(kind)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_sm()
                    .text_color(theme.warning)
                    .children(is_data.then(|| self.data_note.clone().unwrap_or(unsupported.clone()))),
            )
            .children((is_data && self.data_view.is_some()).then(|| {
                Button::new("mock-body-save")
                    .ghost()
                    .small()
                    .label(t("mocks.editor.save_file"))
                    .on_click(cx.listener(|this, _, _, cx| this.save_data(cx)))
            }));
        let content = match (&self.data_view, is_data) {
            (_, false) => Editor::new(self.body.state()).bordered(false).h_full().into_any_element(),
            (Some(view), true) => view.render(cx),
            (None, true) => page::empty(unsupported, t("mocks.editor.data_help")),
        };
        v_flex().size_full().child(toolbar).child(div().flex_1().min_h_0().child(content)).into_any_element()
    }

    /// What the mock answers with: status, body, headers and timing.
    pub fn render_response(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some(placeholder) = self.placeholder() {
            return placeholder;
        }
        let theme = cx.theme().clone();
        let headers = self.headers.read(cx).pairs(cx).len();
        let standalone = self.standalone;
        let content = match self.section {
            Section::Body => self.render_body(cx),
            Section::Headers => v_flex()
                .size_full()
                .child(
                    v_flex()
                        .flex_none()
                        .px_3()
                        .py_2()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(if self.standalone {
                            t("mocks.editor.headers.standalone").to_owned()
                        } else if self.locked_content_type.is_empty() {
                            t("mocks.editor.headers.locked").to_owned()
                        } else {
                            t_with("mocks.editor.headers.locked_to", &[("type", &self.locked_content_type)])
                        })
                        .children(
                            (!self.standalone && !self.probed_headers.is_empty())
                                .then(|| t("mocks.editor.headers.help")),
                        ),
                )
                .child(div().flex_1().min_h_0().child(self.headers.clone()))
                .into_any_element(),
            Section::Timing => v_flex()
                .size_full()
                .p_3()
                .gap_4()
                .child(v_flex().gap_2().child(page::section_label(t("mocks.editor.timing.delay_label"), cx)).child(
                    h_flex().gap_4().child(div().w_32().child(Input::new(&self.delay).small())).children(
                        (!standalone).then(|| {
                            Self::pass_through(
                                "mock-delay",
                                self.mock_delay,
                                t("mocks.editor.part.delay"),
                                cx,
                                |this, on| this.mock_delay = on,
                            )
                        }),
                    ),
                ))
                .child(
                    v_flex().gap_2().child(page::section_label(t("mocks.editor.timing.streaming_label"), cx)).child(
                        h_flex()
                            .gap_4()
                            .child(div().w_56().child(Select::new(&self.streaming).small()))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child(t("mocks.editor.timing.chunk_label")),
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
                    .child(div().text_sm().text_color(theme.muted_foreground).child(t("mocks.editor.status")))
                    .child(div().w_24().child(Input::new(&self.status).small()))
                    .children((!standalone).then(|| {
                        Self::pass_through(
                            "mock-status",
                            self.mock_status,
                            t("mocks.editor.part.status"),
                            cx,
                            |this, on| this.mock_status = on,
                        )
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
                        .child(Tab::new().label(t("mocks.editor.tab.body")))
                        .child(Tab::new().label(if headers == 0 {
                            t("mocks.editor.tab.headers").to_owned()
                        } else {
                            t_with("mocks.editor.tab.headers_count", &[("count", &headers)])
                        }))
                        .child(Tab::new().label(t("mocks.editor.tab.timing"))),
                ),
            )
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}
