//! A folder opened as a tab, to run every request in it in order.
//!
//! The run is the engine's: `runner.start` hands it over, `event.runner.progress` and
//! `event.runner.finished` say how it goes, and `runner.status` answers for a run that started before this
//! tab was open. The order is the folder's saved runner configuration (`requestOrder`), written back with
//! `runner.saveConfig` when a request is moved. This view keeps what it was told and draws it.

use std::collections::{HashMap, HashSet};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    spinner::Spinner,
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::{Map, Value, json};

use crate::{
    app_model::{AppModel, ModelEvent},
    components::page,
    engine::{call, describe, failure},
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
    /// The pre-request, post-response and test scripts the request has.
    scripts: Vec<(&'static str, String)>,
    /// What the scripts logged.
    logs: Vec<String>,
    method: String,
    url: String,
    request_headers: Vec<(String, String)>,
    request_body: String,
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

impl RowResult {
    fn from_value(value: &Value) -> Self {
        let text = |key: &str| value.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
        let tests = value.get("tests").and_then(Value::as_array).cloned().unwrap_or_default();
        let failed = tests
            .iter()
            .filter(|test| test.get("passed").and_then(Value::as_bool) != Some(true))
            .map(|test| {
                let name = test.get("name").and_then(Value::as_str).unwrap_or("Unnamed test");
                match test.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()) {
                    Some(error) => format!("{name}: {error}"),
                    None => name.to_owned(),
                }
            })
            .collect::<Vec<_>>();
        let own = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        let scripts = [("Pre-request script", "pre"), ("Post-response script", "post"), ("Test script", "test")]
            .into_iter()
            .filter_map(|(label, key)| {
                let source = value.pointer(&format!("/scripts/{key}")).and_then(Value::as_str)?;
                (!source.trim().is_empty()).then(|| (label, source.to_owned()))
            })
            .collect();
        Self {
            tests: tests
                .iter()
                .map(|test| {
                    (
                        test.get("name").and_then(Value::as_str).unwrap_or("Unnamed test").to_owned(),
                        test.get("passed").and_then(Value::as_bool) == Some(true),
                        test.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()).map(str::to_owned),
                    )
                })
                .collect(),
            scripts,
            logs: text_list(value.get("testLogs")),
            method: own("method"),
            url: own("url"),
            request_headers: header_list(value.get("requestHeaders")),
            request_body: own("requestBody"),
            response_headers: header_list(value.get("responseHeaders")),
            response_body: own("responseBody"),
            status: value.get("status").and_then(Value::as_u64).and_then(|s| u16::try_from(s).ok()),
            duration_ms: value.get("responseTime").and_then(Value::as_f64).map_or(0, |ms| ms.max(0.) as u64),
            errors: [
                text("error"),
                text("preScriptError").map(|e| format!("Pre-request script: {e}")),
                text("postScriptError").map(|e| format!("Post-response script: {e}")),
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
            .folder(RequestKind::Rest, &self.folder_id)
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
                this.report(format!("Couldn't save the run order. {message}"), cx);
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
                    Ok(result) => result.get("error").and_then(Value::as_str).unwrap_or("The engine refused.").to_owned(),
                    Err(error) => describe(error),
                };
                this.set_running(false, cx);
                this.report(format!("Couldn't start the run. {message}"), cx);
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
                this.report(format!("Couldn't stop the run. {message}"), cx);
                cx.notify();
            }
        });
    }

    /// One line about the run: how far it is, or how it went.
    fn summary(&self, count: usize) -> String {
        let requests = format!("{count} request{}", if count == 1 { "" } else { "s" });
        if self.results.is_empty() && !self.running {
            return format!("{requests}, sent in this order.");
        }
        let failed = self.results.values().filter(|result| result.is_failure()).count();
        let tests: usize = self.results.values().map(|result| result.passed + result.failed.len()).sum();
        let tests_failed: usize = self.results.values().map(|result| result.failed.len()).sum();
        let mut line = if self.running {
            format!("Running: {} of {} sent", self.completed, self.total)
        } else if self.completed < self.total {
            format!("Stopped after {} of {}", self.completed, self.total)
        } else {
            format!("{} sent", self.completed)
        };
        line.push_str(&format!(" · {failed} failed"));
        if tests > 0 {
            line.push_str(&format!(" · {}/{tests} tests passed", tests - tests_failed));
        }
        line
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
            None => Tag::danger().small().child("No response"),
        };
        let tests = result.passed + result.failed.len();
        h_flex()
            .gap_2()
            .text_xs()
            .text_color(theme.muted_foreground)
            .children((tests > 0).then(|| {
                let text = format!("{}/{tests} tests", result.passed);
                if result.failed.is_empty() {
                    Tag::success().small().child(text)
                } else {
                    Tag::danger().small().child(text)
                }
            }))
            .child(format!("{} ms", result.duration_ms))
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
                            .tooltip(if open { "Hide details" } else { "Show scripts, tests and the response" })
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
                                    .tooltip("Run earlier")
                                    .disabled(self.running || ix == 0)
                                    .on_click(cx.listener(move |this, _, _, cx| this.move_row(ix, true, cx))),
                            )
                            .child(
                                Button::new("move-down")
                                    .icon(IconName::ChevronDown)
                                    .ghost()
                                    .xsmall()
                                    .tooltip("Run later")
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
            .children(self.results.get(&id).filter(|_| open).map(|result| self.render_details(result, cx)))
            .into_any_element()
    }

    fn block(&self, text: String, tint: Option<gpui_kit::Hsla>, cx: &Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme();
        div()
            .w_full()
            .max_h_64()
            .overflow_hidden()
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

    fn section(&self, title: String, body: gpui_kit::AnyElement, cx: &Context<Self>) -> gpui_kit::AnyElement {
        v_flex()
            .w_full()
            .gap_1()
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(title))
            .child(body)
            .into_any_element()
    }

    /// What one request did: its tests, what went wrong, the scripts that ran, and what was sent and received.
    fn render_details(&self, result: &RowResult, cx: &Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let pairs = |headers: &[(String, String)]| {
            headers.iter().map(|(key, value)| format!("{key}: {value}")).collect::<Vec<_>>().join("\n")
        };
        let joined = |parts: [String; 3]| {
            parts.into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join("\n\n")
        };
        let mut sections: Vec<gpui_kit::AnyElement> = Vec::new();
        if !result.errors.is_empty() {
            sections.push(self.section("Errors".into(), self.block(result.errors.join("\n"), Some(theme.danger), cx), cx));
        }
        if result.tests.is_empty() {
            let none = div().text_sm().text_color(theme.muted_foreground).child("No tests ran.");
            sections.push(self.section("Tests".into(), none.into_any_element(), cx));
        } else {
            let passed = result.tests.iter().filter(|(_, ok, _)| *ok).count();
            let list = v_flex().gap_1().children(result.tests.iter().map(|(name, ok, error)| {
                let (mark, color) = if *ok { ("\u{2713}", theme.success) } else { ("\u{2717}", theme.danger) };
                v_flex()
                    .child(h_flex().gap_2().text_sm().child(div().text_color(color).child(mark)).child(name.clone()))
                    .children(error.as_ref().map(|error| {
                        div()
                            .pl_5()
                            .text_xs()
                            .text_color(theme.danger)
                            .font_family(theme.mono_font_family.clone())
                            .child(error.clone())
                    }))
            }));
            let title = format!("Tests, {passed}/{} passed", result.tests.len());
            sections.push(self.section(title, list.into_any_element(), cx));
        }
        for (label, source) in &result.scripts {
            sections.push(self.section((*label).into(), self.block(source.clone(), None, cx), cx));
        }
        if !result.logs.is_empty() {
            sections.push(self.section("Console".into(), self.block(result.logs.join("\n"), None, cx), cx));
        }
        let request = joined([
            format!("{} {}", result.method, result.url),
            pairs(&result.request_headers),
            result.request_body.clone(),
        ]);
        sections.push(self.section("Request".into(), self.block(request, None, cx), cx));
        if result.status.is_some() {
            let response = joined([pairs(&result.response_headers), result.response_body.clone(), String::new()]);
            sections.push(self.section("Response".into(), self.block(response, None, cx), cx));
        }
        v_flex().w_full().gap_3().pl_8().pt_1().pb_2().children(sections).into_any_element()
    }
}

impl Render for FolderRunner {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let rows: Vec<RequestStub> = self.rows(cx).into_iter().cloned().collect();
        let last = rows.len().saturating_sub(1);
        let action = if self.running {
            Button::new("cancel-run")
                .label("Stop")
                .loading(self.cancelling)
                .disabled(self.cancelling)
                .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))
        } else {
            Button::new("start-run")
                .primary()
                .icon(IconName::Play)
                .label("Run folder")
                .disabled(rows.is_empty())
                .on_click(cx.listener(|this, _, _, cx| this.run(cx)))
        };
        let body = if rows.is_empty() {
            page::empty("Nothing to run", "This folder has no requests in it. Folders inside it are run on their own.")
        } else {
            v_flex()
                .id("run-rows")
                .size_full()
                .overflow_y_scroll()
                .children(rows.iter().enumerate().map(|(ix, request)| self.render_row(ix, last, request, cx)))
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
            "status": 200, "responseTime": 12, "method": "GET", "url": "http://a/b",
            "tests": [{ "name": "ok", "passed": true }, { "name": "bad", "passed": false, "error": "expected 1" }],
            "testLogs": ["hello"],
            "scripts": { "pre": "", "post": "lp.environment.set('a', 1)", "test": "lp.test('ok', () => {})" },
            "requestHeaders": { "a": "1" }, "responseHeaders": { "b": "2" }, "responseBody": "{}",
            "postScriptError": "boom",
        }));
        assert_eq!((result.passed, result.failed.len(), result.tests.len()), (1, 1, 2));
        assert_eq!(result.tests[1], ("bad".to_owned(), false, Some("expected 1".to_owned())));
        let labels: Vec<&str> = result.scripts.iter().map(|(label, _)| *label).collect();
        assert_eq!(labels, vec!["Post-response script", "Test script"]);
        assert_eq!(result.logs, vec!["hello"]);
        assert_eq!(result.request_headers, vec![("a".to_owned(), "1".to_owned())]);
        assert_eq!(result.response_body, "{}");
        assert_eq!(result.errors, vec!["Post-response script: boom"]);
        assert!(result.is_failure());
    }
}
