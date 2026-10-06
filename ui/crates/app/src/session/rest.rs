//! An open REST request: the editor, its save/send commands and its response.
//!
//! The saved request lives in the engine; this entity holds an *edit* of it. Fields are read from the
//! controls when needed (the controls are the single source of truth for what is typed), compared to a
//! baseline taken at the last load or save to know if there is anything to save, and written back as one
//! `entity.update`. Fields the editor does not show — ids, timestamps — are carried through unchanged from
//! the entity the engine returned, so saving here cannot erase what another client set.
//!
//! A draft is created by Save. Once it exists there is no Save step: an edit is written a moment after it
//! is made. The request's name is edited in the window's breadcrumb, which shows this session's name field.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use gpui_kit::component::{
    ActiveTheme as _, IndexPath, Sizable as _, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, InputEvent, InputState},
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

use super::{Carry, LoadState, SaveMode, SessionEvent, engine_folder, exchange::Exchange};
use crate::{
    app_model::{AppModel, ModelEvent},
    components::{
        Autosave, FileData, KeyValueEvent, KeyValueTable, Part, PartValue, PartsEvent, PartsTable, Syntax, VarEditor,
        binary::{BinaryBody, pick_file},
        page,
        parts_table::{parts_from_json, parts_to_json},
        request_bar,
    },
    engine::{call_in, describe},
    model::{
        RequestKind,
        http::{
            BodyEncoding, BodyMode, HTTP_METHODS, decode_form, encode_form, header_map, header_pairs, join_query,
            pretty_json, split_query,
        },
        media::{BodyClass, classify, size_label},
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
    /// The fields of a multipart body; empty in every other mode.
    parts: Vec<Part>,
    /// The file that is the body; `None` in every other mode.
    file: Option<FileData>,
    pre_script: String,
    post_script: String,
    test_script: String,
}

/// The file as it can be previewed: an image is decoded once, when the file arrives.
fn preview_of(file: &FileData) -> Option<BinaryBody> {
    if classify(&file.content_type) != BodyClass::Image {
        return None;
    }
    let bytes = STANDARD.decode(&*file.data).ok()?;
    Some(BinaryBody::new(bytes, file.content_type.clone())).filter(BinaryBody::is_image)
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
    /// The folder the engine has it in. It differs from `folder_id` while a move is waiting to be written.
    saved_folder: Option<String>,
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
    parts: Entity<PartsTable>,
    /// The file that is the body in File mode. Kept when the mode changes, so switching back finds it.
    file: Option<FileData>,
    /// The file shown as an image, built when `file` is set.
    file_preview: Option<BinaryBody>,
    /// A file just chosen, applied by the next render: it needs a window to update the header table.
    pending_file: Option<FileData>,
    file_error: Option<SharedString>,
    pre_script: VarEditor,
    post_script: VarEditor,
    test_script: VarEditor,
    section: Section,
    script: Script,
    baseline: Fields,
    exchange: Entity<Exchange>,
    sending: bool,
    saving: bool,
    autosave: Autosave,
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
        let parts = cx.new(|cx| PartsTable::new(window, cx));
        let body = VarEditor::code(Syntax::Json, "Request body", window, cx);
        let pre_script = VarEditor::code(Syntax::JavaScript, "// Runs before the request is sent", window, cx);
        let post_script = VarEditor::code(Syntax::JavaScript, "// Runs after the response arrives", window, cx);
        let test_script = VarEditor::code(Syntax::JavaScript, "// test(\"status is 200\", () => { ... })", window, cx);
        let exchange = cx.new(|cx| Exchange::new(window, cx));

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
        subscriptions.push(cx.subscribe(&parts, |this, _, _: &PartsEvent, cx| this.changed(cx)));
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
            saved_folder: folder_id.clone(),
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
            parts,
            file: None,
            file_preview: None,
            pending_file: None,
            file_error: None,
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
                parts: Vec::new(),
                file: None,
                pre_script: String::new(),
                post_script: String::new(),
                test_script: String::new(),
            },
            exchange,
            sending: false,
            saving: false,
            autosave: Autosave::new(window),
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
        let payload = json!({ "workspaceId": self.workspace_id, "kind": RequestKind::Rest.entity_kind(), "id": id });
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
        if name.is_empty() { DEFAULT_NAME.into() } else { name }
    }

    pub fn method(&self, cx: &gpui_kit::App) -> &'static str {
        self.method.read(cx).selected_value().copied().unwrap_or("GET")
    }

    pub fn is_dirty(&self, cx: &gpui_kit::App) -> bool {
        self.load == LoadState::Ready && (self.fields(cx) != self.baseline || self.folder_id != self.saved_folder)
    }

    /// The request as the editor holds it, for writing out as a code snippet.
    pub fn request_parts(&self, cx: &gpui_kit::App) -> RequestParts {
        let fields = self.fields(cx);
        RequestParts { method: fields.method, url: fields.url, headers: fields.headers, body: fields.body }
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
            body: fields.body,
        }
    }

    /// Fills this draft with what was typed into a draft of another type.
    pub fn apply_carry(&mut self, carry: &Carry, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(name) = &carry.name {
            self.name.update(cx, |s, cx| s.set_value(name.clone(), window, cx));
        }
        self.url.set_value(carry.url.clone(), window, cx);
        let (_, query) = split_query(&carry.url);
        self.params.update(cx, |table, cx| table.set_pairs(&query, window, cx));
        self.headers.update(cx, |table, cx| table.set_pairs(&carry.headers, window, cx));
        self.show_body(carry.body.clone(), window, cx);
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

    fn fields(&self, cx: &gpui_kit::App) -> Fields {
        let body = match self.body_mode {
            BodyMode::None | BodyMode::Multipart | BodyMode::File => String::new(),
            BodyMode::Form => encode_form(&self.form.read(cx).pairs(cx)),
            _ => self.body.value(cx),
        };
        let parts = if self.body_mode == BodyMode::Multipart { self.parts.read(cx).parts(cx) } else { Vec::new() };
        let file = if self.body_mode == BodyMode::File { self.file.clone() } else { None };
        Fields {
            parts,
            file,
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
            BodyMode::None | BodyMode::Text | BodyMode::Form | BodyMode::Multipart | BodyMode::File => Syntax::Plain,
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
            (BodyMode::Form, BodyMode::Multipart) => {
                let parts: Vec<Part> = self
                    .form
                    .read(cx)
                    .pairs(cx)
                    .into_iter()
                    .map(|(name, value)| Part { name, value: PartValue::Text(value) })
                    .collect();
                self.parts.update(cx, |table, cx| table.set_parts(&parts, window, cx));
            }
            (BodyMode::Multipart, BodyMode::Form) => {
                let pairs: Vec<(String, String)> = self
                    .parts
                    .read(cx)
                    .parts(cx)
                    .into_iter()
                    .filter_map(|part| match part.value {
                        PartValue::Text(value) => Some((part.name, value)),
                        PartValue::File(_) => None,
                    })
                    .collect();
                self.form.update(cx, |table, cx| table.set_pairs(&pairs, window, cx));
            }
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

        self.sync_content_type(window, cx);
        self.changed(cx);
    }

    /// Sets the `Content-Type` header the current mode calls for. In File mode that is the chosen file's
    /// type, and no header while there is no file.
    fn sync_content_type(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let content_type = match self.body_mode {
            BodyMode::File => self.file.as_ref().map(|file| file.content_type.clone()),
            mode => mode.content_type().map(str::to_owned),
        };
        let mut headers = self.headers.read(cx).pairs(cx);
        headers.retain(|(name, _)| !name.eq_ignore_ascii_case("content-type"));
        if let Some(content_type) = content_type {
            headers.push(("Content-Type".to_owned(), content_type));
        }
        self.headers.update(cx, |table, cx| table.set_pairs(&headers, window, cx));
    }

    fn set_file(&mut self, file: Option<FileData>) {
        self.file_preview = file.as_ref().and_then(preview_of);
        self.file = file;
        self.file_error = None;
    }

    /// Asks for the file that is the body. It is applied by the next render, which has the window the
    /// header table needs.
    fn choose_file(&mut self, cx: &mut Context<Self>) {
        pick_file("Choose a file to send", cx, |this, outcome, cx| {
            match outcome {
                Ok(file) => this.pending_file = Some(FileData::from_picked(file)),
                Err(message) => this.file_error = Some(format!("Couldn't read the file: {message}").into()),
            }
            cx.notify();
        });
    }

    fn remove_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_file(None);
        self.sync_content_type(window, cx);
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
        self.saved_folder = self.folder_id.clone();

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

        self.show_entity_body(&entity, body, window, cx);
        self.pre_script.set_value(text("preScript"), window, cx);
        self.post_script.set_value(text("postScript"), window, cx);
        self.test_script.set_value(text("testScript"), window, cx);

        self.entity = entity;
        // The baseline is read back from the controls, so that what they normalise (a trimmed name, a form
        // body re-encoded) is not mistaken for an edit.
        self.baseline = self.fields(cx);
    }

    /// Shows the body of a loaded request. Multipart parts and a base64 file decide the mode first; a
    /// request without either has a text body.
    fn show_entity_body(
        &mut self,
        entity: &Map<String, Value>,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parts = entity.get("bodyParts").map(parts_from_json).unwrap_or_default();
        let is_file = entity.get("bodyEncoding").and_then(Value::as_str) == Some("base64");
        if !parts.is_empty() {
            self.body_mode = BodyMode::Multipart;
            self.set_file(None);
            self.parts.update(cx, |table, cx| table.set_parts(&parts, window, cx));
            self.body.set_value("", window, cx);
        } else if is_file {
            self.body_mode = BodyMode::File;
            let name = entity.get("bodyFileName").and_then(Value::as_str).unwrap_or("file").to_owned();
            let content_type = self.content_type(cx).unwrap_or_else(|| "application/octet-stream".to_owned());
            self.set_file(Some(FileData::from_base64(name, content_type, text.trim().to_owned())));
            self.parts.update(cx, |table, cx| table.set_parts(&[], window, cx));
            self.body.set_value("", window, cx);
        } else {
            self.set_file(None);
            self.parts.update(cx, |table, cx| table.set_parts(&[], window, cx));
            self.show_body(text, window, cx);
        }
    }

    /// Shows `body` in the mode its content and the `Content-Type` header (already in the table) call for.
    fn show_body(&mut self, body: String, window: &mut Window, cx: &mut Context<Self>) {
        let multipart = body.is_empty()
            && self.content_type(cx).is_some_and(|ct| ct.to_ascii_lowercase().contains("multipart/form-data"));
        self.body_mode =
            if multipart { BodyMode::Multipart } else { BodyMode::detect(self.content_type(cx).as_deref(), &body) };
        self.body.set_syntax(Self::body_syntax(self.body_mode), cx);
        if self.body_mode == BodyMode::Form {
            let pairs = decode_form(&body);
            self.form.update(cx, |table, cx| table.set_pairs(&pairs, window, cx));
        }
        self.body.set_value(body, window, cx);
    }

    /// The loaded entity with the editor's fields written over it.
    fn edited_entity(&self, fields: &Fields) -> Map<String, Value> {
        let mut entity = self.entity.clone();
        entity.insert("name".into(), json!(if fields.name.is_empty() { DEFAULT_NAME } else { fields.name.as_str() }));
        entity.insert("method".into(), json!(fields.method));
        entity.insert("url".into(), json!(fields.url));
        entity.insert("headers".into(), Value::Object(header_map(&fields.headers)));
        if !fields.parts.is_empty() {
            entity.insert("body".into(), json!(""));
            entity.insert("bodyParts".into(), parts_to_json(&fields.parts));
            entity.remove("bodyEncoding");
            entity.remove("bodyFileName");
        } else if let Some(file) = &fields.file {
            entity.insert("body".into(), json!(&*file.data));
            entity.insert("bodyEncoding".into(), json!("base64"));
            entity.insert("bodyFileName".into(), json!(file.name));
            entity.remove("bodyParts");
        } else {
            entity.insert("body".into(), json!(fields.body));
            entity.remove("bodyEncoding");
            entity.remove("bodyParts");
            entity.remove("bodyFileName");
        }
        entity.insert("preScript".into(), json!(fields.pre_script));
        entity.insert("postScript".into(), json!(fields.post_script));
        entity.insert("testScript".into(), json!(fields.test_script));
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
        entity.insert("protocol".into(), json!("rest"));
        entity.insert("workspaceId".into(), json!(self.workspace_id));
        entity.insert("folderId".into(), json!(self.folder_id));
        let folder = self.folder_id.clone();

        self.saving = true;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        let kind = RequestKind::Rest.entity_kind();
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
                let mut request = Map::new();
                request.insert("method".into(), json!(fields.method));
                request.insert("url".into(), json!(fields.url));
                request.insert("headers".into(), Value::Object(header_map(&fields.headers)));
                if !fields.parts.is_empty() {
                    request.insert("body".into(), json!(""));
                    request.insert("bodyParts".into(), parts_to_json(&fields.parts));
                } else if let Some(file) = &fields.file {
                    request.insert("body".into(), json!(&*file.data));
                    request.insert("bodyEncoding".into(), json!("base64"));
                } else {
                    request.insert("body".into(), json!(fields.body));
                }
                json!({ "workspaceId": self.workspace_id, "request": request, "scripts": scripts })
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
            BodyMode::Multipart => self.parts.clone().into_any_element(),
            BodyMode::File => self.render_file(cx),
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

    /// The file that is the body: what it is and how to change it, and the image itself when it is one.
    fn render_file(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let choose = |label: &'static str| {
            Button::new("choose-body-file")
                .small()
                .label(label)
                .on_click(cx.listener(|this, _, _, cx| this.choose_file(cx)))
        };
        let Some(file) = &self.file else {
            return v_flex()
                .size_full()
                .child(div().flex_1().min_h_0().child(page::empty_with_action(
                    "No file chosen",
                    "The file's bytes are sent as the request body.",
                    choose("Choose file…"),
                )))
                .children(
                    self.file_error.clone().map(|message| {
                        div().flex_none().px_3().pb_2().text_xs().text_color(theme.warning).child(message)
                    }),
                )
                .into_any_element();
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .px_3()
                    .py_2()
                    .gap_3()
                    .items_center()
                    .child(
                        v_flex().flex_1().min_w_0().child(div().truncate().text_sm().child(file.name.clone())).child(
                            div().text_xs().text_color(theme.muted_foreground).child(format!(
                                "{} · {}",
                                size_label(file.size),
                                file.content_type
                            )),
                        ),
                    )
                    .child(choose("Choose another file…").ghost())
                    .child(
                        Button::new("remove-body-file")
                            .ghost()
                            .small()
                            .label("Remove file")
                            .on_click(cx.listener(|this, _, window, cx| this.remove_file(window, cx))),
                    ),
            )
            .children(
                self.file_error
                    .clone()
                    .map(|message| div().flex_none().px_3().pb_2().text_xs().text_color(theme.warning).child(message)),
            )
            .child(div().flex_1().min_h_0().children(self.file_preview.as_ref().map(|preview| preview.render(cx))))
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A file chosen in the dialog is applied here, where a window is at hand.
        if let Some(file) = self.pending_file.take()
            && self.load == LoadState::Ready
        {
            self.set_file(Some(file));
            self.sync_content_type(window, cx);
            self.changed(cx);
        }
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
            .child(request_bar(
                Some(div().w_32().flex_none().child(Select::new(&self.method).h(row_height)).into_any_element()),
                self.url.line_field(cx),
                Button::new("send-request")
                    .primary()
                    .label("Send")
                    .loading(self.sending)
                    .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                [],
                cx,
            ))
            .child(div().flex_none().px_3().child(self.render_sections(cx)))
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}
