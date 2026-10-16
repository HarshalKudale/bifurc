//! An SSE request's live side: stream state and the events as they arrive.
//!
//! The SSE counterpart of [`SocketConnection`](super::connection::SocketConnection), without a composer: an
//! event stream only sends one way. The engine owns the stream; this entity mirrors it ([`StreamView`]).

use gpui_kit::component::{ActiveTheme as _, tag::Tag};
use gpui_kit::{
    Context, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _, Window, div,
};
use serde_json::Value;

use crate::{
    i18n::{t, t_with},
    components::stream_log::{LogFrame, LogRow, LogState, found, log_frame, log_row},
    model::stream::{StreamMessage, StreamState, StreamView},
};

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Clear,
}

pub struct SseStream {
    view: StreamView,
    log: LogState,
}

impl EventEmitter<StreamEvent> for SseStream {}

impl SseStream {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            view: StreamView::default(),
            log: LogState::new(window, cx, |this, cx| {
                this.log.search_changed(cx);
                cx.notify();
            }),
        }
    }

    pub fn state(&self) -> StreamState {
        self.view.state
    }

    pub fn replace_view(&mut self, view: StreamView, cx: &mut Context<Self>) {
        self.view = view;
        self.forget_dropped();
        self.log.follow();
        cx.notify();
    }

    pub fn apply_status_event(&mut self, payload: &Value, cx: &mut Context<Self>) {
        self.view.apply_status_event(payload);
        cx.notify();
    }

    pub fn apply_message_event(&mut self, payload: &Value, cx: &mut Context<Self>) {
        self.view.apply_message_event(payload);
        self.forget_dropped();
        self.log.follow();
        cx.notify();
    }

    pub fn clear_messages(&mut self, cx: &mut Context<Self>) {
        self.view.clear_messages();
        self.log.close_all();
        cx.notify();
    }

    /// The log is bounded, so an open event can leave it.
    fn forget_dropped(&mut self) {
        let messages = &self.view.messages;
        self.log.forget_dropped(|id| messages.iter().any(|m| m.id == id));
    }

    /// One event: the shared log row, which opens to the whole event.
    fn render_message(&self, message: &StreamMessage, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let row = LogRow {
            id: message.id.clone(),
            ts: message.ts,
            tag: message.event.clone(),
            outline: !message.is_named(),
            way: None,
            note: message.event_id.as_ref().map(|id| t_with("session.sse.event_id", &[("id", id)])),
            data: message.data.clone(),
        };
        let id = message.id.clone();
        log_row(
            &row,
            self.log.open_row(&message.id),
            cx.listener(move |this, _, window, cx| {
                let Some(message) = this.view.messages.iter().find(|m| m.id == id) else { return };
                this.log.toggle_open(&id, &message.data, window, cx);
                cx.notify();
            }),
            cx,
        )
    }
}

impl Render for SseStream {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let state = self.view.state;
        let state_tag = match state {
            StreamState::Connected => Tag::success(),
            StreamState::Connecting => Tag::warning(),
            StreamState::Error => Tag::danger(),
            StreamState::Disconnected => Tag::secondary(),
        };
        let needle = self.log.needle(cx);
        let rows: Vec<_> = self
            .view
            .messages
            .iter()
            .filter(|m| found(needle.as_deref(), &[&m.data, &m.event, m.event_id.as_deref().unwrap_or_default()]))
            .map(|m| self.render_message(m, cx))
            .collect();
        let count = self.view.messages.len();
        let mut lead = vec![state_tag.child(state.label()).into_any_element()];
        lead.extend(
            self.view
                .http_status
                .map(|status| Tag::secondary().outline().child(format!("HTTP {status}")).into_any_element()),
        );
        lead.extend(
            self.view.error.clone().map(|error| div().text_sm().text_color(theme.danger).child(error).into_any_element()),
        );
        log_frame(
            LogFrame {
                ids: ["sse-events", "search-events", "clear-events"],
                lead,
                summary: if count == 1 {
                    t_with("session.sse.count_one", &[("count", &count)])
                } else {
                    t_with("session.sse.count_other", &[("count", &count)])
                },
                summary_fixed: false,
                total: count,
                empty: (
                    t("session.sse.no_events"),
                    if state.is_live() { t("session.sse.empty_live") } else { t("session.sse.empty_idle") },
                ),
                rows,
            },
            &mut self.log,
            cx.listener(|this, _, window, cx| {
                this.log.toggle_search(window, cx);
                cx.notify();
            }),
            cx.listener(|_, _, _, cx| cx.emit(StreamEvent::Clear)),
            cx,
        )
    }
}
