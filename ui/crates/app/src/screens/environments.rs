//! The Environments screen: named sets of variables, and the global set every one of them layers over.
//!
//! The list is the model's snapshot. The detail pane holds an *edit* of the selected environment: its name
//! and variables as typed, compared with what the engine has to know whether there is anything to save.
//! There is no Save step: an edit is written over the engine's copy by id a moment after it is made, or at
//! once when another environment is selected. The name is edited in the window's breadcrumb, which shows
//! this screen's name field.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{InputEvent, InputState},
    list::ListItem,
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::{Map, Value, json};

use crate::{
    app_model::{AppModel, Deleted, ModelEvent},
    components::{
        Autosave, EntityAction, EntityBar, KeyValueEvent, KeyValueTable,
        dialogs::{PromptField, confirm_danger, delete_consequence, prompt, resolve_deleted},
        entity_bar, page,
    },
    engine::{call, failure, patch_entity},
    model::config::EnvironmentInfo,
    session::SaveMode,
    shell::ActiveTarget,
};

type Edit = (String, Vec<(String, String)>);

pub struct EnvironmentsScreen {
    model: Entity<AppModel>,
    selected: Option<String>,
    name: Entity<InputState>,
    variables: Entity<KeyValueTable>,
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
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Environment name"));
        let variables = cx.new(|cx| KeyValueTable::new("Variable", "Value", window, cx));
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
        ];
        let mut this = Self {
            model,
            selected: None,
            name,
            variables,
            baseline: (String::new(), Vec::new()),
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
        (self.name.read(cx).value().trim().to_owned(), self.variables.read(cx).pairs(cx))
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
        (environment.name.trim().to_owned(), pairs)
    }

    pub fn is_dirty(&self, cx: &gpui_kit::App) -> bool {
        self.selected.is_some() && self.current(cx) != self.baseline
    }

    /// The selected environment, for the window's breadcrumb. The shell shows this screen's name field there.
    pub fn active_target(&self, cx: &gpui_kit::App) -> Option<ActiveTarget> {
        let id = self.selected.clone()?;
        let name = self.name.read(cx).value();
        let title = if name.is_empty() { SharedString::from("Environment") } else { name };
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
        self.baseline = self.current(cx);
        cx.notify();
    }

    /// Keeps the detail pane on something that exists. The controls are only written when the engine's copy
    /// differs from what they were last shown or saved as, and there is no edit here to lose: every autosave
    /// makes the model change, and writing the controls would move the cursor under the user. `force` shows
    /// the engine's copy whatever the controls hold.
    fn follow_model(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let config = self.model.read(cx).config().clone();
        // A deleted environment has nothing to edit: the selection leaves it.
        let selected =
            self.selected.as_deref().and_then(|id| config.environments.iter().find(|e| e.id == id && !e.deleted));
        match selected {
            Some(environment) => {
                let differs = Self::engine_edit(environment) != self.baseline;
                if force || (differs && !self.saving && !self.is_dirty(cx)) {
                    let environment = environment.clone();
                    self.load(&environment, window, cx);
                } else {
                    cx.notify();
                }
            }
            None => {
                let fallback = config
                    .active_environment()
                    .filter(|e| !e.deleted)
                    .or_else(|| config.environments.iter().find(|e| !e.deleted))
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
            format!("Discard changes to “{name}”?"),
            "The edits to this environment have not been saved.",
            "Discard changes",
            move |this, window, cx| this.load(&target, window, cx),
        );
    }

    fn report(&mut self, what: &str, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(format!("{what} {message}"), cx));
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
                        format!("Revert “{name}” to the last synced version?"),
                        "The changes saved since then are discarded.",
                        "Revert",
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
        let (name, pairs) = self.current(cx);
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
                    this.report("Couldn't save the environment.", message, cx);
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
            "New environment",
            "Create",
            vec![PromptField::new("Name", "Staging")],
            |this, values, _, cx| {
                let model = this.model.read(cx);
                let client = model.client().clone();
                let payload = json!({
                    "kind": "environments",
                    "workspaceId": model.workspace_id(),
                    "entity": { "name": values[0], "variables": [] },
                });
                call(&client, cx, "entity.create", payload, |this, outcome, cx| match failure(&outcome) {
                    Some(message) => this.report("Couldn't create the environment.", message, cx),
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
            format!("Delete environment “{}”?", environment.name),
            consequence,
            "Delete",
            move |this, _, cx| {
                let client = this.model.read(cx).client().clone();
                // Nothing here is worth keeping once the environment is gone.
                this.autosave.cancel();
                this.baseline = this.current(cx);
                call(&client, cx, "entity.delete", json!({ "kind": "environments", "id": id }), |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.report("Couldn't delete the environment.", message, cx);
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

        v_flex()
            .id("environment-list")
            .w_64()
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .p_2()
            .gap_1()
            .children(environments.into_iter().map(|environment| {
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
                                    .child(environment.name.clone()),
                            )
                            .when(environment.is_global(), |this| {
                                this.child(div().text_xs().text_color(theme.muted_foreground).child("Always applied"))
                            })
                            .when(is_active, |this| this.child(Tag::secondary().small().child("Active"))),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.select(id.clone(), window, cx)))
            }))
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(environment) = self.environment(cx) else {
            return page::empty("No environment", "Create an environment to give your requests variables.");
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
                                    .label(if is_active { "In use" } else { "Use" })
                                    .disabled(is_active)
                                    .tooltip("Resolve variables against this environment")
                                    .on_click(move |_, _, cx| {
                                        model.update(cx, |m, cx| m.set_active_environment(Some(&use_id), cx))
                                    }),
                            ),
                        )
                    })
                    .child(div().flex_1().min_h_0().child(self.variables.clone())),
            )
            .child(entity_bar(bar, cx, |this, action, window, cx| this.entity_action(action, window, cx)))
            .into_any_element()
    }
}

impl Render for EnvironmentsScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = page::header(
            "Environments",
            "Sets of variables that {{name}} references in requests resolve against. Global applies under all of them.",
            Button::new("new-environment")
                .small()
                .icon(IconName::Plus)
                .label("New environment")
                .on_click(cx.listener(|this, _, window, cx| this.prompt_new(window, cx))),
            cx,
        );
        let body = h_flex()
            .size_full()
            .items_stretch()
            .child(self.render_list(cx))
            .child(div().flex_1().min_w_0().child(self.render_detail(cx)));
        page::screen(header, body, cx)
    }
}
