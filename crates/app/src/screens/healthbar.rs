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
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState},
    list::ListItem,
    menu::{DropdownMenu as _, PopupMenuItem},
    switch::Switch,
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div,
};
use serde_json::{Map, Value, json};

use crate::{
    app_model::{AppModel, ModelEvent},
    components::{
        Syntax,
        dialogs::{PromptField, confirm_danger, prompt},
        master_detail::list_over_detail,
        page,
    },
    engine::{call_in, describe, failure},
    i18n::{t, t_with},
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
                // A revert puts the file back; what the screen shows must follow it.
                ModelEvent::ConfigChanged => this.refresh_list(window, cx),
                _ => {}
            }
        })];
        let auto = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(AUTO_INTERVAL).await;
                if this.update_in(cx, |this, window, cx| this.check_matching(is_auto, false, window, cx)).is_err() {
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
        t_with("healthbar.status_line", &[("healthy", &healthy), ("total", &self.services.len())])
    }

    fn report(&mut self, what: &str, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| {
            model.notice(t_with("healthbar.report", &[("what", &what), ("message", &message)]), cx)
        });
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
                    this.check_matching(|_| true, false, window, cx);
                }
                Err(error) => this.report(t("healthbar.error_read"), describe(&error), cx),
            }
            cx.notify();
        });
    }

    /// Re-reads the saved list and keeps the screen as it is when nothing changed (no checks are started).
    fn refresh_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        if model.workspace_id().is_empty() {
            return;
        }
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id() });
        call_in(&client, window, cx, "healthbar.getServices", payload, |this, outcome, window, cx| {
            let Ok(result) = outcome else { return };
            let services: Vec<Map<String, Value>> = result
                .as_array()
                .map(|list| list.iter().filter_map(|service| service.as_object().cloned()).collect())
                .unwrap_or_default();
            if services == this.services {
                return;
            }
            this.services = services;
            let known: Vec<String> = this.services.iter().map(|s| text(s, "id")).collect();
            this.results.retain(|id, _| known.contains(id));
            if this.selected.as_ref().is_some_and(|id| !known.contains(id)) {
                this.selected = None;
            }
            for id in known {
                if !this.results.contains_key(&id) {
                    this.check(&id, false, window, cx);
                }
            }
            cx.notify();
        });
    }

    /// Commits the health checks file (and pushes, when the workspace has a remote).
    fn sync(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.publish(paths, cx));
    }

    fn confirm_revert(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            t("healthbar.revert_title"),
            t("healthbar.revert_body"),
            t("healthbar.revert"),
            move |this, _, cx| this.model.update(cx, |model, cx| model.revert_paths(paths.clone(), cx)),
        );
    }

    /// Stores the list. The engine's command takes the whole list, so that is what is sent.
    fn store(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "services": self.services });
        call_in(&client, window, cx, "healthbar.saveServices", payload, |this, outcome, window, cx| {
            if let Some(message) = failure(&outcome) {
                this.report(t("healthbar.error_save"), message, cx);
                this.reload(window, cx);
            }
        });
        cx.notify();
    }

    /// Checks every URL now.
    pub fn check_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.check_matching(|_| true, true, window, cx);
    }

    fn check_matching(
        &mut self,
        include: fn(&Map<String, Value>) -> bool,
        manual: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids: Vec<String> =
            self.services.iter().filter(|service| include(service)).map(|service| text(service, "id")).collect();
        for id in ids {
            self.check(&id, manual, window, cx);
        }
    }

    /// `manual` says the user asked for this check, rather than the screen running it on its own.
    fn check(&mut self, id: &str, manual: bool, window: &mut Window, cx: &mut Context<Self>) {
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
            let check = json!({ "url": resolved, "manual": manual });
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

    pub fn prompt_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t("healthbar.new_health_check"),
            t("healthbar.create"),
            vec![
                PromptField::new(t("healthbar.name"), t("healthbar.name_example")),
                PromptField::new(t("healthbar.url"), "{{HOST}}/health"),
            ],
            |this, values, window, cx| {
                let id = format!("hb{:x}", now_ms());
                let service = json!({
                    "id": id, "name": values[0], "url": values[1], "autoRefreshEnabled": false, "createdAt": now_ms(),
                });
                if let Value::Object(service) = service {
                    this.services.push(service);
                }
                this.store(window, cx);
                this.check(&id, false, window, cx);
            },
        );
    }

    fn prompt_edit(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(service) = self.services.iter().find(|s| text(s, "id") == id).cloned() else { return };
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t_with("healthbar.edit_title", &[("name", &text(&service, "name"))]),
            t("healthbar.save"),
            vec![
                PromptField::new(t("healthbar.name"), t("healthbar.name_example")).initial(text(&service, "name")),
                PromptField::new(t("healthbar.url"), "{{HOST}}/health").initial(text(&service, "url")),
            ],
            move |this, values, window, cx| {
                if let Some(service) = this.services.iter_mut().find(|s| text(s, "id") == id) {
                    service.insert("name".into(), json!(values[0]));
                    service.insert("url".into(), json!(values[1]));
                }
                this.store(window, cx);
                this.check(&id, false, window, cx);
            },
        );
    }

    fn confirm_delete(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.services.iter().find(|s| text(s, "id") == id).map(|s| text(s, "name")).unwrap_or_default();
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            t_with("healthbar.delete_title", &[("name", &name)]),
            t("healthbar.delete_body"),
            t("healthbar.delete"),
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
            None => Tag::secondary().small().child(t("healthbar.not_checked")),
            Some(Outcome::Checking) => Tag::secondary().small().child(t("healthbar.checking")),
            Some(Outcome::Done { ok: true, status, .. }) => {
                Tag::success().small().child(status.map_or(t("healthbar.state_healthy").to_owned(), |s| {
                    t_with("healthbar.state_healthy_status", &[("status", &s)])
                }))
            }
            Some(Outcome::Done { ok: false, status, .. }) => {
                Tag::danger().small().child(status.map_or(t("healthbar.state_failing").to_owned(), |s| {
                    t_with("healthbar.state_failing_status", &[("status", &s)])
                }))
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
                    Some(Outcome::Done { duration_ms, checked_at, .. }) => (
                        t_with("healthbar.timing", &[("ms", &duration_ms)]),
                        t_with("healthbar.checked_at", &[("time", &clock_utc(*checked_at))]),
                    ),
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
                                    .label(t("healthbar.auto"))
                                    .tooltip(t("healthbar.auto_tooltip"))
                                    .on_change(cx.listener(move |this, checked: &bool, window, cx| {
                                        cx.stop_propagation();
                                        this.set_auto(&auto_id, *checked, window, cx);
                                    })),
                            )
                            .child(Button::new(key("check")).small().label(t("healthbar.check")).on_click(cx.listener(
                                move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.check(&check_id, true, window, cx);
                                },
                            )))
                            .child(Button::new(key("actions")).icon(IconName::Ellipsis).ghost().small().dropdown_menu(
                                move |menu, _, _| {
                                    let (edit_view, delete_view) = (menu_view.clone(), menu_view.clone());
                                    let (edit_id, delete_id) = (edit_id.clone(), delete_id.clone());
                                    menu.item(PopupMenuItem::new(t("healthbar.edit_menu")).on_click(
                                        move |_, window, cx| {
                                            let id = edit_id.clone();
                                            edit_view.update(cx, |this, cx| this.prompt_edit(id, window, cx)).ok();
                                        },
                                    ))
                                    .separator()
                                    .item(
                                        PopupMenuItem::new(t("healthbar.delete_menu")).on_click(
                                            move |_, window, cx| {
                                                let id = delete_id.clone();
                                                delete_view
                                                    .update(cx, |this, cx| this.confirm_delete(id, window, cx))
                                                    .ok();
                                            },
                                        ),
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
            None => page::empty(t("healthbar.not_checked_yet"), t("healthbar.not_checked_yet_body")),
            Some(Outcome::Checking) => page::loading(t("healthbar.checking")),
            Some(Outcome::Done { error: Some(error), body, .. }) if body.is_empty() => {
                page::empty(t("healthbar.check_failed"), error.clone())
            }
            Some(Outcome::Done { body, .. }) if body.is_empty() => {
                page::empty(t("healthbar.empty_body"), t("healthbar.empty_body_text"))
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
                            .child(t_with("healthbar.last_response", &[("name", &name)])),
                    )
                    .child(Self::state_tag(outcome))
                    .child(
                        Button::new("health-close-detail")
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .tooltip(t("healthbar.close_response_tooltip"))
                            .on_click(cx.listener(|this, _, window, cx| this.select(None, window, cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}

impl Render for HealthBarScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let changed = model.git().rollup(&["healthbar/"]).changed;
        let busy = model.is_publishing();
        let (sync_paths, revert_paths) = (changed.clone(), changed.clone());
        let header = page::header(
            t("healthbar.title"),
            t("healthbar.description"),
            h_flex()
                .gap_2()
                .child(
                    Button::new("health-revert")
                        .small()
                        .label(t("healthbar.revert"))
                        .tooltip(t("healthbar.revert_tooltip"))
                        .disabled(changed.is_empty() || busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.confirm_revert(revert_paths.clone(), window, cx)
                        })),
                )
                .child(
                    Button::new("health-sync")
                        .small()
                        .label(t("healthbar.sync"))
                        .tooltip(t("healthbar.sync_tooltip"))
                        .loading(busy)
                        .disabled(changed.is_empty() || busy)
                        .on_click(cx.listener(move |this, _, _, cx| this.sync(sync_paths.clone(), cx))),
                )
                .child(
                    Button::new("health-check-all")
                        .small()
                        .icon(IconName::RefreshCw)
                        .label(t("healthbar.check_all"))
                        .on_click(cx.listener(|this, _, window, cx| this.check_matching(|_| true, true, window, cx))),
                )
                .child(
                    Button::new("health-new")
                        .small()
                        .icon(IconName::Plus)
                        .label(t("healthbar.new_check"))
                        .on_click(cx.listener(|this, _, window, cx| this.prompt_new(window, cx))),
                ),
            cx,
        );
        if !self.loaded {
            return page::screen(header, page::loading(t("healthbar.loading_checks")), cx);
        }
        if self.services.is_empty() {
            let empty = page::empty_with_action(
                t("healthbar.no_checks"),
                t("healthbar.no_checks_body"),
                Button::new("health-empty-new")
                    .label(t("healthbar.new_check"))
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_new(window, cx))),
            );
            return page::screen(header, empty, cx);
        }
        let detail = self.selected.clone().map(|id| self.render_detail(&id, cx));
        let body = list_over_detail("health-split", self.render_list(cx), detail);
        page::screen(header, body, cx)
    }
}
