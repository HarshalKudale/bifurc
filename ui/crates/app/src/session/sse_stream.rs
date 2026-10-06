//! An SSE request's live side: stream state and the events as they arrive.
//!
//! The SSE counterpart of [`SocketConnection`](super::connection::SocketConnection), without a composer: an
//! event stream only sends one way. The engine owns the stream; this entity mirrors it ([`StreamView`]).

use std::collections::HashSet;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle},
    h_flex, v_flex,
    tag::Tag,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, Window, div,
};
use serde_json::Value;

use crate::{
    components::stream_log::{LogRow, log_row},
    model::stream::{StreamMessage, StreamState, StreamView},
};

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Clear,
}

pub struct SseStream {
    view: StreamView,
    scroll: ScrollHandle,
    follow_tail: bool,
    /// The events opened to their whole data, by id.
    open: HashSet<String>,
}

impl EventEmitter<StreamEvent> for SseStream {}

impl Default for SseStream {
    fn default() -> Self {
        Self { view: StreamView::default(), scroll: ScrollHandle::new(), follow_tail: true, open: HashSet::new() }
    }
}

impl SseStream {
    pub fn state(&self) -> StreamState {
        self.view.state
    }

    pub fn replace_view(&mut self, view: StreamView, cx: &mut Context<Self>) {
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
        self.open.clear();
        cx.notify();
    }

    /// One event: the shared log row, which opens to the whole event.
    fn render_message(&self, message: &StreamMessage, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let row = LogRow {
            id: message.id.clone(),
            ts: message.ts,
            tag: message.event.clone(),
            outline: !message.is_named(),
            way: None,
            note: message.event_id.as_ref().map(|id| format!("id {id}")),
            data: message.data.clone(),
        };
        let id = message.id.clone();
        log_row(
            &row,
            self.open.contains(&message.id),
            cx.listener(move |this, _, _, cx| {
                if !this.open.remove(&id) {
                    this.open.insert(id.clone());
                }
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
        if self.follow_tail {
            self.follow_tail = false;
            self.scroll.scroll_to_bottom();
        }

        v_flex()
            .size_full()
            .bg(theme.background)
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .child(state_tag.child(state.label()))
                    .when_some(self.view.http_status, |this, status| {
                        this.child(Tag::secondary().outline().child(format!("HTTP {status}")))
                    })
                    .when_some(self.view.error.clone(), |this, error| {
                        this.child(div().text_sm().text_color(theme.danger).child(error))
                    })
                    .child(div().flex_1())
                    .child(div().text_sm().text_color(theme.muted_foreground).child(format!(
                        "{} event{} · times in UTC",
                        self.view.messages.len(),
                        if self.view.messages.len() == 1 { "" } else { "s" }
                    )))
                    .child(
                        Button::new("clear-events")
                            .ghost()
                            .small()
                            .label("Clear")
                            .disabled(self.view.messages.is_empty())
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(StreamEvent::Clear))),
                    ),
            )
            .child(
                div()
                    .id("sse-events")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .border_t_1()
                    .border_color(theme.border)
                    .when(self.view.messages.is_empty(), |this| {
                        this.child(Empty::new().header(
                            EmptyHeader::new().title(EmptyTitle::new().child("No events")).description(
                                EmptyDescription::new().child(if state.is_live() {
                                    "Events the server sends appear here."
                                } else {
                                    "Connect to start listening."
                                }),
                            ),
                        ))
                    })
                    .children(self.view.messages.iter().map(|m| self.render_message(m, cx))),
            )
    }
}
