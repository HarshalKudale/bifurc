//! A WebSocket request's live side: connection state and the message stream.
//!
//! This is the WebSocket counterpart of [`Exchange`](super::exchange::Exchange): the request panel holds the
//! target, the Connect command and the message to send; this view holds what the connection is doing. The engine owns the socket;
//! this entity mirrors it ([`SocketView`]) and asks the engine to act through the events it emits.

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
    model::socket::{Direction, SocketMessage, SocketState, SocketView},
};

#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    Clear,
}

pub struct SocketConnection {
    view: SocketView,
    scroll: ScrollHandle,
    follow_tail: bool,
    /// The messages opened to their whole text, by id.
    open: OpenRows,
    search: LogSearch,
}

impl EventEmitter<ConnectionEvent> for SocketConnection {}

impl SocketConnection {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            view: SocketView::default(),
            scroll: ScrollHandle::new(),
            follow_tail: true,
            open: OpenRows::default(),
            search: LogSearch::new(window, cx, |this, cx| {
                this.open.highlight(&this.search.query(cx), cx);
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
            self.open.get(&message.id),
            cx.listener(move |this, _, window, cx| {
                let Some(message) = this.view.messages.iter().find(|m| m.id == id) else { return };
                let query = this.search.query(cx);
                this.open.toggle(&id, &message.data, &query, window, cx);
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
        if self.follow_tail {
            self.follow_tail = false;
            self.scroll.scroll_to_bottom();
        }
        let needle = self.search.needle(cx);
        let shown: Vec<&SocketMessage> =
            self.view.messages.iter().filter(|m| found(needle.as_deref(), &[&m.data])).collect();
        let search_bar = self.search.bar(shown.len(), self.view.messages.len(), cx);
        let none_found = shown.is_empty() && !self.view.messages.is_empty();

        v_flex()
            .size_full()
            .bg(theme.background)
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .child(state_tag.child(state.label()))
                    .when_some(self.view.error.clone(), |this, error| {
                        this.child(div().text_sm().text_color(theme.danger).child(error))
                    })
                    .child(div().flex_1())
                    .child(div().text_sm().text_color(theme.muted_foreground).child(if self.view.messages.len() == 1 {
                        t_with("session.log.count_one", &[("count", &self.view.messages.len())])
                    } else {
                        t_with("session.log.count_other", &[("count", &self.view.messages.len())])
                    }))
                    .child(search_button("search-messages").on_click(cx.listener(|this, _, window, cx| {
                        this.search.toggle(window, cx);
                        cx.notify();
                    })))
                    .child(
                        Button::new("clear-messages")
                            .ghost()
                            .small()
                            .label(t("session.connection.clear"))
                            .disabled(self.view.messages.is_empty())
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectionEvent::Clear))),
                    ),
            )
            .children(search_bar)
            .child(
                div()
                    .id("socket-messages")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .border_t_1()
                    .border_color(theme.border)
                    .when(self.view.messages.is_empty(), |this| {
                        this.child(Empty::new().header(
                            EmptyHeader::new().title(EmptyTitle::new().child(t("session.log.no_messages"))).description(
                                EmptyDescription::new().child(if state == SocketState::Connected {
                                    t("session.connection.empty_connected")
                                } else {
                                    t("session.connection.empty_disconnected")
                                }),
                            ),
                        ))
                    })
                    .when(none_found, |this| this.child(nothing_found()))
                    .children(shown.into_iter().map(|m| self.render_message(m, cx))),
            )
    }
}
