//! A WebSocket request's live side: connection state, the message stream and the composer.
//!
//! This is the WebSocket counterpart of [`Exchange`](super::exchange::Exchange): the request panel holds the
//! target and the Connect command, this view holds what the connection is doing. The engine owns the socket;
//! this entity mirrors it ([`SocketView`]) and asks the engine to act through the events it emits.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle},
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    ScrollHandle, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::Value;

use crate::model::{
    socket::{Direction, SocketMessage, SocketState, SocketView},
    traffic::clock_utc,
};

#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    /// The user asked to send this text over the open connection.
    Send(String),
    Clear,
}

pub struct SocketConnection {
    view: SocketView,
    composer: Entity<TextareaState>,
    scroll: ScrollHandle,
    follow_tail: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ConnectionEvent> for SocketConnection {}

impl SocketConnection {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| TextareaState::new(window, cx).placeholder("Message to send").rows(3));
        let subscription = cx.subscribe_in(&composer, window, |_, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            view: SocketView::default(),
            composer,
            scroll: ScrollHandle::new(),
            follow_tail: true,
            _subscriptions: vec![subscription],
        }
    }

    pub fn state(&self) -> SocketState {
        self.view.state
    }

    pub fn replace_view(&mut self, view: SocketView, cx: &mut Context<Self>) {
        self.view = view;
        self.follow_tail = true;
        cx.notify();
    }

    pub fn apply_status_event(&mut self, payload: &Value, cx: &mut Context<Self>) {
        self.view.apply_status_event(payload);
        cx.notify();
    }

    pub fn apply_message_event(&mut self, payload: &Value, cx: &mut Context<Self>) {
        self.view.apply_message_event(payload);
        self.follow_tail = true;
        cx.notify();
    }

    pub fn clear_messages(&mut self, cx: &mut Context<Self>) {
        self.view.clear_messages();
        cx.notify();
    }

    pub fn clear_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.update(cx, |state, cx| state.set_value("", window, cx));
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().to_string();
        if self.view.state == SocketState::Connected && !text.is_empty() {
            cx.emit(ConnectionEvent::Send(text));
        }
    }

    fn render_message(message: &SocketMessage, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let (tag, label) = match message.direction {
            Direction::Sent => (Tag::secondary().outline(), "Sent"),
            Direction::Received => (Tag::secondary(), "Received"),
        };
        h_flex()
            .id(gpui_kit::ElementId::Name(message.id.clone().into()))
            .gap_3()
            .px_3()
            .py_1()
            .items_start()
            .child(div().w_20().flex_none().text_sm().text_color(theme.muted_foreground).child(clock_utc(message.ts)))
            .child(div().w_20().flex_none().child(tag.child(label)))
            .child(div().flex_1().min_w_0().child(message.data.clone()))
            .into_any_element()
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
        let can_send = state == SocketState::Connected && !self.composer.read(cx).value().is_empty();

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
                    .child(div().text_sm().text_color(theme.muted_foreground).child(format!(
                        "{} message{} · times in UTC",
                        self.view.messages.len(),
                        if self.view.messages.len() == 1 { "" } else { "s" }
                    )))
                    .child(
                        Button::new("clear-messages")
                            .ghost()
                            .small()
                            .label("Clear")
                            .disabled(self.view.messages.is_empty())
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectionEvent::Clear))),
                    ),
            )
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
                            EmptyHeader::new().title(EmptyTitle::new().child("No messages")).description(
                                EmptyDescription::new().child(if state == SocketState::Connected {
                                    "Messages sent and received appear here."
                                } else {
                                    "Connect to start the stream."
                                }),
                            ),
                        ))
                    })
                    .children(self.view.messages.iter().map(|m| Self::render_message(m, cx))),
            )
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(Textarea::new(&self.composer).disabled(state != SocketState::Connected))
                    .child(
                        h_flex().justify_end().child(
                            Button::new("send-message")
                                .primary()
                                .label("Send")
                                .disabled(!can_send)
                                .on_click(cx.listener(|this, _, _, cx| this.send(cx))),
                        ),
                    ),
            )
    }
}
