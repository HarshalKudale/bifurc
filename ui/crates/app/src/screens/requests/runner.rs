//! A folder opened as a tab, to run every request in it in order.
//!
//! The run is the engine's: `runner.start` hands it over, `event.runner.progress` and
//! `event.runner.finished` say how it goes, and `runner.status` answers for a run that started before this
//! tab was open. The order is the folder's saved runner configuration (`requestOrder`), written back with
//! `runner.saveConfig` when a request is moved. This view keeps what it was told and draws it.

use std::collections::{HashMap, HashSet};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    progress::Progress,
    spinner::Spinner,
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::{Map, Value, json};

use crate::{
    app_model::{AppModel, ModelEvent},
    components::page,
    engine::{call, describe, failure},
    i18n::{t, t_with},
    model::{NodeRef, RequestKind, config::RequestStub},
};

#[derive(Debug, Clone)]
pub enum RunnerEvent {
    /// A request in the list was chosen, to be opened in its own tab.
    Open(NodeRef),
    /// Something the tab strip shows may have changed.
    Changed,
}

/// What the engine reported for one request of the run.
#[derive(Debug, Clone, Default, PartialEq)]
struct RowResult {
    /// `None` when no response came back.
    status: Option<u16>,
    duration_ms: u64,
    /// Why there is no response, or which script failed.
    errors: Vec<String>,
    passed: usize,
    /// The names of the tests that failed, each with its reason.
    failed: Vec<String>,
    /// Every test that ran: its name, whether it passed, and why not.
    tests: Vec<(String, bool, Option<String>)>,
    /// The pre-request, post-response and test scripts the request has, `None` for one it has not.
    pre: Option<String>,
    post: Option<String>,
    test_script: Option<String>,
    /// Why the pre-request or post-response script failed.
    pre_error: Option<String>,
    post_error: Option<String>,
    /// What the scripts logged.
    logs: Vec<String>,
    response_headers: Vec<(String, String)>,
    response_body: String,
}

fn text_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().map(|item| item.as_str().map(str::to_owned).unwrap_or_else(|| item.to_string())).collect())
        .unwrap_or_default()
}

fn header_list(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_object)
        .map(|map| {
            map.iter().map(|(k, v)| (k.clone(), v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string()))).collect()
        })
        .unwrap_or_default()
}

/// The source of the `test('name', …)` call in `script` that defines the test called `name`.
fn test_source(script: &str, name: &str) -> Option<String> {
    let (start, open) = ['\'', '"', '`'].iter().find_map(|quote| {
        let at = script.find(&format!("test({quote}{name}{quote}"))?;
        let head = script[..at].trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '.').len();
        Some((head, at + "test".len()))
    })?;
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (offset, c) in script[open..].char_indices() {
        if let Some(q) = quote {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, c) if c == q => quote = None,
                _ => {}
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => quote = Some(c),
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let mut end = open + offset + 1;
                    if script[end..].starts_with(';') {
                        end += 1;
                    }
                    return Some(script[start..end].to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

impl RowResult {
    fn from_value(value: &Value) -> Self {
        let text = |key: &str| value.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
        let tests = value.get("tests").and_then(Value::as_array).cloned().unwrap_or_default();
        let failed = tests
            .iter()
            .filter(|test| test.get("passed").and_then(Value::as_bool) != Some(true))
            .map(|test| {
                let name = test.get("name").and_then(Value::as_str).unwrap_or(t("requests.runner.unnamed_test"));
                match test.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()) {
                    Some(error) => format!("{name}: {error}"),
                    None => name.to_owned(),
                }
            })
            .collect::<Vec<_>>();
        let own = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        let script = |key: &str| {
            let source = value.pointer(&format!("/scripts/{key}")).and_then(Value::as_str)?;
            (!source.trim().is_empty()).then(|| source.to_owned())
        };
        Self {
            pre: script("pre"),
            post: script("post"),
            test_script: script("test"),
            pre_error: text("preScriptError"),
            post_error: text("postScriptError"),
            tests: tests
                .iter()
                .map(|test| {
                    (
                        test.get("name").and_then(Value::as_str).unwrap_or(t("requests.runner.unnamed_test")).to_owned(),
                        test.get("passed").and_then(Value::as_bool) == Some(true),
                        test.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()).map(str::to_owned),
                    )
                })
                .collect(),
            logs: text_list(value.get("testLogs")),
            response_headers: header_list(value.get("responseHeaders")),
            response_body: own("responseBody"),
            status: value.get("status").and_then(Value::as_u64).and_then(|s| u16::try_from(s).ok()),
            duration_ms: value.get("responseTime").and_then(Value::as_f64).map_or(0, |ms| ms.max(0.) as u64),
            errors: [
                text("error"),
                text("preScriptError").map(|e| t_with("requests.runner.pre_script_error", &[("error", &e)])),
                text("postScriptError").map(|e| t_with("requests.runner.post_script_error", &[("error", &e)])),
            ]
            .into_iter()
            .flatten()
            .collect(),
            passed: tests.len() - failed.len(),
            failed,
        }
    }

    fn is_failure(&self) -> bool {
        self.status.is_none_or(|status| status >= 400) || !self.errors.is_empty() || !self.failed.is_empty()
    }
}

pub struct FolderRunner {
    model: Entity<AppModel>,
    folder_id: String,
    /// The folder's saved runner configuration, as the engine holds it. It is written back whole, so what
    /// this view does not show (the delay between requests, say) is kept.
    config: Map<String, Value>,
    /// The saved run order. Requests it does not name follow, in the workspace's order.
    order: Vec<String>,
    running: bool,
    cancelling: bool,
    /// How many requests the run in progress, or the last one, set out to send.
    total: usize,
    completed: usize,
    /// Results of the run in progress or the last one, by request id.
    results: HashMap<String, RowResult>,
    /// The rows opened to show their details.
    expanded: HashSet<String>,
    /// Which tab of an opened row is showing, and which of its tests is chosen, by request id.
    tabs: HashMap<String, usize>,
    picked: HashMap<String, usize>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<RunnerEvent> for FolderRunner {}

impl FolderRunner {
    pub fn new(model: Entity<AppModel>, folder_id: String, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.subscribe(&model, |this, _, event: &ModelEvent, cx| match event {
                ModelEvent::RunnerProgress(payload) if this.is_about(payload, cx) => {
                    this.total = payload.get("total").and_then(Value::as_u64).unwrap_or(0) as usize;
                    if let Some(result) = payload.get("result") {
                        this.take_result(result);
                    }
                    this.running = true;
                    cx.notify();
                }
                ModelEvent::RunnerFinished(payload) if this.is_about(payload, cx) => {
                    if let Some(report) = payload.get("report") {
                        this.take_results(report.get("results"));
                    }
                    this.set_running(false, cx);
                }
                _ => {}
            }),
            // The list is the folder's requests, which the model holds.
            cx.observe(&model, |_, _, cx| cx.notify()),
        ];
        let mut this = Self {
            model,
            folder_id,
            config: Map::new(),
            order: Vec::new(),
            running: false,
            cancelling: false,
            total: 0,
            completed: 0,
            results: HashMap::new(),
            expanded: HashSet::new(),
            tabs: HashMap::new(),
            picked: HashMap::new(),
            _subscriptions: subscriptions,
        };
        this.load(cx);
        this
    }

    pub fn folder_id(&self) -> &str {
        &self.folder_id
    }

    pub fn title(&self, cx: &gpui_kit::App) -> SharedString {
        self.model
            .read(cx)
            .config()
            .folder(&self.folder_id)
            .map(|folder| folder.name.clone())
            .unwrap_or_default()
            .into()
    }

    fn is_about(&self, payload: &Value, cx: &gpui_kit::App) -> bool {
        payload.get("folderId").and_then(Value::as_str) == Some(self.folder_id.as_str())
            && payload.get("workspaceId").and_then(Value::as_str) == Some(self.model.read(cx).workspace_id())
    }

    fn scope(&self, cx: &gpui_kit::App) -> Value {
        json!({ "workspaceId": self.model.read(cx).workspace_id(), "folderId": self.folder_id })
    }

    fn set_running(&mut self, running: bool, cx: &mut Context<Self>) {
        self.running = running;
        if !running {
            self.cancelling = false;
        }
        cx.emit(RunnerEvent::Changed);
        cx.notify();
    }

    fn take_result(&mut self, result: &Value) {
        if let Some(id) = result.get("requestId").and_then(Value::as_str) {
            self.results.insert(id.to_owned(), RowResult::from_value(result));
            self.completed = self.results.len();
        }
    }

    fn take_results(&mut self, results: Option<&Value>) {
        for result in results.and_then(Value::as_array).into_iter().flatten() {
            self.take_result(result);
        }
    }

    /// Reads the saved order, and the run the engine may already hold for this folder.
    fn load(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "runner.loadConfig", self.scope(cx), |this, outcome, cx| {
            if let Ok(Value::Object(config)) = outcome {
                this.order = config
                    .get("requestOrder")
                    .and_then(Value::as_array)
                    .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                    .unwrap_or_default();
                this.config = config;
                cx.notify();
            }
        });
        call(&client, cx, "runner.status", self.scope(cx), |this, outcome, cx| {
            let Ok(run) = outcome else { return };
            if run.is_null() {
                return;
            }
            this.total = run.get("total").and_then(Value::as_u64).unwrap_or(0) as usize;
            this.take_results(run.get("results"));
            this.set_running(run.get("running").and_then(Value::as_bool).unwrap_or(false), cx);
        });
    }

    /// The folder's requests in run order.
    fn rows<'a>(&self, cx: &'a gpui_kit::App) -> Vec<&'a RequestStub> {
        let rank = |id: &str| self.order.iter().position(|ordered| ordered == id).unwrap_or(usize::MAX);
        let mut rows: Vec<&RequestStub> = self
            .model
            .read(cx)
            .config()
            .requests
            .iter()
            .filter(|r| r.kind == RequestKind::Rest && r.folder_id.as_deref() == Some(self.folder_id.as_str()))
            // One a newer Bifurc saved is not this engine's to send.
            .filter(|r| r.locked.is_none())
            .collect();
        // Stable, so the requests the saved order does not name stay in the workspace's order.
        rows.sort_by_key(|request| rank(&request.id));
        rows
    }

    /// Moves the request at `ix` one place up or down, and saves the order.
    fn move_row(&mut self, ix: usize, up: bool, cx: &mut Context<Self>) {
        let mut ids: Vec<String> = self.rows(cx).into_iter().map(|request| request.id.clone()).collect();
        let other = if up { ix.checked_sub(1) } else { Some(ix + 1).filter(|other| *other < ids.len()) };
        let Some(other) = other else { return };
        ids.swap(ix, other);
        self.order = ids;
        self.config.insert("requestOrder".into(), json!(self.order));
        let mut payload = self.scope(cx);
        payload["config"] = Value::Object(self.config.clone());
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "runner.saveConfig", payload, |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.report(t_with("requests.runner.errors.save_order", &[("message", &message)]), cx);
            }
        });
        cx.notify();
    }

    fn report(&mut self, text: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(text, cx));
    }

    pub fn run(&mut self, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        self.results.clear();
        self.expanded.clear();
        self.completed = 0;
        self.total = self.rows(cx).len();
        self.set_running(true, cx);
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "runner.start", self.scope(cx), |this, outcome, cx| match outcome {
            Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(true) => {
                this.total = result.get("total").and_then(Value::as_u64).unwrap_or(0) as usize;
                // A folder with nothing in it finishes before its first event could say so.
                if this.total == 0 {
                    this.set_running(false, cx);
                }
                cx.notify();
            }
            other => {
                let message = match &other {
                    Ok(result) => {
                        result.get("error").and_then(Value::as_str).unwrap_or(t("requests.runner.engine_refused")).to_owned()
                    }
                    Err(error) => describe(error),
                };
                this.set_running(false, cx);
                this.report(t_with("requests.runner.errors.start", &[("message", &message)]), cx);
            }
        });
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if !self.running || self.cancelling {
            return;
        }
        self.cancelling = true;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "runner.cancel", self.scope(cx), |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.cancelling = false;
                this.report(t_with("requests.runner.errors.stop", &[("message", &message)]), cx);
                cx.notify();
            }
        });
    }

    /// One line about the run: how far it is, or how it went.
    fn summary(&self, count: usize) -> String {
        if self.results.is_empty() && !self.running {
            return if count == 1 {
                t_with("requests.runner.idle_one", &[("count", &count)])
            } else {
                t_with("requests.runner.idle_other", &[("count", &count)])
            };
        }
        let failed = self.results.values().filter(|result| result.is_failure()).count();
        let tests: usize = self.results.values().map(|result| result.passed + result.failed.len()).sum();
        let tests_failed: usize = self.results.values().map(|result| result.failed.len()).sum();
        let progress = if self.running {
            t_with(
                "requests.runner.summary_running",
                &[("done", &self.completed), ("total", &self.total)],
            )
        } else if self.completed < self.total {
            t_with(
                "requests.runner.summary_stopped",
                &[("done", &self.completed), ("total", &self.total)],
            )
        } else {
            t_with("requests.runner.summary_done", &[("done", &self.completed)])
        };
        if tests > 0 {
            t_with(
                "requests.runner.summary_tests",
                &[
                    ("progress", &progress),
                    ("failed", &failed),
                    ("passed", &(tests - tests_failed)),
                    ("total", &tests),
                ],
            )
        } else {
            t_with("requests.runner.summary_failed", &[("progress", &progress), ("failed", &failed)])
        }
    }

    fn render_result(&self, ix: usize, id: &str, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme();
        let Some(result) = self.results.get(id) else {
            // The engine sends one request at a time, in order: the first one without a result is in flight.
            return if self.running && ix == self.completed {
                Spinner::new().small().into_any_element()
            } else {
                div().into_any_element()
            };
        };
        let status = match result.status {
            Some(status) if status < 400 => Tag::success().small().child(status.to_string()),
            Some(status) => Tag::danger().small().child(status.to_string()),
            None => Tag::danger().small().child(t("requests.runner.no_response")),
        };
        let tests = result.passed + result.failed.len();
        h_flex()
            .gap_2()
            .text_xs()
            .text_color(theme.muted_foreground)
            .children((tests > 0).then(|| {
                let text = t_with("requests.runner.tests_count", &[("passed", &result.passed), ("total", &tests)]);
                if result.failed.is_empty() {
                    Tag::success().small().child(text)
                } else {
                    Tag::danger().small().child(text)
                }
            }))
            .child(t_with("requests.runner.duration", &[("ms", &result.duration_ms)]))
            .child(status)
            .into_any_element()
    }

    fn render_row(
        &self,
        ix: usize,
        last: usize,
        request: &RequestStub,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let id = request.id.clone();
        let node = NodeRef::Request { kind: RequestKind::Rest, id: id.clone() };
        let name = if request.name.is_empty() { request.target.clone() } else { request.name.clone() };
        let open = self.expanded.contains(&id);
        let problems: Vec<String> = self
            .results
            .get(&id)
            .filter(|_| !open)
            .map(|result| result.errors.iter().chain(result.failed.iter()).cloned().collect())
            .unwrap_or_default();
        v_flex()
            .id(gpui_kit::ElementId::Name(format!("run-row-{id}").into()))
            .px_3()
            .py_2()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_2()
                    .child(match self.results.get(&id) {
                        Some(_) => Button::new("toggle-details")
                            .icon(if open { IconName::ChevronDown } else { IconName::ChevronRight })
                            .ghost()
                            .xsmall()
                            .tooltip(if open { t("requests.runner.hide_details") } else { t("requests.runner.show_details") })
                            .on_click({
                                let id = id.clone();
                                cx.listener(move |this, _, _, cx| {
                                    if !this.expanded.remove(&id) {
                                        this.expanded.insert(id.clone());
                                    }
                                    cx.notify();
                                })
                            })
                            .into_any_element(),
                        None => div().size_6().flex_none().into_any_element(),
                    })
                    .child(div().w_6().flex_none().text_xs().text_color(theme.muted_foreground).child((ix + 1).to_string()))
                    .child(
                        h_flex()
                            .flex_none()
                            .child(
                                Button::new("move-up")
                                    .icon(IconName::ChevronUp)
                                    .ghost()
                                    .xsmall()
                                    .tooltip(t("requests.runner.move_up"))
                                    .disabled(self.running || ix == 0)
                                    .on_click(cx.listener(move |this, _, _, cx| this.move_row(ix, true, cx))),
                            )
                            .child(
                                Button::new("move-down")
                                    .icon(IconName::ChevronDown)
                                    .ghost()
                                    .xsmall()
                                    .tooltip(t("requests.runner.move_down"))
                                    .disabled(self.running || ix == last)
                                    .on_click(cx.listener(move |this, _, _, cx| this.move_row(ix, false, cx))),
                            ),
                    )
                    .child(
                        div()
                            .w_12()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(request.method.clone().unwrap_or_default()),
                    )
                    .child(
                        div()
                            .id("open-request")
                            .flex_none()
                            .max_w_1_3()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_sm()
                            .cursor_pointer()
                            .hover(|this| this.underline())
                            .child(name)
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(RunnerEvent::Open(node.clone())))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .font_family(theme.mono_font_family.clone())
                            .child(request.target.clone()),
                    )
                    .child(div().flex_none().child(self.render_result(ix, &id, cx))),
            )
            .children(
                problems
                    .into_iter()
                    .map(|problem| div().pl_8().text_xs().text_color(theme.danger).child(problem)),
            )
            .children(self.results.get(&id).filter(|_| open).map(|result| self.render_details(&id, result, cx)))
            .into_any_element()
    }

    fn block(&self, id: String, text: String, tint: Option<gpui_kit::Hsla>, cx: &Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme();
        div()
            .id(SharedString::from(id))
            .w_full()
            .max_h_64()
            .overflow_y_scroll()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(tint.map_or(theme.muted, |color| color.opacity(0.12)))
            .text_xs()
            .font_family(theme.mono_font_family.clone())
            .child(text)
            .into_any_element()
    }

    fn label(&self, text: &str, cx: &Context<Self>) -> gpui_kit::AnyElement {
        div().text_xs().text_color(cx.theme().muted_foreground).child(text.to_owned()).into_any_element()
    }

    /// One of the request's scripts: whether it ran or failed, and its code.
    fn render_script(
        &self,
        id: &str,
        which: &str,
        source: Option<&String>,
        error: Option<&String>,
        cx: &Context<Self>,
    ) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let Some(source) = source else {
            return div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(t("requests.runner.no_script"))
                .into_any_element();
        };
        let outcome = match error {
            Some(_) => Tag::danger().small().child(t("requests.common.failed")),
            None => Tag::success().small().child(t("requests.runner.script_ran")),
        };
        let tint = Some(if error.is_some() { theme.danger } else { theme.success });
        v_flex()
            .gap_2()
            .child(h_flex().gap_2().child(outcome))
            .children(error.map(|error| {
                div().text_sm().text_color(theme.danger).font_family(theme.mono_font_family.clone()).child(error.clone())
            }))
            .child(self.block(format!("script-{which}-{id}"), source.clone(), tint, cx))
            .into_any_element()
    }

    /// The tests: the list on the left, and on the right the code of the one chosen, green or red by how it went.
    fn render_tests(&self, id: &str, result: &RowResult, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        if result.tests.is_empty() {
            return div().text_sm().text_color(theme.muted_foreground).child(t("requests.runner.no_tests")).into_any_element();
        }
        let picked = self.picked.get(id).copied().filter(|ix| *ix < result.tests.len()).unwrap_or(0);
        let list = v_flex().w_64().flex_none().gap_1().children(result.tests.iter().enumerate().map(|(ix, (name, ok, _))| {
            let (mark, color) = if *ok { ("\u{2713}", theme.success) } else { ("\u{2717}", theme.danger) };
            let key = id.to_owned();
            h_flex()
                .id(SharedString::from(format!("pick-{id}-{ix}")))
                .gap_2()
                .px_2()
                .py_1()
                .rounded_md()
                .text_sm()
                .cursor_pointer()
                .when(ix == picked, |row| row.bg(theme.muted))
                .hover(|row| row.bg(theme.muted))
                .child(div().flex_none().text_color(color).child(mark))
                .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().child(name.clone()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.picked.insert(key.clone(), ix);
                    cx.notify();
                }))
        }));
        let (name, ok, error) = &result.tests[picked];
        let code = result
            .test_script
            .as_deref()
            .map(|script| test_source(script, name).unwrap_or_else(|| script.to_owned()))
            .unwrap_or_else(|| t("requests.runner.no_test_script").to_owned());
        let detail = v_flex()
            .flex_1()
            .min_w_0()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(if *ok {
                        Tag::success().small().child(t("requests.common.passed"))
                    } else {
                        Tag::danger().small().child(t("requests.common.failed"))
                    })
                    .child(div().text_sm().child(name.clone())),
            )
            .children(error.as_ref().map(|error| {
                div().text_sm().text_color(theme.danger).font_family(theme.mono_font_family.clone()).child(error.clone())
            }))
            .child(self.block(
                format!("test-code-{id}"),
                code,
                Some(if *ok { theme.success } else { theme.danger }),
                cx,
            ))
            .children((!result.logs.is_empty()).then(|| {
                v_flex()
                    .gap_1()
                    .child(self.label(t("requests.runner.console"), cx))
                    .child(self.block(format!("test-logs-{id}"), result.logs.join("\n"), None, cx))
            }));
        h_flex().w_full().gap_4().items_start().child(list).child(detail).into_any_element()
    }

    /// The response: status and time, then its headers and body.
    fn render_response(&self, id: &str, result: &RowResult, cx: &Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let Some(status) = result.status else {
            let reason = if result.errors.is_empty() {
                t("requests.runner.no_response_came_back").to_owned()
            } else {
                result.errors.join("\n")
            };
            return self.block(format!("response-error-{id}"), reason, Some(theme.danger), cx);
        };
        let tag = if status < 400 { Tag::success() } else { Tag::danger() };
        let headers =
            result.response_headers.iter().map(|(key, value)| format!("{key}: {value}")).collect::<Vec<_>>().join("\n");
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_3()
                    .child(tag.small().child(status.to_string()))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(t_with("requests.runner.duration", &[("ms", &result.duration_ms)])),
                    ),
            )
            .child(self.label(t("requests.common.headers"), cx))
            .child(self.block(format!("response-headers-{id}"), headers, None, cx))
            .child(self.label(t("requests.common.body"), cx))
            .child(self.block(format!("response-body-{id}"), result.response_body.clone(), None, cx))
            .into_any_element()
    }

    /// An opened row: Response, Pre script, Post script and Tests.
    fn render_details(&self, id: &str, result: &RowResult, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let tab = self.tabs.get(id).copied().unwrap_or(0);
        let key = id.to_owned();
        let content = match tab {
            0 => self.render_response(id, result, cx),
            1 => self.render_script(id, "pre", result.pre.as_ref(), result.pre_error.as_ref(), cx),
            2 => self.render_script(id, "post", result.post.as_ref(), result.post_error.as_ref(), cx),
            _ => self.render_tests(id, result, cx),
        };
        v_flex()
            .w_full()
            .gap_3()
            .pl_8()
            .pt_1()
            .pb_2()
            .child(
                TabBar::new(SharedString::from(format!("run-tabs-{id}")))
                    .underline()
                    .small()
                    .selected_index(tab)
                    .on_click(cx.listener(move |this, ix: &usize, _, cx| {
                        this.tabs.insert(key.clone(), *ix);
                        cx.notify();
                    }))
                    .child(Tab::new().label(t("requests.common.response")))
                    .child(Tab::new().label(t("requests.runner.tab_pre_script")))
                    .child(Tab::new().label(t("requests.runner.tab_post_script")))
                    .child(Tab::new().label(t_with(
                        "requests.runner.tab_tests",
                        &[
                            ("passed", &result.tests.iter().filter(|(_, ok, _)| *ok).count()),
                            ("total", &result.tests.len()),
                        ],
                    ))),
            )
            .child(content)
            .into_any_element()
    }

    /// One figure of the run, large, with an optional bar under it.
    fn stat_card(
        &self,
        id: &'static str,
        title: &str,
        value: String,
        color: gpui_kit::Hsla,
        note: String,
        bar: Option<f32>,
        cx: &Context<Self>,
    ) -> gpui_kit::AnyElement {
        let theme = cx.theme();
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(div().text_xs().text_color(theme.muted_foreground).child(title.to_uppercase()))
            .child(div().text_3xl().font_semibold().text_color(color).child(value))
            .child(div().text_xs().text_color(theme.muted_foreground).child(note))
            .children(bar.map(|percent| Progress::new(id).value(percent).color(color)))
            .into_any_element()
    }

    /// The run at a glance: requests sent, requests that failed, tests passed and how long it all took.
    fn render_stats(&self, cx: &Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let failed = self.results.values().filter(|result| result.is_failure()).count();
        let tests: usize = self.results.values().map(|result| result.tests.len()).sum();
        let passed: usize = self.results.values().map(|result| result.passed).sum();
        let elapsed: u64 = self.results.values().map(|result| result.duration_ms).sum();
        let percent = |part: usize, whole: usize| if whole == 0 { 0. } else { part as f32 * 100. / whole as f32 };
        let state = if self.running {
            t("requests.runner.state_running")
        } else if self.completed < self.total {
            t("requests.runner.state_stopped")
        } else {
            t("requests.runner.state_done")
        };
        h_flex()
            .flex_none()
            .w_full()
            .gap_3()
            .p_4()
            .border_b_1()
            .border_color(theme.border)
            .child(self.stat_card(
                "stat-sent",
                t("requests.runner.stat.sent"),
                format!("{}/{}", self.completed, self.total),
                theme.foreground,
                state.to_owned(),
                Some(percent(self.completed, self.total)),
                cx,
            ))
            .child(self.stat_card(
                "stat-failed",
                t("requests.runner.stat.failed"),
                failed.to_string(),
                if failed > 0 { theme.danger } else { theme.success },
                t_with("requests.runner.stat.failed_note", &[("count", &self.completed)]),
                Some(percent(failed, self.completed)),
                cx,
            ))
            .child(self.stat_card(
                "stat-tests",
                t("requests.runner.stat.tests"),
                format!("{passed}/{tests}"),
                if tests == passed { theme.success } else { theme.danger },
                if tests == 0 {
                    t("requests.runner.stat.no_tests").to_owned()
                } else {
                    t_with("requests.runner.stat.tests_failed", &[("count", &(tests - passed))])
                },
                Some(percent(passed, tests)),
                cx,
            ))
            .child(self.stat_card(
                "stat-time",
                t("requests.runner.stat.time"),
                t_with("requests.runner.duration", &[("ms", &elapsed)]),
                theme.foreground,
                t("requests.runner.stat.time_note").to_owned(),
                None,
                cx,
            ))
            .into_any_element()
    }
}

impl Render for FolderRunner {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let rows: Vec<RequestStub> = self.rows(cx).into_iter().cloned().collect();
        let last = rows.len().saturating_sub(1);
        let action = if self.running {
            Button::new("cancel-run")
                .label(t("requests.runner.stop"))
                .loading(self.cancelling)
                .disabled(self.cancelling)
                .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))
        } else {
            Button::new("start-run")
                .primary()
                .icon(IconName::Play)
                .label(t("requests.runner.run"))
                .disabled(rows.is_empty())
                .on_click(cx.listener(|this, _, _, cx| this.run(cx)))
        };
        let body = if rows.is_empty() {
            page::empty(t("requests.runner.empty.title"), t("requests.runner.empty.description"))
        } else {
            let rows = v_flex()
                .id("run-rows")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .children(rows.iter().enumerate().map(|(ix, request)| self.render_row(ix, last, request, cx)));
            let started = self.running || !self.results.is_empty();
            v_flex()
                .size_full()
                .children(started.then(|| self.render_stats(cx)))
                .child(rows)
                .into_any_element()
        };
        page::screen(page::header(self.title(cx), self.summary(rows.len()), action, cx), body, cx)
            .text_color(theme.foreground)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_keeps_its_tests_scripts_and_what_was_exchanged() {
        let result = RowResult::from_value(&json!({
            "status": 200, "responseTime": 12, 
            "tests": [{ "name": "ok", "passed": true }, { "name": "bad", "passed": false, "error": "expected 1" }],
            "testLogs": ["hello"],
            "scripts": { "pre": "", "post": "bc.environment.set('a', 1)", "test": "bc.test('ok', () => {})" },
            "responseHeaders": { "b": "2" }, "responseBody": "{}",
            "postScriptError": "boom",
        }));
        assert_eq!((result.passed, result.failed.len(), result.tests.len()), (1, 1, 2));
        assert_eq!(result.tests[1], ("bad".to_owned(), false, Some("expected 1".to_owned())));
        assert_eq!((result.pre.is_some(), result.post.is_some(), result.test_script.is_some()), (false, true, true));
        assert_eq!(result.post_error.as_deref(), Some("boom"));
        assert_eq!(result.logs, vec!["hello"]);
        assert_eq!(result.response_body, "{}");
        assert_eq!(result.errors, vec!["Post-response script: boom"]);
        assert!(result.is_failure());
    }

    #[test]
    fn a_test_is_found_in_its_script_by_name() {
        let script = "const x = 1;\nbc.test('a (b)', () => {\n  expect(f(')')).to.equal(1);\n});\ntest(\"second\", () => {});";
        assert_eq!(
            test_source(script, "a (b)").as_deref(),
            Some("bc.test('a (b)', () => {\n  expect(f(')')).to.equal(1);\n});")
        );
        assert_eq!(test_source(script, "second").as_deref(), Some("test(\"second\", () => {});"));
        assert_eq!(test_source(script, "missing"), None);
    }
}
