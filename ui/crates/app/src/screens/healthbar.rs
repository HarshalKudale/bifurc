//! The Health bar screen: a list of URLs and whether each one answers.
//!
//! The list is the workspace's (`healthbar.getServices` / `healthbar.saveServices`). A check is the engine's
//! too: the URL's variables are resolved (`env.resolveText`) and the engine calls it (`healthbar.checkUrl`).
//! This screen keeps only the last result of each check, which is not saved anywhere.

use std::{
    collections::HashMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState},
    list::ListItem,
    menu::{DropdownMenu as _, PopupMenuItem},
    resizable::{resizable_panel, v_resizable},
    switch::Switch,
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, px,
};
use serde_json::{Map, Value, json};

use crate::{
    app_model::{AppModel, ModelEvent},
    components::{
        Syntax,
        dialogs::{PromptField, confirm_danger, prompt},
        page,
    },
    engine::{call_in, describe, failure},
    model::{http::pretty_json, traffic::clock_utc},
};

/// How often the checks marked "Auto" run.
const AUTO_INTERVAL: Duration = Duration::from_secs(30);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    Checking,
    Done { ok: bool, status: Option<u16>, duration_ms: u64, body: String, error: Option<String>, checked_at: u64 },
}

pub struct HealthBarScreen {
    model: Entity<AppModel>,
    /// The checks as the engine stores them: `{ id, name, url, autoRefreshEnabled, createdAt }`.
    services: Vec<Map<String, Value>>,
    loaded: bool,
    results: HashMap<String, Outcome>,
    selected: Option<String>,
    body: Entity<EditorState>,
    _auto: Task<()>,
    _subscriptions: Vec<Subscription>,
}

fn text(service: &Map<String, Value>, key: &str) -> String {
    service.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn is_auto(service: &Map<String, Value>) -> bool {
    service.get("autoRefreshEnabled").and_then(Value::as_bool).unwrap_or(false)
}

impl HealthBarScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let body = cx.new(|cx| {
            let mut state =
                EditorState::new(window, cx).language(Syntax::Plain.language()).line_number(true).soft_wrap(true);
            state.set_readonly(true, cx);
            state
        });
        let subscriptions = vec![cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| {
            match event {
                ModelEvent::WorkspaceSwitched => {
                    this.results.clear();
                    this.selected = None;
                    this.reload(window, cx);
                }
                // The screen is built before the workspace is known; the first snapshot is when it can load.
                ModelEvent::ConfigChanged if !this.loaded => this.reload(window, cx),
                _ => {}
            }
        })];
        let auto = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(AUTO_INTERVAL).await;
                if this.update_in(cx, |this, window, cx| this.check_matching(is_auto, window, cx)).is_err() {
                    return;
                }
            }
        });
        let mut this = Self {
            model,
            services: Vec::new(),
            loaded: false,
            results: HashMap::new(),
            selected: None,
            body,
            _auto: auto,
            _subscriptions: subscriptions,
        };
        this.reload(window, cx);
        this
    }

    pub fn status_line(&self) -> String {
        if self.services.is_empty() {
            return String::new();
        }
        let healthy = self
            .services
            .iter()
            .filter(|service| matches!(self.results.get(&text(service, "id")), Some(Outcome::Done { ok: true, .. })))
            .count();
        format!("{healthy} of {} healthy", self.services.len())
    }

    fn report(&mut self, what: &str, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(format!("{what} {message}"), cx));
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        if model.workspace_id().is_empty() {
            return;
        }
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id() });
        call_in(&client, window, cx, "healthbar.getServices", payload, |this, outcome, window, cx| {
            match outcome {
                Ok(result) => {
                    this.services = result
                        .as_array()
                        .map(|list| list.iter().filter_map(|service| service.as_object().cloned()).collect())
                        .unwrap_or_default();
                    this.loaded = true;
                    // The first look at the list checks everything once.
                    this.check_matching(|_| true, window, cx);
                }
                Err(error) => this.report("Couldn't read the health checks.", describe(&error), cx),
            }
            cx.notify();
        });
    }

    /// Stores the list. The engine's command takes the whole list, so that is what is sent.
    fn store(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "services": self.services });
        call_in(&client, window, cx, "healthbar.saveServices", payload, |this, outcome, window, cx| {
            if let Some(message) = failure(&outcome) {
                this.report("Couldn't save the health checks.", message, cx);
                this.reload(window, cx);
            }
        });
        cx.notify();
    }

    fn check_matching(
        &mut self,
        include: fn(&Map<String, Value>) -> bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids: Vec<String> =
            self.services.iter().filter(|service| include(service)).map(|service| text(service, "id")).collect();
        for id in ids {
            self.check(&id, window, cx);
        }
    }

    fn check(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(url) = self.services.iter().find(|s| text(s, "id") == id).map(|s| text(s, "url")) else { return };
        if self.results.get(id) == Some(&Outcome::Checking) {
            return;
        }
        self.results.insert(id.to_owned(), Outcome::Checking);
        cx.notify();
        let model = self.model.read(cx);
        let client = model.client().clone();
        let check_client = client.clone();
        let id = id.to_owned();
        // Variables are the engine's to resolve; the check then calls what they resolved to.
        let resolve = json!({ "workspaceId": model.workspace_id(), "text": url });
        call_in(&client, window, cx, "env.resolveText", resolve, move |_, outcome, window, cx| {
            let resolved = outcome
                .ok()
                .and_then(|result| result.get("text").and_then(Value::as_str).map(str::to_owned))
                .unwrap_or(url);
            let check = json!({ "url": resolved });
            call_in(&check_client, window, cx, "healthbar.checkUrl", check, move |this, outcome, window, cx| {
                let done = match outcome {
                    Ok(result) => Outcome::Done {
                        ok: result.get("ok").and_then(Value::as_bool).unwrap_or(false),
                        status: result.get("statusCode").and_then(Value::as_u64).and_then(|s| u16::try_from(s).ok()),
                        duration_ms: result.get("durationMs").and_then(Value::as_u64).unwrap_or(0),
                        body: result.get("body").and_then(Value::as_str).unwrap_or_default().to_owned(),
                        error: result.get("error").and_then(Value::as_str).map(str::to_owned),
                        checked_at: now_ms(),
                    },
                    Err(error) => Outcome::Done {
                        ok: false,
                        status: None,
                        duration_ms: 0,
                        body: String::new(),
                        error: Some(describe(&error)),
                        checked_at: now_ms(),
                    },
                };
                // The detail pane may be showing this check's previous response.
                let showing = this.selected.as_deref() == Some(id.as_str());
                this.results.insert(id, done);
                if showing {
                    this.show_body(window, cx);
                }
                cx.notify();
            });
        });
    }

    fn prompt_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            "New health check",
            "Create",
            vec![PromptField::new("Name", "Users API"), PromptField::new("URL", "{{HOST}}/health")],
            |this, values, window, cx| {
                let id = format!("hb{:x}", now_ms());
                let service = json!({
                    "id": id, "name": values[0], "url": values[1], "autoRefreshEnabled": false, "createdAt": now_ms(),
                });
                if let Value::Object(service) = service {
                    this.services.push(service);
                }
                this.store(window, cx);
                this.check(&id, window, cx);
            },
        );
    }

    fn prompt_edit(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(service) = self.services.iter().find(|s| text(s, "id") == id).cloned() else { return };
        prompt(
            cx.weak_entity(),
            window,
            cx,
            format!("Edit “{}”", text(&service, "name")),
            "Save",
            vec![
                PromptField::new("Name", "Users API").initial(text(&service, "name")),
                PromptField::new("URL", "{{HOST}}/health").initial(text(&service, "url")),
            ],
            move |this, values, window, cx| {
                if let Some(service) = this.services.iter_mut().find(|s| text(s, "id") == id) {
                    service.insert("name".into(), json!(values[0]));
                    service.insert("url".into(), json!(values[1]));
                }
                this.store(window, cx);
                this.check(&id, window, cx);
            },
        );
    }

    fn confirm_delete(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.services.iter().find(|s| text(s, "id") == id).map(|s| text(s, "name")).unwrap_or_default();
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            format!("Delete health check “{name}”?"),
            "It will no longer be checked.",
            "Delete",
            move |this, window, cx| {
                this.services.retain(|service| text(service, "id") != id);
                this.results.remove(&id);
                if this.selected.as_deref() == Some(id.as_str()) {
                    this.selected = None;
                }
                this.store(window, cx);
            },
        );
    }

    fn set_auto(&mut self, id: &str, auto: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(service) = self.services.iter_mut().find(|s| text(s, "id") == id) {
            service.insert("autoRefreshEnabled".into(), json!(auto));
        }
        self.store(window, cx);
    }

    fn select(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = id;
        self.show_body(window, cx);
        cx.notify();
    }

    /// Puts the selected check's last response into the read-only editor.
    fn show_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let body = match self.selected.as_ref().and_then(|id| self.results.get(id)) {
            Some(Outcome::Done { body, .. }) => body.clone(),
            _ => String::new(),
        };
        let (shown, syntax) = match pretty_json(&body) {
            Some(pretty) => (pretty, Syntax::Json),
            None => (body, Syntax::Plain),
        };
        self.body.update(cx, |state, cx| {
            state.set_highlighter(syntax.language(), cx);
            state.set_value(shown, window, cx);
        });
    }

    fn state_tag(outcome: Option<&Outcome>) -> Tag {
        match outcome {
            None => Tag::secondary().small().child("Not checked"),
            Some(Outcome::Checking) => Tag::secondary().small().child("Checking…"),
            Some(Outcome::Done { ok: true, status, .. }) => {
                Tag::success().small().child(status.map_or("Healthy".to_owned(), |s| format!("Healthy · {s}")))
            }
            Some(Outcome::Done { ok: false, status, .. }) => {
                Tag::danger().small().child(status.map_or("Failing".to_owned(), |s| format!("Failing · {s}")))
            }
        }
    }

    fn render_list(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let this = cx.weak_entity();
        let rows: Vec<gpui_kit::AnyElement> = self
            .services
            .iter()
            .map(|service| {
                let id = text(service, "id");
                let outcome = self.results.get(&id);
                let (timing, checked) = match outcome {
                    Some(Outcome::Done { duration_ms, checked_at, .. }) => {
                        (format!("{duration_ms} ms"), format!("{} UTC", clock_utc(*checked_at)))
                    }
                    _ => (String::new(), String::new()),
                };
                let key = |part: &str| gpui_kit::ElementId::Name(format!("health-{part}-{id}").into());
                let (select_id, check_id, auto_id) = (id.clone(), id.clone(), id.clone());
                let (menu_view, edit_id, delete_id) = (this.clone(), id.clone(), id.clone());
                ListItem::new(key("row"))
                    .selected(self.selected.as_deref() == Some(id.as_str()))
                    .px_4()
                    .py_2()
                    .text_sm()
                    .child(
                        h_flex()
                            .gap_3()
                            // A tag hugs its text; the lane around it keeps the column.
                            .child(h_flex().w_32().flex_none().child(Self::state_tag(outcome)))
                            .child(
                                div().w_56().flex_none().overflow_hidden().text_ellipsis().child(text(service, "name")),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .font_family(theme.mono_font_family.clone())
                                    .text_color(theme.muted_foreground)
                                    .child(text(service, "url")),
                            )
                            .child(
                                div().w_20().flex_none().text_right().text_color(theme.muted_foreground).child(timing),
                            )
                            .child(div().w_32().flex_none().text_color(theme.muted_foreground).child(checked))
                            .child(
                                Switch::new(key("auto"))
                                    .checked(is_auto(service))
                                    .small()
                                    .label("Auto")
                                    .tooltip("Check every 30 seconds")
                                    .on_change(cx.listener(move |this, checked: &bool, window, cx| {
                                        cx.stop_propagation();
                                        this.set_auto(&auto_id, *checked, window, cx);
                                    })),
                            )
                            .child(Button::new(key("check")).small().label("Check").on_click(cx.listener(
                                move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.check(&check_id, window, cx);
                                },
                            )))
                            .child(Button::new(key("actions")).icon(IconName::Ellipsis).ghost().small().dropdown_menu(
                                move |menu, _, _| {
                                    let (edit_view, delete_view) = (menu_view.clone(), menu_view.clone());
                                    let (edit_id, delete_id) = (edit_id.clone(), delete_id.clone());
                                    menu.item(PopupMenuItem::new("Edit…").on_click(move |_, window, cx| {
                                        let id = edit_id.clone();
                                        edit_view.update(cx, |this, cx| this.prompt_edit(id, window, cx)).ok();
                                    }))
                                    .separator()
                                    .item(
                                        PopupMenuItem::new("Delete…").on_click(move |_, window, cx| {
                                            let id = delete_id.clone();
                                            delete_view.update(cx, |this, cx| this.confirm_delete(id, window, cx)).ok();
                                        }),
                                    )
                                },
                            )),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.select(Some(select_id.clone()), window, cx)))
                    .into_any_element()
            })
            .collect();
        v_flex().id("health-rows").size_full().overflow_y_scroll().children(rows).into_any_element()
    }

    fn render_detail(&self, id: &str, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let name = self.services.iter().find(|s| text(s, "id") == id).map(|s| text(s, "name")).unwrap_or_default();
        let outcome = self.results.get(id);
        let content = match outcome {
            None => page::empty("Not checked yet", "Run the check to see what the URL answers."),
            Some(Outcome::Checking) => page::loading("Checking…"),
            Some(Outcome::Done { error: Some(error), body, .. }) if body.is_empty() => {
                page::empty("The check failed", error.clone())
            }
            Some(Outcome::Done { body, .. }) if body.is_empty() => {
                page::empty("Empty body", "The URL answered without a body.")
            }
            Some(Outcome::Done { .. }) => Editor::new(&self.body).bordered(false).h_full().into_any_element(),
        };
        v_flex()
            .size_full()
            .bg(theme.background)
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(format!("Last response · {name}")),
                    )
                    .child(Self::state_tag(outcome))
                    .child(
                        Button::new("health-close-detail")
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .tooltip("Close the response")
                            .on_click(cx.listener(|this, _, window, cx| this.select(None, window, cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}

impl Render for HealthBarScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = page::header(
            "Health bar",
            "URLs the engine checks on demand, or every 30 seconds when set to Auto. {{variables}} are resolved.",
            h_flex()
                .gap_2()
                .child(
                    Button::new("health-check-all")
                        .small()
                        .icon(IconName::RefreshCw)
                        .label("Check all")
                        .on_click(cx.listener(|this, _, window, cx| this.check_matching(|_| true, window, cx))),
                )
                .child(
                    Button::new("health-new")
                        .small()
                        .icon(IconName::Plus)
                        .label("New check")
                        .on_click(cx.listener(|this, _, window, cx| this.prompt_new(window, cx))),
                ),
            cx,
        );
        if !self.loaded {
            return page::screen(header, page::loading("Loading health checks…"), cx);
        }
        if self.services.is_empty() {
            let empty = page::empty_with_action(
                "No health checks",
                "Add the URLs you want to keep an eye on, such as each service's /health.",
                Button::new("health-empty-new")
                    .label("New check")
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_new(window, cx))),
            );
            return page::screen(header, empty, cx);
        }
        let detail = self.selected.clone().map(|id| self.render_detail(&id, cx));
        let has_detail = detail.is_some();
        // The split's size is a physical starting point the user resizes, not part of the rem scale.
        let body = v_resizable("health-split").child(resizable_panel().child(self.render_list(cx))).child(
            resizable_panel().visible(has_detail).size(px(320.)).size_range(px(160.)..px(900.)).children(detail),
        );
        page::screen(header, body, cx)
    }
}
