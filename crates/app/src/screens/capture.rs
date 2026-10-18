//! The Capture screen: the traffic the proxy has seen, newest first, and one exchange in detail, much like
//! a browser's Network tab.
//!
//! The history is the engine's (a window opened later still sees what passed) and is extended live by
//! `event.log.entry`. Both carry only a light summary of each exchange; the headers and bodies of one are
//! fetched by id (`capture.get`) when it is selected. Pausing stops the engine recording
//! (`capture.setPaused`); nothing passes into the history until it is resumed. An exchange can be turned into a saved request or a mock: the engine builds the entity
//! from the stored entry (`capture.buildRequest`, `capture.buildMock`) and this screen saves what it built.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _, StyledExt as _, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonGroup, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::DialogFooter,
    form::{Field, Form},
    h_flex,
    input::{Editor, EditorState, Input, InputEvent, InputState},
    list::ListItem,
    notification::Notification,
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    TestSupportExt as _, UniformListScrollHandle, Window, div, px, uniform_list,
};
use serde_json::{Value, json};
use std::{collections::HashSet, ops::Range};

use crate::{
    app_model::{AppModel, ModelEvent},
    components::{
        Syntax,
        binary::{BinaryBody, save_to_file, suggested_file_name},
        dialogs::{PromptField, prompt},
        master_detail::list_over_detail,
        page,
    },
    engine::{call, call_in, describe, failure, fetch_artifact},
    i18n::{t, t_with},
    model::{
        git::date_time_utc,
        http::{pretty_json, status_text},
        logs::{self, Found},
        media::{self, BodyClass},
        snippet::{RequestParts, SnippetKind, snippet},
        traffic::{self, Exchange, Handler, RESPONSE_BODY_LIMIT, ResourceType, TrafficEntry, body_class, clock_utc},
        vault::VaultState,
    },
};

/// The origin the Webhooks screen's listeners record their requests under.
const WEBHOOK: &str = "webhook";

/// Several exchanges being saved one after the other as requests or mocks.
struct SaveRun {
    /// `(capture id, name)` still to save, in the order they are saved.
    pending: Vec<(String, String)>,
    mock: bool,
    /// The folder they go into; `None` saves at the top level.
    folder: Option<(String, String)>,
    saved: usize,
    last: Option<String>,
    first_failure: Option<String>,
}

/// Where the credentials found in exchanges being saved as requests go.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SecretsTo {
    /// Left written in the requests.
    Keep,
    /// Variables of the selected environment.
    Environment,
    Vault,
}

impl SecretsTo {
    const ALL: [SecretsTo; 3] = [SecretsTo::Vault, SecretsTo::Environment, SecretsTo::Keep];

    fn wire(self) -> &'static str {
        match self {
            SecretsTo::Keep => "keep",
            SecretsTo::Environment => "environment",
            SecretsTo::Vault => "vault",
        }
    }

    fn label(self) -> &'static str {
        t(match self {
            SecretsTo::Keep => "capture.import.keep",
            SecretsTo::Environment => "capture.import.environment",
            SecretsTo::Vault => "capture.import.vault",
        })
    }
}

/// What the dialog before saving requests asks for a vault: a new one's name and passphrase, or a sealed one's.
#[derive(Clone)]
struct VaultFields {
    name: Entity<InputState>,
    passphrase: Entity<InputState>,
    repeat: Entity<InputState>,
}

/// What happened on this screen that the window should act on.
#[derive(Debug, Clone)]
pub enum CaptureEvent {
    /// An exchange was saved as a REST request with this id.
    SavedRequest { id: String },
    /// An exchange was saved as a mock with this id.
    SavedMock { id: String },
}

impl EventEmitter<CaptureEvent> for CaptureScreen {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Request,
    Response,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Headers,
    Payload,
    Response,
    Logs,
}

const SECTIONS: [Section; 4] = [Section::Headers, Section::Payload, Section::Response, Section::Logs];

/// The dev servers' lines for the selected exchange, looked up when the Logs tab is opened.
enum TraceLogs {
    Idle,
    Loading,
    Failed(String),
    /// `approximate`: no line carried the id, these were matched by time and path.
    Found {
        lines: Vec<Found>,
        approximate: bool,
    },
    /// Nothing matched; the key of the text that says why.
    Empty(&'static str),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BodyView {
    Pretty,
    Raw,
}

/// One body of an exchange, prepared for showing.
enum Body {
    Empty,
    /// Text for the read-only editor: as received, and re-indented when that is possible.
    Text {
        raw: String,
        pretty: Option<String>,
    },
    /// An image or other data, decoded once here rather than per frame.
    Data(BinaryBody),
}

impl Body {
    /// The body, and how to highlight it when it is text.
    fn new(bytes: &[u8], content_type: &str) -> (Self, Syntax) {
        match body_class(content_type, bytes) {
            None => (Body::Empty, Syntax::Plain),
            Some(BodyClass::Text) => {
                let raw = String::from_utf8_lossy(bytes).into_owned();
                let mut syntax = Syntax::for_content_type(content_type);
                // A JSON body is recognised by its content too: servers often mislabel it.
                if syntax == Syntax::Plain && pretty_json(&raw).is_some() {
                    syntax = Syntax::Json;
                }
                let pretty = if syntax == Syntax::Json { pretty_json(&raw) } else { None };
                (Body::Text { raw, pretty }, syntax)
            }
            Some(_) => (Body::Data(BinaryBody::new(bytes.to_vec(), content_type)), Syntax::Plain),
        }
    }

    fn text(&self, view: BodyView) -> Option<String> {
        match self {
            Body::Text { pretty: Some(pretty), .. } if view == BodyView::Pretty => Some(pretty.clone()),
            Body::Text { raw, .. } => Some(raw.clone()),
            _ => None,
        }
    }

    fn can_pretty(&self) -> bool {
        matches!(self, Body::Text { pretty: Some(_), .. })
    }
}

/// A fetched exchange with its two bodies prepared.
struct Shown {
    exchange: Exchange,
    request: Body,
    response: Body,
}

enum Detail {
    Idle,
    Loading,
    Failed(String),
    Ready(Box<Shown>),
}

pub struct CaptureScreen {
    model: Entity<AppModel>,
    entries: Vec<TrafficEntry>,
    /// The engine's history has been read at least once.
    loaded: bool,
    /// Mirrors the engine: while capture is paused the engine records nothing, and the list stops following.
    paused: bool,
    /// Mirrors the engine: cache headers are stripped from forwarded requests (on by default).
    disable_cache: bool,
    filter: Entity<InputState>,
    selected: Option<String>,
    /// Exchanges ticked in the list, for saving several at once.
    checked: HashSet<String>,
    /// Narrows the screen to the requests of one origin, without its toolbar: the Webhooks screen embeds a
    /// Capture of what its listeners received. `None` is the Capture screen itself.
    origin: Option<&'static str>,
    /// Show only this kind of response; `None` shows all.
    type_filter: Option<ResourceType>,
    /// Which handlers' exchanges are listed, in [`Handler::ALL`] order. Mappings are off by default.
    handlers: [bool; Handler::ALL.len()],
    /// Show only the pinned exchanges.
    pinned_only: bool,
    /// The unpinned exchanges the engine keeps (`captureLimit`), as `capture.list` reported it.
    limit: usize,
    detail: Detail,
    section: Section,
    trace_logs: TraceLogs,
    response_view: BodyView,
    request_body: Entity<EditorState>,
    response_body: Entity<EditorState>,
    /// A save as request or mock is in flight.
    saving: bool,
    /// What the user chose, in the dialog before saving requests, for the credentials and cookies they carry.
    secrets_to: SecretsTo,
    cookies_to_jar: bool,
    /// The vault credentials go into; `None` creates one.
    import_vault: Option<String>,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

/// The name of a folder several saved exchanges go into: when they were saved.
fn new_folder_name() -> String {
    let now = traffic::now_ms();
    format!("capture-{}-{:02}", date_time_utc(now).replace(' ', "_").replace(':', "-"), now / 1000 % 60)
}

fn read_only_editor(window: &mut Window, cx: &mut Context<CaptureScreen>) -> Entity<EditorState> {
    cx.new(|cx| {
        let mut state =
            EditorState::new(window, cx).language(Syntax::Plain.language()).line_number(true).soft_wrap(true);
        state.set_readonly(true, cx);
        state
    })
}

impl CaptureScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build(model, None, window, cx)
    }

    /// A Capture of what the webhook listeners received (origin `webhook`), for the Webhooks screen.
    pub fn received(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build(model, Some(WEBHOOK), window, cx)
    }

    fn build(
        model: Entity<AppModel>,
        origin: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(t("capture.filter.placeholder")));
        let request_body = read_only_editor(window, cx);
        let response_body = read_only_editor(window, cx);
        let subscriptions = vec![
            cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| match event {
                ModelEvent::Traffic(payload) if !this.paused => {
                    let active = this.model.read(cx).workspace_id().to_owned();
                    let mut incoming = traffic::only_workspace(traffic::parse_event(payload), &active);
                    incoming.retain(|entry| this.origin.is_none_or(|origin| entry.origin.as_deref() == Some(origin)));
                    // The selected exchange changed (its status landed, say): its detail is out of date.
                    let refresh =
                        this.selected.as_ref().filter(|id| incoming.iter().any(|entry| &entry.id == *id)).cloned();
                    // A batch is oldest first; the list is newest first.
                    traffic::merge_live(&mut this.entries, incoming, this.limit);
                    if let Some(id) = refresh {
                        this.fetch(id, window, cx);
                    }
                    cx.notify();
                }
                ModelEvent::WorkspaceSwitched => {
                    this.selected = None;
                    this.checked.clear();
                    this.detail = Detail::Idle;
                    this.reload(window, cx);
                }
                // The screen is built before the workspace is known; the first snapshot is when it can load.
                ModelEvent::ConfigChanged if !this.loaded => this.reload(window, cx),
                _ => {}
            }),
            cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        let mut this = Self {
            model,
            entries: Vec::new(),
            loaded: false,
            paused: false,
            disable_cache: true,
            filter,
            selected: None,
            checked: HashSet::new(),
            type_filter: None,
            origin,
            handlers: [false, true, true, true],
            pinned_only: false,
            limit: traffic::MAX_ENTRIES,
            detail: Detail::Idle,
            section: Section::Headers,
            trace_logs: TraceLogs::Idle,
            response_view: BodyView::Pretty,
            request_body,
            response_body,
            saving: false,
            secrets_to: SecretsTo::Vault,
            cookies_to_jar: true,
            import_vault: None,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        this.reload(window, cx);
        this
    }

    /// One line for the footer: how much is captured, how much of it the filter shows.
    pub fn status_line(&self, cx: &gpui_kit::App) -> String {
        let total = self.entries.len();
        let shown = self.visible(cx).len();
        match (shown < total, self.paused) {
            (false, false) => t_with("capture.status.captured", &[("total", &total)]),
            (true, false) => t_with("capture.status.captured_shown", &[("total", &total), ("shown", &shown)]),
            (false, true) => t_with("capture.status.captured_paused", &[("total", &total)]),
            (true, true) => t_with("capture.status.captured_shown_paused", &[("total", &total), ("shown", &shown)]),
        }
    }

    /// Whether the list shows `entry`: it passes the text filter, the type filter and the handler filter.
    fn shows(&self, entry: &TrafficEntry, needle: &str) -> bool {
        (match self.origin {
            Some(origin) => entry.origin.as_deref() == Some(origin),
            None => Handler::ALL.iter().zip(self.handlers).any(|(handler, on)| on && *handler == entry.handler()),
        }) && (!self.pinned_only || entry.pinned)
            && self.type_filter.is_none_or(|kind| entry.resource_type() == kind)
            && entry.matches(needle)
    }

    fn visible(&self, cx: &gpui_kit::App) -> Vec<&TrafficEntry> {
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        self.entries.iter().filter(|entry| self.shows(entry, &needle)).collect()
    }

    /// Reads the history again (also the Webhooks screen's, when it is brought to the front).
    pub fn reload(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        if model.workspace_id().is_empty() {
            return;
        }
        let client = model.client().clone();
        let mut payload = json!({ "workspaceId": model.workspace_id(), "summary": true });
        if let Some(origin) = self.origin {
            payload["origin"] = json!(origin);
        }
        call(&client, cx, "capture.list", payload, |this, outcome, cx| {
            match outcome {
                Ok(result) => {
                    this.entries = traffic::parse_list(&result);
                    this.limit = result
                        .get("limit")
                        .and_then(Value::as_u64)
                        .and_then(|limit| usize::try_from(limit).ok())
                        .unwrap_or(traffic::MAX_ENTRIES);
                    this.paused = result.get("paused").and_then(Value::as_bool).unwrap_or(false);
                    this.disable_cache = result.get("disableCache").and_then(Value::as_bool).unwrap_or(true);
                    this.loaded = true;
                }
                Err(error) => this.report(t("capture.error.read"), describe(&error), cx),
            }
            cx.notify();
        });
    }

    fn report(&mut self, what: &str, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(format!("{what} {message}"), cx));
    }

    pub fn toggle_paused(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let resume = self.paused;
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "paused": !resume });
        call(&client, cx, "capture.setPaused", payload, move |this, outcome, cx| {
            match failure(&outcome) {
                Some(message) => {
                    let what = if resume { t("capture.error.resume") } else { t("capture.error.pause") };
                    this.report(what, message, cx);
                }
                None => {
                    if let Ok(result) = &outcome {
                        this.paused = result.get("paused").and_then(Value::as_bool).unwrap_or(!resume);
                    }
                }
            }
            cx.notify();
        });
    }

    /// Asks the engine to strip (or keep) the cache headers of the requests it forwards.
    fn set_disable_cache(&mut self, disable: bool, cx: &mut Context<Self>) {
        self.disable_cache = disable;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "capture.setDisableCache", json!({ "disableCache": disable }), move |this, outcome, cx| {
            match failure(&outcome) {
                Some(message) => {
                    this.disable_cache = !disable;
                    this.report(t("capture.error.cache_setting"), message, cx);
                }
                None => {
                    if let Ok(result) = &outcome {
                        this.disable_cache = result.get("disableCache").and_then(Value::as_bool).unwrap_or(disable);
                    }
                }
            }
            cx.notify();
        });
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id() });
        call(&client, cx, "capture.clear", payload, |this, outcome, cx| match failure(&outcome) {
            Some(message) => this.report(t("capture.error.clear"), message, cx),
            None => {
                this.entries.clear();
                this.checked.clear();
                this.selected = None;
                this.detail = Detail::Idle;
                cx.notify();
            }
        });
    }

    fn selected_entry(&self) -> Option<&TrafficEntry> {
        let id = self.selected.as_deref()?;
        self.entries.iter().find(|entry| entry.id == id)
    }

    fn select(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = id.clone();
        self.trace_logs = TraceLogs::Idle;
        self.response_view = BodyView::Pretty;
        match id {
            Some(id) => {
                self.detail = Detail::Loading;
                self.fetch(id, window, cx);
            }
            None => self.detail = Detail::Idle,
        }
        cx.notify();
    }

    /// Asks the engine for the whole exchange. An answer for an exchange that is no longer selected is dropped.
    fn fetch(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "id": id });
        call_in(&client, window, cx, "capture.get", payload, move |this, outcome, window, cx| {
            if this.selected.as_deref() != Some(id.as_str()) {
                return;
            }
            if let Some(message) = failure(&outcome) {
                this.detail = Detail::Failed(message);
                return cx.notify();
            }
            let exchange = outcome.ok().and_then(|result| Exchange::from_value(result.get("entry")?));
            match exchange {
                Some(exchange) => this.show(exchange, window, cx),
                None => {
                    this.detail = Detail::Failed(t("capture.error.no_longer_in_history").to_owned());
                }
            }
            cx.notify();
        });
    }

    /// Takes a fetched exchange and puts its bodies into the editors.
    fn show(&mut self, exchange: Exchange, window: &mut Window, cx: &mut Context<Self>) {
        let (request, request_syntax) = Body::new(&exchange.request_body, exchange.request_content_type());
        let (response, response_syntax) = Body::new(&exchange.response_body, exchange.response_content_type());
        if let Some(text) = request.text(BodyView::Pretty) {
            self.request_body.update(cx, |state, cx| {
                state.set_highlighter(request_syntax.language(), cx);
                state.set_value(text, window, cx);
            });
        }
        if let Some(text) = response.text(self.response_view) {
            self.response_body.update(cx, |state, cx| {
                state.set_highlighter(response_syntax.language(), cx);
                state.set_value(text, window, cx);
            });
        }
        self.detail = Detail::Ready(Box::new(Shown { exchange, request, response }));
        if self.section == Section::Logs && matches!(self.trace_logs, TraceLogs::Idle) {
            self.load_logs(cx);
        }
    }

    /// Looks the selected exchange up in the dev servers' output, as `capture show --logs` does: the lines
    /// that carry its id or trace id, else those that mention its path around the time it was served.
    fn load_logs(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Detail::Ready(shown)) = (self.selected.clone(), &self.detail) else { return };
        let exchange = &shown.exchange;
        let traced = exchange.trace_id.is_some();
        let workspace = self.model.read(cx).workspace_id().to_owned();
        let exact = logs::exact_query(&workspace, &id, exchange.trace_id.as_deref());
        let near = logs::near_query(
            &workspace,
            &exchange.url,
            self.selected_entry().map_or(0, |entry| entry.ts),
            exchange.duration_ms.unwrap_or_default(),
        );
        self.trace_logs = TraceLogs::Loading;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "devservice.grep", exact, move |this, outcome, cx| {
            if this.selected.as_deref() != Some(id.as_str()) {
                return;
            }
            if let Some(message) = failure(&outcome) {
                this.trace_logs = TraceLogs::Failed(message);
                return cx.notify();
            }
            if let Some(lines) = outcome.ok().as_ref().and_then(logs::found) {
                this.trace_logs = TraceLogs::Found { lines, approximate: false };
                return cx.notify();
            }
            let Some(near) = near else { return this.nothing_in_logs(id, traced, cx) };
            let client = this.model.read(cx).client().clone();
            call(&client, cx, "devservice.grep", near, move |this, outcome, cx| {
                if this.selected.as_deref() != Some(id.as_str()) {
                    return;
                }
                match outcome.ok().as_ref().and_then(logs::found) {
                    Some(lines) => {
                        this.trace_logs = TraceLogs::Found { lines, approximate: true };
                        cx.notify();
                    }
                    None => this.nothing_in_logs(id, traced, cx),
                }
            });
        });
    }

    /// No line matched: says why, which depends on whether any dev server is running at all.
    fn nothing_in_logs(&mut self, id: String, traced: bool, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let payload = json!({ "workspaceId": model.workspace_id() });
        let client = model.client().clone();
        call(&client, cx, "devservice.list", payload, move |this, outcome, cx| {
            if this.selected.as_deref() != Some(id.as_str()) {
                return;
            }
            let running = outcome.ok().is_some_and(|result| {
                result["services"].as_array().is_some_and(|list| list.iter().any(|s| s["state"] == "running"))
            });
            this.trace_logs = TraceLogs::Empty(match (running, traced) {
                (false, _) => "capture.logs.empty.no_servers",
                (true, false) => "capture.logs.empty.not_traced",
                (true, true) => "capture.logs.empty.none",
            });
            cx.notify();
        });
    }

    fn set_response_view(&mut self, view: BodyView, window: &mut Window, cx: &mut Context<Self>) {
        if self.response_view == view {
            return;
        }
        self.response_view = view;
        if let Detail::Ready(shown) = &self.detail
            && let Some(text) = shown.response.text(view)
        {
            self.response_body.update(cx, |state, cx| state.set_value(text, window, cx));
        }
        cx.notify();
    }

    /// Pins or unpins exchanges: the rows change at once, and are read again when the engine refuses.
    fn pin(&mut self, ids: Vec<String>, pinned: bool, window: &mut Window, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        for entry in self.entries.iter_mut().filter(|entry| ids.contains(&entry.id)) {
            entry.pinned = pinned;
        }
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "ids": ids, "pinned": pinned });
        call_in(&client, window, cx, "capture.pin", payload, move |this, outcome, window, cx| {
            if let Some(message) = failure(&outcome) {
                this.report(t(if pinned { "capture.error.pin" } else { "capture.error.unpin" }), message, cx);
                this.reload(window, cx);
            }
        });
        cx.notify();
    }

    fn pin_checked(&mut self, pinned: bool, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.checked_exchanges().into_iter().map(|(id, _)| id).collect();
        self.pin(ids, pinned, window, cx);
    }

    fn remove_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.take() else { return };
        self.detail = Detail::Idle;
        self.checked.remove(&id);
        self.entries.retain(|entry| entry.id != id);
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "ids": [id] });
        call(&client, cx, "capture.remove", payload, |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.report(t("capture.error.remove"), message, cx);
            }
        });
        cx.notify();
    }

    fn save_failed(&mut self, what: &str, message: String, cx: &mut Context<Self>) {
        self.saving = false;
        self.report(what, message, cx);
        cx.notify();
    }

    /// Saves exchanges as requests through the engine, which first says which credentials and cookies they
    /// carry; when they carry any, the user chooses whether they are taken out. `flow` also chains them into a
    /// flow in the order they were captured.
    fn import_requests(&mut self, ids: Vec<String>, flow: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || ids.is_empty() {
            return;
        }
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "ids": ids });
        self.saving = true;
        cx.notify();
        call_in(&client, window, cx, "capture.inspectImport", payload, move |this, outcome, window, cx| {
            if let Some(message) = failure(&outcome) {
                return this.save_failed(t("capture.error.save_some"), message, cx);
            }
            let found = outcome.ok().unwrap_or_default();
            let list = |key: &str| found.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
            let (secrets, cookies) = (list("secrets"), list("cookies"));
            if secrets.is_empty() && cookies.is_empty() {
                return this.run_import(ids, flow, SecretsTo::Keep, None, false, window, cx);
            }
            this.saving = false;
            this.secrets_to = SecretsTo::Vault;
            this.cookies_to_jar = true;
            // An unsealed vault needs no passphrase, so it is offered first; with none, a new one.
            let vaults = this.model.read(cx).vaults();
            let first = vaults.iter().find(|vault| vault.state == VaultState::Unsealed).or(vaults.first());
            this.import_vault = first.map(|vault| vault.id.clone());
            cx.notify();
            this.ask_import(ids, flow, secrets, cookies, window, cx);
        });
    }

    /// Shows what was found and lets the user choose where credentials go, and whether cookies go to the jar.
    fn ask_import(
        &mut self,
        ids: Vec<String>,
        flow: bool,
        secrets: Vec<Value>,
        cookies: Vec<Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let masked = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).masked(true))
        };
        let fields = VaultFields {
            name: cx.new(|cx| InputState::new(window, cx).placeholder("api_keys")),
            passphrase: masked(t("vaults.passphrase_hint"), window, cx),
            repeat: masked("", window, cx),
        };
        let screen = cx.entity();
        window.defer(cx, move |window, cx| {
            window.open_dialog(cx, move |dialog, _, cx| {
                let theme = cx.theme().clone();
                let (secrets_to, jar, picked) = {
                    let screen = screen.read(cx);
                    (screen.secrets_to, screen.cookies_to_jar, screen.import_vault.clone())
                };
                let vaults = screen.read(cx).model.read(cx).vaults().to_vec();
                let picked = picked.and_then(|id| vaults.iter().find(|vault| vault.id == id).cloned());
                let row = |label: String, detail: String| {
                    v_flex()
                        .min_w_0()
                        .child(div().text_sm().font_family(theme.mono_font_family.clone()).child(label))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(detail),
                        )
                };
                let section = |id: &'static str, body: &'static str, rows: Vec<gpui_kit::Div>| {
                    v_flex()
                        .gap_2()
                        .child(div().text_sm().text_color(theme.muted_foreground).child(body))
                        .child(div().id(id).max_h(px(180.)).overflow_y_scroll().child(v_flex().gap_2().children(rows)))
                };
                let secret_rows = secrets
                    .iter()
                    .map(|secret| {
                        let places: Vec<&str> =
                            secret["places"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                        let preview = secret["preview"].as_str().unwrap_or("");
                        row(format!("{}  {preview}", secret["name"].as_str().unwrap_or("")), places.join(", "))
                    })
                    .collect();
                let cookie_rows = cookies
                    .iter()
                    .map(|domain| {
                        let names: Vec<&str> =
                            domain["names"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                        row(domain["domain"].as_str().unwrap_or("").to_owned(), names.join(", "))
                    })
                    .collect();

                let to_screen = screen.clone();
                let destinations = SecretsTo::ALL.iter().fold(
                    TabBar::new("capture-import-secrets-to")
                        .segmented()
                        .small()
                        .selected_index(SecretsTo::ALL.iter().position(|to| *to == secrets_to).unwrap_or(0))
                        .on_click(move |ix: &usize, window, cx| {
                            to_screen.update(cx, |screen, _| screen.secrets_to = SecretsTo::ALL[*ix]);
                            window.refresh();
                        }),
                    |bar, to| bar.child(Tab::new().label(to.label())),
                );
                let vault_screen = screen.clone();
                let vault_ids: Vec<String> = vaults.iter().map(|vault| vault.id.clone()).collect();
                let picker = ButtonGroup::new("capture-import-vaults")
                    .outline()
                    .small()
                    .children(vaults.iter().enumerate().map(|(ix, vault)| {
                        let label = match vault.state {
                            VaultState::Unsealed => vault.name.clone(),
                            VaultState::Sealed => format!("{} · {}", vault.name, vault.state.label()),
                        };
                        Button::new(("capture-import-vault", ix))
                            .label(label)
                            .selected(picked.as_ref().is_some_and(|picked| picked.id == vault.id))
                    }))
                    .child(
                        Button::new("capture-import-new-vault")
                            .label(t("capture.import.new_vault"))
                            .selected(picked.is_none()),
                    )
                    .on_click(move |selected: &Vec<usize>, window, cx| {
                        let id = selected.first().and_then(|ix| vault_ids.get(*ix)).cloned();
                        vault_screen.update(cx, |screen, _| screen.import_vault = id);
                        window.refresh();
                    });
                let vault_form = match &picked {
                    None => Form::new()
                        .child(Field::new().label(t("vaults.name")).child(Input::new(&fields.name).py_1p5()))
                        .child(
                            Field::new()
                                .label(t("vaults.passphrase"))
                                .child(Input::new(&fields.passphrase).py_1p5().mask_toggle()),
                        )
                        .child(
                            Field::new()
                                .label(t("vaults.repeat_passphrase"))
                                .child(Input::new(&fields.repeat).py_1p5().mask_toggle()),
                        ),
                    Some(vault) if vault.state == VaultState::Sealed => Form::new().child(
                        Field::new()
                            .label(t("vaults.passphrase"))
                            .child(Input::new(&fields.passphrase).py_1p5().mask_toggle()),
                    ),
                    Some(_) => Form::new(),
                };

                let (jar_screen, save_screen) = (screen.clone(), screen.clone());
                let (ids, fields) = (ids.clone(), fields.clone());
                dialog
                    .title(t("capture.import.title"))
                    .w(px(600.))
                    .child(
                        v_flex()
                            .gap_5()
                            .when(!secrets.is_empty(), |this| {
                                this.child(
                                    section("capture-import-secrets", t("capture.import.secrets_body"), secret_rows)
                                        .child(destinations)
                                        .when(secrets_to == SecretsTo::Vault, |this| {
                                            this.child(picker).child(vault_form)
                                        }),
                                )
                            })
                            .when(!cookies.is_empty(), |this| {
                                this.child(
                                    section("capture-import-cookies", t("capture.import.cookies_body"), cookie_rows)
                                        .child(
                                            Checkbox::new("capture-import-jar")
                                                .label(t("capture.import.jar"))
                                                .checked(jar)
                                                .on_change(move |checked: &bool, window, cx| {
                                                    jar_screen.update(cx, |screen, _| screen.cookies_to_jar = *checked);
                                                    window.refresh();
                                                }),
                                        ),
                                )
                            }),
                    )
                    .footer(
                        DialogFooter::new()
                            .child(
                                Button::new("capture-import-cancel")
                                    .outline()
                                    .label(t("components.cancel"))
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                            .child(
                                Button::new("capture-import-save").primary().label(t("capture.import.save")).on_click(
                                    move |_, window, cx| {
                                        let ids = ids.clone();
                                        let started = save_screen.update(cx, |screen, cx| {
                                            screen.confirm_import(ids, flow, &fields, window, cx)
                                        });
                                        if started {
                                            window.close_dialog(cx);
                                        }
                                    },
                                ),
                            ),
                    )
            });
        });
    }

    /// Saves as the dialog says. For a vault that is new or sealed, it is created or unsealed first with what was
    /// typed. `false` when something needed was not typed, and the dialog stays.
    fn confirm_import(
        &mut self,
        ids: Vec<String>,
        flow: bool,
        fields: &VaultFields,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let (to, jar) = (self.secrets_to, self.cookies_to_jar);
        if to != SecretsTo::Vault {
            self.run_import(ids, flow, to, None, jar, window, cx);
            return true;
        }
        let value = |input: &Entity<InputState>, cx: &Context<Self>| input.read(cx).value().trim().to_owned();
        let (name, passphrase, repeat) =
            (value(&fields.name, cx), value(&fields.passphrase, cx), value(&fields.repeat, cx));
        let model = self.model.read(cx);
        let (client, workspace_id) = (model.client().clone(), model.workspace_id().to_owned());
        let vault =
            self.import_vault.as_ref().and_then(|id| model.vaults().iter().find(|vault| &vault.id == id).cloned());
        let refuse = |text: &str, window: &mut Window, cx: &mut Context<Self>| {
            window.push_notification(
                Notification::warning(text.to_owned()).title(t("components.dialogs.cant_continue")),
                cx,
            );
            false
        };
        match vault {
            Some(vault) if vault.state == VaultState::Unsealed => {
                self.run_import(ids, flow, to, Some(vault.id), jar, window, cx);
            }
            Some(vault) => {
                if passphrase.is_empty() {
                    return refuse(t("components.dialogs.fill_required"), window, cx);
                }
                let payload = json!({ "workspaceId": workspace_id, "id": vault.id, "passphrase": passphrase });
                call_in(&client, window, cx, "vault.unseal", payload, move |this, outcome, window, cx| {
                    match failure(&outcome) {
                        Some(message) => this.report(t("vaults.error_unseal"), message, cx),
                        None => this.run_import(ids, flow, to, Some(vault.id), jar, window, cx),
                    }
                });
            }
            None => {
                if name.is_empty() || passphrase.is_empty() {
                    return refuse(t("components.dialogs.fill_required"), window, cx);
                }
                if passphrase != repeat {
                    return refuse(t("vaults.passphrases_differ"), window, cx);
                }
                let payload = json!({ "workspaceId": workspace_id, "name": name, "passphrase": passphrase });
                call_in(&client, window, cx, "vault.create", payload, move |this, outcome, window, cx| {
                    if let Some(message) = failure(&outcome) {
                        return this.report(t("vaults.error_create"), message, cx);
                    }
                    let id = outcome.ok().and_then(|vault| vault.get("id")?.as_str().map(str::to_owned));
                    this.import_vault = id.clone();
                    this.run_import(ids, flow, to, id, jar, window, cx);
                });
            }
        }
        true
    }

    /// Saves exchanges as requests (in a new folder when there are several), as the user chose.
    #[allow(clippy::too_many_arguments)]
    fn run_import(
        &mut self,
        ids: Vec<String>,
        flow: bool,
        to: SecretsTo,
        vault_id: Option<String>,
        jar: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let folder = new_folder_name();
        let mut payload = json!({
            "workspaceId": model.workspace_id(), "ids": ids, "secrets": to.wire(), "cookiesToJar": jar, "flow": flow,
            "folderName": folder, "keepAll": true,
        });
        if let Some(vault_id) = vault_id {
            payload["vaultId"] = json!(vault_id);
        }
        self.saving = true;
        cx.notify();
        call_in(&client, window, cx, "capture.import", payload, move |this, outcome, window, cx| {
            if let Some(message) = failure(&outcome) {
                return this.save_failed(t("capture.error.save_some"), message, cx);
            }
            this.saving = false;
            let result = outcome.ok().unwrap_or_default();
            let names = |key: &str| -> Vec<String> {
                result[key].as_array().into_iter().flatten().filter_map(|id| id.as_str().map(str::to_owned)).collect()
            };
            let (saved, kept) = (names("requestIds"), names("keptInRequests"));
            let count = saved.len();
            let text = match (result["flowId"].is_string(), count) {
                (true, _) => t_with("capture.saved_flow", &[("count", &count), ("folder", &folder)]),
                (false, 1) => t("capture.saved_as_request").to_owned(),
                (false, _) => {
                    t_with("capture.saved_requests_in_folder_other", &[("count", &count), ("folder", &folder)])
                }
            };
            window.push_notification(Notification::success(text), cx);
            if !kept.is_empty() {
                let text = t_with("capture.import.kept", &[("names", &kept.join(", "))]);
                window.push_notification(Notification::warning(text), cx);
            }
            if let [id] = saved.as_slice() {
                cx.emit(CaptureEvent::SavedRequest { id: id.clone() });
            }
            this.checked.clear();
            cx.notify();
        });
    }

    /// Saves the selected exchange as a request or as a mock. The engine builds the entity from the entry it
    /// holds; what it built is created as it is, named after the exchange when it has no name (the method is shown beside the name already).
    fn save_as(&mut self, mock: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let Some(entry) = self.selected_entry() else { return };
        if !mock {
            return self.import_requests(vec![entry.id.clone()], false, window, cx);
        }
        let run = SaveRun {
            pending: vec![(entry.id.clone(), entry.name())],
            mock,
            folder: None,
            saved: 0,
            last: None,
            first_failure: None,
        };
        self.saving = true;
        cx.notify();
        self.save_next(run, window, cx);
    }

    /// The ticked exchanges, oldest first, with the names they are saved under.
    fn checked_exchanges(&self) -> Vec<(String, String)> {
        self.entries
            .iter()
            .rev()
            .filter(|entry| self.checked.contains(&entry.id))
            .map(|entry| (entry.id.clone(), entry.name()))
            .collect()
    }

    /// Saves the ticked exchanges, or every listed one when none are ticked, as a HAR file the request importer
    /// reads back. The engine holds the bodies, so only the ids go out.
    fn export_har(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<String> = if self.checked.is_empty() {
            self.visible(cx).into_iter().rev().map(|entry| entry.id.clone()).collect()
        } else {
            self.checked_exchanges().into_iter().map(|(id, _)| id).collect()
        };
        let model = self.model.read(cx);
        let client = model.client().clone();
        let workspace = model.workspace_id().to_owned();
        let payload = json!({ "ids": ids, "workspaceId": workspace });
        call(&client.clone(), cx, "capture.shareJson", payload, move |this, outcome, cx| match outcome {
            Ok(artifact) => {
                fetch_artifact(this, &client, cx, &workspace, &artifact, |this, fetched, cx| match fetched {
                    Ok((bytes, name)) => save_to_file(bytes, name, cx, |this, outcome, cx| match outcome {
                        Ok(path) => this.model.update(cx, |model, cx| {
                            model.notice(t_with("capture.saved_to", &[("path", &path.display())]), cx)
                        }),
                        Err(message) => this.report(t("capture.error.save_file"), message, cx),
                    }),
                    Err(message) => this.report(t("capture.error.export"), message, cx),
                })
            }
            Err(error) => this.report(t("capture.error.export"), describe(&error), cx),
        });
    }

    /// Saves every ticked exchange as a request or a mock. One goes to the top level like a single save; several
    /// go into a new folder, which is asked for first.
    fn save_checked(&mut self, mock: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let pending = self.checked_exchanges();
        if !mock {
            return self.import_requests(pending.into_iter().map(|(id, _)| id).collect(), false, window, cx);
        }
        if pending.len() < 2 {
            if pending.is_empty() {
                return;
            }
            self.saving = true;
            cx.notify();
            let run = SaveRun { pending, mock, folder: None, saved: 0, last: None, first_failure: None };
            return self.save_next(run, window, cx);
        }
        self.create_folder_then_save(pending, mock, new_folder_name(), window, cx);
    }

    fn create_folder_then_save(
        &mut self,
        pending: Vec<(String, String)>,
        mock: bool,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.saving {
            return;
        }
        let model = self.model.read(cx);
        let client = model.client().clone();
        let folder_kind = if mock { "mock" } else { "request" };
        let payload = json!({
            "kind": folder_kind, "name": name, "parentId": Value::Null, "workspaceId": model.workspace_id(),
        });
        self.saving = true;
        cx.notify();
        call_in(&client, window, cx, "folder.add", payload, move |this, outcome, window, cx| {
            if let Some(message) = failure(&outcome) {
                return this.save_failed(t("capture.error.create_folder"), message, cx);
            }
            let id = outcome.ok().and_then(|folder| folder.get("id")?.as_str().map(str::to_owned));
            let Some(id) = id else {
                return this.save_failed(t("capture.error.create_folder"), t("capture.error.no_answer").into(), cx);
            };
            let run = SaveRun { pending, mock, folder: Some((id, name)), saved: 0, last: None, first_failure: None };
            this.save_next(run, window, cx);
        });
    }

    /// Saves the next pending exchange, then the one after it; reports once they have all been tried. The engine
    /// builds each entity from the entry it holds; what it built is created as it is, named after the exchange
    /// when it has no name (the method is shown beside the name already).
    fn save_next(&mut self, mut run: SaveRun, window: &mut Window, cx: &mut Context<Self>) {
        if run.pending.is_empty() {
            return self.finish_save(run, window, cx);
        }
        let (id, name) = run.pending.remove(0);
        let (build, field, kind) = if run.mock {
            ("capture.buildMock", "mock", "mocks")
        } else {
            ("capture.buildRequest", "request", "requests")
        };
        let model = self.model.read(cx);
        let client = model.client().clone();
        let workspace_id = model.workspace_id().to_owned();
        let create_client = client.clone();
        let folder_id = run.folder.as_ref().map(|(id, _)| id.clone());
        let payload = json!({ "workspaceId": workspace_id, "id": id });
        call_in(&client, window, cx, build, payload, move |this, outcome, window, cx| {
            let entity = match failure(&outcome) {
                Some(message) => Err(message),
                None => outcome
                    .ok()
                    .and_then(|result| result.get(field).cloned())
                    .filter(Value::is_object)
                    .ok_or_else(|| t("capture.error.no_answer").to_owned()),
            };
            let mut entity = match entity {
                Ok(entity) => entity,
                Err(message) => {
                    run.first_failure.get_or_insert(message);
                    return this.save_next(run, window, cx);
                }
            };
            if let Some(object) = entity.as_object_mut() {
                let unnamed = object.get("name").and_then(Value::as_str).is_none_or(str::is_empty);
                if unnamed {
                    object.insert("name".to_owned(), Value::String(name));
                }
                if let Some(folder_id) = folder_id {
                    object.insert("folderId".to_owned(), Value::String(folder_id));
                }
            }
            let payload = json!({ "kind": kind, "workspaceId": workspace_id, "entity": entity });
            call_in(&create_client, window, cx, "entity.create", payload, move |this, outcome, window, cx| {
                let created = match failure(&outcome) {
                    Some(message) => Err(message),
                    None => outcome
                        .ok()
                        .and_then(|result| result.get("entity")?.get("id")?.as_str().map(str::to_owned))
                        .ok_or_else(|| t("capture.error.no_answer").to_owned()),
                };
                match created {
                    Ok(created) => {
                        run.saved += 1;
                        run.last = Some(created);
                    }
                    Err(message) => {
                        run.first_failure.get_or_insert(message);
                    }
                }
                this.save_next(run, window, cx);
            });
        });
    }

    fn finish_save(&mut self, run: SaveRun, window: &mut Window, cx: &mut Context<Self>) {
        self.saving = false;
        if let Some(message) = run.first_failure {
            self.report(t("capture.error.save_some"), message, cx);
        }
        match (&run.folder, run.last, run.saved) {
            (_, _, 0) => {}
            (None, Some(created), _) => {
                let text = if run.mock { t("capture.saved_as_mock") } else { t("capture.saved_as_request") };
                window.push_notification(Notification::success(text), cx);
                cx.emit(if run.mock {
                    CaptureEvent::SavedMock { id: created }
                } else {
                    CaptureEvent::SavedRequest { id: created }
                });
            }
            (folder, _, saved) => {
                let key = match (run.mock, saved == 1, folder.is_some()) {
                    (false, true, true) => "capture.saved_requests_in_folder_one",
                    (false, false, true) => "capture.saved_requests_in_folder_other",
                    (true, true, true) => "capture.saved_mocks_in_folder_one",
                    (true, false, true) => "capture.saved_mocks_in_folder_other",
                    (false, true, false) => "capture.saved_requests_one",
                    (false, false, false) => "capture.saved_requests_other",
                    (true, true, false) => "capture.saved_mocks_one",
                    (true, false, false) => "capture.saved_mocks_other",
                };
                let folder_name = folder.as_ref().map_or("", |(_, name)| name.as_str());
                let text = t_with(key, &[("count", &saved), ("folder", &folder_name)]);
                window.push_notification(Notification::success(text), cx);
            }
        }
        if run.saved > 0 {
            self.checked.clear();
        }
        cx.notify();
    }

    /// Sends the selected webhook again (`webhook.replay`): to the listener's forward URL, else to where it
    /// came from, or to `to` when given. The answer's status is shown as a notification.
    fn replay(&mut self, to: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else { return };
        let model = self.model.read(cx);
        let client = model.client().clone();
        let mut payload = json!({ "workspaceId": model.workspace_id(), "id": id });
        if let Some(to) = to {
            payload["to"] = json!(to);
        }
        call_in(&client, window, cx, "webhook.replay", payload, |this, outcome, window, cx| match failure(&outcome) {
            Some(message) => this.report(t("webhooks.error.replay"), message, cx),
            None => {
                let status = outcome.ok().and_then(|sent| sent.get("status")?.as_u64()).unwrap_or_default();
                let text = t_with("webhooks.replayed", &[("status", &status)]);
                let note = if status < 400 { Notification::success(text) } else { Notification::warning(text) };
                window.push_notification(note, cx);
            }
        });
    }

    fn ask_replay_to(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t("webhooks.replay_to.title"),
            t("webhooks.replay"),
            vec![PromptField::new(t("webhooks.field.url"), t("webhooks.field.url_placeholder"))],
            |this, values, window, cx| this.replay(values.into_iter().next(), window, cx),
        );
    }

    /// Puts the selected exchange's request on the clipboard as a cURL command, like the REST editor's snippet.
    fn copy_curl(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Detail::Ready(shown) = &self.detail else { return };
        let exchange = &shown.exchange;
        let parts = RequestParts {
            method: exchange.method.clone(),
            url: exchange.url.clone(),
            headers: exchange.request_headers.clone(),
            body: shown.request.text(BodyView::Raw).unwrap_or_default(),
        };
        cx.write_to_clipboard(ClipboardItem::new_string(snippet(SnippetKind::Curl, &parts)));
        window.push_notification(Notification::success(t("capture.copied_curl")), cx);
    }

    /// Asks where to save the body of one side and writes it there.
    fn save_body(&mut self, side: Side, cx: &mut Context<Self>) {
        let Detail::Ready(shown) = &self.detail else { return };
        let (body, name, content_type) = match side {
            Side::Request => (&shown.request, "request", shown.exchange.request_content_type()),
            Side::Response => (&shown.response, "response", shown.exchange.response_content_type()),
        };
        let Body::Data(binary) = body else { return };
        let file_name = suggested_file_name(&shown.exchange.url, name, content_type);
        save_to_file(binary.bytes().to_vec(), file_name, cx, |this, outcome, cx| match outcome {
            Ok(path) => this
                .model
                .update(cx, |model, cx| model.notice(t_with("capture.saved_to", &[("path", &path.display())]), cx)),
            Err(message) => this.report(t("capture.error.save_file"), message, cx),
        });
    }

    fn status_tag(status: Option<u16>) -> Tag {
        match status {
            Some(code) if (200..400).contains(&code) => Tag::success().small().child(code.to_string()),
            Some(code) => Tag::danger().small().child(code.to_string()),
            None => Tag::secondary().small().child(t("capture.pending")),
        }
    }

    /// The type filter, the switch for local traffic, and what can be done with the ticked exchanges.
    fn render_toolbar(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let ticked = self.checked.len();
        let selected_tab = self
            .type_filter
            .map_or(0, |kind| ResourceType::ALL.iter().position(|candidate| *candidate == kind).map_or(0, |ix| ix + 1));
        let types = ResourceType::ALL.iter().fold(
            TabBar::new("capture-types")
                .segmented()
                .small()
                .selected_index(selected_tab)
                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                    this.type_filter = ix.checked_sub(1).and_then(|ix| ResourceType::ALL.get(ix).copied());
                    cx.notify();
                }))
                .child(Tab::new().label(t("capture.types.all"))),
            |bar, kind| bar.child(Tab::new().label(kind.label())),
        );
        h_flex()
            .flex_none()
            .flex_wrap()
            .gap_3()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(types)
            .child(
                Checkbox::new("capture-disable-cache")
                    .small()
                    .label(t("capture.disable_cache.label"))
                    .tooltip(t("capture.disable_cache.tooltip"))
                    .checked(self.disable_cache)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| this.set_disable_cache(*checked, cx))),
            )
            .child(
                Checkbox::new("capture-pinned-only")
                    .small()
                    .label(t("capture.pinned_only"))
                    .checked(self.pinned_only)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        this.pinned_only = *checked;
                        cx.notify();
                    })),
            )
            .child(
                ButtonGroup::new("capture-handlers")
                    .multiple(true)
                    .outline()
                    .small()
                    .children(Handler::ALL.iter().zip(self.handlers).map(|(handler, on)| {
                        Button::new(handler.label()).label(handler.label()).tooltip(handler.tooltip()).selected(on)
                    }))
                    .on_click(cx.listener(|this, selected: &Vec<usize>, _, cx| {
                        this.handlers = std::array::from_fn(|ix| selected.contains(&ix));
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                Button::new("capture-create-flow")
                    .small()
                    .outline()
                    .label(t("capture.create_flow"))
                    .tooltip(t("capture.create_flow_tooltip"))
                    .disabled(self.saving || ticked < 2)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let ids = this.checked_exchanges().into_iter().map(|(id, _)| id).collect();
                        this.import_requests(ids, true, window, cx);
                    })),
            )
            .child(
                Button::new("capture-export")
                    .small()
                    .outline()
                    .label(t("capture.export"))
                    .tooltip(t("capture.export_tooltip"))
                    .disabled(self.entries.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| this.export_har(cx))),
            )
            .children((ticked > 0).then(|| {
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(t_with("capture.selected", &[("count", &ticked)])),
                    )
                    .child(
                        Button::new("capture-save-checked-requests")
                            .small()
                            .label(t("capture.save_checked_requests"))
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| this.save_checked(false, window, cx))),
                    )
                    .child(
                        Button::new("capture-save-checked-mocks")
                            .small()
                            .label(t("capture.save_checked_mocks"))
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| this.save_checked(true, window, cx))),
                    )
                    .child(
                        Button::new("capture-pin-checked")
                            .small()
                            .label(t("capture.pin_checked"))
                            .tooltip(t("capture.pin_tooltip"))
                            .on_click(cx.listener(|this, _, window, cx| this.pin_checked(true, window, cx))),
                    )
                    .child(
                        Button::new("capture-unpin-checked")
                            .small()
                            .label(t("capture.unpin_checked"))
                            .tooltip(t("capture.unpin_tooltip"))
                            .on_click(cx.listener(|this, _, window, cx| this.pin_checked(false, window, cx))),
                    )
                    .child(Button::new("capture-uncheck").ghost().small().label(t("capture.clear_selection")).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.checked.clear();
                            cx.notify();
                        }),
                    ))
            }))
            .into_any_element()
    }

    fn render_list(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        // Positions in `entries`, so the rows in view can be read in place instead of copied.
        let visible: Vec<usize> =
            self.entries.iter().enumerate().filter(|(_, entry)| self.shows(entry, &needle)).map(|(ix, _)| ix).collect();
        let visible_ids: Vec<String> = visible.iter().map(|ix| self.entries[*ix].id.clone()).collect();
        let all_checked = !visible_ids.is_empty() && visible_ids.iter().all(|id| self.checked.contains(id));
        if self.entries.is_empty() && self.origin.is_some() {
            return page::empty(t("webhooks.received.empty.title"), t("webhooks.received.empty.body"));
        }
        if self.entries.is_empty() {
            return page::empty(
                t("capture.empty.no_traffic.title"),
                if self.paused { t("capture.empty.no_traffic.paused") } else { t("capture.empty.no_traffic.body") },
            );
        }
        if visible.is_empty() {
            return page::empty(t("capture.empty.no_matches.title"), t("capture.empty.no_matches.body"));
        }
        let count = visible.len();
        let row_theme = theme.clone();
        // Only the rows in view are built; the list measures one row and scrolls by that height.
        let rows = uniform_list(
            "capture-rows",
            count,
            cx.processor(move |this, range: Range<usize>, _, cx| {
                let theme = &row_theme;
                range
                    .filter_map(|ix| this.entries.get(*visible.get(ix)?))
                    .map(|entry| {
                        let id = entry.id.clone();
                        ListItem::new(gpui_kit::ElementId::Name(format!("capture-{}", entry.id).into()))
                            .selected(this.selected.as_deref() == Some(entry.id.as_str()))
                            .w_full()
                            .px_4()
                            .py_1()
                            .text_sm()
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_3()
                                    .child({
                                        let toggled = id.clone();
                                        // The tick is not a selection of the row: stop the press reaching it.
                                        div()
                                            .flex_none()
                                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                            .child(
                                                Checkbox::new(gpui_kit::ElementId::Name(
                                                    format!("capture-check-{}", entry.id).into(),
                                                ))
                                                .small()
                                                .checked(this.checked.contains(&entry.id))
                                                .on_change(
                                                    cx.listener(move |this, checked: &bool, _, cx| {
                                                        if *checked {
                                                            this.checked.insert(toggled.clone());
                                                        } else {
                                                            this.checked.remove(&toggled);
                                                        }
                                                        cx.notify();
                                                    }),
                                                ),
                                            )
                                    })
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_2()
                                            .overflow_hidden()
                                            .children(entry.pinned.then(|| {
                                                div()
                                                    .id(gpui_kit::ElementId::Name(
                                                        format!("capture-pinned-{}", entry.id).into(),
                                                    ))
                                                    .test_support()
                                                    .flex_none()
                                                    .tooltip(|window, cx| {
                                                        gpui_kit::component::tooltip::Tooltip::new(t(
                                                            "capture.pinned_marker",
                                                        ))
                                                        .build(window, cx)
                                                    })
                                                    .child(Icon::new(IconName::Star).small())
                                            }))
                                            .child(
                                                div().min_w_0().overflow_hidden().text_ellipsis().child(entry.name()),
                                            )
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .text_xs()
                                                    .text_color(theme.muted_foreground)
                                                    .child(entry.host()),
                                            ),
                                    )
                                    .child(div().w_16().flex_none().child(entry.method.clone()))
                                    .child(h_flex().w_20().flex_none().child(Self::status_tag(entry.status)))
                                    .child(
                                        div()
                                            .w_16()
                                            .flex_none()
                                            .text_color(theme.muted_foreground)
                                            .child(entry.type_label()),
                                    )
                                    .child(
                                        div()
                                            .w_24()
                                            .flex_none()
                                            .text_color(theme.muted_foreground)
                                            .child(entry.via_label()),
                                    )
                                    .child(
                                        div().w_20().flex_none().text_right().text_color(theme.muted_foreground).child(
                                            entry.size.map(|size| media::size_label(size as usize)).unwrap_or_default(),
                                        ),
                                    )
                                    .child(
                                        div().w_20().flex_none().text_right().text_color(theme.muted_foreground).child(
                                            entry
                                                .duration_ms
                                                .map(|ms| t_with("capture.duration", &[("ms", &ms)]))
                                                .unwrap_or_default(),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .w_20()
                                            .flex_none()
                                            .text_right()
                                            .text_color(theme.muted_foreground)
                                            .child(clock_utc(entry.ts)),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| this.select(Some(id.clone()), window, cx)))
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_none().child(
                        Checkbox::new("capture-check-all").small().checked(all_checked).on_change(cx.listener(
                            move |this, checked: &bool, _, cx| {
                                for id in &visible_ids {
                                    if *checked {
                                        this.checked.insert(id.clone());
                                    } else {
                                        this.checked.remove(id);
                                    }
                                }
                                cx.notify();
                            },
                        )),
                    ))
                    .child(div().flex_1().child(t("capture.field.name")))
                    .child(div().w_16().flex_none().child(t("capture.field.method")))
                    .child(div().w_20().flex_none().child(t("capture.field.status")))
                    .child(div().w_16().flex_none().child(t("capture.field.type")))
                    .child(div().w_24().flex_none().child(t("capture.field.handled_by")))
                    .child(div().w_20().flex_none().text_right().child(t("capture.field.size")))
                    .child(div().w_20().flex_none().text_right().child(t("capture.field.time_taken")))
                    .child(div().w_20().flex_none().text_right().child(t("capture.field.time_utc"))),
            )
            .child(rows)
            .into_any_element()
    }

    /// A name and its value, name muted.
    fn pair(
        name: impl Into<SharedString>,
        value: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        h_flex()
            .gap_3()
            .items_start()
            .text_sm()
            .child(div().w_48().flex_none().text_color(cx.theme().muted_foreground).child(name.into()))
            .child(div().flex_1().min_w_0().child(value.into()))
            .into_any_element()
    }

    fn pairs(
        label: &'static str,
        rows: &[(String, String)],
        empty: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .gap_1()
            .child(page::section_label(label, cx))
            .children((rows.is_empty()).then(|| div().text_sm().text_color(muted).child(empty)))
            .children(rows.iter().map(|(name, value)| Self::pair(name.clone(), value.clone(), cx)))
            .into_any_element()
    }

    fn render_headers(&self, shown: &Shown, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let exchange = &shown.exchange;
        let status = match exchange.status {
            Some(code) if status_text(code).is_empty() => code.to_string(),
            Some(code) => format!("{code} {}", status_text(code)),
            None => t("capture.pending").to_owned(),
        };
        let via = self.selected_entry().map(TrafficEntry::via_label).unwrap_or_default();
        let handled_by = match (via.is_empty(), &exchange.target) {
            (false, Some(target)) => format!("{via} · {target}"),
            (false, None) => via,
            (true, Some(target)) => target.clone(),
            (true, None) => String::new(),
        };
        let mut general = vec![
            Self::pair(t("capture.field.url"), exchange.url.clone(), cx),
            Self::pair(t("capture.field.method"), exchange.method.clone(), cx),
            Self::pair(t("capture.field.status"), status, cx),
        ];
        if !handled_by.is_empty() {
            general.push(Self::pair(t("capture.field.handled_by"), handled_by, cx));
        }
        if let Some(ms) = exchange.duration_ms {
            general.push(Self::pair(t("capture.field.time_taken"), t_with("capture.duration", &[("ms", &ms)]), cx));
        }
        v_flex()
            .id("capture-headers")
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .gap_4()
            .child(v_flex().gap_1().child(page::section_label(t("capture.section.general"), cx)).children(general))
            .child(Self::pairs(
                t("capture.section.response_headers"),
                &exchange.response_headers,
                t("capture.no_headers"),
                cx,
            ))
            .child(Self::pairs(
                t("capture.section.request_headers"),
                &exchange.request_headers,
                t("capture.no_headers"),
                cx,
            ))
            .into_any_element()
    }

    fn render_payload(&self, shown: &Shown, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let query = shown.exchange.query();
        let query_block = (!query.is_empty()).then(|| {
            div().id("capture-query").flex_none().max_h(px(160.)).overflow_y_scroll().child(Self::pairs(
                t("capture.section.query_parameters"),
                &query,
                "",
                cx,
            ))
        });
        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .children(query_block)
            .child(page::section_label(t("capture.section.request_body"), cx))
            .child(div().flex_1().min_h_0().child(self.render_body(Side::Request, shown, cx)))
            .into_any_element()
    }

    fn render_response(&self, shown: &Shown, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let muted = cx.theme().muted_foreground;
        let truncated = shown.exchange.response_body.len() == RESPONSE_BODY_LIMIT;
        v_flex()
            .size_full()
            .child(div().flex_1().min_h_0().child(self.render_body(Side::Response, shown, cx)))
            .children(truncated.then(|| {
                div().flex_none().px_4().py_2().text_xs().text_color(muted).child(t("capture.response_truncated"))
            }))
            .into_any_element()
    }

    /// One body: text in the read-only editor, an image or other data by itself, or a note that there is none.
    fn render_body(&self, side: Side, shown: &Shown, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let (body, editor, key) = match side {
            Side::Request => (&shown.request, &self.request_body, "request"),
            Side::Response => (&shown.response, &self.response_body, "response"),
        };
        match body {
            Body::Empty => page::empty(
                t("capture.no_body.title"),
                match side {
                    Side::Request => t("capture.no_body.request"),
                    Side::Response => t("capture.no_body.response"),
                },
            ),
            Body::Text { .. } => {
                let copy_from = editor.clone();
                let toggle = (side == Side::Response && body.can_pretty()).then(|| {
                    TabBar::new("capture-body-view")
                        .segmented()
                        .small()
                        .selected_index(if self.response_view == BodyView::Pretty { 0 } else { 1 })
                        .on_click(cx.listener(|this, ix: &usize, window, cx| {
                            this.set_response_view(if *ix == 0 { BodyView::Pretty } else { BodyView::Raw }, window, cx)
                        }))
                        .child(Tab::new().label(t("capture.view.pretty")))
                        .child(Tab::new().label(t("capture.view.raw")))
                });
                v_flex()
                    .size_full()
                    .child(
                        h_flex().flex_none().py_2().gap_2().children(toggle).child(div().flex_1()).child(
                            Button::new(SharedString::from(format!("capture-copy-{key}")))
                                .ghost()
                                .small()
                                .icon(IconName::Copy)
                                .label(t("capture.copy"))
                                .on_click(move |_, _, cx| {
                                    let text = copy_from.read(cx).value().to_string();
                                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                                }),
                        ),
                    )
                    .child(div().flex_1().min_h_0().child(Editor::new(editor).bordered(false).h_full()))
                    .into_any_element()
            }
            Body::Data(binary) => v_flex()
                .size_full()
                .child(
                    h_flex().flex_none().py_2().gap_2().child(div().flex_1()).child(
                        Button::new(SharedString::from(format!("capture-save-{key}-body")))
                            .ghost()
                            .small()
                            .label(t("capture.save_to_file"))
                            .on_click(cx.listener(move |this, _, _, cx| this.save_body(side, cx))),
                    ),
                )
                .child(div().flex_1().min_h_0().child(binary.render(cx)))
                .into_any_element(),
        }
    }

    /// The lines the dev servers printed for this exchange, grouped by service, the context around a match muted.
    fn render_logs(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        match &self.trace_logs {
            TraceLogs::Idle | TraceLogs::Loading => page::loading(t("capture.logs.loading")),
            TraceLogs::Failed(message) => div()
                .p_4()
                .child(
                    Alert::error("capture-logs-failed", SharedString::from(message.clone()))
                        .title(t("capture.logs.error")),
                )
                .into_any_element(),
            TraceLogs::Empty(key) => page::empty(t("capture.logs.empty.title"), t(key)),
            TraceLogs::Found { lines, approximate } => {
                let mut service = None;
                let rows = lines.iter().enumerate().map(|(ix, line)| {
                    let heading = service != Some(line.service.as_str());
                    service = Some(line.service.as_str());
                    v_flex()
                        .when(heading, |this| {
                            this.child(
                                div()
                                    .id(("capture-log-service", ix))
                                    .test_support()
                                    .pt_3()
                                    .pb_1()
                                    .text_xs()
                                    .font_medium()
                                    .child(line.service.clone()),
                            )
                        })
                        .when(line.gap && !heading, |this| this.child(div().h_px().my_1().bg(theme.border)))
                        .child(
                            div()
                                .id(("capture-log", ix))
                                .test_support()
                                .text_xs()
                                .font_family(theme.mono_font_family.clone())
                                .when(!line.matched, |this| this.text_color(theme.muted_foreground))
                                .child(line.text.clone()),
                        )
                });
                v_flex()
                    .id("capture-logs")
                    .size_full()
                    .overflow_y_scroll()
                    .px_4()
                    .pb_3()
                    .when(*approximate, |this| {
                        this.child(
                            div()
                                .id("capture-logs-approximate")
                                .test_support()
                                .pt_3()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(t("capture.logs.approximate")),
                        )
                    })
                    .children(rows)
                    .into_any_element()
            }
        }
    }

    fn render_sections(&self, shown: &Shown, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let selected = SECTIONS.iter().position(|s| *s == self.section).unwrap_or(0);
        let content = match self.section {
            Section::Headers => self.render_headers(shown, cx),
            Section::Payload => self.render_payload(shown, cx),
            Section::Response => self.render_response(shown, cx),
            Section::Logs => self.render_logs(cx),
        };
        v_flex()
            .size_full()
            .child(
                div().id("capture-section-tabs").test_support().flex_none().px_4().child(
                    TabBar::new("capture-sections")
                        .underline()
                        .small()
                        .selected_index(selected)
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.section = SECTIONS[*ix];
                            if this.section == Section::Logs && matches!(this.trace_logs, TraceLogs::Idle) {
                                this.load_logs(cx);
                            }
                            cx.notify();
                        }))
                        .child(Tab::new().label(t("capture.tab.headers")))
                        .child(Tab::new().label(t("capture.tab.payload")))
                        .child(Tab::new().label(t("capture.tab.response")))
                        .child(Tab::new().label(t("capture.tab.logs"))),
                ),
            )
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }

    fn render_detail(&self, entry: &TrafficEntry, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let content = match &self.detail {
            Detail::Idle => div().into_any_element(),
            Detail::Loading => page::loading(t("capture.loading")),
            Detail::Failed(message) => div()
                .p_4()
                .child(
                    Alert::error("capture-load-failed", SharedString::from(message.clone()))
                        .title(t("capture.error.load_title")),
                )
                .into_any_element(),
            Detail::Ready(shown) => self.render_sections(shown, cx),
        };
        v_flex()
            .size_full()
            .bg(theme.background)
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_none().text_sm().child(entry.method.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_family(theme.mono_font_family.clone())
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(entry.url.clone()),
                    )
                    .child(Self::status_tag(entry.status))
                    .when(self.origin == Some(WEBHOOK), |this| {
                        this.child(
                            Button::new("webhook-replay")
                                .small()
                                .label(t("webhooks.replay"))
                                .on_click(cx.listener(|this, _, window, cx| this.replay(None, window, cx))),
                        )
                        .child(
                            Button::new("webhook-replay-to")
                                .small()
                                .label(t("webhooks.replay_to"))
                                .on_click(cx.listener(|this, _, window, cx| this.ask_replay_to(window, cx))),
                        )
                    })
                    .child({
                        let pinned = entry.pinned;
                        Button::new("capture-pin")
                            .small()
                            .label(t(if pinned { "capture.unpin" } else { "capture.pin" }))
                            .tooltip(t(if pinned { "capture.unpin_tooltip" } else { "capture.pin_tooltip" }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(id) = this.selected.clone() {
                                    this.pin(vec![id], !pinned, window, cx);
                                }
                            }))
                    })
                    .child(
                        Button::new("capture-save-request")
                            .small()
                            .label(t("capture.save_as_request"))
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| this.save_as(false, window, cx))),
                    )
                    .child(
                        Button::new("capture-save-mock")
                            .small()
                            .label(t("capture.save_as_mock"))
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| this.save_as(true, window, cx))),
                    )
                    .child(
                        Button::new("capture-copy-curl")
                            .ghost()
                            .small()
                            .icon(IconName::Copy)
                            .tooltip(t("capture.copy_curl"))
                            .disabled(!matches!(self.detail, Detail::Ready(_)))
                            .on_click(cx.listener(|this, _, window, cx| this.copy_curl(window, cx))),
                    )
                    .child(
                        Button::new("capture-remove")
                            .ghost()
                            .small()
                            .icon(IconName::Delete)
                            .tooltip(t("capture.remove_tooltip"))
                            .on_click(cx.listener(|this, _, _, cx| this.remove_selected(cx))),
                    )
                    .child(
                        Button::new("capture-close-detail")
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .tooltip(t("capture.close_detail_tooltip"))
                            .on_click(cx.listener(|this, _, window, cx| this.select(None, window, cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}

impl Render for CaptureScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = page::header(
            t("capture.page.title"),
            t("capture.page.subtitle"),
            h_flex()
                .gap_2()
                .child(div().w_72().child(
                    Input::new(&self.filter).small().prefix(Icon::new(IconName::Search).small()).cleanable(true),
                ))
                .child(
                    Button::new("capture-pause")
                        .small()
                        .icon(if self.paused { IconName::Play } else { IconName::Pause })
                        .label(if self.paused { t("capture.page.resume") } else { t("capture.page.pause") })
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_paused(window, cx))),
                )
                .child(
                    Button::new("capture-clear")
                        .small()
                        .label(t("capture.page.clear"))
                        .disabled(self.entries.is_empty())
                        .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
                ),
            cx,
        );
        let detail = self.selected_entry().cloned().map(|entry| self.render_detail(&entry, cx));
        let split = list_over_detail("capture-split", self.render_list(cx), detail);
        if self.origin.is_some() {
            return split.into_any_element();
        }
        let body = v_flex().size_full().child(self.render_toolbar(cx)).child(div().flex_1().min_h_0().child(split));
        page::screen(header, body, cx).into_any_element()
    }
}
