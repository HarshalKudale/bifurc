//! The Environments screen: named sets of variables, and the global set every one of them layers over.
//!
//! The list is the model's snapshot. The detail pane holds an *edit* of the selected environment: its name
//! and variables as typed, compared with what the engine has to know whether there is anything to save.
//! Saving writes those two fields over the engine's copy by id.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::{Map, Value, json};

use crate::{
    app_model::AppModel,
    components::{
        KeyValueEvent, KeyValueTable,
        dialogs::{PromptField, confirm_danger, prompt},
        page,
    },
    engine::{call, failure, patch_entity},
    model::config::EnvironmentInfo,
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
    _subscriptions: Vec<Subscription>,
}

impl EnvironmentsScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Environment name"));
        let variables = cx.new(|cx| KeyValueTable::new("Variable", "Value", window, cx));
        let subscriptions = vec![
            cx.observe_in(&model, window, |this, _, window, cx| this.follow_model(window, cx)),
            cx.subscribe(&name, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe(&variables, |_, _, _: &KeyValueEvent, cx| cx.notify()),
        ];
        let mut this = Self {
            model,
            selected: None,
            name,
            variables,
            baseline: (String::new(), Vec::new()),
            saving: false,
            _subscriptions: subscriptions,
        };
        this.follow_model(window, cx);
        this
    }

    fn environment(&self, cx: &gpui_kit::App) -> Option<EnvironmentInfo> {
        let id = self.selected.as_deref()?;
        self.model.read(cx).config().environments.iter().find(|e| e.id == id).cloned()
    }

    fn current(&self, cx: &gpui_kit::App) -> Edit {
        (self.name.read(cx).value().trim().to_owned(), self.variables.read(cx).pairs(cx))
    }

    pub fn is_dirty(&self, cx: &gpui_kit::App) -> bool {
        self.selected.is_some() && self.current(cx) != self.baseline
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

    /// Keeps the detail pane on something that exists, and refreshes it when the engine's copy changed and
    /// there is no edit here to lose.
    fn follow_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = self.model.read(cx).config().clone();
        let selected = self.selected.as_deref().and_then(|id| config.environments.iter().find(|e| e.id == id));
        match selected {
            Some(environment) if !self.is_dirty(cx) => {
                let environment = environment.clone();
                self.load(&environment, window, cx);
            }
            Some(_) => cx.notify(),
            None => {
                let fallback = config.active_environment().or_else(|| config.environments.first()).cloned();
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

    fn select(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_deref() == Some(id.as_str()) {
            return;
        }
        let Some(target) = self.model.read(cx).config().environments.iter().find(|e| e.id == id).cloned() else {
            return;
        };
        if !self.is_dirty(cx) {
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

    pub fn revert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(environment) = self.environment(cx) {
            self.load(&environment, window, cx);
        }
    }

    pub fn save(&mut self, _: &mut Window, cx: &mut Context<Self>) {
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
        patch_entity(&client, cx, &workspace_id, "environments", &environment.id, patch, move |this, outcome, cx| {
            this.saving = false;
            match outcome {
                // What was saved is the new baseline; the snapshot that follows will agree with it.
                Ok(()) => this.baseline = saved,
                Err(message) => this.report("Couldn't save the environment.", message, cx),
            }
            cx.notify();
        });
    }

    fn prompt_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            format!("Delete environment “{}”?", environment.name),
            "Its variables are deleted. Requests that use them will send the names unresolved.",
            "Delete",
            move |this, _, cx| {
                let client = this.model.read(cx).client().clone();
                // Nothing here is worth keeping once the environment is gone.
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

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div().flex_1().min_w_0().font_medium().child(Input::new(&self.name).small().appearance(false)),
                    )
                    .when(!environment.is_global(), |this| {
                        this.child(
                            Button::new("use-environment")
                                .ghost()
                                .small()
                                .label(if is_active { "In use" } else { "Use" })
                                .disabled(is_active)
                                .tooltip("Resolve variables against this environment")
                                .on_click(move |_, _, cx| {
                                    model.update(cx, |m, cx| m.set_active_environment(Some(&use_id), cx))
                                }),
                        )
                        .child(
                            Button::new("delete-environment")
                                .ghost()
                                .small()
                                .icon(IconName::Delete)
                                .tooltip("Delete this environment…")
                                .on_click(cx.listener(|this, _, window, cx| this.confirm_delete(window, cx))),
                        )
                    })
                    .child(
                        Button::new("revert-environment")
                            .ghost()
                            .small()
                            .label("Revert")
                            .disabled(!dirty)
                            .on_click(cx.listener(|this, _, window, cx| this.revert(window, cx))),
                    )
                    .child(
                        Button::new("save-environment")
                            .small()
                            .label("Save")
                            .loading(self.saving)
                            .disabled(!dirty)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(self.variables.clone()))
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
