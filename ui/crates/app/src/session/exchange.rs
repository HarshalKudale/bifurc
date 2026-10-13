//! The response half of an HTTP-like request: what the engine sent back.
//!
//! One `Exchange` belongs to one open request. The response panel shows whichever exchange belongs to the
//! active tab, so switching tabs switches responses without any copying.
//!
//! The body is shown in a read-only code editor highlighted for its content type, either re-indented
//! ("Pretty") or exactly as received ("Raw").

use base64::{Engine as _, engine::general_purpose::STANDARD};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    h_flex,
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ListAlignment, ListOffset, ListState,
    ParentElement as _, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    list, px,
};
use serde_json::Value;

use crate::{
    i18n::{t, t_with},
    components::{
        BodyPane, Syntax,
        stream_log::{LogRow, LogSearch, OpenRows, Way, compact, found, log_row, nothing_found, search_button},
        binary::{BinaryBody, save_to_file, suggested_file_name},
        page,
    },
    model::{
        http::{BodyEncoding, HttpResponse, TestOutcome, pretty_json, status_text},
        media::{self, BodyClass},
        stream::MAX_MESSAGES,
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Body,
    Headers,
    Tests,
}

const SECTIONS: [Section; 3] = [Section::Body, Section::Headers, Section::Tests];

enum Phase {
    Idle,
    Sending,
    Failed(String),
    Http(HttpResponse),
    /// A result that is not HTTP-shaped (SOAP), shown as the JSON the engine returned.
    Raw,
    Grpc(GrpcReply),
    /// A gRPC call that is open, or was: its messages as they came, in order.
    GrpcLive(LiveCall),
}

struct LiveEntry {
    id: String,
    sent: bool,
    ts: u64,
    data: String,
    /// The start of `data` on one line, which is what a closed row shows. Made once: a row is drawn often.
    line: String,
    /// The name and id of an event, for a message that is one.
    event: Option<String>,
    event_id: Option<String>,
}

impl LiveEntry {
    fn new(id: String, sent: bool, ts: u64, data: String) -> Self {
        let mut line = compact(&data);
        if let Some((end, _)) = line.char_indices().nth(LINE_CHARS) {
            line.truncate(end);
        }
        Self { id, sent, ts, data, line, event: None, event_id: None }
    }
}

/// More characters than a row has room for.
const LINE_CHARS: usize = 400;

struct LiveCall {
    entries: Vec<LiveEntry>,
    running: bool,
    code: i64,
    message: String,
    metadata: Vec<(String, String)>,
    /// Set when this is an HTTP response that arrived in pieces rather than a gRPC call: its status, and
    /// once it has ended how long it took and what its tests found.
    http: Option<HttpResponse>,
}

/// What a gRPC call came back with, apart from the messages (those are in the body editor).
struct GrpcReply {
    code: i64,
    message: String,
    duration_ms: u64,
    metadata: Vec<(String, String)>,
    messages: usize,
}

/// The canonical name of a gRPC status code, as tools print it.
pub fn grpc_status_name(code: i64) -> &'static str {
    match code {
        0 => "OK",
        1 => "CANCELLED",
        2 => "UNKNOWN",
        3 => "INVALID_ARGUMENT",
        4 => "DEADLINE_EXCEEDED",
        5 => "NOT_FOUND",
        6 => "ALREADY_EXISTS",
        7 => "PERMISSION_DENIED",
        8 => "RESOURCE_EXHAUSTED",
        9 => "FAILED_PRECONDITION",
        10 => "ABORTED",
        11 => "OUT_OF_RANGE",
        12 => "UNIMPLEMENTED",
        13 => "INTERNAL",
        14 => "UNAVAILABLE",
        15 => "DATA_LOSS",
        16 => "UNAUTHENTICATED",
        _ => "UNKNOWN",
    }
}

pub struct Exchange {
    phase: Phase,
    section: Section,
    body: Entity<BodyPane>,
    /// The body as bytes, when it is an image or other data rather than text.
    binary: Option<BinaryBody>,
    /// What saving the body to a file came to, shown beside the button until the next response.
    saved_note: Option<SharedString>,
    live_scroll: ScrollHandle,
    follow_tail: bool,
    /// The list of an HTTP response's messages, and the entry each of its rows shows, newest first.
    live_list: ListState,
    live_shown: Vec<usize>,
    /// The live messages that are open, by id.
    live_open: OpenRows,
    live_search: LogSearch,
}

impl Exchange {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let body = cx.new(|cx| BodyPane::new(window, cx));
        Self {
            phase: Phase::Idle,
            section: Section::Body,
            body,
            binary: None,
            saved_note: None,
            live_scroll: ScrollHandle::new(),
            follow_tail: false,
            live_list: ListState::new(0, ListAlignment::Top, px(400.)),
            live_shown: Vec::new(),
            live_open: OpenRows::default(),
            live_search: LogSearch::new(window, cx, |this, cx| {
                this.live_open.highlight(&this.live_search.query(cx), cx);
                cx.notify();
            }),
        }
    }

    pub fn begin(&mut self, cx: &mut Context<Self>) {
        self.phase = Phase::Sending;
        cx.notify();
    }

    /// The HTTP response that is arriving in pieces, while it still is.
    fn arriving(&mut self) -> Option<&mut LiveCall> {
        match &mut self.phase {
            Phase::GrpcLive(live) if live.running && live.http.is_some() => Some(live),
            _ => None,
        }
    }

    pub fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        // A response that broke off keeps the messages that came.
        if let Some(live) = self.arriving() {
            live.running = false;
            live.message = message;
        } else {
            self.phase = Phase::Failed(message);
        }
        cx.notify();
    }

    /// Takes an `event.request.stream` payload: messages of an HTTP response that is arriving in pieces. The
    /// first one carries the status and headers. The response is shown as the log of its messages, and stays
    /// that when it ends.
    pub fn receive(&mut self, payload: &Value, cx: &mut Context<Self>) {
        if self.arriving().is_none() {
            let Some(response) = HttpResponse::from_value(payload, BodyEncoding::Text) else { return };
            self.binary = None;
            self.section = Section::Body;
            self.live_open.clear();
            self.live_list.reset(0);
            self.phase = Phase::GrpcLive(LiveCall {
                entries: Vec::new(),
                running: true,
                code: 0,
                message: String::new(),
                metadata: response.headers.clone(),
                http: Some(response),
            });
        }
        let Phase::GrpcLive(live) = &mut self.phase else { return };
        let text = |message: &Value, key: &str| message.get(key).and_then(Value::as_str).map(str::to_owned);
        for message in payload.get("messages").and_then(Value::as_array).into_iter().flatten() {
            let mut entry = LiveEntry::new(
                text(message, "id").unwrap_or_default(),
                false,
                message.get("ts").and_then(Value::as_u64).unwrap_or(0),
                text(message, "data").unwrap_or_default(),
            );
            entry.event = text(message, "event");
            entry.event_id = text(message, "eventId");
            live.entries.push(entry);
        }
        if live.entries.len() > MAX_MESSAGES {
            let excess = live.entries.len() - MAX_MESSAGES;
            live.entries.drain(..excess);
            let entries = &live.entries;
            self.live_open.retain(|id| entries.iter().any(|entry| entry.id == id));
        }
        cx.notify();
    }

    /// Takes the engine's send result. A result the HTTP reader does not recognise is another protocol's
    /// shape; it is shown as formatted JSON rather than refused. Returns the HTTP status, if there is one.
    pub fn finish(
        &mut self,
        result: &Value,
        encoding: BodyEncoding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u16> {
        // A response that arrived in pieces stays the log of them; the result adds what is known at the end.
        if let Some(live) = self.arriving() {
            let mut end = serde_json::Map::new();
            for key in ["status", "headers", "durationMs", "tests", "preScriptError", "postScriptError"] {
                if let Some(value) = result.get(key) {
                    end.insert(key.to_owned(), value.clone());
                }
            }
            live.running = false;
            if let Some(response) = HttpResponse::from_value(&Value::Object(end), encoding) {
                live.http = Some(response);
            }
            cx.notify();
            return live.http.as_ref().map(|response| response.status);
        }
        self.binary = None;
        self.saved_note = None;
        let (raw, pretty, syntax, status) = match HttpResponse::from_value(result, encoding) {
            Some(response) => {
                let content_type = response.content_type().unwrap_or_default().to_owned();
                // An image is shown as an image even when its bytes happen to be text (SVG aside, which
                // is text and reads as such).
                if encoding == BodyEncoding::Base64
                    && (response.is_binary || media::classify(&content_type) == BodyClass::Image)
                    && let Some(bytes) = result
                        .get("body")
                        .and_then(Value::as_str)
                        .and_then(|raw| STANDARD.decode(raw.trim()).ok())
                        .filter(|bytes| !bytes.is_empty())
                {
                    self.binary = Some(BinaryBody::new(bytes, content_type));
                }
                let syntax = if response.is_binary {
                    Syntax::Plain
                } else {
                    match Syntax::for_content_type(response.content_type().unwrap_or_default()) {
                        // A JSON body is recognised by its content too: servers often mislabel it.
                        Syntax::Plain if pretty_json(&response.body).is_some() => Syntax::Json,
                        other => other,
                    }
                };
                let raw = response.display_body_raw();
                let pretty = (syntax == Syntax::Json).then(|| pretty_json(&response.body)).flatten();
                let status = response.status;
                self.phase = Phase::Http(response);
                (raw, pretty, syntax, Some(status))
            }
            None => {
                self.phase = Phase::Raw;
                let raw = serde_json::to_string(result).unwrap_or_default();
                (raw, serde_json::to_string_pretty(result).ok(), Syntax::Json, None)
            }
        };
        self.section = Section::Body;
        self.body.update(cx, |body, cx| body.set(raw, pretty, syntax, window, cx));
        cx.notify();
        status
    }

    /// A live gRPC call starts: the panel shows its messages as they arrive.
    pub fn begin_live(&mut self, cx: &mut Context<Self>) {
        self.binary = None;
        self.section = Section::Body;
        self.live_open.clear();
        self.phase = Phase::GrpcLive(LiveCall {
            entries: Vec::new(),
            running: true,
            code: 0,
            message: String::new(),
            metadata: Vec::new(),
            http: None,
        });
        cx.notify();
    }

    pub fn live_message(&mut self, payload: &Value, cx: &mut Context<Self>) {
        let Phase::GrpcLive(live) = &mut self.phase else { return };
        let message = &payload["message"];
        live.entries.push(LiveEntry::new(
            message.get("id").and_then(Value::as_str).unwrap_or_default().to_owned(),
            message.get("direction").and_then(Value::as_str) == Some("sent"),
            message.get("ts").and_then(Value::as_u64).unwrap_or(0),
            message.get("data").and_then(Value::as_str).unwrap_or_default().to_owned(),
        ));
        self.follow_tail = true;
        cx.notify();
    }

    pub fn live_status(&mut self, payload: &Value, cx: &mut Context<Self>) {
        let Phase::GrpcLive(live) = &mut self.phase else { return };
        if let Some(metadata) = payload.get("metadata").and_then(Value::as_object) {
            live.metadata = metadata
                .iter()
                .map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_owned()))
                .collect();
        }
        if payload.get("status").and_then(Value::as_str) == Some("closed") {
            live.running = false;
            live.code = payload.get("grpcStatus").and_then(Value::as_i64).unwrap_or(0);
            live.message = payload.get("statusMessage").and_then(Value::as_str).unwrap_or_default().to_owned();
        }
        cx.notify();
    }

    /// Takes a `grpc.execute` / `grpc.send` result: the messages go in the body as one JSON document (an array
    /// when the call streamed several), the status and metadata beside it.
    pub fn finish_grpc(&mut self, result: &Value, window: &mut Window, cx: &mut Context<Self>) {
        self.binary = None;
        self.saved_note = None;
        let responses: Vec<&str> =
            result.get("responses").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        let text = match responses.as_slice() {
            [] => String::new(),
            [one] => (*one).to_owned(),
            many => format!("[\n{}\n]", many.join(",\n")),
        };
        let metadata = result
            .get("metadata")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_owned()))
            .collect();
        self.phase = Phase::Grpc(GrpcReply {
            code: result.get("status").and_then(Value::as_i64).unwrap_or(0),
            message: result.get("statusMessage").and_then(Value::as_str).unwrap_or_default().to_owned(),
            duration_ms: result.get("durationMs").and_then(Value::as_u64).unwrap_or(0),
            metadata,
            messages: responses.len(),
        });
        self.section = Section::Body;
        self.body.update(cx, |body, cx| body.set(text, None, Syntax::Json, window, cx));
        cx.notify();
    }

    fn tabs(&self, response: &HttpResponse, cx: &mut Context<Self>) -> TabBar {
        let selected = SECTIONS.iter().position(|s| *s == self.section).unwrap_or(0);
        TabBar::new("response-sections")
            .underline()
            .small()
            .selected_index(selected)
            .on_click(cx.listener(move |this, ix: &usize, _, cx| {
                this.section = SECTIONS[*ix];
                cx.notify();
            }))
            .child(Tab::new().label(t("session.response.tab_body")))
            .child(Tab::new().label(t_with("session.response.tab_headers", &[("count", &response.headers.len())])))
            .child(Self::tests_tab(&response.tests))
    }

    fn tests_tab(tests: &[TestOutcome]) -> Tab {
        Tab::new().label(if tests.is_empty() {
            t("session.response.tab_tests").to_owned()
        } else {
            let passed = tests.iter().filter(|t| t.passed).count();
            t_with("session.response.tab_tests_count", &[("passed", &passed), ("total", &tests.len())])
        })
    }

    fn status_tag(response: &HttpResponse) -> Tag {
        let tag = if response.is_success() { Tag::success() } else { Tag::danger() };
        let reason = status_text(response.status);
        tag.child(if reason.is_empty() { response.status.to_string() } else { format!("{} {reason}", response.status) })
    }

    fn render_body(&self) -> gpui_kit::AnyElement {
        self.body.clone().into_any_element()
    }

    /// The search button of a single response: it opens the body's own find.
    fn find_button(&self, enabled: bool, cx: &mut Context<Self>) -> impl IntoElement {
        search_button("find-in-body")
            .disabled(!enabled)
            .on_click(cx.listener(|this, _, _, cx| this.body.update(cx, |body, cx| body.find(cx))))
    }

    fn save_binary(&mut self, cx: &mut Context<Self>) {
        let Some(binary) = &self.binary else { return };
        let name = suggested_file_name("", "response", binary.content_type());
        save_to_file(binary.bytes().to_vec(), name, cx, |this, outcome, cx| {
            this.saved_note = Some(match outcome {
                Ok(path) => t_with("session.response.saved_to", &[("path", &path.display())]).into(),
                Err(error) => t_with("session.response.save_failed", &[("error", &error)]).into(),
            });
            cx.notify();
        });
    }

    /// A body that is an image or other data: the image or a line about the data, and a way to keep it.
    fn render_binary(&self, binary: &BinaryBody, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .children(self.saved_note.clone()),
                    )
                    .child(
                        Button::new("save-body")
                            .ghost()
                            .small()
                            .icon(IconName::File)
                            .label(t("session.response.save_file"))
                            .on_click(cx.listener(|this, _, _, cx| this.save_binary(cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(binary.render(cx)))
            .into_any_element()
    }

    /// What the test script found, a test to a row.
    fn render_tests(tests: &[TestOutcome], cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        if tests.is_empty() {
            return page::empty(t("session.response.no_tests_title"), t("session.response.no_tests_description"));
        }
        let theme = cx.theme().clone();
        v_flex()
            .id("response-tests")
            .overflow_y_scroll()
            .size_full()
            .p_3()
            .gap_2()
            .children(tests.iter().map(|test| {
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(div().w_16().flex_none().child(if test.passed {
                        Tag::success().child(t("session.response.test_passed"))
                    } else {
                        Tag::danger().child(t("session.response.test_failed"))
                    }))
                    .child(v_flex().flex_1().min_w_0().child(test.name.clone()).when_some(test.error.clone(), |this, error| {
                        this.child(div().text_sm().text_color(theme.muted_foreground).child(error))
                    }))
            }))
            .into_any_element()
    }

    fn render_http(&self, response: &HttpResponse, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let content =
            match self.section {
                Section::Body if self.binary.is_some() => {
                    let binary = self.binary.clone().expect("checked by the guard");
                    self.render_binary(&binary, cx)
                }
                Section::Body if response.is_binary => page::empty(
                    t("session.response.binary_title"),
                    t_with("session.response.binary_description", &[("size", &response.body_size_label())]),
                ),
                Section::Body if response.size_bytes == 0 => {
                    page::empty(t("session.response.empty_title"), t("session.response.empty_description"))
                }
                Section::Body => self.render_body(),
                Section::Headers => v_flex()
                    .id("response-headers")
                    .overflow_y_scroll()
                    .size_full()
                    .p_3()
                    .gap_2()
                    .children(response.headers.iter().map(|(name, value)| {
                        h_flex()
                            .gap_3()
                            .items_start()
                            .text_sm()
                            .child(div().w_48().flex_none().text_color(theme.muted_foreground).child(name.clone()))
                            .child(div().flex_1().min_w_0().child(value.clone()))
                    }))
                    .into_any_element(),
                Section::Tests => Self::render_tests(&response.tests, cx),
            };

        let can_find = self.section == Section::Body
            && self.binary.is_none()
            && !response.is_binary
            && response.size_bytes != 0;
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .px_3()
                    .pt_2()
                    .gap_3()
                    .child(Self::status_tag(response))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(t_with("session.response.duration", &[("ms", &response.duration_ms)])),
                    )
                    .child(div().text_sm().text_color(theme.muted_foreground).child(response.body_size_label()))
                    .child(div().flex_1())
                    .child(self.find_button(can_find, cx)),
            )
            .children(response.script_errors.iter().enumerate().map(|(ix, error)| {
                div().px_3().pt_2().child(
                    Alert::warning(("script-error", ix), SharedString::from(error.clone())).title(t("session.response.script_failed")),
                )
            }))
            .child(div().flex_none().px_3().child(self.tabs(response, cx)))
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}

impl Exchange {
    /// One message of a live call: the shared log row, which opens to the whole message.
    fn render_live_row(&self, entry: &LiveEntry, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let row = LogRow {
            id: entry.id.clone(),
            ts: entry.ts,
            tag: entry.event.clone().unwrap_or_else(|| {
                if entry.sent { t("session.log.sent") } else { t("session.log.received") }.to_owned()
            }),
            outline: entry.sent || entry.event.as_deref() == Some("message"),
            way: entry.event.is_none().then_some(if entry.sent { Way::Sent } else { Way::Received }),
            note: entry.event_id.as_ref().map(|id| t_with("session.sse.event_id", &[("id", id)])),
            data: entry.line.clone(),
        };
        let id = entry.id.clone();
        log_row(
            &row,
            self.live_open.get(&entry.id),
            cx.listener(move |this, _, window, cx| {
                let Phase::GrpcLive(live) = &this.phase else { return };
                let Some(entry) = live.entries.iter().find(|entry| entry.id == id) else { return };
                let query = this.live_search.query(cx);
                this.live_open.toggle(&id, &entry.data, &query, window, cx);
                cx.notify();
            }),
            cx,
        )
    }

    /// The row at `ix` of the list of an HTTP response's messages.
    fn render_shown_row(&self, ix: usize, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Phase::GrpcLive(live) = &self.phase else { return div().into_any_element() };
        match self.live_shown.get(ix).and_then(|at| live.entries.get(*at)) {
            Some(entry) => self.render_live_row(entry, cx),
            None => div().into_any_element(),
        }
    }

    fn render_live(&mut self, live: &LiveCall, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        if self.follow_tail {
            self.follow_tail = false;
            self.live_scroll.scroll_to_bottom();
        }
        let received = live.entries.iter().filter(|e| !e.sent).count();
        let sent = live.entries.len() - received;
        let ok = live.code == 0;
        let selected = SECTIONS.iter().position(|s| *s == self.section).unwrap_or(0);
        let tag = match &live.http {
            _ if live.running => Tag::warning().child(t("session.log.open")),
            Some(response) => Self::status_tag(response),
            None if ok => Tag::success().child("OK"),
            None => Tag::danger().child(grpc_status_name(live.code)),
        };
        let needle = self.live_search.needle(cx);
        let shown: Vec<&LiveEntry> = live
            .entries
            .iter()
            .filter(|entry| found(needle.as_deref(), &[&entry.data, entry.event.as_deref().unwrap_or_default()]))
            .collect();
        let search_bar = if self.section == Section::Body {
            self.live_search.bar(shown.len(), live.entries.len(), cx)
        } else {
            None
        };
        let content = match self.section {
            Section::Tests => {
                Self::render_tests(live.http.as_ref().map(|response| response.tests.as_slice()).unwrap_or_default(), cx)
            }
            Section::Headers if live.metadata.is_empty() => {
                page::empty(t("session.response.no_metadata_title"), t("session.response.no_metadata_live_description"))
            }
            Section::Headers => v_flex()
                .id("grpc-live-metadata")
                .overflow_y_scroll()
                .size_full()
                .p_3()
                .gap_2()
                .children(live.metadata.iter().map(|(name, value)| {
                    h_flex()
                        .gap_3()
                        .items_start()
                        .text_sm()
                        .child(div().w_48().flex_none().text_color(theme.muted_foreground).child(name.clone()))
                        .child(div().flex_1().min_w_0().child(value.clone()))
                }))
                .into_any_element(),
            _ if live.entries.is_empty() => {
                page::empty(
                    if live.running { t("session.log.waiting_for_messages") } else { t("session.log.no_messages") },
                    t("session.log.messages_appear_here"),
                )
            }
            _ if shown.is_empty() => nothing_found(),
            // An HTTP response has the newest message first, and builds only the rows that are in view.
            _ if live.http.is_some() => {
                self.live_shown = (0..live.entries.len())
                    .rev()
                    .filter(|ix| {
                        let entry = &live.entries[*ix];
                        found(needle.as_deref(), &[&entry.data, entry.event.as_deref().unwrap_or_default()])
                    })
                    .collect();
                let (old, new) = (self.live_list.item_count(), self.live_shown.len());
                if new > old && needle.is_none() {
                    // Messages came: the list keeps its place, and a list that was at its top stays there.
                    let top = self.live_list.logical_scroll_top();
                    self.live_list.splice(0..0, new - old);
                    if top.item_ix == 0 && top.offset_in_item == px(0.) {
                        self.live_list.scroll_to(ListOffset::default());
                    }
                } else if new != old {
                    self.live_list.reset(new);
                }
                list(self.live_list.clone(), cx.processor(|this, ix: usize, _, cx| this.render_shown_row(ix, cx)))
                    .size_full()
                    .into_any_element()
            }
            _ => v_flex()
                .id("grpc-live-log")
                .track_scroll(&self.live_scroll)
                .overflow_y_scroll()
                .size_full()
                .children(shown.into_iter().map(|entry| self.render_live_row(entry, cx)))
                .into_any_element(),
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .px_3()
                    .pt_2()
                    .gap_3()
                    .child(tag)
                    .children(live.http.as_ref().filter(|_| !live.running && live.message.is_empty()).map(|response| {
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(t_with("session.response.duration", &[("ms", &response.duration_ms)]))
                    }))
                    .child(div().text_sm().text_color(theme.muted_foreground).child(match &live.http {
                        Some(_) if received == 1 => t_with("session.response.messages_one", &[("count", &received)]),
                        Some(_) => t_with("session.response.messages_other", &[("count", &received)]),
                        None => t_with("session.log.received_sent", &[("received", &received), ("sent", &sent)]),
                    }))
                    .child(div().text_sm().text_color(theme.muted_foreground).child(t("session.log.times_utc")))
                    .child(div().flex_1())
                    .child(search_button("search-log").disabled(self.section != Section::Body).on_click(
                        cx.listener(|this, _, window, cx| {
                            this.live_search.toggle(window, cx);
                            cx.notify();
                        }),
                    )),
            )
            .when(!live.running && !live.message.is_empty() && (!ok || live.http.is_some()), |this| {
                this.child(div().px_3().pt_2().child(
                    Alert::error("grpc-live-status", SharedString::from(live.message.clone())).title(t(
                        if live.http.is_some() { "session.response.send_failed" } else { "session.response.call_failed" },
                    )),
                ))
            })
            .children(live.http.iter().flat_map(|response| response.script_errors.iter()).enumerate().map(|(ix, error)| {
                div().px_3().pt_2().child(
                    Alert::warning(("script-error", ix), SharedString::from(error.clone())).title(t("session.response.script_failed")),
                )
            }))
            .child(
                div().flex_none().px_3().child(
                    TabBar::new("grpc-live-sections")
                        .underline()
                        .small()
                        .selected_index(selected)
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.section = SECTIONS[*ix];
                            cx.notify();
                        }))
                        .child(Tab::new().label(t("session.response.tab_messages")))
                        .child(Tab::new().label(t_with(
                            if live.http.is_some() { "session.response.tab_headers" } else { "session.response.tab_metadata" },
                            &[("count", &live.metadata.len())],
                        )))
                        .children(live.http.as_ref().map(|response| Self::tests_tab(&response.tests))),
                ),
            )
            .children(search_bar.map(|bar| div().flex_none().pt_2().child(bar)))
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }

    fn render_grpc(&self, reply: &GrpcReply, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let ok = reply.code == 0;
        let selected = if self.section == Section::Headers { 1 } else { 0 };
        let content = match self.section {
            Section::Headers if reply.metadata.is_empty() => {
                page::empty(t("session.response.no_metadata_title"), t("session.response.no_metadata_description"))
            }
            Section::Headers => v_flex()
                .id("grpc-metadata")
                .overflow_y_scroll()
                .size_full()
                .p_3()
                .gap_2()
                .children(reply.metadata.iter().map(|(name, value)| {
                    h_flex()
                        .gap_3()
                        .items_start()
                        .text_sm()
                        .child(div().w_48().flex_none().text_color(theme.muted_foreground).child(name.clone()))
                        .child(div().flex_1().min_w_0().child(value.clone()))
                }))
                .into_any_element(),
            _ if reply.messages == 0 => page::empty(
                if ok { t("session.log.no_messages") } else { t("session.response.no_response_title") },
                if ok { t("session.response.finished_without_message") } else { t("session.response.failed_before_message") },
            ),
            _ => self.render_body(),
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .px_3()
                    .pt_2()
                    .gap_3()
                    .child(if ok { Tag::success() } else { Tag::danger() }.child(grpc_status_name(reply.code)))
                    .child(div().text_sm().text_color(theme.muted_foreground).child(t_with("session.response.duration", &[("ms", &reply.duration_ms)])))
                    .child(div().text_sm().text_color(theme.muted_foreground).child(match reply.messages {
                        1 => t_with("session.response.messages_one", &[("count", &reply.messages)]),
                        n => t_with("session.response.messages_other", &[("count", &n)]),
                    }))
                    .child(div().flex_1())
                    .child(self.find_button(self.section == Section::Body && reply.messages != 0, cx)),
            )
            .when(!ok && !reply.message.is_empty(), |this| {
                this.child(div().px_3().pt_2().child(
                    Alert::error("grpc-status", SharedString::from(reply.message.clone())).title(t("session.response.call_failed")),
                ))
            })
            .child(
                div().flex_none().px_3().child(
                    TabBar::new("grpc-sections")
                        .underline()
                        .small()
                        .selected_index(selected)
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.section = if *ix == 1 { Section::Headers } else { Section::Body };
                            cx.notify();
                        }))
                        .child(Tab::new().label(t("session.response.tab_body")))
                        .child(Tab::new().label(t_with("session.response.tab_metadata", &[("count", &reply.metadata.len())]))),
                ),
            )
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}

impl Render for Exchange {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.phase {
            Phase::Idle => page::empty(t("session.response.idle_title"), t("session.response.idle_description")),
            Phase::Sending => page::loading(t("session.response.waiting")),
            Phase::Failed(message) => div()
                .p_3()
                .child(
                    Alert::error("send-failed", SharedString::from(message.clone())).title(t("session.response.send_failed")),
                )
                .into_any_element(),
            Phase::Http(_) => {
                // The renderer borrows `self` for its handlers while reading the response, so the response
                // is moved out for the call and put back.
                let Phase::Http(response) = std::mem::replace(&mut self.phase, Phase::Idle) else { unreachable!() };
                let element = self.render_http(&response, cx);
                self.phase = Phase::Http(response);
                element
            }
            Phase::Raw => v_flex()
                .size_full()
                .child(h_flex().flex_none().px_3().pt_2().child(div().flex_1()).child(self.find_button(true, cx)))
                .child(div().flex_1().min_h_0().child(self.render_body()))
                .into_any_element(),
            Phase::GrpcLive(_) => {
                let Phase::GrpcLive(live) = std::mem::replace(&mut self.phase, Phase::Idle) else { unreachable!() };
                let element = self.render_live(&live, cx);
                self.phase = Phase::GrpcLive(live);
                element
            }
            Phase::Grpc(_) => {
                let Phase::Grpc(reply) = std::mem::replace(&mut self.phase, Phase::Idle) else { unreachable!() };
                let element = self.render_grpc(&reply, cx);
                self.phase = Phase::Grpc(reply);
                element
            }
        };
        div().size_full().bg(cx.theme().background).child(body)
    }
}
