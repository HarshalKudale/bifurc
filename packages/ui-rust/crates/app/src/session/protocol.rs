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
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    combobox::{Combobox, ComboboxEvent, ComboboxState},
    h_flex,
    searchable_list::SearchableListItem,
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

use super::{Carry, LoadState, SaveMode, SessionEvent, engine_folder, exchange::Exchange, graphql::SchemaPanel, wsdl::WsdlPanel};
use crate::{
    i18n::{t, t_with},
    app_model::{AppModel, ModelEvent},
    components::{Autosave, KeyValueEvent, KeyValueTable, Syntax, VarEditor, page, request_bar},
    engine::{call_in, describe},
    model::{
        RequestKind,
        http::{BodyEncoding, header_map, header_pairs},
        graphql::Schema,
        wsdl,
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
    field("endpointUrl", "session.protocol.endpoint", "https://api.example.test/graphql", FieldKind::Line),
    field("operationName", "session.protocol.operation", "session.protocol.operation_placeholder", FieldKind::Line),
    field("query", "session.protocol.query", "query { ... }", FieldKind::Document(Syntax::GraphQl)),
    field("variables", "session.protocol.variables", "{ }", FieldKind::Document(Syntax::Json)),
    field("headers", "session.protocol.headers", "", FieldKind::Pairs),
];

const GRPC_FIELDS: &[FieldSpec] = &[
    field("serverAddress", "session.protocol.server", "localhost:50051", FieldKind::Line),
    field("serviceName", "session.protocol.service", "package.Service", FieldKind::Line),
    field("methodName", "session.protocol.method", "MethodName", FieldKind::Line),
    field("requestBody", "session.protocol.message", "{ }", FieldKind::Document(Syntax::Json)),
    field("metadata", "session.protocol.metadata", "", FieldKind::Pairs),
];

const SOAP_FIELDS: &[FieldSpec] = &[
    field("endpointUrl", "session.protocol.endpoint", "https://example.test/service", FieldKind::Line),
    field("soapAction", "session.protocol.soap_action", "session.protocol.soap_action_placeholder", FieldKind::Line),
    field("body", "session.protocol.envelope", "<soap:Envelope>…</soap:Envelope>", FieldKind::Document(Syntax::Markup)),
    field("headers", "session.protocol.headers", "", FieldKind::Pairs),
];

/// A hint is a key; an example value (a URL, a sample document) is shown as it is.
fn placeholder_of(spec: &FieldSpec) -> &'static str {
    if spec.placeholder.starts_with("session.") { t(spec.placeholder) } else { spec.placeholder }
}

/// The controls of each protocol's editor. The first `Line` field is the target, shown beside Send.
fn fields_of(kind: RequestKind) -> &'static [FieldSpec] {
    match kind {
        RequestKind::GraphQl => GRAPHQL_FIELDS,
        RequestKind::Grpc => GRPC_FIELDS,
        RequestKind::Soap => SOAP_FIELDS,
        RequestKind::Rest | RequestKind::WebSocket | RequestKind::Sse | RequestKind::SocketIo | RequestKind::Mcp => &[],
    }
}

/// The fields `*.send` takes inline for a request of this kind, taken from the edited entity.
fn inline_request(kind: RequestKind, entity: &Map<String, Value>) -> Value {
    let mut request = Map::new();
    for spec in fields_of(kind) {
        request.insert(spec.key.into(), entity.get(spec.key).cloned().unwrap_or(Value::Null));
    }
    if kind == RequestKind::Grpc {
        request.insert("protoFileId".into(), entity.get("protoFileId").cloned().unwrap_or(Value::Null));
        request.insert("useReflection".into(), entity.get("useReflection").cloned().unwrap_or(json!(false)));
    }
    Value::Object(request)
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
        RequestKind::GraphQl
        | RequestKind::Rest
        | RequestKind::WebSocket
        | RequestKind::Sse
        | RequestKind::SocketIo
        | RequestKind::Mcp => None,
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
        RequestKind::Rest | RequestKind::WebSocket | RequestKind::Sse | RequestKind::SocketIo | RequestKind::Mcp => json!({}),
    };
    value.as_object().cloned().unwrap_or_default()
}

/// A method of a service the server reported.
#[derive(Clone)]
struct MethodInfo {
    name: String,
    client_streaming: bool,
    server_streaming: bool,
}

impl MethodInfo {
    /// The value of the request's `streamingType` field.
    fn streaming_type(&self) -> &'static str {
        match (self.client_streaming, self.server_streaming) {
            (false, false) => "unary",
            (false, true) => "server",
            (true, false) => "client",
            (true, true) => "bidi",
        }
    }

    fn kind_label(&self) -> &'static str {
        t(match self.streaming_type() {
            "unary" => "session.protocol.unary",
            "server" => "session.protocol.server_streaming",
            "client" => "session.protocol.client_streaming",
            _ => "session.protocol.bidi_streaming",
        })
    }
}

/// How a method moves messages, drawn as arrows: one arrow in each direction for a single message, two for
/// a stream. Right is what the client sends, left is what the server answers.
fn kind_arrows(client_streaming: bool, server_streaming: bool, color: gpui_kit::Hsla) -> gpui_kit::AnyElement {
    let group = |icon: IconName, count: usize| {
        h_flex().gap_0p5().children((0..count).map(move |_| Icon::new(icon.clone()).small().text_color(color)))
    };
    h_flex()
        .flex_none()
        .gap_2()
        .items_center()
        .child(group(IconName::ArrowRight, if client_streaming { 2 } else { 1 }))
        .child(group(IconName::ArrowLeft, if server_streaming { 2 } else { 1 }))
        .into_any_element()
}

/// A method in the method list: its name and arrows for how it streams.
#[derive(Clone)]
struct MethodItem {
    name: String,
    client_streaming: bool,
    server_streaming: bool,
}

impl SearchableListItem for MethodItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &String {
        &self.name
    }

    fn display_title(&self) -> Option<gpui_kit::AnyElement> {
        None
    }

    fn render(&self, _: &mut Window, cx: &mut gpui_kit::App) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        h_flex()
            .w_full()
            .gap_3()
            .justify_between()
            .child(div().min_w_0().child(self.name.clone()))
            .child(kind_arrows(self.client_streaming, self.server_streaming, muted))
    }
}

#[derive(Clone)]
struct ServiceInfo {
    name: String,
    methods: Vec<MethodInfo>,
}

enum Reflection {
    /// There is no address to ask.
    Idle,
    Loading,
    Ready,
    /// The server could not be asked, or does not answer reflection. The names are typed instead.
    Failed(String),
}

/// What a gRPC request knows about the server it points at: the services and methods the server reports
/// through reflection, offered in two searchable lists.
struct GrpcPicker {
    services: Vec<ServiceInfo>,
    state: Reflection,
    /// The address the schema is, or is being, read for.
    address: String,
    /// Which read is current; the answer to an older one is dropped.
    generation: u64,
    wait: Autosave,
    service_box: Entity<ComboboxState<Vec<String>>>,
    method_box: Entity<ComboboxState<Vec<MethodItem>>>,
}

fn parse_services(result: &Value) -> Vec<ServiceInfo> {
    result
        .get("services")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|service| ServiceInfo {
            name: service.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
            methods: service
                .get("methods")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|method| MethodInfo {
                    name: method.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
                    client_streaming: method.get("clientStreaming").and_then(Value::as_bool).unwrap_or(false),
                    server_streaming: method.get("serverStreaming").and_then(Value::as_bool).unwrap_or(false),
                })
                .collect(),
        })
        .collect()
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
    /// The folder the engine has it in. It differs from `folder_id` while a move is waiting to be written.
    saved_folder: Option<String>,
    entity: Map<String, Value>,
    load: LoadState,
    name: Entity<InputState>,
    controls: Vec<Control>,
    /// Index into the fields that are shown as sections (documents and tables).
    section: usize,
    baseline: (String, Vec<FieldValue>),
    exchange: Entity<Exchange>,
    /// Only a gRPC request has one.
    grpc: Option<GrpcPicker>,
    /// Only a GraphQL request has one: the schema read from its endpoint, kept in memory.
    schema: Option<SchemaPanel>,
    /// Only a SOAP request has one: the operations read from its WSDL, kept in memory.
    wsdl: Option<WsdlPanel>,
    /// What the engine knows this tab's live call as.
    stream_id: String,
    /// A live call is open (or opening) for this request.
    live: bool,
    /// The engine has the call open, so it can be fed, ended and cancelled. Until then only the wait is on.
    connected: bool,
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
            cx.new(|cx| InputState::new(window, cx).placeholder(t("session.rest.name_placeholder")).default_value(default_name(kind)));
        let exchange = cx.new(|cx| Exchange::new(window, cx));

        let mut subscriptions =
            vec![cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.changed(cx),
                InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit_name(window, cx),
                InputEvent::Focus => {}
            })];
        subscriptions.push(cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| {
            match event {
                ModelEvent::EntityReverted(id) if this.saved_id.as_deref() == Some(id.as_str()) => {
                    this.reload(window, cx);
                }
                ModelEvent::GrpcMessage(payload) if this.is_my_stream(payload) => {
                    this.exchange.update(cx, |exchange, cx| exchange.live_message(payload, cx));
                }
                ModelEvent::GrpcStatus(payload) if this.is_my_stream(payload) => {
                    if payload.get("status").and_then(Value::as_str) == Some("closed") {
                        this.live = false;
                        this.connected = false;
                    } else if this.live {
                        this.connected = true;
                    }
                    this.exchange.update(cx, |exchange, cx| exchange.live_status(payload, cx));
                    cx.notify();
                }
                _ => {}
            }
        }));
        let controls: Vec<Control> = fields_of(kind)
            .iter()
            .map(|spec| match spec.kind {
                FieldKind::Line => {
                    let editor = VarEditor::line(placeholder_of(spec), window, cx);
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
                    let editor = VarEditor::code(syntax, placeholder_of(spec), window, cx);
                    subscriptions.push(cx.subscribe(editor.state(), |this, _, event: &InputEvent, cx| {
                        if matches!(event, InputEvent::Change) {
                            this.changed(cx);
                        }
                    }));
                    Control::Text(editor)
                }
                FieldKind::Pairs => {
                    let table = cx.new(|cx| KeyValueTable::new(t("session.protocol.column_name"), t("session.rest.column_value"), window, cx));
                    subscriptions.push(cx.subscribe(&table, |this, _, _: &KeyValueEvent, cx| this.changed(cx)));
                    Control::Pairs(table)
                }
            })
            .collect();

        let schema = (kind == RequestKind::GraphQl).then(|| {
            let panel = SchemaPanel::new(window, cx);
            subscriptions.push(cx.subscribe(&panel.filter, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
            let query = fields_of(kind).iter().position(|spec| spec.key == "query");
            if let Some(Control::Text(editor)) = query.and_then(|ix| controls.get(ix)) {
                editor.state().update(cx, |state, _| state.lsp_mut().completion_provider = Some(panel.completions()));
            }
            panel
        });

        let wsdl = (kind == RequestKind::Soap).then(|| WsdlPanel::new(window, cx));

        let grpc = (kind == RequestKind::Grpc).then(|| {
            let service_box = cx.new(|cx| ComboboxState::new(Vec::<String>::new(), vec![], window, cx).searchable(true));
            let method_box = cx.new(|cx| ComboboxState::new(Vec::<MethodItem>::new(), vec![], window, cx).searchable(true));
            subscriptions.push(cx.subscribe_in(
                &service_box,
                window,
                |this, _, event: &ComboboxEvent<Vec<String>>, window, cx| {
                    if let ComboboxEvent::Change(values) = event {
                        this.choose_service(values.first().cloned().unwrap_or_default(), window, cx);
                    }
                },
            ));
            subscriptions.push(cx.subscribe_in(
                &method_box,
                window,
                |this, _, event: &ComboboxEvent<Vec<MethodItem>>, window, cx| {
                    if let ComboboxEvent::Change(values) = event {
                        this.choose_method(values.first().cloned().unwrap_or_default(), window, cx);
                    }
                },
            ));
            GrpcPicker {
                services: Vec::new(),
                state: Reflection::Idle,
                address: String::new(),
                generation: 0,
                wait: Autosave::new(window),
                service_box,
                method_box,
            }
        });

        let mut this = Self {
            model,
            kind,
            workspace_id,
            saved_id,
            saved_folder: folder_id.clone(),
            folder_id,
            entity: defaults_of(kind),
            load: LoadState::Ready,
            name,
            controls,
            section: 0,
            baseline: (String::new(), Vec::new()),
            exchange,
            grpc,
            schema,
            wsdl,
            stream_id: format!("grpc-{}", cx.entity_id().as_u64()),
            live: false,
            connected: false,
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
                    .ok_or_else(|| t("session.rest.gone").to_owned())
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
        if name.is_empty() {
            t_with("session.protocol.untitled", &[("kind", &self.kind.label())]).into()
        } else {
            name
        }
    }

    pub fn is_dirty(&self, cx: &gpui_kit::App) -> bool {
        self.load == LoadState::Ready && (self.values(cx) != self.baseline || self.folder_id != self.saved_folder)
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
            .then(|| t_with("session.protocol.problem_no_target", &[("target", &self.target_label())]).into())
    }

    /// What the main button does for this request.
    fn send_label(&self, cx: &gpui_kit::App) -> &'static str {
        match (self.kind, self.live) {
            (RequestKind::Grpc, true) => t("session.protocol.cancel"),
            (RequestKind::Grpc, false) if self.streaming_kind(cx) != "unary" => t("session.protocol.invoke"),
            _ => t("session.rest.send"),
        }
    }

    fn is_my_stream(&self, payload: &Value) -> bool {
        payload.get("workspaceId").and_then(Value::as_str) == Some(self.workspace_id.as_str())
            && payload.get("streamId").and_then(Value::as_str) == Some(self.stream_id.as_str())
    }

    /// How the selected method streams: from the schema when it is known, else as the request last recorded.
    fn streaming_kind(&self, cx: &gpui_kit::App) -> &'static str {
        let service = self.text_of("serviceName", cx).unwrap_or_default();
        let method = self.text_of("methodName", cx).unwrap_or_default();
        if let Some(info) = self.method_info(service.trim(), method.trim()) {
            return info.streaming_type();
        }
        match self.entity.get("streamingType").and_then(Value::as_str) {
            Some("server") => "server",
            Some("client") => "client",
            Some("bidi") => "bidi",
            _ => "unary",
        }
    }

    /// Starts the call as a live one: its messages arrive as events until it ends.
    fn invoke(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sending || self.live || self.load != LoadState::Ready {
            return;
        }
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
        // Live from the start, so that an end that arrives before the answer to this command is not undone by it.
        self.live = true;
        self.connected = false;
        self.exchange.update(cx, |exchange, cx| exchange.begin_live(cx));
        self.changed(cx);
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "streamId": self.stream_id, "id": id });
        call_in(&client, window, cx, "grpc.stream.open", payload, move |this, outcome, _, cx| {
            this.sending = false;
            let failure = match outcome {
                Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(true) => {
                    // A call that has already ended by the time this answer arrives stays ended.
                    this.connected = this.live;
                    None
                }
                Ok(result) => Some(result.get("error").and_then(Value::as_str).unwrap_or(t("session.protocol.call_failed")).to_owned()),
                Err(error) => Some(describe(&error)),
            };
            if let Some(message) = failure {
                this.live = false;
                this.connected = false;
                this.exchange.update(cx, |exchange, cx| exchange.fail(message, cx));
            }
            this.changed(cx);
        });
    }

    fn live_command(&mut self, action: &'static str, payload: Value, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call_in(&client, window, cx, action, payload, move |_, outcome, window, cx| {
            let failure = match outcome {
                Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(false) => {
                    result.get("error").and_then(Value::as_str).map(str::to_owned)
                }
                Ok(_) => None,
                Err(error) => Some(describe(&error)),
            };
            if let Some(message) = failure {
                window.push_notification(Notification::error(message).title(t("session.protocol.call_not_taken")), cx);
            }
        });
    }

    fn cancel_call(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connected {
            self.request_cancel(window, cx);
        }
    }

    fn request_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let payload = json!({ "workspaceId": self.workspace_id, "streamId": self.stream_id });
        self.live_command("grpc.stream.cancel", payload, window, cx);
    }

    fn end_streaming(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.connected {
            return;
        }
        let payload = json!({ "workspaceId": self.workspace_id, "streamId": self.stream_id });
        self.live_command("grpc.stream.end", payload, window, cx);
    }

    /// Sends what is in the message editor to the open call.
    fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.connected {
            return;
        }
        let data = self.text_of("requestBody", cx).unwrap_or_default();
        let payload = json!({ "workspaceId": self.workspace_id, "streamId": self.stream_id, "data": data });
        self.live_command("grpc.stream.send", payload, window, cx);
    }

    /// A new server address is read for its schema once typing pauses.
    fn address_changed(&mut self, cx: &mut Context<Self>) {
        let address = self.text_of("serverAddress", cx).unwrap_or_default().trim().to_owned();
        let Some(picker) = self.grpc.as_mut() else { return };
        if picker.address == address {
            return;
        }
        picker.address = address.clone();
        picker.generation += 1;
        picker.services.clear();
        if address.is_empty() {
            picker.state = Reflection::Idle;
            picker.wait.cancel();
        } else {
            picker.state = Reflection::Loading;
            picker.wait.schedule(cx, |this, window, cx| this.reflect_now(window, cx));
        }
        cx.notify();
    }

    /// Asks the server what it serves.
    fn reflect_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.grpc.as_mut() else { return };
        if picker.address.is_empty() {
            return;
        }
        picker.generation += 1;
        picker.state = Reflection::Loading;
        let generation = picker.generation;
        let payload = json!({ "serverAddress": picker.address, "workspaceId": self.workspace_id });
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call_in(&client, window, cx, "grpc.reflect", payload, move |this, outcome, window, cx| {
            let Some(picker) = this.grpc.as_mut() else { return };
            if picker.generation != generation {
                return;
            }
            let failure = match outcome {
                Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(true) => {
                    picker.services = parse_services(&result);
                    picker.state = Reflection::Ready;
                    None
                }
                Ok(result) => Some(
                    result.get("error").and_then(Value::as_str).unwrap_or(t("session.protocol.server_no_answer")).to_owned(),
                ),
                Err(error) => Some(describe(&error)),
            };
            if let Some(message) = failure {
                picker.services.clear();
                picker.state = Reflection::Failed(message);
            }
            this.sync_picker(window, cx);
            cx.notify();
        });
    }

    /// Puts the lists' items and selections in line with the schema and the typed names.
    fn sync_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let service_name = self.text_of("serviceName", cx).unwrap_or_default().trim().to_owned();
        let method_name = self.text_of("methodName", cx).unwrap_or_default().trim().to_owned();
        let Some(picker) = self.grpc.as_ref() else { return };
        let services: Vec<String> = picker.services.iter().map(|service| service.name.clone()).collect();
        let methods: Vec<MethodItem> = picker
            .services
            .iter()
            .find(|service| service.name == service_name)
            .map(|service| {
                service
                    .methods
                    .iter()
                    .map(|method| MethodItem {
                        name: method.name.clone(),
                        client_streaming: method.client_streaming,
                        server_streaming: method.server_streaming,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let (service_box, method_box) = (picker.service_box.clone(), picker.method_box.clone());
        service_box.update(cx, |list, cx| {
            list.set_items(services, window, cx);
            list.set_selected_values(&[service_name], window, cx);
        });
        method_box.update(cx, |list, cx| {
            list.set_items(methods, window, cx);
            list.set_selected_values(&[method_name], window, cx);
        });
    }

    fn set_text(&self, key: &str, value: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Control::Text(editor)) = self.control(key) {
            editor.set_value(value, window, cx);
        }
    }

    fn choose_service(&mut self, service: String, window: &mut Window, cx: &mut Context<Self>) {
        if service.is_empty() {
            return;
        }
        let current = self.text_of("methodName", cx).unwrap_or_default().trim().to_owned();
        let methods: Vec<String> = self
            .grpc
            .as_ref()
            .and_then(|picker| picker.services.iter().find(|s| s.name == service))
            .map(|s| s.methods.iter().map(|m| m.name.clone()).collect())
            .unwrap_or_default();
        // The method stays when the new service has it; a service with one method has no choice to make.
        let method = match methods.as_slice() {
            _ if methods.contains(&current) => current,
            [only] => only.clone(),
            _ => String::new(),
        };
        self.set_text("serviceName", service, window, cx);
        self.set_text("methodName", method.clone(), window, cx);
        self.note_streaming_type(&method);
        self.sync_picker(window, cx);
    }

    fn choose_method(&mut self, method: String, window: &mut Window, cx: &mut Context<Self>) {
        if method.is_empty() {
            return;
        }
        self.set_text("methodName", method.clone(), window, cx);
        self.note_streaming_type(&method);
        cx.notify();
    }

    /// The saved request records how its method streams.
    fn note_streaming_type(&mut self, method: &str) {
        let service = self.entity_text("serviceName");
        if let Some(info) = self.method_info(&service, method) {
            self.entity.insert("streamingType".into(), json!(info.streaming_type()));
        }
    }

    fn entity_text(&self, key: &str) -> String {
        self.entity.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
    }

    fn method_info(&self, service: &str, method: &str) -> Option<MethodInfo> {
        let picker = self.grpc.as_ref()?;
        picker.services.iter().find(|s| s.name == service)?.methods.iter().find(|m| m.name == method).cloned()
    }

    /// The service and method controls: two searchable lists once the schema is known, the typed names
    /// when the server cannot be asked.
    fn grpc_row(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let Some(picker) = &self.grpc else { return div().into_any_element() };
        let specs = fields_of(self.kind);
        let line = |key: &str| {
            specs
                .iter()
                .position(|spec| spec.key == key)
                .and_then(|ix| match &self.controls[ix] {
                    Control::Text(editor) => Some(editor.line_field(cx).into_any_element()),
                    Control::Pairs(_) => None,
                })
                .unwrap_or_else(|| div().into_any_element())
        };
        let typed = matches!(picker.state, Reflection::Failed(_));
        let ready = matches!(picker.state, Reflection::Ready);
        let (service_control, method_control) = if typed {
            (line("serviceName"), line("methodName"))
        } else {
            let service_text = self.text_of("serviceName", cx).unwrap_or_default();
            let service_hint = match picker.state {
                Reflection::Loading => t("session.protocol.reading_schema"),
                _ if !ready => t("session.protocol.enter_address_first"),
                _ => t("session.protocol.select_service"),
            };
            (
                Combobox::new(&picker.service_box)
                    .placeholder(service_hint)
                    .search_placeholder(t("session.protocol.search_services"))
                    .disabled(!ready || self.live)
                    .w_full()
                    .into_any_element(),
                Combobox::new(&picker.method_box)
                    .placeholder(if service_text.trim().is_empty() {
                        t("session.protocol.select_service_first")
                    } else {
                        t("session.protocol.select_method")
                    })
                    .search_placeholder(t("session.protocol.search_methods"))
                    .disabled(!ready || self.live || service_text.trim().is_empty())
                    .w_full()
                    .into_any_element(),
            )
        };
        let muted = theme.muted_foreground;
        let note: Option<gpui_kit::AnyElement> = match &picker.state {
            Reflection::Failed(message) => Some(
                div()
                    .text_xs()
                    .text_color(theme.warning)
                    .child(t_with("session.protocol.reflect_failed", &[("message", message)]))
                    .into_any_element(),
            ),
            Reflection::Ready => {
                let service = self.text_of("serviceName", cx).unwrap_or_default();
                let method = self.text_of("methodName", cx).unwrap_or_default();
                match self.method_info(service.trim(), method.trim()) {
                    Some(info) => Some(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .text_xs()
                            .text_color(muted)
                            .child(kind_arrows(info.client_streaming, info.server_streaming, muted))
                            .child(info.kind_label())
                            .into_any_element(),
                    ),
                    None if service.trim().is_empty() => {
                        let count = picker.services.len();
                        Some(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(if count == 1 {
                                    t_with("session.protocol.services_on_server_one", &[("count", &count)])
                                } else {
                                    t_with("session.protocol.services_on_server_other", &[("count", &count)])
                                })
                                .into_any_element(),
                        )
                    }
                    None => None,
                }
            }
            _ => None,
        };
        v_flex()
            .flex_none()
            .gap_1()
            .px_3()
            .pb_2()
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(v_flex().flex_1().min_w_0().gap_1().child(page::section_label(t("session.protocol.service"), cx)).child(service_control))
                    .child(v_flex().flex_1().min_w_0().gap_1().child(page::section_label(t("session.protocol.method"), cx)).child(method_control)),
            )
            .children(note)
            .into_any_element()
    }

    pub(super) fn schema_panel_mut(&mut self) -> Option<&mut SchemaPanel> {
        self.schema.as_mut()
    }

    /// Reads the endpoint's schema. The URL and header values are resolved here, as sending would, because
    /// `graphql.introspect` calls exactly what it is given.
    pub(super) fn fetch_schema(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        /// Separates the texts resolved in one call; no URL or header value holds it.
        const SEPARATOR: char = '\u{1f}';
        let url = self.text_of("endpointUrl", cx).unwrap_or_default().replace(['\r', '\n'], "").trim().to_owned();
        if url.is_empty() {
            window.push_notification(
                Notification::warning(t_with("session.protocol.enter_target_first", &[("target", &self.target_label())]))
                    .title(t("session.schema.fetch_failed_title")),
                cx,
            );
            return;
        }
        let headers = self.pairs_of("headers", cx).unwrap_or_default();
        let Some(panel) = self.schema.as_mut().filter(|panel| !panel.fetching) else { return };
        panel.fetching = true;
        cx.notify();
        let unresolved: Vec<String> = std::iter::once(url).chain(headers.iter().map(|(_, value)| value.clone())).collect();
        let resolve = json!({ "workspaceId": self.workspace_id, "text": unresolved.join(&SEPARATOR.to_string()) });
        let client = self.model.read(cx).client().clone();
        let introspect_client = client.clone();
        call_in(&client, window, cx, "env.resolveText", resolve, move |_, outcome, window, cx| {
            let resolved = outcome
                .ok()
                .and_then(|result| {
                    result.get("text").and_then(Value::as_str).map(|text| text.split(SEPARATOR).map(str::to_owned).collect::<Vec<_>>())
                })
                .filter(|parts| parts.len() == unresolved.len())
                .unwrap_or(unresolved);
            let values = resolved[1..].iter().cloned();
            let pairs: Vec<(String, String)> = headers.into_iter().map(|(name, _)| name).zip(values).collect();
            let payload = json!({ "url": resolved[0], "headers": header_map(&pairs) });
            call_in(&introspect_client, window, cx, "graphql.introspect", payload, |this, outcome, window, cx| {
                let failure = match outcome {
                    Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(true) => {
                        match Schema::parse(result.get("sdl").and_then(Value::as_str).unwrap_or_default()) {
                            Ok(schema) => {
                                if let Some(panel) = this.schema.as_mut() {
                                    panel.set_schema(schema);
                                }
                                None
                            }
                            Err(message) => Some(message),
                        }
                    }
                    Ok(result) => Some(result.get("error").and_then(Value::as_str).unwrap_or_default().to_owned()),
                    Err(error) => Some(describe(&error)),
                };
                if let Some(panel) = this.schema.as_mut() {
                    panel.fetching = false;
                }
                if let Some(message) = failure {
                    window.push_notification(Notification::error(message).title(t("session.schema.fetch_failed_title")), cx);
                }
                cx.notify();
            });
        });
    }

    /// Reads the service's WSDL: the typed address, else the endpoint's with `?wsdl`. Resolved here, as sending would.
    pub(super) fn fetch_wsdl(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = self.wsdl.as_ref().filter(|panel| !panel.fetching) else { return };
        let mut url = panel.url.read(cx).value().trim().to_owned();
        if url.is_empty() {
            url = self.text_of("endpointUrl", cx).unwrap_or_default().replace(['\r', '\n'], "").trim().to_owned();
            if url.is_empty() {
                window.push_notification(
                    Notification::warning(t_with("session.protocol.enter_target_first", &[("target", &self.target_label())]))
                        .title(t("session.wsdl.fetch_failed_title")),
                    cx,
                );
                return;
            }
            if !url.contains('?') {
                url.push_str("?wsdl");
            }
        }
        if let Some(panel) = self.wsdl.as_mut() {
            panel.fetching = true;
        }
        cx.notify();
        let client = self.model.read(cx).client().clone();
        let fetch_client = client.clone();
        let resolve = json!({ "workspaceId": self.workspace_id, "text": url });
        call_in(&client, window, cx, "env.resolveText", resolve, move |_, outcome, window, cx| {
            let resolved = outcome.ok().and_then(|result| result.get("text").and_then(Value::as_str).map(str::to_owned)).unwrap_or(url);
            call_in(&fetch_client, window, cx, "soap.fetchWsdl", json!({ "url": resolved }), |this, outcome, window, cx| {
                let failure = match outcome {
                    Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(true) => {
                        match wsdl::parse(result.get("content").and_then(Value::as_str).unwrap_or_default()) {
                            Ok(operations) => {
                                if let Some(panel) = this.wsdl.as_mut() {
                                    panel.set_operations(operations);
                                }
                                None
                            }
                            Err(message) => Some(message),
                        }
                    }
                    Ok(result) => Some(result.get("error").and_then(Value::as_str).unwrap_or_default().to_owned()),
                    Err(error) => Some(describe(&error)),
                };
                if let Some(panel) = this.wsdl.as_mut() {
                    panel.fetching = false;
                }
                if let Some(message) = failure {
                    window.push_notification(Notification::error(message).title(t("session.wsdl.fetch_failed_title")), cx);
                }
                cx.notify();
            });
        });
    }

    /// Fills the SOAPAction, the endpoint when it is empty, and the envelope when the body is empty or is still
    /// an envelope this tab generated for one of the operations; a body the user has written is left alone.
    pub(super) fn choose_operation(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = self.wsdl.as_mut() else { return };
        let Some(operation) = panel.operations().get(ix).cloned() else { return };
        let generated: Vec<String> = panel.operations().iter().map(wsdl::skeleton).collect();
        panel.select(ix);
        self.set_text("soapAction", operation.action.clone(), window, cx);
        if self.text_of("endpointUrl", cx).unwrap_or_default().trim().is_empty() && !operation.endpoint.is_empty() {
            self.set_text("endpointUrl", operation.endpoint.clone(), window, cx);
        }
        let body = self.text_of("body", cx).unwrap_or_default();
        if body.trim().is_empty() || generated.contains(&body) {
            self.set_text("body", wsdl::skeleton(&operation), window, cx);
        }
        cx.notify();
    }

    /// The first field is the target; a request without one cannot be written.
    fn target_is_empty(values: &(String, Vec<FieldValue>)) -> bool {
        matches!(values.1.first(), Some(FieldValue::Text(text)) if text.is_empty())
    }

    fn target_label(&self) -> String {
        t(fields_of(self.kind)[0].label).to_lowercase()
    }

    /// A saved request is written a moment after each change; a draft waits for Save.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.address_changed(cx);
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
        // The tab is closing: ask the engine to drop the call whether or not it has finished connecting.
        if self.live {
            self.request_cancel(window, cx);
        }
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
        self.sync_picker(window, cx);
    }

    /// Fills this draft with a request the engine built, from a cURL command. It stays a draft, in the
    /// folder it was started in, and what was filled in is its first edit. What the command does not say
    /// keeps the value a new request of this type starts with.
    pub fn adopt_import(&mut self, entity: Map<String, Value>, window: &mut Window, cx: &mut Context<Self>) {
        let (folder, baseline) = (self.folder_id.clone(), self.baseline.clone());
        let mut merged = self.entity.clone();
        merged.extend(entity);
        self.adopt_entity(merged, window, cx);
        (self.folder_id, self.saved_folder) = (folder.clone(), folder);
        self.baseline = baseline;
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
        self.saved_folder = self.folder_id.clone();
        if let Some(name) = entity.get("name").and_then(Value::as_str) {
            self.name.update(cx, |state, cx| state.set_value(name.to_owned(), window, cx));
        }
        self.fill_controls(&entity, window, cx);
        self.entity = entity;
        self.baseline = self.values(cx);
        self.address_changed(cx);
        self.sync_picker(window, cx);
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
        if !creating && values == self.baseline && self.folder_id == self.saved_folder {
            return;
        }
        if Self::target_is_empty(&values) {
            self.send_after_save = false;
            if mode == SaveMode::Explicit {
                window.push_notification(
                    Notification::warning(t_with("session.protocol.enter_target_first", &[("target", &self.target_label())]))
                        .title(t("session.rest.save_no_url_title")),
                    cx,
                );
            }
            return;
        }
        let mut entity = self.edited_entity(&values);
        entity.insert("workspaceId".into(), json!(self.workspace_id));
        entity.insert("folderId".into(), json!(self.folder_id));
        let folder = self.folder_id.clone();

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
                            let placed = engine_folder(saved);
                            this.entity = saved.clone();
                            this.baseline = values;
                            if this.folder_id == folder {
                                this.folder_id = placed.clone();
                            }
                            this.saved_folder = placed;
                            if let Some(id) = this.saved_id.clone() {
                                cx.emit(SessionEvent::Saved { id });
                            }
                        }
                        None => window
                            .push_notification(Notification::error(t("session.rest.engine_no_saved")), cx),
                    }
                    saved.is_some()
                }
                Ok(_) => {
                    this.entity = edited;
                    this.baseline = values;
                    this.saved_folder = folder.clone();
                    true
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
            };
            // An edit made since this write was sent is the next write. (`changed` schedules it.)
            this.changed(cx);
            if send_next && saved {
                this.send(window, cx);
            }
        });
    }

    /// Sends the request: a saved, unedited one by id, anything else as the editor holds it.
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self.kind.send_command() else { return };
        if self.kind == RequestKind::Grpc {
            if self.live {
                self.cancel_call(window, cx);
                return;
            }
            if self.streaming_kind(cx) != "unary" {
                self.invoke(window, cx);
                return;
            }
        }
        if self.sending || self.load != LoadState::Ready {
            return;
        }
        let values = self.values(cx);
        let payload = match self.saved_id.clone().filter(|_| !self.is_dirty(cx)) {
            Some(id) => json!({ "workspaceId": self.workspace_id, "id": id }),
            // A draft, or an edit not yet saved, is sent as the editor holds it, without saving it.
            None => {
                if Self::target_is_empty(&values) {
                    window.push_notification(
                        Notification::warning(t_with("session.protocol.enter_target_first", &[("target", &self.target_label())]))
                            .title(t("session.rest.send_no_url_title")),
                        cx,
                    );
                    return;
                }
                let entity = self.edited_entity(&values);
                json!({ "workspaceId": self.workspace_id, "request": inline_request(self.kind, &entity) })
            }
        };
        self.sending = true;
        self.exchange.update(cx, |exchange, cx| exchange.begin(cx));
        self.changed(cx);
        let client = self.model.read(cx).client().clone();
        call_in(&client, window, cx, action, payload, move |this, outcome, window, cx| {
            this.sending = false;
            match outcome {
                // A gRPC call that never got an answer (no connection, a bad message) has no status to show.
                Ok(result) if this.kind == RequestKind::Grpc => {
                    let unanswered = result.get("ok").and_then(Value::as_bool) == Some(false) && result.get("status").is_none();
                    if unanswered {
                        let message =
                            result.get("error").and_then(Value::as_str).unwrap_or(t("session.protocol.call_failed")).to_owned();
                        this.exchange.update(cx, |exchange, cx| exchange.fail(message, cx));
                    } else {
                        this.exchange.update(cx, |exchange, cx| exchange.finish_grpc(&result, window, cx));
                    }
                }
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
            LoadState::Loading => return page::loading(t("session.rest.loading")),
            LoadState::Failed(message) => {
                return div()
                    .size_full()
                    .p_4()
                    .child(
                        Alert::error("load-failed", SharedString::from(message.clone()))
                            .title(t("session.rest.open_failed")),
                    )
                    .into_any_element();
            }
            LoadState::Ready => {}
        }

        let sections = self.sections();
        // A GraphQL request has one more tab than it has documents: the schema. A SOAP request's is the WSDL.
        let (extra_label, extra_view) = match (&self.schema, &self.wsdl) {
            (Some(panel), _) => (Some("session.schema.tab"), Some(panel.view(cx))),
            (_, Some(panel)) => (Some("session.wsdl.tab"), Some(panel.view(cx))),
            _ => (None, None),
        };
        let tabs = sections.len() + usize::from(extra_label.is_some());
        let section = self.section.min(tabs.saturating_sub(1));
        let content = match sections.get(section) {
            Some((ix, _)) => Some(match &self.controls[*ix] {
                Control::Text(editor) => Editor::new(editor.state()).bordered(false).h_full().into_any_element(),
                Control::Pairs(table) => table.clone().into_any_element(),
            }),
            None => extra_view,
        };
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

        // What a method the client streams to adds: after the call is invoked, each message is sent by hand.
        let mut secondary = Vec::new();
        if self.grpc.is_some() && matches!(self.streaming_kind(cx), "client" | "bidi") {
            let open = self.live && self.connected;
            secondary.push(
                Button::new("grpc-send-message")
                    .primary()
                    .label(t("session.protocol.send_message"))
                    .disabled(!open)
                    .on_click(cx.listener(|this, _, window, cx| this.send_message(window, cx))),
            );
            secondary.push(
                Button::new("grpc-end-streaming")
                    .outline()
                    .label(t("session.protocol.end_streaming"))
                    .disabled(!open)
                    .on_click(cx.listener(|this, _, window, cx| this.end_streaming(window, cx))),
            );
        }

        v_flex()
            .size_full()
            .bg(theme.background)
            .child(request_bar(
                None,
                line(0),
                Button::new("send-request")
                    .primary()
                    .label(self.send_label(cx))
                    .disabled(self.live && !self.connected)
                    .loading(self.sending)
                    .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                secondary,
                cx,
            ))
            .child(if self.grpc.is_some() {
                self.grpc_row(cx)
            } else {
                h_flex()
                    .flex_none()
                    .gap_3()
                    .px_3()
                    .pb_2()
                    .items_start()
                    .children(extra_lines.into_iter().map(|ix| {
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(page::section_label(t(specs[ix].label), cx))
                            .child(line(ix))
                    }))
                    .into_any_element()
            })
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
                        .children(sections.iter().map(|(_, spec)| Tab::new().label(t(spec.label))))
                        .children(extra_label.map(|key| Tab::new().label(t(key)))),
                ),
            )
            .child(div().flex_1().min_h_0().children(content))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inline_request_holds_only_what_the_engine_takes() {
        let entity = json!({
            "id": "r1", "name": "n", "workspaceId": "w", "endpointUrl": "u", "query": "{ a }", "variables": "",
            "operationName": "", "headers": { "a": "b" },
        });
        let request = inline_request(RequestKind::GraphQl, entity.as_object().unwrap());
        assert_eq!(
            request,
            json!({ "endpointUrl": "u", "operationName": "", "query": "{ a }", "variables": "", "headers": { "a": "b" } })
        );
        let grpc = inline_request(RequestKind::Grpc, json!({ "serverAddress": "s", "metadata": {} }).as_object().unwrap());
        assert_eq!(grpc["useReflection"], json!(false));
        assert_eq!(grpc["protoFileId"], Value::Null);
    }
}
