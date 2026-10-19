//! Contract check in the Mocks screen: `contract.check` sends the saved request paired with a mock and
//! compares the real answer's shape with the mock's. One mock is checked from its editor, all (or a folder)
//! from the header, and the answer is shown inline.

use std::{cell::Cell, collections::HashSet, rc::Rc};

use bifurc_engine_client::ClientError;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::{DialogButtonProps, DialogFooter},
    form::{Field, Form},
    h_flex,
    input::{Input, InputState},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::MocksScreen;
use crate::{
    engine::{call, failure},
    i18n::{t, t_with},
};

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Difference {
    pub kind: String,
    pub path: String,
    pub text: String,
    pub left: Option<String>,
    pub right: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Check {
    pub mock: String,
    pub mock_id: String,
    pub method: String,
    pub path: String,
    pub request: String,
    pub source: String,
    pub ok: bool,
    pub differences: Vec<Difference>,
    /// How many drift lines were left out.
    pub truncated: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Unpaired {
    pub mock: String,
    pub mock_id: String,
    pub method: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Report {
    pub drifted: u64,
    pub ok: u64,
    pub failed: u64,
    pub unpaired_count: u64,
    pub checks: Vec<Check>,
    pub unpaired: Vec<Unpaired>,
}

/// What the dialog before "Check all" asks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Options {
    pub from_capture: bool,
    pub from_snapshot: bool,
    pub ignore: String,
}

#[derive(Debug, Clone, Copy)]
pub enum Scope<'a> {
    Mock(&'a str),
    Folder(&'a str),
    All,
}

/// The `contract.check` payload: one mock, a folder or all, with the options and the active environment.
pub fn payload(workspace: &str, scope: Scope<'_>, options: &Options, environment: Option<&str>) -> Value {
    let mut p = json!({ "workspaceId": workspace });
    match scope {
        Scope::Mock(id) => p["mock"] = json!(id),
        Scope::Folder(id) => p["folder"] = json!(id),
        Scope::All => p["all"] = json!(true),
    }
    if options.from_capture {
        p["fromCapture"] = json!(true);
    }
    if options.from_snapshot {
        p["fromSnapshot"] = json!(true);
    }
    let ignore: Vec<&str> = options.ignore.split(',').map(str::trim).filter(|g| !g.is_empty()).collect();
    if !ignore.is_empty() {
        p["ignore"] = json!(ignore);
    }
    if let Some(environment) = environment {
        p["environmentId"] = json!(environment);
    }
    p
}

/// Where a run is, or what it ended with.
#[derive(Debug, Clone)]
pub enum Outcome {
    Running,
    Done(Result<Report, String>),
}

/// The screen's contract-check state: the latest run for one mock, and the latest run over many.
#[derive(Default)]
pub struct ContractState {
    single: Option<(String, Outcome)>,
    all: Option<Outcome>,
    expanded: HashSet<String>,
}

fn parse(outcome: Result<Value, ClientError>) -> Result<Report, String> {
    if let Some(message) = failure(&outcome) {
        return Err(message);
    }
    serde_json::from_value(outcome.unwrap_or_default()).map_err(|e| e.to_string())
}

impl MocksScreen {
    fn run_contract(&mut self, scope: Scope<'_>, options: &Options, cx: &mut Context<Self>) {
        let workspace = self.workspace(cx);
        if workspace.is_empty() {
            return;
        }
        let single = match scope {
            Scope::Mock(id) => Some(id.to_owned()),
            _ => None,
        };
        let environment = self.model.read(cx).config().active_environment_id.clone();
        let payload = payload(&workspace, scope, options, environment.as_deref());
        match &single {
            Some(id) => self.contract.single = Some((id.clone(), Outcome::Running)),
            None => self.contract.all = Some(Outcome::Running),
        }
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "contract.check", payload, move |this, outcome, cx| {
            let done = Outcome::Done(parse(outcome));
            match single {
                Some(id) => this.contract.single = Some((id, done)),
                None => this.contract.all = Some(done),
            }
            cx.notify();
        });
    }

    pub fn check_active_contract(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.active_editor().and_then(|e| e.read(cx).saved_id().map(str::to_owned)) else { return };
        self.run_contract(Scope::Mock(&id), &Options::default(), cx);
    }

    /// Asks for the options, then checks `folder` (or all mocks).
    pub fn prompt_contract(&mut self, folder: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let state = Rc::new((Cell::new(false), Cell::new(false)));
        let ignore = cx.new(|cx| InputState::new(window, cx).placeholder(t("contract.options.ignore_placeholder")));
        let view = cx.weak_entity();
        let title = if folder.is_some() { t("contract.options.folder_title") } else { t("contract.options.title") };
        window.defer(cx, move |window, cx| {
            window.open_dialog(cx, move |dialog, _, _| {
                let (state, ignore, view, folder) = (state.clone(), ignore.clone(), view.clone(), folder.clone());
                let run = Rc::new({
                    let (state, ignore) = (state.clone(), ignore.clone());
                    move |window: &mut Window, cx: &mut App| {
                        let options = Options {
                            from_capture: state.0.get(),
                            from_snapshot: state.1.get(),
                            ignore: ignore.read(cx).value().to_string(),
                        };
                        view.update(cx, |this, cx| match &folder {
                            Some(f) => this.run_contract(Scope::Folder(f), &options, cx),
                            None => this.run_contract(Scope::All, &options, cx),
                        })
                        .ok();
                        window.close_dialog(cx);
                    }
                });
                let (ok, button) = (run.clone(), run.clone());
                let (capture, snapshot) = (state.clone(), state.clone());
                dialog
                    .title(title)
                    .w_96()
                    .child(
                        Form::new()
                            .child(
                                Field::new().child(
                                    Checkbox::new("contract-from-capture")
                                        .small()
                                        .label(t("contract.options.from_capture"))
                                        .checked(state.0.get())
                                        .on_click(move |checked: &bool, window, _| {
                                            capture.0.set(*checked);
                                            window.refresh();
                                        }),
                                ),
                            )
                            .child(
                                Field::new().child(
                                    Checkbox::new("contract-from-snapshot")
                                        .small()
                                        .label(t("contract.options.from_snapshot"))
                                        .checked(state.1.get())
                                        .on_click(move |checked: &bool, window, _| {
                                            snapshot.1.set(*checked);
                                            window.refresh();
                                        }),
                                ),
                            )
                            .child(
                                Field::new().label(t("contract.options.ignore")).child(Input::new(&ignore).py_1p5()),
                            ),
                    )
                    .button_props(DialogButtonProps::default().on_ok(move |_, window, cx| {
                        ok(window, cx);
                        false
                    }))
                    .footer(
                        DialogFooter::new()
                            .child(
                                Button::new("contract-cancel")
                                    .outline()
                                    .label(t("components.cancel"))
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                            .child(
                                Button::new("contract-run")
                                    .primary()
                                    .label(t("contract.options.run"))
                                    .on_click(move |_, window, cx| button(window, cx)),
                            ),
                    )
            });
        });
    }

    /// The result for the open mock, under its bar, with the button that asks for it.
    pub fn render_contract_bar(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let id = self.active_editor()?.read(cx).saved_id()?.to_owned();
        let theme = cx.theme().clone();
        let state = self.contract.single.as_ref().filter(|(mock, _)| *mock == id).map(|(_, o)| o);
        let running = matches!(state, Some(Outcome::Running));
        let body = match state {
            None | Some(Outcome::Running) => None,
            Some(Outcome::Done(Err(message))) => Some(line_error(message, cx)),
            Some(Outcome::Done(Ok(report))) => Some(match report.checks.first() {
                Some(check) => check_detail(check, cx),
                None => {
                    let reason = report.unpaired.first().map_or_else(String::new, |u| u.reason.clone());
                    div()
                        .id("contract-unpaired")
                        .test_support()
                        .text_color(theme.muted_foreground)
                        .child(t_with("contract.unpaired", &[("reason", &reason)]))
                        .into_any_element()
                }
            }),
        };
        Some(
            v_flex()
                .id("mock-contract")
                .test_support()
                .flex_none()
                .gap_1()
                .px_3()
                .py_1()
                .text_sm()
                .border_b_1()
                .border_color(theme.border)
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("contract-check")
                                .small()
                                .outline()
                                .icon(IconName::CircleCheck)
                                .label(t("contract.check"))
                                .disabled(running)
                                .tooltip(t("contract.check_tooltip"))
                                .on_click(cx.listener(|this, _, _, cx| this.check_active_contract(cx))),
                        )
                        .children(running.then(|| {
                            h_flex()
                                .id("contract-busy")
                                .test_support()
                                .gap_1()
                                .text_color(theme.muted_foreground)
                                .child(Spinner::new())
                                .child(t("contract.running"))
                        })),
                )
                .children(body)
                .into_any_element(),
        )
    }

    /// The summary of the last run over many mocks, and a row for each check and each mock without a pair.
    pub fn render_contract_report(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let theme = cx.theme().clone();
        let outcome = self.contract.all.as_ref()?;
        let panel = v_flex().id("contract-report").test_support().flex_none().border_b_1().border_color(theme.border);
        let head = |text: String, cx: &mut Context<Self>| {
            h_flex()
                .px_3()
                .py_1()
                .gap_2()
                .text_sm()
                .child(div().id("contract-summary").test_support().flex_1().child(text))
                .child(Button::new("contract-close").ghost().small().icon(IconName::Close).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.contract.all = None;
                        cx.notify();
                    },
                )))
        };
        let report = match outcome {
            Outcome::Running => {
                return Some(panel.child(head(t("contract.running_all").to_owned(), cx)).into_any_element());
            }
            Outcome::Done(Err(message)) => {
                let text = t_with("contract.failed", &[("error", message)]);
                return Some(panel.child(head(text, cx)).into_any_element());
            }
            Outcome::Done(Ok(report)) => report,
        };
        let summary = t_with(
            "contract.summary",
            &[
                ("ok", &report.ok),
                ("drifted", &report.drifted),
                ("failed", &report.failed),
                ("unpaired", &report.unpaired_count),
            ],
        );
        let rows: Vec<_> = report
            .checks
            .iter()
            .map(|check| {
                let id = check.mock_id.clone();
                let open = self.contract.expanded.contains(&id);
                let mark = if check.error.is_some() {
                    ("!", theme.warning)
                } else if check.ok {
                    ("✓", theme.success)
                } else {
                    ("≠", theme.danger)
                };
                let (target, toggled) = (id.clone(), id.clone());
                v_flex()
                    .child(
                        h_flex()
                            .id(ElementId::Name(format!("contract-row-{id}").into()))
                            .test_support()
                            .gap_2()
                            .px_3()
                            .py_1()
                            .text_sm()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.muted))
                            .on_click(cx.listener(move |this, _, window, cx| this.open_mock(&target, window, cx)))
                            .child(
                                Button::new(ElementId::Name(format!("contract-toggle-{id}").into()))
                                    .ghost()
                                    .xsmall()
                                    .icon(if open { IconName::ChevronDown } else { IconName::ChevronRight })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        if !this.contract.expanded.remove(&toggled) {
                                            this.contract.expanded.insert(toggled.clone());
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(div().text_color(mark.1).child(mark.0))
                            .child(div().flex_1().min_w_0().truncate().child(check.mock.clone()))
                            .child(
                                div()
                                    .text_color(theme.muted_foreground)
                                    .child(format!("{} {}", check.method, check.path)),
                            ),
                    )
                    .children(open.then(|| div().pl_10().pr_3().pb_1().text_sm().child(check_detail(check, cx))))
            })
            .collect();
        let unpaired: Vec<_> = report
            .unpaired
            .iter()
            .map(|u| {
                let target = u.mock_id.clone();
                h_flex()
                    .id(ElementId::Name(format!("contract-unpaired-{}", u.mock_id).into()))
                    .test_support()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| this.open_mock(&target, window, cx)))
                    .child(div().flex_1().truncate().child(u.mock.clone()))
                    .child(t_with("contract.unpaired", &[("reason", &u.reason)]))
            })
            .collect();
        Some(
            panel
                .child(head(summary, cx))
                .child(v_flex().id("contract-rows").max_h_64().overflow_y_scroll().children(rows).children(unpaired))
                .into_any_element(),
        )
    }
}

fn line_error(message: &str, cx: &App) -> gpui_kit::AnyElement {
    div().id("contract-error").test_support().text_color(cx.theme().danger).child(message.to_owned()).into_any_element()
}

/// ✓ for a match, the drift lines otherwise, or the error.
fn check_detail(check: &Check, cx: &App) -> gpui_kit::AnyElement {
    let theme = cx.theme();
    if let Some(error) = &check.error {
        return line_error(error, cx);
    }
    if check.ok {
        return div()
            .id("contract-match")
            .test_support()
            .text_color(theme.success)
            .child(t_with("contract.matches", &[("request", &check.request), ("source", &check.source)]))
            .into_any_element();
    }
    v_flex()
        .id("contract-drift")
        .test_support()
        .font_family(theme.mono_font_family.clone())
        .children(check.differences.iter().map(|d| {
            let color = match d.kind.as_str() {
                "+" => theme.success,
                "-" => theme.danger,
                _ => theme.warning,
            };
            h_flex().gap_2().child(div().text_color(color).child(d.text.clone())).children(
                // The text of a changed value does not always carry both sides.
                d.left
                    .as_ref()
                    .zip(d.right.as_ref())
                    .filter(|_| !d.text.contains('→'))
                    .map(|(l, r)| div().text_color(theme.muted_foreground).child(format!("{l} → {r}"))),
            )
        }))
        .children((check.truncated > 0).then(|| {
            div().text_color(theme.muted_foreground).child(t_with("contract.truncated", &[("count", &check.truncated)]))
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_carries_scope_options_and_environment() {
        assert_eq!(
            payload("w1", Scope::Mock("m1"), &Options::default(), Some("e1")),
            json!({"workspaceId": "w1", "mock": "m1", "environmentId": "e1"})
        );
        let options = Options { from_capture: true, from_snapshot: false, ignore: " .a , ,.b ".into() };
        assert_eq!(
            payload("w1", Scope::Folder("f"), &options, None),
            json!({"workspaceId": "w1", "folder": "f", "fromCapture": true, "ignore": [".a", ".b"]})
        );
        assert_eq!(payload("w1", Scope::All, &Options::default(), None), json!({"workspaceId": "w1", "all": true}));
    }
}
