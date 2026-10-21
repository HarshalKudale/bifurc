//! The Environments screen: named sets of variables, and the global set every one of them layers over.
//!
//! The list is the model's snapshot. The detail pane holds an *edit* of the selected environment: its name
//! and variables as typed, compared with what the engine has to know whether there is anything to save.
//! There is no Save step: an edit is written over the engine's copy by id a moment after it is made, or at
//! once when another environment is selected. The name is edited in the window's breadcrumb, which shows
//! this screen's name field.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{InputEvent, InputState},
    list::ListItem,
    select::{Select, SelectEvent, SelectState},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription,
    Window, div,
};
use serde_json::{Map, Value, json};

use crate::{
    app_model::{AppModel, Deleted, ModelEvent},
    components::{
        Autosave, EntityAction, EntityBar, KeyValueEvent, KeyValueTable,
        dialogs::{PromptField, confirm_danger, delete_consequence, needs_update, prompt, resolve_deleted},
        entity_bar,
        master_detail::{list_rows, master_detail},
        page,
    },
    engine::{call, failure, patch_entity},
    i18n::{t, t_with},
    model::{
        config::{ConfigSnapshot, EnvLogin, EnvironmentInfo},
        kind::RequestKind,
    },
    session::SaveMode,
    shell::ActiveTarget,
};

/// The name, the variables and the login as the controls hold them.
type Edit = (String, Vec<(String, String)>, Option<EnvLogin>);

/// A request or flow the login can be.
#[derive(Debug, Clone, PartialEq)]
struct LoginOption {
    flow: bool,
    id: String,
    label: String,
}

/// What the login picker lists: the workspace's REST requests (the only ones the engine can log in with) and
/// its flows, and `held` too when it names one that is no longer there, so saving does not drop it.
fn login_options(config: &ConfigSnapshot, held: Option<&EnvLogin>) -> Vec<LoginOption> {
    let mut requests: Vec<LoginOption> = config
        .requests
        .iter()
        .filter(|r| r.kind == RequestKind::Rest && !r.deleted && r.locked.is_none())
        .map(|r| LoginOption { flow: false, id: r.id.clone(), label: r.name.clone() })
        .collect();
    let mut flows: Vec<LoginOption> = config
        .flows
        .iter()
        .filter(|f| !f.deleted && f.locked.is_none())
        .map(|f| LoginOption {
            flow: true,
            id: f.id.clone(),
            label: t_with("environments.login_flow", &[("name", &f.name)]),
        })
        .collect();
    requests.sort_by_key(|o| o.label.to_lowercase());
    flows.sort_by_key(|o| o.label.to_lowercase());
    requests.extend(flows);
    if let Some(held) = held
        && !requests.iter().any(|o| o.flow == held.flow && o.id == held.id)
    {
        requests.push(LoginOption {
            flow: held.flow,
            id: held.id.clone(),
            label: t_with("environments.login_missing", &[("id", &held.id)]),
        });
    }
    requests
}

pub struct EnvironmentsScreen {
    model: Entity<AppModel>,
    selected: Option<String>,
    name: Entity<InputState>,
    variables: Entity<KeyValueTable>,
    /// The request or flow that logs in, chosen among `login_options` (same order).
    login: Entity<SelectState<Vec<String>>>,
    login_options: Vec<LoginOption>,
    /// Log in again and send once more when a send answers 401. Only meaningful with a login.
    retry: bool,
    /// The selected environment as the controls showed it when it was last loaded or saved.
    baseline: Edit,
    saving: bool,
    autosave: Autosave,
    /// The selected environment was reverted on disk: the next snapshot is shown whatever the controls hold.
    reload_due: bool,
    _subscriptions: Vec<Subscription>,
}

impl EnvironmentsScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t("environments.name_placeholder")));
        let variables =
            cx.new(|cx| KeyValueTable::new(t("environments.variable"), t("environments.value"), window, cx));
        let login = cx.new(|cx| SelectState::new(Vec::<String>::new(), None, window, cx));
        let subscriptions = vec![
            cx.observe_in(&model, window, |this, _, window, cx| this.follow_model(false, window, cx)),
            cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| match event {
                ModelEvent::EntityReverted(id) if this.selected.as_deref() == Some(id.as_str()) => {
                    // An edit waiting to be written belongs to the version that was just replaced.
                    this.autosave.cancel();
                    this.reload_due = true;
                }
                ModelEvent::ConfigChanged if this.reload_due => {
                    this.reload_due = false;
                    this.follow_model(true, window, cx);
                }
                _ => {}
            }),
            cx.subscribe(&name, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.changed(cx);
                }
            }),
            cx.subscribe(&variables, |this, _, _: &KeyValueEvent, cx| this.changed(cx)),
            cx.subscribe(&login, |this, _, _: &SelectEvent<Vec<String>>, cx| this.changed(cx)),
        ];
        let mut this = Self {
            model,
            selected: None,
            name,
            variables,
            login,
            login_options: Vec::new(),
            retry: false,
            baseline: (String::new(), Vec::new(), None),
            saving: false,
            autosave: Autosave::new(window),
            reload_due: false,
            _subscriptions: subscriptions,
        };
        this.follow_model(false, window, cx);
        this
    }

    fn environment(&self, cx: &gpui_kit::App) -> Option<EnvironmentInfo> {
        let id = self.selected.as_deref()?;
        self.model.read(cx).config().environments.iter().find(|e| e.id == id).cloned()
    }

    fn current(&self, cx: &gpui_kit::App) -> Edit {
        (self.name.read(cx).value().trim().to_owned(), self.variables.read(cx).pairs(cx), self.current_login(cx))
    }

    /// The login the picker shows, with whether it retries on 401.
    fn current_login(&self, cx: &gpui_kit::App) -> Option<EnvLogin> {
        let row = self.login.read(cx).selected_index(cx)?.row;
        let option = self.login_options.get(row)?;
        Some(EnvLogin { flow: option.flow, id: option.id.clone(), retry_on_401: self.retry })
    }

    /// Fills the picker from the workspace and shows `login` in it. The list is only replaced when it changed,
    /// so a menu that is open is not closed by an unrelated snapshot.
    fn show_login(&mut self, login: Option<&EnvLogin>, window: &mut Window, cx: &mut Context<Self>) {
        let options = login_options(self.model.read(cx).config(), login);
        if options != self.login_options {
            let labels: Vec<String> = options.iter().map(|o| o.label.clone()).collect();
            self.login.update(cx, |select, cx| select.set_items(labels, window, cx));
            self.login_options = options;
        }
        let row = login.and_then(|l| self.login_options.iter().position(|o| o.flow == l.flow && o.id == l.id));
        self.login.update(cx, |select, cx| match row {
            Some(row) => select.set_selected_index(Some(IndexPath::default().row(row)), window, cx),
            None => select.set_selected_index(None, window, cx),
        });
        self.retry = login.is_some_and(|l| l.retry_on_401);
    }

    /// What the controls hold once `environment` is shown in them, to compare with the baseline without
    /// showing it.
    fn engine_edit(environment: &EnvironmentInfo) -> Edit {
        let pairs = environment
            .variables
            .iter()
            .map(|v| (v.key.trim().to_owned(), v.value.clone()))
            .filter(|(key, _)| !key.is_empty())
            .collect();
        (environment.name.trim().to_owned(), pairs, environment.login.clone())
    }

    pub fn is_dirty(&self, cx: &gpui_kit::App) -> bool {
        self.selected.is_some() && self.current(cx) != self.baseline
    }

    /// The selected environment, for the window's breadcrumb. The shell shows this screen's name field there.
    pub fn active_target(&self, cx: &gpui_kit::App) -> Option<ActiveTarget> {
        let id = self.selected.clone()?;
        let name = self.name.read(cx).value();
        let title = if name.is_empty() { SharedString::from(t("environments.untitled")) } else { name };
        Some(ActiveTarget::named(title, Some(id), self.name.clone()))
    }

    /// An environment is written a moment after each change.
    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        if self.selected.is_some() {
            self.autosave.schedule(cx, |this, window, cx| this.save(SaveMode::Auto, window, cx));
        }
    }

    /// Writes an edit that is still waiting, now: another environment is about to be shown.
    fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.autosave.cancel();
        self.save(SaveMode::Auto, window, cx);
    }

    /// Shows `environment` in the detail pane as the engine has it.
    fn load(&mut self, environment: &EnvironmentInfo, window: &mut Window, cx: &mut Context<Self>) {
        let pairs: Vec<(String, String)> =
            environment.variables.iter().map(|v| (v.key.clone(), v.value.clone())).collect();
        self.selected = Some(environment.id.clone());
        self.name.update(cx, |state, cx| {
            state.set_value(environment.name.clone(), window, cx);
            // The global environment is the engine's own; it has no name to change.
            state.set_readonly(environment.is_global(), cx);
        });
        self.variables.update(cx, |table, cx| table.set_pairs(&pairs, window, cx));
        self.show_login(environment.login.as_ref(), window, cx);
        self.baseline = self.current(cx);
        cx.notify();
    }

    /// Keeps the detail pane on something that exists. The controls are only written when the engine's copy
    /// differs from what they were last shown or saved as, and there is no edit here to lose: every autosave
    /// makes the model change, and writing the controls would move the cursor under the user. `force` shows
    /// the engine's copy whatever the controls hold.
    fn follow_model(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let config = self.model.read(cx).config().clone();
        // A deleted environment has nothing to edit, nor has one a newer Bifurc saved: the selection leaves it.
        let selected = self
            .selected
            .as_deref()
            .and_then(|id| config.environments.iter().find(|e| e.id == id && !e.deleted && e.locked.is_none()));
        match selected {
            Some(environment) => {
                let differs = Self::engine_edit(environment) != self.baseline;
                if force || (differs && !self.saving && !self.is_dirty(cx)) {
                    let environment = environment.clone();
                    self.load(&environment, window, cx);
                } else {
                    // Requests or flows came or went: the picker lists them, keeping what it shows.
                    let held = self.current_login(cx);
                    if login_options(&config, held.as_ref()) != self.login_options {
                        self.show_login(held.as_ref(), window, cx);
                    }
                    cx.notify();
                }
            }
            None => {
                let fallback = config
                    .active_environment()
                    .filter(|e| !e.deleted && e.locked.is_none())
                    .or_else(|| config.environments.iter().find(|e| !e.deleted && e.locked.is_none()))
                    .cloned();
                match fallback {
                    Some(environment) => self.load(&environment, window, cx),
                    None => {
                        self.selected = None;
                        cx.notify();
                    }
                }
            }
        }
    }

    pub fn select(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_deref() == Some(id.as_str()) {
            return;
        }
        let Some(target) = self.model.read(cx).config().environments.iter().find(|e| e.id == id).cloned() else {
            return;
        };
        // A deleted environment is brought back or deleted for good, not edited.
        if target.deleted {
            let deleted = Deleted::Entity { kind: "environments", id: target.id.clone() };
            resolve_deleted(self.model.clone(), deleted, target.name.clone(), window, cx);
            return;
        }
        // Neither is one a newer Bifurc saved: the engine does not read it until this one is updated.
        if let Some(version) = target.locked.clone() {
            needs_update(target.name.clone(), version, window, cx);
            return;
        }
        if !self.is_dirty(cx) {
            self.load(&target, window, cx);
            return;
        }
        // A write that is already in flight cannot also carry this edit, so that one is asked about.
        if !self.saving {
            self.flush(window, cx);
            self.load(&target, window, cx);
            return;
        }
        let name = self.baseline.0.clone();
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            t_with("environments.discard_title", &[("name", &name)]),
            t("environments.discard_body"),
            t("environments.discard_action"),
            move |this, window, cx| this.load(&target, window, cx),
        );
    }

    fn report(&mut self, what: &str, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| {
            model.notice(t_with("environments.report", &[("what", &what), ("message", &message)]), cx)
        });
    }

    /// What the footer of the detail pane asks for. There is no draft here, so Save has nothing to do.
    pub fn entity_action(&mut self, action: EntityAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            EntityAction::Save | EntityAction::Close | EntityAction::Move(_) => {}
            EntityAction::Sync => {
                if let Some(id) = self.selected.clone() {
                    self.model.update(cx, |model, cx| model.sync_entity(&id, cx));
                }
            }
            EntityAction::Revert => {
                if let Some(id) = self.selected.clone() {
                    let name = self.baseline.0.clone();
                    confirm_danger(
                        cx.weak_entity(),
                        window,
                        cx,
                        t_with("environments.revert_title", &[("name", &name)]),
                        t("environments.revert_body"),
                        t("environments.revert"),
                        move |this, _, cx| this.model.update(cx, |model, cx| model.revert_entity(&id, cx)),
                    );
                }
            }
            EntityAction::Delete => self.confirm_delete(window, cx),
        }
    }

    /// Writes the edit over the saved environment. A write that is asked for while another is in flight is
    /// not lost: the one in flight looks for a newer edit when it lands.
    fn save(&mut self, _: SaveMode, _: &mut Window, cx: &mut Context<Self>) {
        let Some(environment) = self.environment(cx) else { return };
        if self.saving || !self.is_dirty(cx) {
            return;
        }
        let (name, pairs, login) = self.current(cx);
        // A variable keeps its id when it stays in place; a new row gets a fresh one.
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let variables: Vec<Value> = pairs
            .iter()
            .enumerate()
            .map(|(ix, (key, value))| {
                let id = environment
                    .variables
                    .get(ix)
                    .map(|v| v.id.clone())
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(|| format!("var-{stamp}-{ix}"));
                json!({ "id": id, "key": key, "value": value })
            })
            .collect();
        let mut patch = Map::new();
        if !environment.is_global() && !name.is_empty() {
            patch.insert("name".into(), json!(name));
        }
        patch.insert("variables".into(), Value::Array(variables));
        // The key is left out while the login is as it was, so a stored file without one stays as it is, and
        // one with fields this does not show keeps them. Taking the login away is `null`.
        if login != self.baseline.2 {
            patch.insert(
                "auth".into(),
                match &login {
                    Some(login) => json!({
                        "login": { "kind": if login.flow { "flow" } else { "request" }, "id": login.id },
                        "retryOn401": login.retry_on_401,
                    }),
                    None => Value::Null,
                },
            );
        }

        self.saving = true;
        cx.notify();
        let model = self.model.read(cx);
        let client = model.client().clone();
        let workspace_id = model.workspace_id().to_owned();
        let saved = self.current(cx);
        let environment_id = environment.id.clone();
        patch_entity(&client, cx, &workspace_id, "environments", &environment.id, patch, move |this, outcome, cx| {
            this.saving = false;
            match outcome {
                Ok(()) => {
                    // The controls are left alone: they may hold what was typed while this was in flight. What
                    // was sent is the new baseline, so that newer text still reads as an edit. If another
                    // environment was selected meanwhile, the controls and baseline are its.
                    if this.selected.as_deref() == Some(environment_id.as_str()) {
                        this.baseline = saved;
                    }
                }
                Err(message) => {
                    this.report(t("environments.error_save"), message, cx);
                    cx.notify();
                    return;
                }
            }
            // An edit made since this write was sent is the next write. (`changed` schedules it.)
            this.changed(cx);
        });
    }

    pub fn prompt_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t("environments.new_environment"),
            t("environments.create"),
            vec![PromptField::new(t("environments.name"), t("environments.name_example"))],
            |this, values, _, cx| {
                let model = this.model.read(cx);
                let client = model.client().clone();
                let payload = json!({
                    "kind": "environments",
                    "workspaceId": model.workspace_id(),
                    "entity": { "name": values[0], "variables": [] },
                });
                call(&client, cx, "entity.create", payload, |this, outcome, cx| match failure(&outcome) {
                    Some(message) => this.report(t("environments.error_create"), message, cx),
                    None => {
                        // Selecting it is deferred to the snapshot that lists it.
                        if let Some(id) =
                            outcome.ok().and_then(|r| r.get("id").and_then(Value::as_str).map(str::to_owned))
                            && !this.is_dirty(cx)
                        {
                            this.selected = Some(id);
                        }
                    }
                });
            },
        );
    }

    fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(environment) = self.environment(cx).filter(|e| !e.is_global()) else { return };
        let id = environment.id.clone();
        let consequence = delete_consequence(self.model.read(cx).git().entity_state(&id));
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            t_with("environments.delete_title", &[("name", &environment.name)]),
            consequence,
            t("environments.delete"),
            move |this, _, cx| {
                let client = this.model.read(cx).client().clone();
                // Nothing here is worth keeping once the environment is gone.
                this.autosave.cancel();
                this.baseline = this.current(cx);
                call(&client, cx, "entity.delete", json!({ "kind": "environments", "id": id }), |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.report(t("environments.error_delete"), message, cx);
                    }
                });
            },
        );
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let config = self.model.read(cx).config();
        let active = config.active_environment_id.clone();
        // The global set first: it applies under whichever environment is selected.
        let mut environments: Vec<&EnvironmentInfo> = config.environments.iter().collect();
        environments.sort_by_key(|e| (!e.is_global(), e.name.to_lowercase()));

        list_rows("environment-list").children(environments.into_iter().map(|environment| {
            let id = environment.id.clone();
            let is_active = active.as_deref() == Some(environment.id.as_str());
            ListItem::new(gpui_kit::ElementId::Name(format!("environment-{}", environment.id).into()))
                .selected(self.selected.as_deref() == Some(environment.id.as_str()))
                .px_2()
                .py_1()
                .text_sm()
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .when(environment.deleted, |this| {
                                    this.line_through().text_color(theme.muted_foreground)
                                })
                                .when(environment.locked.is_some(), |this| this.text_color(theme.muted_foreground))
                                .child(environment.name.clone()),
                        )
                        // Saved by a newer Bifurc: the sign for an update. Opening it says which version.
                        .when(environment.locked.is_some(), |this| {
                            this.child(Icon::new(IconName::ArrowUp).small().text_color(theme.muted_foreground))
                        })
                        .when(environment.is_global(), |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(t("environments.always_applied")),
                            )
                        })
                        .when(is_active, |this| this.child(Tag::secondary().small().child(t("environments.active")))),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.select(id.clone(), window, cx)))
        }))
    }

    /// The request or flow that logs in, and whether a 401 runs it and sends again.
    fn render_login(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let has_login = self.login.read(cx).selected_index(cx).is_some();
        v_flex()
            .flex_none()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_3()
                    .child(div().flex_none().text_sm().child(t("environments.login")))
                    .child(
                        div().flex_1().min_w_0().child(
                            Select::new(&self.login)
                                .small()
                                .id("environment-login")
                                .placeholder(t("environments.login_none"))
                                .cleanable(true),
                        ),
                    )
                    .child(
                        Checkbox::new("environment-retry-401")
                            .small()
                            .label(t("environments.retry_on_401"))
                            .checked(self.retry)
                            .disabled(!has_login)
                            .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                this.retry = *checked;
                                this.changed(cx);
                            })),
                    ),
            )
            .child(div().text_xs().text_color(theme.muted_foreground).child(t("environments.login_help")))
            .into_any_element()
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(environment) = self.environment(cx) else {
            return page::empty(t("environments.no_environment"), t("environments.no_environment_body"));
        };
        let theme = cx.theme().clone();
        let dirty = self.is_dirty(cx);
        let is_active = self.model.read(cx).config().active_environment_id.as_deref() == Some(environment.id.as_str());
        let model = self.model.clone();
        let use_id = environment.id.clone();
        let bar = EntityBar {
            saving: self.saving,
            unsaved: dirty,
            problem: None,
            closable: false,
            deletable: !environment.is_global(),
            ..EntityBar::of(self.model.read(cx), Some(environment.id.as_str()))
        };

        v_flex()
            .size_full()
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .when(!environment.is_global(), |this| {
                        this.child(
                            h_flex().flex_none().px_3().py_1().border_b_1().border_color(theme.border).child(
                                Button::new("use-environment")
                                    .ghost()
                                    .small()
                                    .label(if is_active { t("environments.use_in_use") } else { t("environments.use") })
                                    .disabled(is_active)
                                    .tooltip(t("environments.use_tooltip"))
                                    .on_click(move |_, _, cx| {
                                        model.update(cx, |m, cx| m.set_active_environment(Some(&use_id), cx))
                                    }),
                            ),
                        )
                    })
                    .children((!environment.is_global()).then(|| self.render_login(cx)))
                    .child(div().flex_1().min_h_0().child(self.variables.clone())),
            )
            .child(entity_bar(bar, cx, |this, action, window, cx| this.entity_action(action, window, cx)))
            .into_any_element()
    }
}

impl Render for EnvironmentsScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = page::header(
            t("environments.title"),
            t("environments.description"),
            Button::new("new-environment")
                .small()
                .icon(IconName::Plus)
                .label(t("environments.new_environment"))
                .on_click(cx.listener(|this, _, window, cx| this.prompt_new(window, cx))),
            cx,
        );
        let list = self.render_list(cx).into_any_element();
        let detail = self.render_detail(cx);
        let body = master_detail(Some(list), detail, cx);
        page::screen(header, body, cx)
    }
}
