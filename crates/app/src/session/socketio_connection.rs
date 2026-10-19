//! A Socket.IO request's live side: connection state and the events as they go by.
//!
//! The Socket.IO counterpart of [`SocketConnection`](super::connection::SocketConnection): the request panel
//! holds the target, the Connect command and the event to send; this view holds what the connection is doing.
//! The engine owns the connection; this entity mirrors it ([`SocketIoView`]).

use gpui_kit::component::{ActiveTheme as _, tag::Tag};
use gpui_kit::{Context, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _, Window, div};
use serde_json::Value;

use crate::{
    components::stream_log::{LogFrame, LogRow, LogState, Way, found, log_frame, log_row},
    i18n::{t, t_with},
    model::{
        socket::{Direction, SocketState},
        socketio::{SocketIoMessage, SocketIoView},
    },
};

#[derive(Debug, Clone)]
pub enum SocketIoEvent {
    Clear,
}

pub struct SocketIoConnection {
    view: SocketIoView,
    /// How many events the request listens to; `None` when it hears every event.
    listening: Option<usize>,
    log: LogState,
}

impl EventEmitter<SocketIoEvent> for SocketIoConnection {}

impl SocketIoConnection {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            view: SocketIoView::default(),
            listening: Some(0),
            log: LogState::new(window, cx, |this, cx| {
                this.log.search_changed(cx);
                cx.notify();
            }),
        }
    }

    /// What the request listens to changed: `None` is every event, else how many are named.
    pub fn set_listening(&mut self, listening: Option<usize>, cx: &mut Context<Self>) {
        if self.listening != listening {
            self.listening = listening;
            cx.notify();
        }
    }

    pub fn state(&self) -> SocketState {
        self.view.state
    }

    pub fn replace_view(&mut self, view: SocketIoView, cx: &mut Context<Self>) {
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

    /// One message: the shared log row, which opens to the whole arguments.
    fn render_message(&self, message: &SocketIoMessage, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let sent = message.direction == Direction::Sent;
        let row = LogRow {
            id: message.id.clone(),
            ts: message.ts,
            tag: message.label(),
            outline: sent,
            way: Some(if sent { Way::Sent } else { Way::Received }),
            note: message.note(),
            data: message.shown(),
        };
        let id = message.id.clone();
        log_row(
            &row,
            self.log.open_row(&message.id),
            cx.listener(move |this, _, window, cx| {
                let Some(message) = this.view.messages.iter().find(|m| m.id == id) else { return };
                this.log.toggle_open(&id, &message.shown(), window, cx);
                cx.notify();
            }),
            cx,
        )
    }
}

impl Render for SocketIoConnection {
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
            .filter(|m| found(needle.as_deref(), &[&m.data, &m.event]))
            .map(|m| self.render_message(m, cx))
            .collect();
        let count = self.view.messages.len();
        // The main namespace needs no label; a server that ended the connection said why.
        let namespace = Some(self.view.namespace.clone()).filter(|ns| !ns.is_empty() && ns != "/");
        let ended = self.view.close_reason.clone().filter(|_| state == SocketState::Disconnected);
        let listening = match self.listening {
            None => t("session.socketio.listening_every").to_owned(),
            Some(0) => t("session.socketio.listening_none").to_owned(),
            Some(1) => t("session.socketio.listening_one").to_owned(),
            Some(count) => t_with("session.socketio.listening_other", &[("count", &count)]),
        };
        let nothing_heard = self.listening == Some(0);
        let mut lead = vec![state_tag.child(state.label()).into_any_element()];
        lead.extend(namespace.map(|namespace| Tag::secondary().outline().child(namespace).into_any_element()));
        lead.extend(
            self.view
                .error
                .clone()
                .map(|error| div().text_sm().text_color(theme.danger).child(error).into_any_element()),
        );
        lead.extend(
            ended.map(|reason| div().text_sm().text_color(theme.muted_foreground).child(reason).into_any_element()),
        );
        log_frame(
            LogFrame {
                ids: ["socketio-messages", "search-socketio-messages", "clear-socketio-messages"],
                lead,
                summary: if count == 1 {
                    t_with("session.socketio.count_one", &[("listening", &listening), ("count", &count)])
                } else {
                    t_with("session.socketio.count_other", &[("listening", &listening), ("count", &count)])
                },
                summary_fixed: false,
                total: count,
                empty: (
                    t("session.socketio.no_events"),
                    if nothing_heard {
                        t("session.socketio.empty_nothing_heard")
                    } else if state == SocketState::Connected {
                        t("session.socketio.empty_connected")
                    } else {
                        t("session.socketio.empty_disconnected")
                    },
                ),
                rows,
            },
            &mut self.log,
            cx.listener(|this, _, window, cx| {
                this.log.toggle_search(window, cx);
                cx.notify();
            }),
            cx.listener(|_, _, _, cx| cx.emit(SocketIoEvent::Clear)),
            cx,
        )
    }
}
