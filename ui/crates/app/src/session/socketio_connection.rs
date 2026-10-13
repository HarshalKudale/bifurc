//! A Socket.IO request's live side: connection state and the events as they go by.
//!
//! The Socket.IO counterpart of [`SocketConnection`](super::connection::SocketConnection): the request panel
//! holds the target, the Connect command and the event to send; this view holds what the connection is doing.
//! The engine owns the connection; this entity mirrors it ([`SocketIoView`]).

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle},
    h_flex,
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, Window, div,
};
use serde_json::Value;

use crate::{
    i18n::{t, t_with},
    components::stream_log::{LogRow, LogSearch, OpenRows, Way, found, log_row, nothing_found, search_button},
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
    scroll: ScrollHandle,
    follow_tail: bool,
    /// The messages opened to their whole arguments, by id.
    open: OpenRows,
    search: LogSearch,
}

impl EventEmitter<SocketIoEvent> for SocketIoConnection {}

impl SocketIoConnection {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            view: SocketIoView::default(),
            listening: Some(0),
            scroll: ScrollHandle::new(),
            follow_tail: true,
            open: OpenRows::default(),
            search: LogSearch::new(window, cx, |this, cx| {
                this.open.highlight(&this.search.query(cx), cx);
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
        self.follow_tail = true;
        cx.notify();
    }

    pub fn apply_status_event(&mut self, payload: &Value, cx: &mut Context<Self>) {
        self.view.apply_status_event(payload);
        cx.notify();
    }

    pub fn apply_message_event(&mut self, payload: &Value, cx: &mut Context<Self>) {
        self.view.apply_message_event(payload);
        self.forget_dropped();
        self.follow_tail = true;
        cx.notify();
    }

    pub fn clear_messages(&mut self, cx: &mut Context<Self>) {
        self.view.clear_messages();
        self.open.clear();
        cx.notify();
    }

    /// The log is bounded, so an open message can leave it.
    fn forget_dropped(&mut self) {
        if !self.open.is_empty() {
            let messages = &self.view.messages;
            self.open.retain(|id| messages.iter().any(|m| m.id == id));
        }
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
            self.open.get(&message.id),
            cx.listener(move |this, _, window, cx| {
                let Some(message) = this.view.messages.iter().find(|m| m.id == id) else { return };
                let query = this.search.query(cx);
                this.open.toggle(&id, &message.shown(), &query, window, cx);
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
        if self.follow_tail {
            self.follow_tail = false;
            self.scroll.scroll_to_bottom();
        }
        let needle = self.search.needle(cx);
        let shown: Vec<&SocketIoMessage> =
            self.view.messages.iter().filter(|m| found(needle.as_deref(), &[&m.data, &m.event])).collect();
        let search_bar = self.search.bar(shown.len(), self.view.messages.len(), cx);
        let none_found = shown.is_empty() && !self.view.messages.is_empty();
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

        v_flex()
            .size_full()
            .bg(theme.background)
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .child(state_tag.child(state.label()))
                    .when_some(namespace, |this, namespace| {
                        this.child(Tag::secondary().outline().child(namespace))
                    })
                    .when_some(self.view.error.clone(), |this, error| {
                        this.child(div().text_sm().text_color(theme.danger).child(error))
                    })
                    .when_some(ended, |this, reason| {
                        this.child(div().text_sm().text_color(theme.muted_foreground).child(reason))
                    })
                    .child(div().flex_1())
                    .child(div().text_sm().text_color(theme.muted_foreground).child(if self.view.messages.len() == 1 {
                        t_with("session.socketio.count_one", &[("listening", &listening), ("count", &self.view.messages.len())])
                    } else {
                        t_with("session.socketio.count_other", &[("listening", &listening), ("count", &self.view.messages.len())])
                    }))
                    .child(search_button("search-socketio-messages").on_click(cx.listener(|this, _, window, cx| {
                        this.search.toggle(window, cx);
                        cx.notify();
                    })))
                    .child(
                        Button::new("clear-socketio-messages")
                            .ghost()
                            .small()
                            .label(t("session.connection.clear"))
                            .disabled(self.view.messages.is_empty())
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SocketIoEvent::Clear))),
                    ),
            )
            .children(search_bar)
            .child(
                div()
                    .id("socketio-messages")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .border_t_1()
                    .border_color(theme.border)
                    .when(self.view.messages.is_empty(), |this| {
                        this.child(Empty::new().header(
                            EmptyHeader::new().title(EmptyTitle::new().child(t("session.socketio.no_events"))).description(
                                EmptyDescription::new().child(if nothing_heard {
                                    t("session.socketio.empty_nothing_heard")
                                } else if state == SocketState::Connected {
                                    t("session.socketio.empty_connected")
                                } else {
                                    t("session.socketio.empty_disconnected")
                                }),
                            ),
                        ))
                    })
                    .when(none_found, |this| this.child(nothing_found()))
                    .children(shown.into_iter().map(|m| self.render_message(m, cx))),
            )
    }
}
