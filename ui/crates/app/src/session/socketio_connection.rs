//! A Socket.IO request's live side: connection state, the events as they go by and the composer that emits one.
//!
//! The Socket.IO counterpart of [`SocketConnection`](super::connection::SocketConnection): the request panel
//! holds the target and the Connect command, this view holds what the connection is doing. The engine owns the
//! connection; this entity mirrors it ([`SocketIoView`]) and asks the engine to act through the events it emits.
//! What is composed is an event: its name, its arguments and whether the server is to acknowledge it.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle},
    h_flex,
    input::{InputEvent, InputState, Textarea, TextareaState},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    ScrollHandle, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::Value;

use crate::{
    components::{
        VarEditor,
        stream_log::{LogRow, LogSearch, OpenRows, Way, found, log_row, nothing_found, search_button},
    },
    model::{
        socket::{Direction, SocketState},
        socketio::{SocketIoMessage, SocketIoView},
    },
};

#[derive(Debug, Clone)]
pub enum SocketIoEvent {
    /// The user asked to emit this event over the open connection: `args` is what was written for its
    /// arguments, `ack` whether the server is to acknowledge it.
    Emit { event: String, args: String, ack: bool },
    Clear,
}

pub struct SocketIoConnection {
    view: SocketIoView,
    event_name: Entity<InputState>,
    args: Entity<TextareaState>,
    ack: bool,
    scroll: ScrollHandle,
    follow_tail: bool,
    /// The messages opened to their whole arguments, by id.
    open: OpenRows,
    search: LogSearch,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SocketIoEvent> for SocketIoConnection {}

impl SocketIoConnection {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let event_name = cx.new(|cx| InputState::new(window, cx).placeholder("Event name"));
        let args = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Arguments: a JSON array is one argument per element, anything else is one argument")
                .rows(3)
        });
        let subscriptions = vec![
            cx.subscribe_in(&event_name, window, |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => this.emit_event(cx),
                _ => {}
            }),
        ];
        Self {
            view: SocketIoView::default(),
            event_name,
            args,
            ack: false,
            scroll: ScrollHandle::new(),
            follow_tail: true,
            open: OpenRows::default(),
            search: LogSearch::new(window, cx, |this, cx| {
                this.open.highlight(&this.search.query(cx), cx);
                cx.notify();
            }),
            _subscriptions: subscriptions,
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

    fn can_emit(&self, cx: &gpui_kit::App) -> bool {
        self.view.state == SocketState::Connected && !self.event_name.read(cx).value().trim().is_empty()
    }

    fn emit_event(&mut self, cx: &mut Context<Self>) {
        if self.can_emit(cx) {
            cx.emit(SocketIoEvent::Emit {
                event: self.event_name.read(cx).value().trim().to_owned(),
                args: self.args.read(cx).value().to_string(),
                ack: self.ack,
            });
        }
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
        let connected = state == SocketState::Connected;
        let can_emit = self.can_emit(cx);
        // The main namespace needs no label; a server that ended the connection said why.
        let namespace = Some(self.view.namespace.clone()).filter(|ns| !ns.is_empty() && ns != "/");
        let ended = self.view.close_reason.clone().filter(|_| state == SocketState::Disconnected);

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
                    .child(div().text_sm().text_color(theme.muted_foreground).child(format!(
                        "{} message{} · times in UTC",
                        self.view.messages.len(),
                        if self.view.messages.len() == 1 { "" } else { "s" }
                    )))
                    .child(search_button("search-socketio-messages").on_click(cx.listener(|this, _, window, cx| {
                        this.search.toggle(window, cx);
                        cx.notify();
                    })))
                    .child(
                        Button::new("clear-socketio-messages")
                            .ghost()
                            .small()
                            .label("Clear")
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
                            EmptyHeader::new().title(EmptyTitle::new().child("No events")).description(
                                EmptyDescription::new().child(if connected {
                                    "Events emitted and received appear here."
                                } else {
                                    "Connect to start emitting and receiving events."
                                }),
                            ),
                        ))
                    })
                    .when(none_found, |this| this.child(nothing_found()))
                    .children(shown.into_iter().map(|m| self.render_message(m, cx))),
            )
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(VarEditor::row_input(&self.event_name, cx).disabled(!connected)),
                            )
                            .child(
                                Checkbox::new("socketio-ack")
                                    .checked(self.ack)
                                    .small()
                                    .label("Ask for an acknowledgement")
                                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                        this.ack = *checked;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("emit-socketio-event")
                                    .primary()
                                    .h(VarEditor::row_height(cx))
                                    .label("Emit")
                                    .disabled(!can_emit)
                                    .on_click(cx.listener(|this, _, _, cx| this.emit_event(cx))),
                            ),
                    )
                    .child(Textarea::new(&self.args).disabled(!connected)),
            )
    }
}
