//! The messages of a captured WebSocket or event stream: what the client sent and the server answered, in the
//! shared live log, growing while the stream is open (`event.log.message`).

use gpui_kit::component::tag::Tag;
use gpui_kit::{Context, IntoElement, ParentElement as _, Render, Window};

use crate::{
    components::stream_log::{LogFrame, LogRow, LogState, Way, found, log_frame, log_row},
    i18n::{t, t_with},
    model::traffic::{Exchange, StreamMessage, push_message},
};

pub struct CapturedStream {
    /// The capture id of the stream shown.
    log_id: String,
    messages: Vec<StreamMessage>,
    /// `ws` or `sse`.
    kind: String,
    live: bool,
    log: LogState,
}

impl CapturedStream {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            log_id: String::new(),
            messages: Vec::new(),
            kind: String::new(),
            live: false,
            log: LogState::new(window, cx, |this, cx| {
                this.log.search_changed(cx);
                cx.notify();
            }),
        }
    }

    /// Shows a fetched exchange's messages. The same stream fetched again (it closed) keeps what arrived live
    /// while the fetch was answered; another stream replaces what was shown.
    pub fn show(&mut self, log_id: &str, exchange: &Exchange, live: bool, cx: &mut Context<Self>) {
        let mut messages = exchange.messages.clone();
        if self.log_id == log_id {
            for held in std::mem::take(&mut self.messages) {
                push_message(&mut messages, held);
            }
        } else {
            self.log.close_all();
        }
        self.log_id = log_id.to_owned();
        self.messages = messages;
        self.kind = exchange.kind.clone().unwrap_or_default();
        self.live = live;
        self.log.follow();
        cx.notify();
    }

    /// A message that arrived live, kept to the same bound as the engine's.
    pub fn push(&mut self, message: StreamMessage, cx: &mut Context<Self>) {
        push_message(&mut self.messages, message);
        let messages = &self.messages;
        self.log.forget_dropped(|id| messages.iter().any(|m| m.id == id));
        self.log.follow();
        cx.notify();
    }

    fn row(message: &StreamMessage) -> LogRow {
        let (tag, outline) = match (&message.close, &message.event) {
            (Some(code), _) => (t_with("capture.stream.close", &[("code", code)]), false),
            (None, Some(event)) => (event.clone(), event == "message"),
            (None, None) => (
                t(if message.sent { "capture.stream.sent" } else { "capture.stream.received" }).to_owned(),
                message.sent,
            ),
        };
        let note = match (&message.event_id, message.binary) {
            (Some(id), _) => Some(t_with("session.sse.event_id", &[("id", id)])),
            (None, true) => Some(t("capture.stream.binary").to_owned()),
            (None, false) => None,
        };
        LogRow {
            id: message.id.clone(),
            ts: message.ts,
            tag,
            outline,
            way: (message.event.is_none()).then_some(if message.sent { Way::Sent } else { Way::Received }),
            note,
            data: message.data.clone(),
        }
    }
}

impl Render for CapturedStream {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let needle = self.log.needle(cx);
        let rows: Vec<_> = self
            .messages
            .iter()
            .filter(|m| found(needle.as_deref(), &[&m.data, m.event.as_deref().unwrap_or_default()]))
            .map(|message| {
                let row = Self::row(message);
                let id = message.id.clone();
                log_row(
                    &row,
                    self.log.open_row(&message.id),
                    cx.listener(move |this, _, window, cx| {
                        let Some(message) = this.messages.iter().find(|m| m.id == id) else { return };
                        let data = message.data.clone();
                        this.log.toggle_open(&id, &data, window, cx);
                        cx.notify();
                    }),
                    cx,
                )
            })
            .collect();
        let count = self.messages.len();
        let state = if self.live {
            Tag::success().child(t("capture.stream.open"))
        } else {
            Tag::secondary().child(t("capture.stream.closed"))
        };
        let protocol = Tag::secondary().outline().child(if self.kind == "ws" { "WebSocket" } else { "SSE" });
        log_frame(
            LogFrame {
                ids: ["capture-stream-messages", "capture-stream-search", "capture-stream-clear"],
                lead: vec![state.into_any_element(), protocol.into_any_element()],
                summary: t_with("capture.stream.count", &[("count", &count)]),
                summary_fixed: false,
                total: count,
                empty: (
                    t("capture.stream.empty_title"),
                    if self.live { t("capture.stream.empty_live") } else { t("capture.stream.empty_closed") },
                ),
                rows,
            },
            &mut self.log,
            cx.listener(|this, _, window, cx| {
                this.log.toggle_search(window, cx);
                cx.notify();
            }),
            // Clears this view only; the engine keeps the messages, and selecting the stream again shows them.
            cx.listener(|this, _, _, cx| {
                this.messages.clear();
                this.log.close_all();
                cx.notify();
            }),
            cx,
        )
    }
}
