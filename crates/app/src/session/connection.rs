//! A WebSocket request's live side: connection state and the message stream.
//!
//! This is the WebSocket counterpart of [`Exchange`](super::exchange::Exchange): the request panel holds the
//! target, the Connect command and the message to send; this view holds what the connection is doing. The engine owns the socket;
//! this entity mirrors it ([`SocketView`]) and asks the engine to act through the events it emits.

use gpui_kit::component::{ActiveTheme as _, tag::Tag};
use gpui_kit::{Context, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _, Window, div};
use serde_json::Value;

use crate::{
    components::stream_log::{LogFrame, LogRow, LogState, Way, found, log_frame, log_row},
    i18n::{t, t_with},
    model::socket::{Direction, SocketMessage, SocketState, SocketView},
};

#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    Clear,
}

pub struct SocketConnection {
    view: SocketView,
    log: LogState,
}

impl EventEmitter<ConnectionEvent> for SocketConnection {}

impl SocketConnection {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            view: SocketView::default(),
            log: LogState::new(window, cx, |this, cx| {
                this.log.search_changed(cx);
                cx.notify();
            }),
        }
    }

    pub fn state(&self) -> SocketState {
        self.view.state
    }

    pub fn replace_view(&mut self, view: SocketView, cx: &mut Context<Self>) {
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

    /// The log is bounded, so an open message can leave it.
    fn forget_dropped(&mut self) {
        let messages = &self.view.messages;
        self.log.forget_dropped(|id| messages.iter().any(|m| m.id == id));
    }

    /// One message: the shared log row, which opens to the whole message.
    fn render_message(&self, message: &SocketMessage, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let sent = message.direction == Direction::Sent;
        let row = LogRow {
            id: message.id.clone(),
            ts: message.ts,
            tag: if sent { t("session.log.sent") } else { t("session.log.received") }.to_owned(),
            outline: sent,
            way: Some(if sent { Way::Sent } else { Way::Received }),
            note: None,
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

impl Render for SocketConnection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let state = self.view.state;
        let state_tag = match state {
            SocketState::Connected => Tag::success(),
            SocketState::Connecting => Tag::warning(),
            SocketState::Error => Tag::danger(),
            SocketState::Disconnected => Tag::secondary(),
        };
        let needle = self.log.needle(cx);
        let rows: Vec<_> = self
            .view
            .messages
            .iter()
            .filter(|m| found(needle.as_deref(), &[&m.data]))
            .map(|m| self.render_message(m, cx))
            .collect();
        let count = self.view.messages.len();
        let mut lead = vec![state_tag.child(state.label()).into_any_element()];
        lead.extend(
            self.view
                .error
                .clone()
                .map(|error| div().text_sm().text_color(theme.danger).child(error).into_any_element()),
        );
        log_frame(
            LogFrame {
                ids: ["socket-messages", "search-messages", "clear-messages"],
                lead,
                summary: if count == 1 {
                    t_with("session.log.count_one", &[("count", &count)])
                } else {
                    t_with("session.log.count_other", &[("count", &count)])
                },
                summary_fixed: false,
                total: count,
                empty: (
                    t("session.log.no_messages"),
                    if state == SocketState::Connected {
                        t("session.connection.empty_connected")
                    } else {
                        t("session.connection.empty_disconnected")
                    },
                ),
                rows,
            },
            &mut self.log,
            cx.listener(|this, _, window, cx| {
                this.log.toggle_search(window, cx);
                cx.notify();
            }),
            cx.listener(|_, _, _, cx| cx.emit(ConnectionEvent::Clear)),
            cx,
        )
    }
}
