//! An MCP request's live side: the connection's state, the server it reached, and the log of what was asked of
//! it and what it answered.
//!
//! The MCP counterpart of [`SocketConnection`](super::connection::SocketConnection): the request panel holds
//! the target, the Connect command and what to ask for; this view holds what the connection is doing. Each
//! call is one row of the log, which opens to the answer. The engine owns the connection; this entity mirrors
//! it ([`McpView`]), which is also where the request panel reads what the server declares.

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
        mcp::{CallStatus, McpLine, McpView},
        socket::SocketState,
    },
};

#[derive(Debug, Clone)]
pub enum McpEvent {
    Clear,
}

pub struct McpConnection {
    view: McpView,
    scroll: ScrollHandle,
    follow_tail: bool,
    /// The lines opened to their answers, by id.
    open: OpenRows,
    search: LogSearch,
}

impl EventEmitter<McpEvent> for McpConnection {}

impl McpConnection {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            view: McpView::default(),
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

    pub fn view(&self) -> &McpView {
        &self.view
    }

    pub fn replace_view(&mut self, view: McpView, cx: &mut Context<Self>) {
        self.view = view;
        self.forget_dropped();
        self.follow_tail = true;
        cx.notify();
    }

    pub fn apply_status_event(&mut self, payload: &Value, cx: &mut Context<Self>) {
        self.view.apply_status_event(payload);
        cx.notify();
    }

    pub fn apply_message_event(&mut self, payload: &Value, window: &mut Window, cx: &mut Context<Self>) {
        let before = self.view.lines.len();
        self.view.apply_message_event(payload);
        self.forget_dropped();
        // A new line is scrolled to; an answer changes a line that is already in view.
        let answered = payload.get("message").and_then(|line| line.get("id")).and_then(Value::as_str);
        if self.view.lines.len() != before {
            self.follow_tail = true;
        } else if let Some(line) = answered
            .and_then(|id| self.view.lines.iter().find(|line| line.id == id))
            .filter(|line| line.kind.is_call() && line.status != CallStatus::Pending)
        {
            // The answer of a call that was open while it waited replaces what the row showed, and a call
            // just answered opens to it: the answer is what was asked for.
            let (id, shown) = (line.id.clone(), line.shown());
            let query = self.search.query(cx);
            if self.open.get(&id).is_some() {
                self.open.toggle(&id, &shown, &query, window, cx);
            }
            self.open.toggle(&id, &shown, &query, window, cx);
        }
        cx.notify();
    }

    pub fn clear_lines(&mut self, cx: &mut Context<Self>) {
        self.view.clear_lines();
        self.open.clear();
        cx.notify();
    }

    /// The log is bounded, so an open line can leave it.
    fn forget_dropped(&mut self) {
        if !self.open.is_empty() {
            let lines = &self.view.lines;
            self.open.retain(|id| lines.iter().any(|line| line.id == id));
        }
    }

    /// One line: the shared log row, which opens to the answer.
    fn render_line(&self, line: &McpLine, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let call = line.kind.is_call();
        let row = LogRow {
            id: line.id.clone(),
            ts: line.ts,
            tag: line.kind.label().to_owned(),
            outline: !call,
            way: Some(if call { Way::Sent } else { Way::Received }),
            note: line.note(),
            data: line.summary(),
        };
        let id = line.id.clone();
        log_row(
            &row,
            self.open.get(&line.id),
            cx.listener(move |this, _, window, cx| {
                let Some(line) = this.view.lines.iter().find(|line| line.id == id) else { return };
                let query = this.search.query(cx);
                this.open.toggle(&id, &line.shown(), &query, window, cx);
                cx.notify();
            }),
            cx,
        )
    }
}

impl Render for McpConnection {
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
        let shown: Vec<&McpLine> = self
            .view
            .lines
            .iter()
            .filter(|line| found(needle.as_deref(), &[&line.name, &line.arguments, &line.text]))
            .collect();
        let search_bar = self.search.bar(shown.len(), self.view.lines.len(), cx);
        let none_found = shown.is_empty() && !self.view.lines.is_empty();
        let server = Some(self.view.server.clone()).filter(|server| !server.is_empty());
        let failed = self.view.lines.iter().filter(|line| line.status == CallStatus::Error).count();
        let mut counts = vec![if self.view.lines.len() == 1 {
            t_with("session.mcp.lines_one", &[("count", &self.view.lines.len())])
        } else {
            t_with("session.mcp.lines_other", &[("count", &self.view.lines.len())])
        }];
        if self.view.waiting() > 0 {
            counts.push(t_with("session.mcp.waiting", &[("count", &self.view.waiting())]));
        }
        if failed > 0 {
            counts.push(t_with("session.mcp.failed", &[("count", &failed)]));
        }
        counts.push(t("session.log.times_utc").to_owned());
        let counts = counts.join(" · ");

        v_flex()
            .size_full()
            .bg(theme.background)
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .child(state_tag.child(state.label()))
                    .when_some(server, |this, server| this.child(Tag::secondary().outline().child(server)))
                    .when_some(self.view.error.clone(), |this, error| {
                        this.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_sm()
                                .text_color(theme.danger)
                                .child(error),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex_none()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(counts),
                    )
                    .child(search_button("search-mcp-lines").on_click(cx.listener(|this, _, window, cx| {
                        this.search.toggle(window, cx);
                        cx.notify();
                    })))
                    .child(
                        Button::new("clear-mcp-lines")
                            .ghost()
                            .small()
                            .label(t("session.connection.clear"))
                            .disabled(self.view.lines.is_empty())
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(McpEvent::Clear))),
                    ),
            )
            .children(search_bar)
            .child(
                div()
                    .id("mcp-lines")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .border_t_1()
                    .border_color(theme.border)
                    .when(self.view.lines.is_empty(), |this| {
                        this.child(Empty::new().header(
                            EmptyHeader::new().title(EmptyTitle::new().child(t("session.mcp.nothing_asked"))).description(
                                EmptyDescription::new().child(match state {
                                    SocketState::Connected => t("session.mcp.empty_connected"),
                                    SocketState::Connecting => t("session.mcp.empty_connecting"),
                                    _ => t("session.mcp.empty_disconnected"),
                                }),
                            ),
                        ))
                    })
                    .when(none_found, |this| this.child(nothing_found()))
                    .children(shown.into_iter().map(|line| self.render_line(line, cx))),
            )
    }
}
