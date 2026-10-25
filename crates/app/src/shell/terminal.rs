//! The terminal dock: a panel under the screens with a tab for each program whose output is followed.
//!
//! A dev server's tab opens when the app starts it (or when its row is clicked); the other tabs are opened by
//! hand for a command of the user's, or for the shell. The engine runs every program (`devservice.*`,
//! `terminal.*`) and keeps its lines; the dock holds the lines each tab has been sent and shows the tab in front.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, ClipboardItem, Context, ElementId, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, ScrollWheelEvent, StatefulInteractiveElement as _, Styled as _,
    Subscription, TestSupportExt as _, UniformListScrollHandle, Window, div, px, uniform_list,
};
use serde_json::{Value, json};
use std::ops::Range;

use crate::{
    app_model::{AppModel, ModelEvent},
    components::{
        dialogs::{PromptField, prompt},
        page,
    },
    engine::{call, failure},
    i18n::{t, t_with},
    model::{
        logs::{self, LogLine, RunStatus, Shown},
        traffic::clock_utc,
    },
};

/// How many lines of a program's output are read when its tab is opened.
const FIRST_LINES: u32 = 1000;

/// What the dock asks the window for.
pub enum DockEvent {
    /// The dock's own close button was pressed.
    Hide,
}

#[derive(Debug, Clone, PartialEq)]
enum Source {
    /// A dev server of the workspace, by name.
    Dev(String),
    /// A program opened by hand, by the engine's id for it.
    Terminal(String),
}

impl Source {
    /// What the tab's element ids are made of.
    fn key(&self) -> String {
        match self {
            Self::Dev(name) => format!("dev-{name}"),
            Self::Terminal(id) => format!("term-{id}"),
        }
    }
}

struct Tab {
    source: Source,
    title: String,
    status: RunStatus,
    lines: Vec<LogLine>,
    /// The first read of the program's output has come back; output that arrives before it is in that read.
    loaded: bool,
}

pub struct TerminalDock {
    model: Entity<AppModel>,
    tabs: Vec<Tab>,
    active: Option<usize>,
    /// The active tab's lines as the panel shows them, worked out when they or the toolbar change rather than
    /// on each frame.
    shown: Vec<Shown>,
    search: Entity<InputState>,
    /// The line sent to the active terminal's input.
    command: Entity<InputState>,
    errors_only: bool,
    follow: bool,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DockEvent> for TerminalDock {}

impl TerminalDock {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t("shell.terminal.search_placeholder")));
        let command = cx.new(|cx| InputState::new(window, cx).placeholder(t("shell.terminal.input_placeholder")));
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe(&model, |this, _, event: &ModelEvent, cx| match event {
                ModelEvent::WorkspaceSwitched => {
                    this.tabs.retain(|tab| matches!(tab.source, Source::Terminal(_)));
                    this.keep_active(None, cx);
                }
                // What the old engine ran is gone with it; the new one is asked what it has.
                ModelEvent::EngineRestarted => {
                    this.tabs.clear();
                    this.keep_active(None, cx);
                    this.adopt_terminals(cx);
                }
                ModelEvent::DevServiceChanged(payload) => this.apply_dev_change(payload, cx),
                ModelEvent::DevServiceOutput(payload) => this.apply_dev_output(payload, cx),
                ModelEvent::TerminalChanged(payload) => this.apply_terminal_change(payload, cx),
                ModelEvent::TerminalOutput(payload) => this.apply_terminal_output(payload, cx),
                _ => {}
            }),
            cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.reload_shown(cx);
                }
            }),
            cx.subscribe_in(&command, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.send_line(window, cx);
                }
            }),
        ];
        let mut this = Self {
            model,
            tabs: Vec::new(),
            active: None,
            shown: Vec::new(),
            search,
            command,
            errors_only: false,
            follow: true,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        this.adopt_terminals(cx);
        this
    }

    /// How many programs have a tab, and how many of them run.
    pub fn counts(&self) -> (usize, usize) {
        (self.tabs.len(), self.tabs.iter().filter(|tab| tab.status == RunStatus::Running).count())
    }

    fn workspace(&self, cx: &gpui_kit::App) -> String {
        self.model.read(cx).workspace_id().to_owned()
    }

    fn notice(&mut self, text: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(text, cx));
    }

    fn position(&self, source: &Source) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.source == *source)
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active?)
    }

    /// Makes `source` the tab in front (the one in front now when none is named), or the first when it is gone.
    fn keep_active(&mut self, source: Option<Source>, cx: &mut Context<Self>) {
        let wanted = source.or_else(|| self.active_tab().map(|tab| tab.source.clone()));
        self.active = wanted.and_then(|source| self.position(&source)).or((!self.tabs.is_empty()).then_some(0));
        self.follow = true;
        self.reload_shown(cx);
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(ix) {
            let source = tab.source.clone();
            self.keep_active(Some(source), cx);
        }
    }

    fn reload_shown(&mut self, cx: &mut Context<Self>) {
        let needle = self.search.read(cx).value().trim().to_lowercase();
        self.shown = match self.active_tab() {
            Some(tab) => logs::visible(&tab.lines, self.errors_only, &needle),
            None => Vec::new(),
        };
        cx.notify();
    }

    /// Shows the output of dev server `name`, opening its tab when it has none. `fresh` is for a server that was
    /// just started again: what the tab holds is of the run before.
    pub fn show_dev(&mut self, name: &str, status: RunStatus, fresh: bool, cx: &mut Context<Self>) {
        let source = Source::Dev(name.to_owned());
        let opened = match self.position(&source) {
            Some(at) => {
                let tab = &mut self.tabs[at];
                tab.status = status;
                if fresh {
                    tab.lines.clear();
                    tab.loaded = false;
                }
                fresh
            }
            None => {
                let (title, lines) = (name.to_owned(), Vec::new());
                self.tabs.push(Tab { source: source.clone(), title, status, lines, loaded: false });
                true
            }
        };
        self.keep_active(Some(source), cx);
        if opened {
            self.read_dev(name.to_owned(), cx);
        }
    }

    fn read_dev(&mut self, name: String, cx: &mut Context<Self>) {
        let payload = json!({ "workspaceId": self.workspace(cx), "name": name, "last": FIRST_LINES });
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "devservice.logs", payload, move |this, outcome, cx| {
            let source = Source::Dev(name.clone());
            let Some(at) = this.position(&source) else { return };
            match failure(&outcome) {
                Some(message) => {
                    let text = t_with("services.dev.error.action", &[("name", &name), ("message", &message)]);
                    this.notice(text, cx);
                }
                None => this.tabs[at].lines = outcome.ok().map(|result| LogLine::list(&result)).unwrap_or_default(),
            }
            this.tabs[at].loaded = true;
            this.reload_shown_if_active(&source, cx);
        });
    }

    fn reload_shown_if_active(&mut self, source: &Source, cx: &mut Context<Self>) {
        if self.active_tab().is_some_and(|tab| tab.source == *source) {
            self.reload_shown(cx);
        }
    }

    /// Adds `lines` to the tab of `source` once its first read is in, and redraws when the tab is in front.
    fn append(&mut self, source: &Source, lines: Vec<LogLine>, cx: &mut Context<Self>) {
        let Some(at) = self.position(source) else { return };
        if !self.tabs[at].loaded {
            return;
        }
        logs::append(&mut self.tabs[at].lines, lines);
        self.reload_shown_if_active(source, cx);
    }

    /// `event.devservice.changed`: one service, or `null` once it is removed. Other workspaces' are not shown.
    fn apply_dev_change(&mut self, payload: &Value, cx: &mut Context<Self>) {
        if payload.get("workspaceId").and_then(Value::as_str) != Some(self.workspace(cx).as_str()) {
            return;
        }
        let Some(name) = payload.get("name").and_then(Value::as_str) else { return };
        let source = Source::Dev(name.to_owned());
        let Some(at) = self.position(&source) else { return };
        match payload.get("service").filter(|service| !service.is_null()) {
            Some(service) => self.tabs[at].status = RunStatus::of(service),
            None => return self.remove_tab(at, cx),
        }
        cx.notify();
    }

    fn apply_dev_output(&mut self, payload: &Value, cx: &mut Context<Self>) {
        if payload.get("workspaceId").and_then(Value::as_str) != Some(self.workspace(cx).as_str()) {
            return;
        }
        if let Some(name) = payload.get("name").and_then(Value::as_str) {
            self.append(&Source::Dev(name.to_owned()), LogLine::list(payload), cx);
        }
    }

    /// Asks the engine for the terminals it has open, which get a tab each.
    fn adopt_terminals(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "terminal.list", json!({}), |this, outcome, cx| {
            let listed = outcome.ok().and_then(|result| result.get("terminals").and_then(Value::as_array).cloned());
            for terminal in listed.into_iter().flatten() {
                this.upsert_terminal(&terminal, cx);
            }
        });
    }

    /// The tab of the terminal `value` describes, made when it has none; its place in `tabs`.
    fn upsert_terminal(&mut self, value: &Value, cx: &mut Context<Self>) -> Option<usize> {
        let id = value.get("id")?.as_str()?.to_owned();
        let source = Source::Terminal(id.clone());
        let status = RunStatus::of(value);
        let title = value.get("title").and_then(Value::as_str).unwrap_or_default().to_owned();
        if let Some(at) = self.position(&source) {
            self.tabs[at].status = status;
            self.tabs[at].title = title;
            cx.notify();
            return Some(at);
        }
        let at = self.tabs.len();
        self.tabs.push(Tab { source: source.clone(), title, status, lines: Vec::new(), loaded: false });
        if self.active.is_none() {
            self.keep_active(Some(source), cx);
        }
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "terminal.logs", json!({ "id": id, "last": FIRST_LINES }), move |this, outcome, cx| {
            let source = Source::Terminal(id);
            let Some(at) = this.position(&source) else { return };
            this.tabs[at].lines = outcome.ok().map(|result| LogLine::list(&result)).unwrap_or_default();
            this.tabs[at].loaded = true;
            this.reload_shown_if_active(&source, cx);
        });
        Some(at)
    }

    fn apply_terminal_change(&mut self, payload: &Value, cx: &mut Context<Self>) {
        let Some(id) = payload.get("id").and_then(Value::as_str) else { return };
        match payload.get("terminal").filter(|terminal| !terminal.is_null()) {
            Some(terminal) => {
                self.upsert_terminal(terminal, cx);
            }
            None => {
                if let Some(at) = self.position(&Source::Terminal(id.to_owned())) {
                    self.remove_tab(at, cx);
                }
            }
        }
    }

    fn apply_terminal_output(&mut self, payload: &Value, cx: &mut Context<Self>) {
        if let Some(id) = payload.get("id").and_then(Value::as_str) {
            self.append(&Source::Terminal(id.to_owned()), LogLine::list(payload), cx);
        }
    }

    /// Takes the tab out; the one in front stays in front unless it was this one, and then the next takes its place.
    fn remove_tab(&mut self, at: usize, cx: &mut Context<Self>) {
        let front = self.active_tab().map(|tab| tab.source.clone());
        let removed = self.tabs.remove(at).source;
        match front.filter(|source| *source != removed) {
            Some(source) => self.keep_active(Some(source), cx),
            None => {
                self.active = (!self.tabs.is_empty()).then(|| at.min(self.tabs.len() - 1));
                self.keep_active(None, cx);
            }
        }
    }

    /// Closes a tab. A terminal of the user's ends with it; a dev server keeps running, and shows again when its
    /// row is clicked or it is started.
    fn close_tab(&mut self, at: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(at) else { return };
        if let Source::Terminal(id) = &tab.source {
            let client = self.model.read(cx).client().clone();
            call(&client, cx, "terminal.close", json!({ "id": id }), |_, _, _| {});
        }
        self.remove_tab(at, cx);
    }

    /// Asks what to run, and opens a terminal for it: no command is the shell.
    pub fn prompt_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t("shell.terminal.prompt_title"),
            t("shell.terminal.open"),
            vec![
                PromptField::new(t("shell.terminal.field_cmd"), t("shell.terminal.field_cmd_placeholder")).optional(),
                PromptField::new(t("shell.terminal.field_cwd"), t("shell.terminal.field_cwd_placeholder")).optional(),
            ],
            |this, values, _, cx| {
                let mut payload = json!({});
                for (key, value) in ["cmd", "cwd"].into_iter().zip(&values) {
                    if !value.is_empty() {
                        payload[key] = json!(value);
                    }
                }
                let client = this.model.read(cx).client().clone();
                call(&client, cx, "terminal.open", payload, |this, outcome, cx| match failure(&outcome) {
                    Some(message) => this.notice(t_with("shell.terminal.error_open", &[("message", &message)]), cx),
                    None => {
                        let opened = outcome.ok().and_then(|result| result.get("terminal").cloned());
                        if let Some(at) = opened.and_then(|terminal| this.upsert_terminal(&terminal, cx)) {
                            this.select(at, cx);
                        }
                    }
                });
            },
        );
    }

    /// Sends the typed line to the input of the terminal in front.
    fn send_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.command.read(cx).value().to_string();
        let Some(Source::Terminal(id)) = self.active_tab().map(|tab| tab.source.clone()) else { return };
        self.command.update(cx, |state, cx| state.set_value("", window, cx));
        self.follow = true;
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "terminal.input", json!({ "id": id, "text": text }), |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.notice(t_with("shell.terminal.error_input", &[("message", &message)]), cx);
            }
        });
    }
}

impl TerminalDock {
    fn render_strip(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let tabs = self.tabs.iter().enumerate().map(|(at, tab)| {
            let key = tab.source.key();
            let dot = match tab.status {
                RunStatus::Running => theme.success,
                RunStatus::Stopped => theme.muted_foreground,
                RunStatus::Crashed(_) => theme.danger,
            };
            let front = self.active == Some(at);
            h_flex()
                .id(ElementId::Name(format!("terminal-tab-{key}").into()))
                .test_support()
                .flex_none()
                .cursor_pointer()
                .gap_2()
                .px_3()
                .h_full()
                .items_center()
                .text_sm()
                .border_r_1()
                .border_color(theme.border)
                .when(front, |this| this.bg(theme.background).text_color(theme.foreground))
                .when(!front, |this| this.text_color(theme.muted_foreground))
                .on_click(cx.listener(move |this, _, _, cx| this.select(at, cx)))
                .child(div().flex_none().size_2().rounded_full().bg(dot))
                .child(div().max_w_40().overflow_hidden().text_ellipsis().whitespace_nowrap().child(tab.title.clone()))
                .child(
                    Button::new(ElementId::Name(format!("terminal-close-{key}").into()))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .tooltip(t("shell.terminal.close_tab"))
                        .on_click(cx.listener(move |this, _, _, cx| this.close_tab(at, cx))),
                )
        });
        h_flex()
            .flex_none()
            .h_9()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.muted)
            .child(
                h_flex().id("terminal-tabs").flex_1().min_w_0().h_full().overflow_x_scroll().children(tabs).child(
                    div().flex_none().px_1().child(
                        Button::new("terminal-new")
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .tooltip(t("shell.terminal.new_tooltip"))
                            .on_click(cx.listener(|this, _, window, cx| this.prompt_new(window, cx))),
                    ),
                ),
            )
            .child(
                div().flex_none().px_1().child(
                    Button::new("terminal-hide")
                        .ghost()
                        .small()
                        .icon(IconName::Close)
                        .tooltip(t("shell.terminal.hide"))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(DockEvent::Hide))),
                ),
            )
            .into_any_element()
    }

    fn render_toolbar(&self, tab: &Tab, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let copied = self.shown.iter().map(Shown::copied).collect::<Vec<_>>().join("\n");
        let (status, color) = match tab.status {
            RunStatus::Running => (t("shell.terminal.status_running").to_owned(), theme.success),
            RunStatus::Stopped => (t("shell.terminal.status_stopped").to_owned(), theme.muted_foreground),
            RunStatus::Crashed(code) => (
                t_with(
                    "shell.terminal.status_exited",
                    &[("code", &code.map_or("?".to_owned(), |code| code.to_string()))],
                ),
                theme.danger,
            ),
        };
        h_flex()
            .flex_none()
            .gap_3()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(theme.border)
            .child(div().id("terminal-status").test_support().text_xs().text_color(color).child(status))
            .child(
                div().text_xs().text_color(theme.muted_foreground).child(t_with(
                    "shell.terminal.count",
                    &[("shown", &self.shown.len()), ("total", &tab.lines.len())],
                )),
            )
            .child(div().flex_1())
            .child(
                div().id("terminal-search").test_support().w_56().child(
                    Input::new(&self.search).small().prefix(Icon::new(IconName::Search).small()).cleanable(true),
                ),
            )
            .child(
                Checkbox::new("terminal-errors")
                    .small()
                    .label(t("shell.terminal.errors_only"))
                    .tooltip(t("shell.terminal.errors_only_tooltip"))
                    .checked(self.errors_only)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        this.errors_only = *checked;
                        this.reload_shown(cx);
                    })),
            )
            .child(
                Checkbox::new("terminal-follow")
                    .small()
                    .label(t("shell.terminal.follow"))
                    .checked(self.follow)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        this.follow = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("terminal-copy")
                    .ghost()
                    .small()
                    .icon(IconName::Copy)
                    .label(t("shell.terminal.copy"))
                    .disabled(self.shown.is_empty())
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))),
            )
            .child(
                Button::new("terminal-clear")
                    .ghost()
                    .small()
                    .label(t("shell.terminal.clear"))
                    .disabled(tab.lines.is_empty())
                    .tooltip(t("shell.terminal.clear_tooltip"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(at) = this.active {
                            this.tabs[at].lines.clear();
                        }
                        this.reload_shown(cx);
                    })),
            )
            .into_any_element()
    }

    fn render_lines(&self, tab: &Tab, cx: &mut Context<Self>) -> AnyElement {
        if !tab.loaded {
            return div().into_any_element();
        }
        if tab.lines.is_empty() {
            return page::empty(t("shell.terminal.no_output_title"), t("shell.terminal.no_output_description"));
        }
        if self.shown.is_empty() {
            return page::empty(t("shell.terminal.no_matches_title"), t("shell.terminal.no_matches_description"));
        }
        if self.follow {
            self.scroll.scroll_to_bottom();
        }
        uniform_list(
            "terminal-lines",
            self.shown.len(),
            cx.processor(|this, range: Range<usize>, _, cx| {
                let theme = cx.theme().clone();
                range
                    .filter_map(|ix| Some((ix, this.shown.get(ix)?)))
                    .map(|(ix, line)| {
                        h_flex()
                            .id(("terminal-line", ix))
                            .test_support()
                            .w_full()
                            .gap_3()
                            .px_3()
                            .text_xs()
                            .font_family(theme.mono_font_family.clone())
                            .child(div().flex_none().text_color(theme.muted_foreground).child(clock_utc(line.ts)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_color(if line.stderr { theme.danger } else { theme.foreground })
                                    .child(line.text.clone()),
                            )
                            .when(line.count > 1, |this| {
                                this.child(
                                    div()
                                        .flex_none()
                                        .text_color(theme.muted_foreground)
                                        .child(format!("\u{d7}{}", line.count)),
                                )
                            })
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full()
        .into_any_element()
    }

    /// The line typed to a terminal's input, for a terminal that still runs.
    fn render_input(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        h_flex()
            .flex_none()
            .px_3()
            .py_1()
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .id("terminal-input")
                    .test_support()
                    .w_full()
                    .child(Input::new(&self.command).small().prefix(Icon::new(IconName::ChevronRight).small())),
            )
            .into_any_element()
    }
}

impl Render for TerminalDock {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let strip = self.render_strip(cx);
        let front = self.active_tab().map(|tab| (self.render_toolbar(tab, cx), self.render_lines(tab, cx)));
        let typing = self
            .active_tab()
            .is_some_and(|tab| matches!(tab.source, Source::Terminal(_)) && tab.status == RunStatus::Running);
        let input = typing.then(|| self.render_input(cx));
        v_flex()
            .id("terminal-dock")
            .test_support()
            .size_full()
            .bg(theme.background)
            .border_t_1()
            .border_color(theme.border)
            // Reading earlier output must not be undone by the next line arriving.
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                if this.follow && event.delta.pixel_delta(window.line_height()).y > px(0.) {
                    this.follow = false;
                    cx.notify();
                }
            }))
            .child(strip)
            .map(|this| match front {
                Some((toolbar, lines)) => this.child(toolbar).child(div().flex_1().min_h_0().child(lines)),
                None => this.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(page::empty(t("shell.terminal.empty_title"), t("shell.terminal.empty_description"))),
                ),
            })
            .children(input)
    }
}
