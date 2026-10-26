//! The logs dock: a panel under the screens with a tab for each dev server the app started.
//!
//! The engine runs the servers (`devservice.*`) and keeps their lines; the dock holds the lines each tab has
//! been sent and shows the tab in front. A tab belongs to its server: stopping the server keeps the tab (the
//! logs stay to be read or copied) until it is closed, starting the server again empties and reuses it, and
//! removing the server takes it away.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, Theme, VirtualListScrollHandle,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, ClipboardItem, Context, Entity, EventEmitter, HighlightStyle, Hsla,
    InteractiveElement as _, InteractiveText, IntoElement, ParentElement as _, Render, ScrollStrategy,
    ScrollWheelEvent, Styled as _, StyledText, Subscription, TestSupportExt as _, UniformListScrollHandle, Window, div,
    px, uniform_list,
};
use serde_json::{Value, json};
use std::ops::Range;

use crate::{
    app_model::{AppModel, ModelEvent},
    components::{StripTab, TabAction, TabStrip, page, tab_strip},
    engine::{call, failure},
    i18n::{t, t_with},
    model::{
        logs::{self, LogLine, RunStatus, Shown, Token},
        traffic::clock_utc,
    },
};

/// How many lines of a server's output are read when its tab is opened.
const FIRST_LINES: u32 = 1000;

/// What the dock asks the window for.
pub enum DockEvent {
    /// The dock's own hide button was pressed.
    Hide,
}

struct LogTab {
    /// The dev server's name; what the tab is linked to.
    name: String,
    status: RunStatus,
    lines: Vec<LogLine>,
    /// The first read of the server's output has come back; output that arrives before it is in that read.
    loaded: bool,
}

pub struct LogsDock {
    model: Entity<AppModel>,
    tabs: Vec<LogTab>,
    active: Option<usize>,
    /// The running dev servers have been looked for in this workspace.
    adopted: bool,
    /// The active tab's lines as the panel shows them, worked out when they or the toolbar change rather than
    /// on each frame.
    shown: Vec<Shown>,
    search: Entity<InputState>,
    errors_only: bool,
    follow: bool,
    scroll: UniformListScrollHandle,
    tab_scroll: VirtualListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DockEvent> for LogsDock {}

impl LogsDock {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t("shell.logs.search_placeholder")));
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe(&model, |this, _, event: &ModelEvent, cx| match event {
                // What the old workspace or engine ran is gone with it.
                ModelEvent::WorkspaceSwitched | ModelEvent::EngineRestarted => {
                    this.tabs.clear();
                    this.keep_active(None, cx);
                    this.adopt_running(cx);
                }
                // The dock is built before the workspace is known; the first snapshot is when it can look.
                ModelEvent::ConfigChanged if !this.adopted => this.adopt_running(cx),
                ModelEvent::DevServiceChanged(payload) => this.apply_dev_change(payload, cx),
                ModelEvent::DevServiceOutput(payload) => this.apply_dev_output(payload, cx),
                _ => {}
            }),
            cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.reload_shown(cx);
                }
            }),
        ];
        let mut this = Self {
            model,
            tabs: Vec::new(),
            active: None,
            adopted: false,
            shown: Vec::new(),
            search,
            errors_only: false,
            follow: true,
            scroll: UniformListScrollHandle::new(),
            tab_scroll: VirtualListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        this.adopt_running(cx);
        this
    }

    /// Gives a tab to each dev server that already runs, which is what a window opened over a running engine
    /// finds: the app did not start them, but their output is as much to be followed.
    fn adopt_running(&mut self, cx: &mut Context<Self>) {
        let workspace = self.workspace(cx);
        if workspace.is_empty() {
            return;
        }
        self.adopted = true;
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "devservice.list", json!({ "workspaceId": workspace }), move |this, outcome, cx| {
            // The answer to a workspace that has been left since.
            if this.workspace(cx) != workspace {
                return;
            }
            let Ok(result) = outcome else {
                this.adopted = false;
                return;
            };
            let running = result["services"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|service| RunStatus::of(service) == RunStatus::Running);
            // A tab the user is looking at stays in front.
            let front = this.active_tab().map(|tab| tab.name.clone());
            for name in running.filter_map(|service| service.get("name")?.as_str()) {
                if this.position(name).is_none() {
                    this.show_dev(name, RunStatus::Running, false, cx);
                }
            }
            if front.is_some() {
                this.keep_active(front, cx);
            }
        });
    }

    /// How many servers have a tab, and how many of them run.
    pub fn counts(&self) -> (usize, usize) {
        (self.tabs.len(), self.tabs.iter().filter(|tab| tab.status == RunStatus::Running).count())
    }

    fn workspace(&self, cx: &gpui_kit::App) -> String {
        self.model.read(cx).workspace_id().to_owned()
    }

    fn notice(&mut self, text: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(text, cx));
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.name == name)
    }

    fn active_tab(&self) -> Option<&LogTab> {
        self.tabs.get(self.active?)
    }

    /// Makes the tab of `name` the one in front (the one in front now when none is named), or the first when
    /// it is gone.
    fn keep_active(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        let wanted = name.or_else(|| self.active_tab().map(|tab| tab.name.clone()));
        self.active = wanted.and_then(|name| self.position(&name)).or((!self.tabs.is_empty()).then_some(0));
        self.follow = true;
        if let Some(ix) = self.active {
            self.tab_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        self.reload_shown(cx);
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(ix) {
            let name = tab.name.clone();
            self.keep_active(Some(name), cx);
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
    /// just started again: what the tab holds is of the run before, so it is emptied and read anew.
    pub fn show_dev(&mut self, name: &str, status: RunStatus, fresh: bool, cx: &mut Context<Self>) {
        let read = match self.position(name) {
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
                self.tabs.push(LogTab { name: name.to_owned(), status, lines: Vec::new(), loaded: false });
                true
            }
        };
        self.keep_active(Some(name.to_owned()), cx);
        if read {
            self.read_dev(name.to_owned(), cx);
        }
    }

    fn read_dev(&mut self, name: String, cx: &mut Context<Self>) {
        let payload = json!({ "workspaceId": self.workspace(cx), "name": name, "last": FIRST_LINES });
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "devservice.logs", payload, move |this, outcome, cx| {
            let Some(at) = this.position(&name) else { return };
            match failure(&outcome) {
                Some(message) => {
                    let text = t_with("services.dev.error.action", &[("name", &name), ("message", &message)]);
                    this.notice(text, cx);
                }
                None => this.tabs[at].lines = outcome.ok().map(|result| LogLine::list(&result)).unwrap_or_default(),
            }
            this.tabs[at].loaded = true;
            this.reload_shown_if_active(&name, cx);
        });
    }

    fn reload_shown_if_active(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.active_tab().is_some_and(|tab| tab.name == name) {
            self.reload_shown(cx);
        }
    }

    /// `event.devservice.changed`: one service, or `null` once it is removed. Other workspaces' are not shown.
    /// A server that runs again, whoever started it (the app, the CLI, an agent), has its tab emptied and reused.
    fn apply_dev_change(&mut self, payload: &Value, cx: &mut Context<Self>) {
        if payload.get("workspaceId").and_then(Value::as_str) != Some(self.workspace(cx).as_str()) {
            return;
        }
        let Some(name) = payload.get("name").and_then(Value::as_str) else { return };
        let Some(at) = self.position(name) else { return };
        let Some(service) = payload.get("service").filter(|service| !service.is_null()) else {
            return self.remove_tab(at, cx);
        };
        let status = RunStatus::of(service);
        let started = status == RunStatus::Running && self.tabs[at].status != RunStatus::Running;
        self.tabs[at].status = status;
        if started {
            self.tabs[at].lines.clear();
            self.tabs[at].loaded = false;
            self.read_dev(name.to_owned(), cx);
            self.reload_shown_if_active(name, cx);
        }
        cx.notify();
    }

    /// Adds the lines of an `event.devservice.output` to the tab of that server once its first read is in, and
    /// redraws when the tab is in front.
    fn apply_dev_output(&mut self, payload: &Value, cx: &mut Context<Self>) {
        if payload.get("workspaceId").and_then(Value::as_str) != Some(self.workspace(cx).as_str()) {
            return;
        }
        let Some(name) = payload.get("name").and_then(Value::as_str) else { return };
        let Some(at) = self.position(name) else { return };
        if !self.tabs[at].loaded {
            return;
        }
        logs::append(&mut self.tabs[at].lines, LogLine::list(payload));
        self.reload_shown_if_active(name, cx);
    }

    /// Takes the tab out; the one in front stays in front unless it was this one, and then the next takes its place.
    fn remove_tab(&mut self, at: usize, cx: &mut Context<Self>) {
        let front = self.active_tab().map(|tab| tab.name.clone());
        let removed = self.tabs.remove(at).name;
        match front.filter(|name| *name != removed) {
            Some(name) => self.keep_active(Some(name), cx),
            None => {
                self.active = (!self.tabs.is_empty()).then(|| at.min(self.tabs.len() - 1));
                self.keep_active(None, cx);
            }
        }
    }

    /// Stops the server of the tab in front. The tab stays, with the logs.
    fn stop(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.active_tab().map(|tab| tab.name.clone()) else { return };
        let payload = json!({ "workspaceId": self.workspace(cx), "name": name });
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "devservice.down", payload, move |this, outcome, cx| match failure(&outcome) {
            Some(message) => {
                let text = t_with("services.dev.error.action", &[("name", &name), ("message", &message)]);
                this.notice(text, cx);
            }
            None => {
                let service = outcome.ok().and_then(|result| result.get("service").cloned());
                if let (Some(at), Some(service)) = (this.position(&name), service) {
                    this.tabs[at].status = RunStatus::of(&service);
                    cx.notify();
                }
            }
        });
    }

    /// Closes the tab in front. Only a stopped server's tab closes; the button is off while it runs.
    fn close(&mut self, cx: &mut Context<Self>) {
        if let Some(at) = self.active.filter(|at| self.tabs[*at].status != RunStatus::Running) {
            self.remove_tab(at, cx);
        }
    }
}

/// How a kind of text is coloured in a line.
fn style_of(token: Token, theme: &Theme) -> HighlightStyle {
    let paint = |color: Hsla| HighlightStyle { color: Some(color), ..Default::default() };
    let bold = |color: Hsla| HighlightStyle { font_weight: Some(gpui_kit::FontWeight::BOLD), ..paint(color) };
    match token {
        Token::Error => bold(theme.danger),
        Token::Warn => bold(theme.warning),
        Token::Info => bold(theme.info),
        Token::Debug => paint(theme.muted_foreground),
        Token::Method => bold(theme.primary),
        Token::Url => paint(theme.link),
        Token::Time => paint(theme.muted_foreground),
        Token::Number => paint(theme.warning),
        Token::Quoted => paint(theme.success),
    }
}

impl LogsDock {
    /// What a tab shows: the server's name and a dot for how it is doing. Its tab cannot be closed from the
    /// strip; the toolbar's Close does that once the server is stopped.
    fn describe_tab(&self, ix: usize, cx: &gpui_kit::App) -> Option<StripTab> {
        let tab = self.tabs.get(ix)?;
        let theme = cx.theme();
        let dot = match tab.status {
            RunStatus::Running => theme.success,
            RunStatus::Stopped => theme.muted_foreground,
            RunStatus::Crashed(_) => theme.danger,
        };
        Some(StripTab {
            id: format!("logs-{}", tab.name).into(),
            title: tab.name.clone().into(),
            dot: Some(dot),
            fixed: true,
            ..Default::default()
        })
    }

    fn tab_action(&mut self, action: TabAction, _: &mut Window, cx: &mut Context<Self>) {
        if let TabAction::Select(ix) = action {
            self.select(ix, cx);
        }
    }

    fn render_strip(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let strip = TabStrip {
            id: "logs-tabs",
            count: self.tabs.len(),
            active: self.active,
            scroll: self.tab_scroll.clone(),
            describe: Self::describe_tab,
            on_action: Self::tab_action,
            active_background: cx.theme().status_bar,
            bar_background: cx.theme().tab_bar_segmented,
        };
        let hide = h_flex().flex_none().px_1().child(
            Button::new("logs-hide")
                .ghost()
                .small()
                .icon(IconName::Close)
                .tooltip(t("shell.logs.hide"))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(DockEvent::Hide))),
        );
        tab_strip(self, strip, div(), hide, window, cx)
    }

    fn render_toolbar(&self, tab: &LogTab, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let running = tab.status == RunStatus::Running;
        let copied = self.shown.iter().map(Shown::copied).collect::<Vec<_>>().join("\n");
        let (status, color) = match tab.status {
            RunStatus::Running => (t("shell.logs.status_running").to_owned(), theme.success),
            RunStatus::Stopped => (t("shell.logs.status_stopped").to_owned(), theme.muted_foreground),
            RunStatus::Crashed(code) => (
                t_with("shell.logs.status_exited", &[("code", &code.map_or("?".to_owned(), |code| code.to_string()))]),
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
            .child(div().id("logs-status").test_support().text_xs().text_color(color).child(status))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t_with("shell.logs.count", &[("shown", &self.shown.len()), ("total", &tab.lines.len())])),
            )
            .child(div().flex_1())
            .child(
                div().id("logs-search").test_support().w_56().child(
                    Input::new(&self.search).small().prefix(Icon::new(IconName::Search).small()).cleanable(true),
                ),
            )
            .child(
                Checkbox::new("logs-errors")
                    .small()
                    .label(t("shell.logs.errors_only"))
                    .tooltip(t("shell.logs.errors_only_tooltip"))
                    .checked(self.errors_only)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        this.errors_only = *checked;
                        this.reload_shown(cx);
                    })),
            )
            .child(Checkbox::new("logs-follow").small().label(t("shell.logs.follow")).checked(self.follow).on_change(
                cx.listener(|this, checked: &bool, _, cx| {
                    this.follow = *checked;
                    cx.notify();
                }),
            ))
            .child(
                Button::new("logs-copy")
                    .ghost()
                    .small()
                    .icon(IconName::Copy)
                    .label(t("shell.logs.copy"))
                    .disabled(self.shown.is_empty())
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))),
            )
            .child(
                Button::new("logs-clear")
                    .ghost()
                    .small()
                    .label(t("shell.logs.clear"))
                    .disabled(tab.lines.is_empty())
                    .tooltip(t("shell.logs.clear_tooltip"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(at) = this.active {
                            this.tabs[at].lines.clear();
                        }
                        this.reload_shown(cx);
                    })),
            )
            .child(
                Button::new("logs-stop")
                    .outline()
                    .small()
                    .icon(IconName::Square)
                    .label(t("shell.logs.stop"))
                    .tooltip(t("shell.logs.stop_tooltip"))
                    .disabled(!running)
                    .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
            )
            .child(
                Button::new("logs-close")
                    .outline()
                    .small()
                    .icon(IconName::Close)
                    .label(t("shell.logs.close"))
                    .tooltip(t("shell.logs.close_tooltip"))
                    .disabled(running)
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
            )
            .into_any_element()
    }

    fn render_lines(&self, tab: &LogTab, cx: &mut Context<Self>) -> AnyElement {
        if !tab.loaded {
            return div().into_any_element();
        }
        if tab.lines.is_empty() {
            return page::empty(t("shell.logs.no_output_title"), t("shell.logs.no_output_description"));
        }
        if self.shown.is_empty() {
            return page::empty(t("shell.logs.no_matches_title"), t("shell.logs.no_matches_description"));
        }
        if self.follow {
            self.scroll.scroll_to_bottom();
        }
        uniform_list(
            "logs-lines",
            self.shown.len(),
            cx.processor(|this, range: Range<usize>, _, cx| {
                let theme = cx.theme().clone();
                range
                    .filter_map(|ix| Some((ix, this.shown.get(ix)?)))
                    .map(|(ix, line)| {
                        let tokens = logs::tokens(&line.text);
                        let marks = tokens
                            .iter()
                            .map(|(range, token)| (range.clone(), style_of(*token, &theme)))
                            .collect::<Vec<_>>();
                        let urls = tokens.into_iter().filter(|(_, token)| *token == Token::Url).map(|(range, _)| range);
                        let urls: Vec<_> = urls.collect();
                        let text = StyledText::new(line.text.clone()).with_highlights(marks);
                        // Ctrl-click (Cmd on a Mac) opens an address in the browser; a plain click does nothing, so
                        // the line can still be pointed at.
                        let text = if urls.is_empty() {
                            text.into_any_element()
                        } else {
                            let targets: Vec<String> =
                                urls.iter().map(|range| line.text[range.clone()].to_owned()).collect();
                            InteractiveText::new(("logs-link", ix), text)
                                .on_click(urls, move |at, window, cx| {
                                    if window.modifiers().secondary() {
                                        cx.open_url(&targets[at]);
                                    }
                                })
                                .into_any_element()
                        };
                        h_flex()
                            .id(("logs-line", ix))
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
                                    .child(text),
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
}

impl Render for LogsDock {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let strip = self.render_strip(window, cx);
        let front = self.active_tab().map(|tab| (self.render_toolbar(tab, cx), self.render_lines(tab, cx)));
        v_flex()
            .id("logs-dock")
            .test_support()
            .size_full()
            .bg(theme.status_bar)
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
                        .child(page::empty(t("shell.logs.empty_title"), t("shell.logs.empty_description"))),
                ),
            })
    }
}
