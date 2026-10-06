//! The response half of an HTTP-like request: what the engine sent back.
//!
//! One `Exchange` belongs to one open request. The response panel shows whichever exchange belongs to the
//! active tab, so switching tabs switches responses without any copying.
//!
//! The body is shown in a read-only code editor highlighted for its content type, either re-indented
//! ("Pretty") or exactly as received ("Raw").

use std::collections::HashSet;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState},
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
};
use serde_json::Value;

use crate::{
    components::{
        Syntax,
        stream_log::{LogRow, Way, log_row},
        binary::{BinaryBody, save_to_file, suggested_file_name},
        page,
    },
    model::{
        http::{BodyEncoding, HttpResponse, pretty_json, status_text},
        media::{self, BodyClass},
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Body,
    Headers,
    Tests,
}

const SECTIONS: [Section; 3] = [Section::Body, Section::Headers, Section::Tests];

#[derive(Clone, Copy, PartialEq, Eq)]
enum BodyView {
    Pretty,
    Raw,
}

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
}

struct LiveCall {
    entries: Vec<LiveEntry>,
    running: bool,
    code: i64,
    message: String,
    metadata: Vec<(String, String)>,
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
    body_view: BodyView,
    body: Entity<EditorState>,
    /// The body as received, and re-indented when that is possible. Both are kept so switching views does
    /// not parse again.
    raw_text: String,
    pretty_text: Option<String>,
    /// The body as bytes, when it is an image or other data rather than text.
    binary: Option<BinaryBody>,
    /// What saving the body to a file came to, shown beside the button until the next response.
    saved_note: Option<SharedString>,
    live_scroll: ScrollHandle,
    follow_tail: bool,
    /// The live messages that are open, by id.
    live_open: HashSet<String>,
}

impl Exchange {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let body = cx.new(|cx| {
            let mut state =
                EditorState::new(window, cx).language(Syntax::Plain.language()).line_number(true).soft_wrap(true);
            state.set_readonly(true, cx);
            state
        });
        Self {
            phase: Phase::Idle,
            section: Section::Body,
            body_view: BodyView::Pretty,
            body,
            raw_text: String::new(),
            pretty_text: None,
            binary: None,
            saved_note: None,
            live_scroll: ScrollHandle::new(),
            follow_tail: false,
            live_open: HashSet::new(),
        }
    }

    pub fn begin(&mut self, cx: &mut Context<Self>) {
        self.phase = Phase::Sending;
        cx.notify();
    }

    pub fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        self.phase = Phase::Failed(message);
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
        self.binary = None;
        self.saved_note = None;
        let (syntax, status) = match HttpResponse::from_value(result, encoding) {
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
                self.raw_text = response.display_body_raw();
                self.pretty_text = (syntax == Syntax::Json).then(|| pretty_json(&response.body)).flatten();
                let status = response.status;
                self.phase = Phase::Http(response);
                (syntax, Some(status))
            }
            None => {
                self.raw_text = serde_json::to_string(result).unwrap_or_default();
                self.pretty_text = serde_json::to_string_pretty(result).ok();
                self.phase = Phase::Raw;
                (Syntax::Json, None)
            }
        };
        self.section = Section::Body;
        self.body.update(cx, |state, cx| state.set_highlighter(syntax.language(), cx));
        self.show_body(window, cx);
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
        });
        cx.notify();
    }

    pub fn live_message(&mut self, payload: &Value, cx: &mut Context<Self>) {
        let Phase::GrpcLive(live) = &mut self.phase else { return };
        let message = &payload["message"];
        live.entries.push(LiveEntry {
            id: message.get("id").and_then(Value::as_str).unwrap_or_default().to_owned(),
            sent: message.get("direction").and_then(Value::as_str) == Some("sent"),
            ts: message.get("ts").and_then(Value::as_u64).unwrap_or(0),
            data: message.get("data").and_then(Value::as_str).unwrap_or_default().to_owned(),
        });
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
        self.raw_text = text;
        self.pretty_text = None;
        self.section = Section::Body;
        self.body.update(cx, |state, cx| state.set_highlighter(Syntax::Json.language(), cx));
        self.show_body(window, cx);
        cx.notify();
    }

    fn show_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = match (self.body_view, &self.pretty_text) {
            (BodyView::Pretty, Some(pretty)) => pretty.clone(),
            _ => self.raw_text.clone(),
        };
        self.body.update(cx, |state, cx| state.set_value(text, window, cx));
    }

    fn set_body_view(&mut self, view: BodyView, window: &mut Window, cx: &mut Context<Self>) {
        if self.body_view != view {
            self.body_view = view;
            self.show_body(window, cx);
            cx.notify();
        }
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
            .child(Tab::new().label("Body"))
            .child(Tab::new().label(format!("Headers ({})", response.headers.len())))
            .child(Tab::new().label(if response.tests.is_empty() {
                "Tests".to_owned()
            } else {
                let passed = response.tests.iter().filter(|t| t.passed).count();
                format!("Tests ({passed}/{})", response.tests.len())
            }))
    }

    fn status_tag(response: &HttpResponse) -> Tag {
        let tag = if response.is_success() { Tag::success() } else { Tag::danger() };
        let reason = status_text(response.status);
        tag.child(if reason.is_empty() { response.status.to_string() } else { format!("{} {reason}", response.status) })
    }

    /// The Pretty/Raw choice and Copy, shown above a body.
    fn render_body_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_pretty = self.pretty_text.is_some();
        h_flex()
            .flex_none()
            .px_3()
            .py_2()
            .gap_2()
            .when(has_pretty, |this| {
                this.child(
                    TabBar::new("body-view")
                        .segmented()
                        .small()
                        .selected_index(if self.body_view == BodyView::Pretty { 0 } else { 1 })
                        .on_click(cx.listener(|this, ix: &usize, window, cx| {
                            this.set_body_view(if *ix == 0 { BodyView::Pretty } else { BodyView::Raw }, window, cx)
                        }))
                        .child(Tab::new().label("Pretty"))
                        .child(Tab::new().label("Raw")),
                )
            })
            .child(div().flex_1())
            .child(Button::new("copy-body").ghost().small().icon(IconName::Copy).label("Copy").on_click(cx.listener(
                |this, _, _, cx| {
                    let text = this.body.read(cx).value().to_string();
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                },
            )))
    }

    fn render_body(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        v_flex()
            .size_full()
            .child(self.render_body_toolbar(cx))
            .child(div().flex_1().min_h_0().child(Editor::new(&self.body).bordered(false).h_full()))
            .into_any_element()
    }

    fn save_binary(&mut self, cx: &mut Context<Self>) {
        let Some(binary) = &self.binary else { return };
        let name = suggested_file_name("", "response", binary.content_type());
        save_to_file(binary.bytes().to_vec(), name, cx, |this, outcome, cx| {
            this.saved_note = Some(match outcome {
                Ok(path) => format!("Saved to {}", path.display()).into(),
                Err(error) => format!("Couldn't save the file. {error}").into(),
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
                            .label("Save to file…")
                            .on_click(cx.listener(|this, _, _, cx| this.save_binary(cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(binary.render(cx)))
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
                    "Binary body",
                    format!("{} of data that can't be shown as text.", response.body_size_label()),
                ),
                Section::Body if response.size_bytes == 0 => page::empty("Empty body", "The response has no body."),
                Section::Body => self.render_body(cx),
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
                Section::Tests if response.tests.is_empty() => {
                    page::empty("No tests ran", "Add a test script to the request to check its response.")
                }
                Section::Tests => v_flex()
                    .id("response-tests")
                    .overflow_y_scroll()
                    .size_full()
                    .p_3()
                    .gap_2()
                    .children(response.tests.iter().map(|test| {
                        h_flex()
                            .gap_2()
                            .items_start()
                            .child(div().w_16().flex_none().child(if test.passed {
                                Tag::success().child("Passed")
                            } else {
                                Tag::danger().child("Failed")
                            }))
                            .child(v_flex().flex_1().min_w_0().child(test.name.clone()).when_some(
                                test.error.clone(),
                                |this, error| {
                                    this.child(div().text_sm().text_color(theme.muted_foreground).child(error))
                                },
                            ))
                    }))
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
                    .child(Self::status_tag(response))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(format!("{} ms", response.duration_ms)),
                    )
                    .child(div().text_sm().text_color(theme.muted_foreground).child(response.body_size_label())),
            )
            .children(response.script_errors.iter().enumerate().map(|(ix, error)| {
                div().px_3().pt_2().child(
                    Alert::warning(("script-error", ix), SharedString::from(error.clone())).title("A script failed"),
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
            tag: if entry.sent { "Sent" } else { "Received" }.to_owned(),
            outline: entry.sent,
            way: Some(if entry.sent { Way::Sent } else { Way::Received }),
            note: None,
            data: entry.data.clone(),
        };
        let id = entry.id.clone();
        log_row(
            &row,
            self.live_open.contains(&entry.id),
            cx.listener(move |this, _, _, cx| {
                if !this.live_open.remove(&id) {
                    this.live_open.insert(id.clone());
                }
                cx.notify();
            }),
            cx,
        )
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
        let selected = if self.section == Section::Headers { 1 } else { 0 };
        let tag = if live.running {
            Tag::warning().child("Open")
        } else if ok {
            Tag::success().child("OK")
        } else {
            Tag::danger().child(grpc_status_name(live.code))
        };
        let content = match self.section {
            Section::Headers if live.metadata.is_empty() => {
                page::empty("No metadata", "The server has sent no headers or trailers yet.")
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
                page::empty(if live.running { "Waiting for messages" } else { "No messages" }, "Messages appear here as they are sent and received.")
            }
            _ => v_flex()
                .id("grpc-live-log")
                .track_scroll(&self.live_scroll)
                .overflow_y_scroll()
                .size_full()
                .children(live.entries.iter().map(|entry| self.render_live_row(entry, cx)))
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
                    .child(div().text_sm().text_color(theme.muted_foreground).child(format!("{received} received · {sent} sent")))
                    .child(div().text_sm().text_color(theme.muted_foreground).child("times in UTC")),
            )
            .when(!live.running && !ok && !live.message.is_empty(), |this| {
                this.child(div().px_3().pt_2().child(
                    Alert::error("grpc-live-status", SharedString::from(live.message.clone())).title("The call failed"),
                ))
            })
            .child(
                div().flex_none().px_3().child(
                    TabBar::new("grpc-live-sections")
                        .underline()
                        .small()
                        .selected_index(selected)
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.section = if *ix == 1 { Section::Headers } else { Section::Body };
                            cx.notify();
                        }))
                        .child(Tab::new().label("Messages"))
                        .child(Tab::new().label(format!("Metadata ({})", live.metadata.len()))),
                ),
            )
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }

    fn render_grpc(&self, reply: &GrpcReply, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let ok = reply.code == 0;
        let selected = if self.section == Section::Headers { 1 } else { 0 };
        let content = match self.section {
            Section::Headers if reply.metadata.is_empty() => {
                page::empty("No metadata", "The server sent no headers or trailers with this response.")
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
                if ok { "No messages" } else { "No response" },
                if ok { "The call finished without a message." } else { "The call failed before a message arrived." },
            ),
            _ => self.render_body(cx),
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
                    .child(div().text_sm().text_color(theme.muted_foreground).child(format!("{} ms", reply.duration_ms)))
                    .child(div().text_sm().text_color(theme.muted_foreground).child(match reply.messages {
                        1 => "1 message".to_owned(),
                        n => format!("{n} messages"),
                    })),
            )
            .when(!ok && !reply.message.is_empty(), |this| {
                this.child(div().px_3().pt_2().child(
                    Alert::error("grpc-status", SharedString::from(reply.message.clone())).title("The call failed"),
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
                        .child(Tab::new().label("Body"))
                        .child(Tab::new().label(format!("Metadata ({})", reply.metadata.len()))),
                ),
            )
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}

impl Render for Exchange {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.phase {
            Phase::Idle => page::empty("No response yet", "Send the request to see what comes back."),
            Phase::Sending => page::loading("Waiting for the response…"),
            Phase::Failed(message) => div()
                .p_3()
                .child(
                    Alert::error("send-failed", SharedString::from(message.clone())).title("Couldn't send the request"),
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
            Phase::Raw => self.render_body(cx),
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
