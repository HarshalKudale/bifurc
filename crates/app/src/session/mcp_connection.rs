//! An MCP request's live side: the connection's state, the server it reached, and the log of what was asked of
//! it and what it answered.
//!
//! The MCP counterpart of [`SocketConnection`](super::connection::SocketConnection): the request panel holds
//! the target, the Connect command and what to ask for; this view holds what the connection is doing. Each
//! call is one row of the log, which opens to the answer. The engine owns the connection; this entity mirrors
//! it ([`McpView`]), which is also where the request panel reads what the server declares.

use gpui_kit::component::{ActiveTheme as _, tag::Tag};
use gpui_kit::{Context, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _, Window, div};
use serde_json::Value;

use crate::{
    components::stream_log::{LogFrame, LogRow, LogState, Way, found, log_frame, log_row},
    i18n::{t, t_with},
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
    log: LogState,
}

impl EventEmitter<McpEvent> for McpConnection {}

impl McpConnection {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            view: McpView::default(),
            log: LogState::new(window, cx, |this, cx| {
                this.log.search_changed(cx);
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
        self.log.follow();
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
            self.log.follow();
        } else if let Some(line) = answered
            .and_then(|id| self.view.lines.iter().find(|line| line.id == id))
            .filter(|line| line.kind.is_call() && line.status != CallStatus::Pending)
        {
            // The answer of a call that was open while it waited replaces what the row showed, and a call
            // just answered opens to it: the answer is what was asked for.
            let (id, shown) = (line.id.clone(), line.shown());
            if self.log.open_row(&id).is_some() {
                self.log.toggle_open(&id, &shown, window, cx);
            }
            self.log.toggle_open(&id, &shown, window, cx);
        }
        cx.notify();
    }

    pub fn clear_lines(&mut self, cx: &mut Context<Self>) {
        self.view.clear_lines();
        self.log.close_all();
        cx.notify();
    }

    /// The log is bounded, so an open line can leave it.
    fn forget_dropped(&mut self) {
        let lines = &self.view.lines;
        self.log.forget_dropped(|id| lines.iter().any(|line| line.id == id));
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
            self.log.open_row(&line.id),
            cx.listener(move |this, _, window, cx| {
                let Some(line) = this.view.lines.iter().find(|line| line.id == id) else { return };
                this.log.toggle_open(&id, &line.shown(), window, cx);
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
        let needle = self.log.needle(cx);
        let rows: Vec<_> = self
            .view
            .lines
            .iter()
            .filter(|line| found(needle.as_deref(), &[&line.name, &line.arguments, &line.text]))
            .map(|line| self.render_line(line, cx))
            .collect();
        let total = self.view.lines.len();
        let server = Some(self.view.server.clone()).filter(|server| !server.is_empty());
        let failed = self.view.lines.iter().filter(|line| line.status == CallStatus::Error).count();
        let mut counts = vec![if total == 1 {
            t_with("session.mcp.lines_one", &[("count", &total)])
        } else {
            t_with("session.mcp.lines_other", &[("count", &total)])
        }];
        if self.view.waiting() > 0 {
            counts.push(t_with("session.mcp.waiting", &[("count", &self.view.waiting())]));
        }
        if failed > 0 {
            counts.push(t_with("session.mcp.failed", &[("count", &failed)]));
        }
        counts.push(t("session.log.times_utc").to_owned());
        let mut lead = vec![state_tag.child(state.label()).into_any_element()];
        lead.extend(server.map(|server| Tag::secondary().outline().child(server).into_any_element()));
        lead.extend(self.view.error.clone().map(|error| {
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_sm()
                .text_color(theme.danger)
                .child(error)
                .into_any_element()
        }));
        log_frame(
            LogFrame {
                ids: ["mcp-lines", "search-mcp-lines", "clear-mcp-lines"],
                lead,
                summary: counts.join(" · "),
                summary_fixed: true,
                total,
                empty: (
                    t("session.mcp.nothing_asked"),
                    match state {
                        SocketState::Connected => t("session.mcp.empty_connected"),
                        SocketState::Connecting => t("session.mcp.empty_connecting"),
                        _ => t("session.mcp.empty_disconnected"),
                    },
                ),
                rows,
            },
            &mut self.log,
            cx.listener(|this, _, window, cx| {
                this.log.toggle_search(window, cx);
                cx.notify();
            }),
            cx.listener(|_, _, _, cx| cx.emit(McpEvent::Clear)),
            cx,
        )
    }
}
