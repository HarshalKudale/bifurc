//! An open MCP request: a client connection to a Model Context Protocol server. Like WebSocket it is an
//! ordinary request type: a name, a saved entity in the engine, a place in the explorer tree, and the same save
//! model as the REST editor (a draft is created by Save, a saved request is written a moment after each edit).
//! What differs is the verb — Connect instead of Send — and that the "response" is a live connection owned by
//! the engine ([`McpConnection`]), which is why closing this tab does not close it. The name is edited in the
//! window's breadcrumb, which shows this session's name field.
//!
//! Where a REST request has a method, this has a transport: STDIO, where the address is the command that starts
//! the server, or HTTP, where it is the server's endpoint. The headers carry the credentials either way: they
//! are sent with every HTTP request, and given to a program as environment variables.
//!
//! Once connected the request panel lists what the server declares. A tool or a prompt is shown with one field
//! per argument it takes; a resource is read by its URI. What is typed for the arguments is saved with the
//! request, as it is typed: the engine reads each value as the type the tool declares. The command that asks —
//! Call tool, Get prompt, Read resource — is in the first row beside Connect, and each answer is a row of the
//! response panel.
//!
//! Connecting a saved, unedited request sends only its id (`mcp.connect {workspaceId, connectionId}`); a draft
//! or an edited request sends the target it holds. The engine resolves environment variables either way.

use std::collections::BTreeMap;

use bifurc_engine_client::ClientError;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IndexPath, Sizable as _, StyledExt as _, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, InputEvent, InputState},
    notification::Notification,
    radio::RadioGroup,
    select::{Select, SelectEvent, SelectState},
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, ElementId, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::{Map, Value, json};

use super::{
    Carry, LoadState, SaveMode, SessionEvent, engine_folder,
    mcp_connection::{McpConnection, McpEvent},
};
use crate::{
    app_model::{AppModel, ModelEvent},
    components::{Autosave, KeyValueEvent, KeyValueTable, Syntax, VarEditor, page, request_bar},
    engine::{call, call_in, describe, error_text},
    i18n::{t, t_with},
    model::{
        RequestKind,
        http::{header_map, header_pairs},
        mcp::{ArgumentInput, McpItem, McpView},
        socket::SocketState,
    },
};

const DEFAULT_NAME: &str = "New MCP request";

/// The transports, as the picker shows them; the engine names them in lower case.
const TRANSPORTS: [&str; 2] = ["STDIO", "HTTP"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Tools,
    Prompts,
    Resources,
    Headers,
}

impl Section {
    const ALL: [Section; 4] = [Section::Tools, Section::Prompts, Section::Resources, Section::Headers];

    /// What the engine calls the thing this section asks for, which is also the prefix its inputs are kept
    /// under.
    fn asks_for(self) -> &'static str {
        match self {
            Section::Tools => "tool",
            Section::Prompts => "prompt",
            Section::Resources | Section::Headers => "resource",
        }
    }

    fn command(self) -> &'static str {
        t(match self {
            Section::Tools => "session.mcp.call_tool",
            Section::Prompts => "session.mcp.get_prompt",
            Section::Resources | Section::Headers => "session.mcp.read_resource",
        })
    }
}

/// What was typed for the arguments of each tool and prompt: by `tool:<name>` or `prompt:<name>`, then by
/// argument. Only what is not empty is kept.
type Inputs = BTreeMap<String, BTreeMap<String, String>>;

#[derive(Debug, Clone, PartialEq)]
struct Fields {
    name: String,
    /// `stdio` or `http`.
    transport: String,
    url: String,
    headers: Vec<(String, String)>,
    inputs: Inputs,
    /// The URI in the Resources tab.
    resource: String,
}

/// Whether an address is an endpoint rather than a command.
fn is_endpoint(address: &str) -> bool {
    address.starts_with("http://") || address.starts_with("https://")
}

/// The fields of the tool or the prompt that is selected: one per argument it takes.
struct Form {
    /// `tool:<name>` or `prompt:<name>`.
    key: String,
    /// What the form was built for; a server that declares it differently gets a new form.
    item: McpItem,
    /// One per argument, in the order of `item.arguments`.
    controls: Vec<Control>,
    _subscriptions: Vec<Subscription>,
}

/// What an argument is filled in with, which follows what the tool's schema says of it. Whatever the control,
/// what it holds is kept as text in [`Inputs`]: the value chosen, `true` or `false`, the JSON, what was typed.
enum Control {
    /// A picker of the values the argument is limited to.
    Choice(Entity<SelectState<Vec<String>>>),
    /// True or false, as radio buttons. They show what [`Inputs`] holds and keep nothing themselves.
    Flag,
    /// An editor for a JSON object or array.
    Json(VarEditor),
    /// A one-row field.
    Text(VarEditor),
}

pub struct McpSession {
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
    transport: Entity<SelectState<Vec<&'static str>>>,
    url: VarEditor,
    /// Whether the address was an endpoint when it last changed: the transport follows it when that flips.
    address_is_endpoint: bool,
    headers: Entity<KeyValueTable>,
    inputs: Inputs,
    resource: VarEditor,
    /// The tool and the prompt that are selected, by name.
    tool: Option<String>,
    prompt: Option<String>,
    tool_form: Option<Form>,
    prompt_form: Option<Form>,
    section: Section,
    /// What the command beside Connect asks for: the last of Tools, Prompts and Resources that was shown.
    asking: Section,
    baseline: Fields,
    connection: Entity<McpConnection>,
    saving: bool,
    autosave: Autosave,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SessionEvent> for McpSession {}

impl McpSession {
    fn build(
        model: Entity<AppModel>,
        saved_id: Option<String>,
        folder_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let workspace_id = model.read(cx).workspace_id().to_owned();
        let connection_id = saved_id.clone().unwrap_or_else(|| format!("draft-{}", cx.entity_id().as_u64()));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t("session.rest.name_placeholder")));
        let transport = cx.new(|cx| SelectState::new(TRANSPORTS.to_vec(), Some(IndexPath::default()), window, cx));
        let url = VarEditor::line("npx -y some-mcp-server   or   https://example.test/mcp", window, cx);
        let headers = cx.new(|cx| {
            KeyValueTable::new(t("session.protocol.column_name"), t("session.rest.column_value"), window, cx)
        });
        let resource = VarEditor::line(t("session.mcp.resource_placeholder"), window, cx);
        let connection = cx.new(|cx| McpConnection::new(window, cx));

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.changed(cx),
            InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit_name(window, cx),
            InputEvent::Focus => {}
        }));
        subscriptions.push(cx.subscribe_in(
            url.state(),
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.follow_address(window, cx);
                    this.changed(cx);
                }
                InputEvent::PressEnter { .. } => this.connect(window, cx),
                _ => {}
            },
        ));
        subscriptions.push(cx.subscribe_in(resource.state(), window, |this, _, event: &InputEvent, window, cx| {
            match event {
                InputEvent::Change => this.changed(cx),
                InputEvent::PressEnter { .. } => this.ask(Section::Resources, window, cx),
                _ => {}
            }
        }));
        subscriptions
            .push(cx.subscribe(&transport, |this, _, _: &SelectEvent<Vec<&'static str>>, cx| this.changed(cx)));
        subscriptions.push(cx.subscribe(&headers, |this, _, _: &KeyValueEvent, cx| this.changed(cx)));
        subscriptions.push(cx.subscribe(&connection, |this, _, event: &McpEvent, cx| match event {
            McpEvent::Clear => this.clear_lines(cx),
        }));
        subscriptions.push(cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| match event {
            ModelEvent::McpStatus(payload) if this.is_mine(payload) => {
                this.connection.update(cx, |c, cx| c.apply_status_event(payload, cx));
                // What the server declares may have changed with the state.
                this.sync_forms(window, cx);
                cx.emit(SessionEvent::Changed);
                // The address and the Connect button follow the connection's state.
                cx.notify();
            }
            ModelEvent::McpMessage(payload) if this.is_mine(payload) => {
                this.connection.update(cx, |c, cx| c.apply_message_event(payload, window, cx));
            }
            ModelEvent::EntityReverted(id) if this.saved_id.as_deref() == Some(id.as_str()) => {
                this.reload(window, cx);
            }
            _ => {}
        }));

        let baseline = Fields {
            name: DEFAULT_NAME.to_owned(),
            transport: "stdio".to_owned(),
            url: String::new(),
            headers: Vec::new(),
            inputs: Inputs::new(),
            resource: String::new(),
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
            transport,
            url,
            address_is_endpoint: false,
            headers,
            inputs: Inputs::new(),
            resource,
            tool: None,
            prompt: None,
            tool_form: None,
            prompt_form: None,
            section: Section::Tools,
            asking: Section::Tools,
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
        let payload = json!({ "workspaceId": self.workspace_id, "kind": RequestKind::Mcp.entity_kind(), "id": id });
        call_in(&client, window, cx, "entity.load", payload, |this, outcome, window, cx| {
            // The connection's status is asked for once, when the tab opens; a re-read does not change it.
            let opening = this.load == LoadState::Loading;
            match outcome.map_err(|e| describe(&e)).and_then(|v| {
                v.get("entity").and_then(Value::as_object).cloned().ok_or_else(|| t("session.rest.gone").to_owned())
            }) {
                Ok(entity) => {
                    this.adopt_entity(entity, window, cx);
                    this.load = LoadState::Ready;
                    if opening {
                        this.attach(window, cx);
                    }
                }
                Err(message) => this.load = LoadState::Failed(message),
            }
            cx.emit(SessionEvent::Changed);
            cx.notify();
        });
    }

    /// The engine keeps the connection (and its log) across tabs and windows. Asking for its status when the
    /// tab opens is what makes a live connection still live, with what its server declares, after the tab was
    /// closed.
    fn attach(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "connectionId": self.connection_id });
        call_in(&client, window, cx, "mcp.status", payload, |this, outcome, window, cx| {
            if let Ok(status) = outcome {
                let view = McpView::from_status(&status);
                this.connection.update(cx, |c, cx| c.replace_view(view, cx));
                this.sync_forms(window, cx);
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

    /// The address field, for the key that jumps to it.
    pub fn url_editor(&self) -> &VarEditor {
        &self.url
    }

    pub fn is_saving(&self) -> bool {
        self.saving
    }

    pub fn connection(&self) -> &Entity<McpConnection> {
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
        (fields != self.baseline && fields.url.is_empty()).then(|| t("session.mcp.problem_no_address").into())
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
        self.follow_address(window, cx);
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

    fn transport(&self, cx: &gpui_kit::App) -> String {
        self.transport.read(cx).selected_value().copied().unwrap_or(TRANSPORTS[0]).to_lowercase()
    }

    /// The transport follows the address when it turns into an endpoint or stops being one: a pasted
    /// `https://` address is not a command to run. Picking a transport by hand stands until then.
    fn follow_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let endpoint = is_endpoint(&self.url.line_value(cx));
        if endpoint != self.address_is_endpoint {
            self.address_is_endpoint = endpoint;
            let wanted = TRANSPORTS[usize::from(endpoint)];
            self.transport.update(cx, |s, cx| s.set_selected_value(&wanted, window, cx));
        }
    }

    /// The name is trimmed here and in [`fields_of`](Self::fields_of), so that a stray space is not an edit.
    fn fields(&self, cx: &gpui_kit::App) -> Fields {
        Fields {
            name: self.name.read(cx).value().trim().to_owned(),
            transport: self.transport(cx),
            url: self.url.line_value(cx),
            headers: self.headers.read(cx).pairs(cx),
            inputs: self.inputs.clone(),
            resource: self.resource.line_value(cx),
        }
    }

    fn fields_of(entity: &Map<String, Value>) -> Fields {
        let text = |key: &str| entity.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        let url = text("url").trim().to_owned();
        let mut inputs = Inputs::new();
        for (key, typed) in entity.get("inputs").and_then(Value::as_object).into_iter().flatten() {
            let typed: BTreeMap<String, String> = typed
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(argument, value)| Some((argument.clone(), value.as_str()?.to_owned())))
                .filter(|(_, value)| !value.is_empty())
                .collect();
            if !typed.is_empty() {
                inputs.insert(key.clone(), typed);
            }
        }
        Fields {
            name: text("name").trim().to_owned(),
            // A request saved without one (by hand) has the transport its address implies, as the engine has it.
            transport: match text("transport").to_lowercase().as_str() {
                "http" => "http",
                "stdio" => "stdio",
                _ if is_endpoint(&url) => "http",
                _ => "stdio",
            }
            .to_owned(),
            url,
            headers: header_pairs(entity.get("headers").unwrap_or(&Value::Null)),
            inputs,
            resource: text("resourceUri").trim().to_owned(),
        }
    }

    fn apply_fields(&mut self, fields: &Fields, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |s, cx| s.set_value(fields.name.clone(), window, cx));
        let transport = TRANSPORTS[usize::from(fields.transport == "http")];
        self.transport.update(cx, |s, cx| s.set_selected_value(&transport, window, cx));
        // The transport is the request's own here, whatever the address looks like.
        self.address_is_endpoint = is_endpoint(&fields.url);
        self.url.set_value(fields.url.clone(), window, cx);
        self.headers.update(cx, |table, cx| table.set_pairs(&fields.headers, window, cx));
        self.resource.set_value(fields.resource.clone(), window, cx);
        self.inputs = fields.inputs.clone();
        // The forms show what was typed: they are filled again from what was just adopted.
        (self.tool_form, self.prompt_form) = (None, None);
        self.sync_forms(window, cx);
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
        entity.insert("transport".into(), json!(fields.transport));
        entity.insert("url".into(), json!(fields.url));
        entity.insert("headers".into(), Value::Object(header_map(&fields.headers)));
        entity.insert("inputs".into(), json!(fields.inputs));
        entity.insert("resourceUri".into(), json!(fields.resource));
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
                    Notification::warning(t("session.mcp.save_no_address")).title(t("session.rest.save_no_url_title")),
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
        let kind = RequestKind::Mcp.entity_kind();
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
                    None => window.push_notification(Notification::error(t("session.rest.engine_no_saved")), cx),
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
        if self.load != LoadState::Ready || self.connection.read(cx).state().is_live() {
            return;
        }
        let fields = self.fields(cx);
        if fields.url.is_empty() {
            window.push_notification(
                Notification::warning(t("session.mcp.connect_no_address"))
                    .title(t("session.connection.connect_no_url_title")),
                cx,
            );
            return;
        }
        self.connection_id = self.saved_id.clone().unwrap_or_else(|| self.connection_id.clone());
        // What is typed for the arguments is not part of the target: editing it does not make the saved
        // target stale.
        let target = |f: &Fields| (f.transport.clone(), f.url.clone(), f.headers.clone());
        let payload = match (&self.saved_id, target(&fields) == target(&self.baseline)) {
            (Some(_), true) => json!({ "workspaceId": self.workspace_id, "connectionId": self.connection_id }),
            _ => json!({
                "workspaceId": self.workspace_id,
                "connectionId": self.connection_id,
                "transport": fields.transport,
                "url": fields.url,
                "headers": header_map(&fields.headers),
            }),
        };
        let client = self.model.read(cx).client().clone();
        call_in(&client, window, cx, "mcp.connect", payload, move |_this, outcome, window, cx| {
            if let Some(message) = refusal(outcome, t("session.websocket.open_failed")) {
                window
                    .push_notification(Notification::error(message).title(t("session.connection.connect_failed")), cx);
            }
        });
    }

    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "connectionId": self.connection_id });
        call(&client, cx, "mcp.disconnect", payload, |_, outcome, cx| {
            if let Err(error) = outcome {
                cx.emit(SessionEvent::Notice(t_with(
                    "session.connection.disconnect_failed",
                    &[("error", &describe(&error))],
                )));
            }
        });
    }

    fn clear_lines(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "connectionId": self.connection_id });
        self.connection.update(cx, |c, cx| c.clear_lines(cx));
        call(&client, cx, "mcp.clearMessages", payload, |_, _, _| {});
    }

    // ── what the server declares ──────────────────────────────────────────────────────────────────

    /// Keeps the selection and the forms in step with what the server declares: the selected tool and prompt
    /// stay selected while the server still has them (and while it has not said yet), the first one is
    /// selected otherwise, and each has a form with a field per argument.
    fn sync_forms(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (tools, prompts) = {
            let view = self.connection.read(cx).view();
            (view.tools.clone(), view.prompts.clone())
        };
        let pick = |selected: &Option<String>, items: &[McpItem]| match selected {
            Some(name) if items.is_empty() || items.iter().any(|item| &item.name == name) => Some(name.clone()),
            _ => items.first().map(|item| item.name.clone()),
        };
        self.tool = pick(&self.tool, &tools);
        self.prompt = pick(&self.prompt, &prompts);
        let tool = self.tool.as_ref().and_then(|name| tools.iter().find(|item| &item.name == name));
        let prompt = self.prompt.as_ref().and_then(|name| prompts.iter().find(|item| &item.name == name));
        let (tool_form, prompt_form) = (self.tool_form.take(), self.prompt_form.take());
        self.tool_form = self.form_for(Section::Tools, tool, tool_form, window, cx);
        self.prompt_form = self.form_for(Section::Prompts, prompt, prompt_form, window, cx);
    }

    /// The form of `item`: the one there is, when it was built for the same declaration, or a new one filled
    /// with what was typed for this item before.
    fn form_for(
        &mut self,
        section: Section,
        item: Option<&McpItem>,
        existing: Option<Form>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Form> {
        let item = item?;
        let key = format!("{}:{}", section.asks_for(), item.name);
        if let Some(form) = existing.filter(|form| form.key == key && &form.item == item) {
            return Some(form);
        }
        let typed = self.inputs.get(&key).cloned().unwrap_or_default();
        let mut controls = Vec::new();
        let mut subscriptions = Vec::new();
        for (index, argument) in item.arguments.iter().enumerate() {
            let held = typed.get(&argument.name).cloned().unwrap_or_default();
            let control = match argument.input() {
                ArgumentInput::Choice => {
                    let choices = argument.choices.clone();
                    let state = cx.new(|cx| SelectState::new(choices, None, window, cx));
                    // A value that is no longer one of the choices is shown as nothing chosen.
                    if argument.choices.contains(&held) {
                        state.update(cx, |state, cx| state.set_selected_value(&held, window, cx));
                    }
                    subscriptions.push(cx.subscribe(&state, move |this, _, event: &SelectEvent<Vec<String>>, cx| {
                        let SelectEvent::Confirm(chosen) = event;
                        this.set_input(section, index, chosen.clone().unwrap_or_default(), cx);
                    }));
                    Control::Choice(state)
                }
                ArgumentInput::Flag => Control::Flag,
                ArgumentInput::Json => {
                    let placeholder = if argument.kind == "array" { "[ … ]" } else { "{ … }" };
                    let editor = VarEditor::code(Syntax::Json, placeholder, window, cx);
                    editor.set_value(held, window, cx);
                    subscriptions.push(cx.subscribe(editor.state(), move |this, _, event: &InputEvent, cx| {
                        if matches!(event, InputEvent::Change) {
                            this.typed(section, index, cx);
                        }
                    }));
                    Control::Json(editor)
                }
                ArgumentInput::Text => {
                    let placeholder = t(match argument.kind.as_str() {
                        "string" => "session.mcp.placeholder_text",
                        "integer" => "session.mcp.placeholder_integer",
                        "number" => "session.mcp.placeholder_number",
                        _ => "session.mcp.placeholder_text_or_json",
                    });
                    let editor = VarEditor::line(placeholder, window, cx);
                    editor.set_value(held, window, cx);
                    subscriptions.push(cx.subscribe_in(
                        editor.state(),
                        window,
                        move |this, _, event: &InputEvent, window, cx| match event {
                            InputEvent::Change => this.typed(section, index, cx),
                            InputEvent::PressEnter { .. } => this.ask(section, window, cx),
                            _ => {}
                        },
                    ));
                    Control::Text(editor)
                }
            };
            controls.push(control);
        }
        Some(Form { key, item: item.clone(), controls, _subscriptions: subscriptions })
    }

    fn form(&self, section: Section) -> Option<&Form> {
        match section {
            Section::Tools => self.tool_form.as_ref(),
            Section::Prompts => self.prompt_form.as_ref(),
            Section::Resources | Section::Headers => None,
        }
    }

    /// A field of a form was typed in.
    fn typed(&mut self, section: Section, index: usize, cx: &mut Context<Self>) {
        let value = match self.form(section).and_then(|form| form.controls.get(index)) {
            // A value is kept as typed, but for the line break a paste can bring into a one-row field.
            Some(Control::Text(editor)) => editor.value(cx).replace(['\r', '\n'], ""),
            Some(Control::Json(editor)) => editor.value(cx),
            _ => return,
        };
        self.set_input(section, index, value, cx);
    }

    /// What is held for an argument of the selected tool or prompt; empty when nothing is.
    fn input(&self, form: &Form, index: usize) -> String {
        let name = form.item.arguments.get(index).map(|argument| &argument.name);
        name.and_then(|name| self.inputs.get(&form.key)?.get(name)).cloned().unwrap_or_default()
    }

    /// An argument was given a value, or none with an empty one: it is kept with the request.
    fn set_input(&mut self, section: Section, index: usize, value: String, cx: &mut Context<Self>) {
        let Some(form) = self.form(section) else { return };
        let Some(argument) = form.item.arguments.get(index) else { return };
        let (key, argument) = (form.key.clone(), argument.name.clone());
        let held = self.inputs.get(&key).and_then(|typed| typed.get(&argument)).cloned().unwrap_or_default();
        if held == value {
            return;
        }
        if value.is_empty() {
            if let Some(typed) = self.inputs.get_mut(&key) {
                typed.remove(&argument);
                if typed.is_empty() {
                    self.inputs.remove(&key);
                }
            }
        } else {
            self.inputs.entry(key).or_default().insert(argument, value);
        }
        self.changed(cx);
    }

    fn select(&mut self, section: Section, name: String, window: &mut Window, cx: &mut Context<Self>) {
        match section {
            Section::Tools => self.tool = Some(name),
            Section::Prompts => self.prompt = Some(name),
            Section::Resources | Section::Headers => {
                // Writing a field from here is not reported as typing, so the edit is announced here.
                self.resource.set_value(name, window, cx);
                self.changed(cx);
            }
        }
        self.sync_forms(window, cx);
        cx.notify();
    }

    /// What `section` would ask the server for: the name, and what was typed for the arguments it declares.
    fn question(&self, section: Section, cx: &gpui_kit::App) -> Option<(String, Map<String, Value>)> {
        if section == Section::Resources || section == Section::Headers {
            return Some((self.resource.line_value(cx), Map::new())).filter(|(uri, _)| !uri.is_empty());
        }
        let form = self.form(section)?;
        let typed = self.inputs.get(&form.key);
        let arguments = form
            .item
            .arguments
            .iter()
            .filter_map(|argument| Some((argument.name.clone(), json!(typed?.get(&argument.name)?))))
            .collect();
        Some((form.item.name.clone(), arguments))
    }

    /// Whether `section` has something to ask for and an open connection to ask over.
    fn can_ask(&self, section: Section, cx: &gpui_kit::App) -> bool {
        self.connection.read(cx).state() == SocketState::Connected && self.question(section, cx).is_some()
    }

    /// Calls the selected tool, gets the selected prompt or reads the resource. The answer is a row of the
    /// response panel; what was typed stays, as a call is often made again.
    fn ask(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_ask(section, cx) {
            return;
        }
        let Some((name, arguments)) = self.question(section, cx) else { return };
        let client = self.model.read(cx).client().clone();
        let mut payload = json!({
            "workspaceId": self.workspace_id, "connectionId": self.connection_id,
            "type": section.asks_for(), "name": name,
        });
        if section != Section::Resources && section != Section::Headers {
            payload["arguments"] = Value::Object(arguments);
        }
        call_in(&client, window, cx, "mcp.call", payload, move |_this, outcome, window, cx| {
            if let Some(message) = refusal(outcome, t("session.mcp.not_asked")) {
                let title = match section {
                    Section::Tools => t("session.mcp.call_failed"),
                    Section::Prompts => t("session.mcp.get_failed"),
                    Section::Resources | Section::Headers => t("session.mcp.read_failed"),
                };
                window.push_notification(Notification::error(message).title(title), cx);
            }
        });
    }

    // ── rendering ─────────────────────────────────────────────────────────────────────────────────

    fn render_sections(&self, cx: &mut Context<Self>) -> TabBar {
        let (tools, prompts, resources) = {
            let view = self.connection.read(cx).view();
            (view.tools.len(), view.prompts.len(), view.resources.len())
        };
        let stdio = self.transport(cx) == "stdio";
        let counted = |key: &str, count: usize| match count {
            0 => t(key).to_owned(),
            count => t_with("session.mcp.counted", &[("label", &t(key)), ("count", &count)]),
        };
        let labels = [
            counted("session.mcp.tab_tools", tools),
            counted("session.mcp.tab_prompts", prompts),
            counted("session.mcp.tab_resources", resources),
            (if stdio { t("session.mcp.tab_environment") } else { t("session.rest.tab_headers") }).to_owned(),
        ];
        TabBar::new("mcp-sections")
            .underline()
            .small()
            .selected_index(Section::ALL.iter().position(|s| *s == self.section).unwrap_or(0))
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.section = Section::ALL[*ix];
                if this.section != Section::Headers {
                    this.asking = this.section;
                }
                cx.notify();
            }))
            .children(labels.map(|label| Tab::new().label(label)))
    }

    /// What a section shows while the server has declared nothing for it.
    fn render_nothing(&self, what: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let noun = t(match what {
            "prompts" => "session.mcp.noun_prompts",
            "resources" => "session.mcp.noun_resources",
            _ => "session.mcp.noun_tools",
        });
        let description = match self.connection.read(cx).state() {
            SocketState::Connected => t_with("session.mcp.none_connected", &[("what", &noun)]),
            SocketState::Connecting => t_with("session.mcp.none_connecting", &[("what", &noun)]),
            _ => t_with("session.mcp.none_disconnected", &[("what", &noun)]),
        };
        page::empty(t_with("session.mcp.none_title", &[("what", &noun)]), description)
    }

    /// One row of a list of what the server declares: its name, and under it what it is.
    fn render_item(&self, section: Section, item: &McpItem, selected: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let name = item.name.clone();
        let detail = match (item.title.as_str(), item.summary()) {
            ("", summary) => summary.to_owned(),
            (title, "") => title.to_owned(),
            (title, summary) => format!("{title} — {summary}"),
        };
        v_flex()
            .id(ElementId::Name(format!("mcp-{}-{}", section.asks_for(), item.name).into()))
            .w_full()
            .px_3()
            .py_2()
            .gap_1()
            .cursor_pointer()
            .border_b_1()
            .border_color(theme.border)
            .when(selected, |this| this.bg(theme.accent))
            .when(!selected, |this| this.hover(|style| style.bg(theme.muted)))
            .on_click(cx.listener(move |this, _, window, cx| this.select(section, name.clone(), window, cx)))
            .child(
                div()
                    .text_sm()
                    .font_family(theme.mono_font_family.clone())
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(item.name.clone()),
            )
            .when(!detail.is_empty(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(detail),
                )
            })
            .into_any_element()
    }

    /// The tools or the prompts: the list of them, and beside it the selected one with a field per argument.
    fn render_catalog(&self, section: Section, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (items, selected, what) = {
            let view = self.connection.read(cx).view();
            match section {
                Section::Prompts => (view.prompts.clone(), self.prompt.clone(), "prompts"),
                _ => (view.tools.clone(), self.tool.clone(), "tools"),
            }
        };
        if items.is_empty() {
            return self.render_nothing(what, cx);
        }
        let rows: Vec<AnyElement> = items
            .iter()
            .map(|item| self.render_item(section, item, selected.as_deref() == Some(item.name.as_str()), cx))
            .collect();
        let list = div()
            .id(ElementId::Name(format!("mcp-{what}-list").into()))
            .w_64()
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(theme.border)
            .children(rows);

        let row_height = VarEditor::row_height(cx);
        let mut detail = v_flex().p_3().gap_4();
        if let Some(form) = self.form(section) {
            let item = &form.item;
            detail = detail.child(
                v_flex()
                    .gap_1()
                    .child(div().font_medium().child(if item.title.is_empty() {
                        item.name.clone()
                    } else {
                        item.title.clone()
                    }))
                    .when(!item.description.is_empty(), |this| {
                        this.child(div().text_sm().text_color(theme.muted_foreground).child(item.description.clone()))
                    }),
            );
            if item.arguments.is_empty() {
                detail = detail
                    .child(div().text_sm().text_color(theme.muted_foreground).child(t("session.mcp.no_arguments")));
            }
            for (index, (argument, control)) in item.arguments.iter().zip(&form.controls).enumerate() {
                let held = self.input(form, index);
                let field = match control {
                    Control::Choice(state) => div()
                        .w_64()
                        .child(
                            Select::new(state)
                                .h(row_height)
                                .placeholder(if argument.required {
                                    t("session.mcp.choose_one")
                                } else {
                                    t("session.mcp.not_set")
                                })
                                // An argument that need not be sent can be put back to nothing chosen.
                                .cleanable(!argument.required),
                        )
                        .into_any_element(),
                    Control::Flag => {
                        let group = ElementId::Name(format!("mcp-flag-{}-{}", form.key, argument.name).into());
                        let clear = ElementId::Name(format!("mcp-flag-clear-{}-{}", form.key, argument.name).into());
                        h_flex()
                            .gap_4()
                            .h(row_height)
                            // The group would otherwise take the row and push what follows to its far end.
                            .child(
                                div().flex_none().child(
                                    RadioGroup::horizontal(group)
                                        .selected_index(match held.as_str() {
                                            "true" => Some(0),
                                            "false" => Some(1),
                                            _ => None,
                                        })
                                        .on_change(cx.listener(move |this, chosen: &usize, _, cx| {
                                            let value = if *chosen == 0 { "true" } else { "false" };
                                            this.set_input(section, index, value.to_owned(), cx);
                                        }))
                                        .child(t("session.mcp.true"))
                                        .child(t("session.mcp.false")),
                                ),
                            )
                            // An argument that need not be sent can be put back to neither.
                            .when(!argument.required && !held.is_empty(), |this| {
                                this.child(Button::new(clear).ghost().small().label(t("session.mcp.not_set")).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.set_input(section, index, String::new(), cx)
                                    }),
                                ))
                            })
                            .into_any_element()
                    }
                    Control::Json(editor) => div()
                        .h_40()
                        .rounded_md()
                        .border_1()
                        .border_color(theme.border)
                        .overflow_hidden()
                        .child(Editor::new(editor.state()).bordered(false).h_full())
                        .into_any_element(),
                    Control::Text(editor) => editor.line_field(cx).into_any_element(),
                };
                let kind = match (argument.input(), argument.kind.as_str()) {
                    (ArgumentInput::Choice, _) => t("session.mcp.one_of").to_owned(),
                    (_, "") => String::new(),
                    (_, kind) => kind.to_owned(),
                };
                let mut hint = argument.description.clone();
                if let Some(default) = &argument.default {
                    hint =
                        t_with("session.mcp.with_default", &[("hint", &hint), ("default", default)]).trim().to_owned();
                }
                detail = detail.child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(page::section_label(argument.name.clone(), cx))
                                .when(argument.required, |this| {
                                    this.child(Tag::secondary().small().outline().child(t("session.mcp.required")))
                                })
                                .when(!kind.is_empty(), |this| {
                                    this.child(div().text_xs().text_color(theme.muted_foreground).child(kind))
                                }),
                        )
                        .child(field)
                        .when(!hint.is_empty(), |this| {
                            this.child(div().text_xs().text_color(theme.muted_foreground).child(hint))
                        }),
                );
            }
        }
        h_flex()
            .size_full()
            .items_start()
            .child(list)
            .child(
                div()
                    .id(ElementId::Name(format!("mcp-{what}-form").into()))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .child(detail),
            )
            .into_any_element()
    }

    /// The resources: the URI to read, and under it what the server lists. Picking one writes its URI, which
    /// for a template is then completed by hand.
    fn render_resources(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let items = self.connection.read(cx).view().resources.clone();
        let uri = self.resource.line_value(cx);
        let rows: Vec<AnyElement> =
            items.iter().map(|item| self.render_item(Section::Resources, item, item.name == uri, cx)).collect();
        let listed = if items.is_empty() {
            self.render_nothing("resources", cx)
        } else {
            div().id("mcp-resources-list").size_full().overflow_y_scroll().children(rows).into_any_element()
        };
        v_flex()
            .size_full()
            .child(div().flex_none().px_3().py_2().child(self.resource.line_field(cx)))
            .child(div().flex_1().min_h_0().border_t_1().border_color(theme.border).child(listed))
            .into_any_element()
    }

    /// The headers, with what becomes of them for the transport that is picked.
    fn render_headers(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let hint = if self.transport(cx) == "stdio" {
            t("session.mcp.headers_stdio_hint")
        } else {
            t("session.mcp.headers_http_hint")
        };
        v_flex()
            .size_full()
            .child(div().flex_none().px_3().py_2().text_xs().text_color(theme.muted_foreground).child(hint))
            .child(div().flex_1().min_h_0().child(self.headers.clone()))
            .into_any_element()
    }
}

/// Why a command the engine answers with `{ ok, error }` did not do what was asked, if it did not.
fn refusal(outcome: Result<Value, ClientError>, fallback: &str) -> Option<String> {
    match outcome {
        Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(false) => {
            Some(error_text(&result).unwrap_or_else(|| fallback.to_owned()))
        }
        Ok(_) => None,
        Err(error) => Some(describe(&error)),
    }
}

impl Render for McpSession {
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

        let live = self.connection.read(cx).state().is_live();
        let row_height = VarEditor::row_height(cx);
        let asking = self.asking;
        let can_ask = self.can_ask(asking, cx);
        let content = match self.section {
            Section::Tools | Section::Prompts => self.render_catalog(self.section, cx),
            Section::Resources => self.render_resources(cx),
            Section::Headers => self.render_headers(cx),
        };
        v_flex()
            .size_full()
            .bg(theme.background)
            .child(request_bar(
                Some(
                    div()
                        .w_32()
                        .flex_none()
                        .child(Select::new(&self.transport).h(row_height).disabled(live))
                        .into_any_element(),
                ),
                self.url.line_field(cx).disabled(live),
                if live {
                    Button::new("disconnect-mcp")
                        .label(t("session.connection.disconnect"))
                        .on_click(cx.listener(|this, _, _, cx| this.disconnect(cx)))
                } else {
                    Button::new("connect-mcp")
                        .primary()
                        .label(t("session.connection.connect"))
                        .on_click(cx.listener(|this, _, window, cx| this.connect(window, cx)))
                },
                [Button::new("ask-mcp")
                    .primary()
                    .label(asking.command())
                    .disabled(!can_ask)
                    .on_click(cx.listener(move |this, _, window, cx| this.ask(asking, window, cx)))],
                cx,
            ))
            .child(div().flex_none().px_3().child(self.render_sections(cx)))
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn a_saved_request_is_read_with_its_transport_and_what_was_typed() {
        let fields = McpSession::fields_of(&entity(json!({
            "name": " Tools ", "transport": "HTTP", "url": " https://x/mcp ", "headers": {"Authorization": "Bearer t"},
            "inputs": {"tool:echo": {"message": "hi", "empty": "", "odd": 3}, "tool:none": {}},
            "resourceUri": "mem://a",
        })));
        assert_eq!(
            (fields.name.as_str(), fields.transport.as_str(), fields.url.as_str()),
            ("Tools", "http", "https://x/mcp")
        );
        assert_eq!(fields.headers, [("Authorization".to_owned(), "Bearer t".to_owned())]);
        // Only what was typed is kept: text that is not empty.
        assert_eq!(fields.inputs.len(), 1);
        assert_eq!(fields.inputs["tool:echo"], BTreeMap::from([("message".to_owned(), "hi".to_owned())]));
        assert_eq!(fields.resource, "mem://a");
    }

    #[test]
    fn a_request_without_a_transport_has_the_one_its_address_implies() {
        let of = |url: &str| McpSession::fields_of(&entity(json!({ "url": url }))).transport;
        assert_eq!(of("https://x/mcp"), "http");
        assert_eq!(of("http://localhost:3000/mcp"), "http");
        assert_eq!(of("npx -y some-server"), "stdio");
        assert_eq!(of(""), "stdio");
        let named = McpSession::fields_of(&entity(json!({ "url": "https-server --stdio", "transport": "stdio" })));
        assert_eq!(named.transport, "stdio");
    }

    #[test]
    fn each_section_asks_for_its_own_thing() {
        assert_eq!(Section::Tools.asks_for(), "tool");
        assert_eq!(Section::Prompts.asks_for(), "prompt");
        assert_eq!(Section::Resources.asks_for(), "resource");
        assert_eq!(Section::ALL.map(Section::command), ["Call tool", "Get prompt", "Read resource", "Read resource"]);
    }
}
