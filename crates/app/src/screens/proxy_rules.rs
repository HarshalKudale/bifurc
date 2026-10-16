//! The Proxy rules screen: which traffic through the forward proxy is sent somewhere else.
//!
//! A rule matches a URL pattern and sends the request to one of the workspace's mappings or to another
//! server, optionally running a script on the request and on the response. The list is read from the engine
//! (`entity.list`); the detail pane holds an *edit* of the selected rule, saved over the engine's copy by id.
//!
//! A new rule is a draft until Save creates it. A saved rule has no Save step: an edit is written a moment
//! after it is made, or at once when another rule is selected. The rule's name is edited in the window's
//! breadcrumb, which shows this screen's name field.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::Button,
    checkbox::Checkbox,
    h_flex,
    input::{Editor, Input, InputEvent, InputState},
    list::ListItem,
    menu::{DropdownMenu as _, PopupMenuItem},
    switch::Switch,
    tab::{Tab, TabBar},
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
        Autosave, EntityAction, EntityBar, Syntax, VarEditor,
        dialogs::{confirm_danger, delete_consequence, needs_update, resolve_deleted},
        entity_bar,
        master_detail::{list_rows, master_detail},
        page,
    },
    engine::{call, call_in, describe, failure},
    i18n::{t, t_with},
    session::SaveMode,
    shell::ActiveTarget,
};

const DEFAULT_NAME: &str = "New rule";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Mapping,
    External,
}

impl Target {
    fn wire(self) -> &'static str {
        match self {
            Target::Mapping => "mapping",
            Target::External => "external",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Fields {
    name: String,
    pattern: String,
    use_regex: bool,
    enabled: bool,
    target: Target,
    mapping_id: String,
    external: String,
    request_script: String,
    response_script: String,
}

/// Which rule the detail pane shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    None,
    Saved(String),
    /// A rule that has not been saved yet.
    Draft,
}

pub struct ProxyRulesScreen {
    model: Entity<AppModel>,
    /// The rules as the engine last listed them.
    rules: Vec<Map<String, Value>>,
    loaded: bool,
    selection: Selection,
    name: Entity<InputState>,
    pattern: Entity<InputState>,
    use_regex: bool,
    enabled: bool,
    target: Target,
    mapping_id: String,
    external: Entity<InputState>,
    request_script: VarEditor,
    response_script: VarEditor,
    /// 0 shows the request script, 1 the response script.
    script_ix: usize,
    baseline: Fields,
    saving: bool,
    autosave: Autosave,
    /// The selected rule was reverted on disk: the next list is shown whatever the controls hold.
    reload_due: bool,
    _subscriptions: Vec<Subscription>,
}

fn text(rule: &Map<String, Value>, key: &str) -> String {
    rule.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn flag(rule: &Map<String, Value>, key: &str) -> bool {
    rule.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// What the controls hold once `rule` is shown in them, to compare with the baseline without showing it.
fn engine_fields(rule: &Map<String, Value>) -> Fields {
    Fields {
        name: text(rule, "name").trim().to_owned(),
        pattern: text(rule, "pattern").trim().to_owned(),
        use_regex: flag(rule, "useRegex"),
        enabled: rule.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        target: if text(rule, "targetType") == "mapping" { Target::Mapping } else { Target::External },
        mapping_id: text(rule, "targetMappingId"),
        external: text(rule, "targetExternal").trim().to_owned(),
        request_script: text(rule, "requestScript"),
        response_script: text(rule, "responseScript"),
    }
}

/// Why a rule cannot be written as it stands, if it cannot.
fn problem_of(fields: &Fields) -> Option<&'static str> {
    if fields.pattern.is_empty() {
        Some(t("proxy_rules.problem.pattern"))
    } else if fields.target == Target::Mapping && fields.mapping_id.is_empty() {
        Some(t("proxy_rules.problem.mapping"))
    } else if fields.target == Target::External && fields.external.is_empty() {
        Some(t("proxy_rules.problem.external"))
    } else {
        None
    }
}

impl ProxyRulesScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t("proxy_rules.field.name_placeholder")));
        let pattern = cx.new(|cx| InputState::new(window, cx).placeholder("api.example.com/v1"));
        let external = cx.new(|cx| InputState::new(window, cx).placeholder("http://localhost:9000"));
        let request_script =
            VarEditor::code(Syntax::JavaScript, t("proxy_rules.script.request_placeholder"), window, cx);
        let response_script =
            VarEditor::code(Syntax::JavaScript, t("proxy_rules.script.response_placeholder"), window, cx);

        let mut subscriptions = vec![cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| {
            match event {
                ModelEvent::ConfigChanged | ModelEvent::WorkspaceSwitched => this.reload(window, cx),
                ModelEvent::EntityReverted(id) if this.selection == Selection::Saved(id.clone()) => {
                    // An edit waiting to be written belongs to the version that was just replaced.
                    this.autosave.cancel();
                    this.reload_due = true;
                    this.reload(window, cx);
                }
                _ => {}
            }
        })];
        subscriptions.push(cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.changed(cx),
            InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit_name(window, cx),
            InputEvent::Focus => {}
        }));
        for input in [&pattern, &external] {
            subscriptions.push(cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.changed(cx);
                }
            }));
        }
        for editor in [&request_script, &response_script] {
            subscriptions.push(cx.subscribe(editor.state(), |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.changed(cx);
                }
            }));
        }

        let mut this = Self {
            model,
            rules: Vec::new(),
            loaded: false,
            selection: Selection::None,
            name,
            pattern,
            use_regex: false,
            enabled: true,
            target: Target::External,
            mapping_id: String::new(),
            external,
            request_script,
            response_script,
            script_ix: 0,
            baseline: Fields {
                name: String::new(),
                pattern: String::new(),
                use_regex: false,
                enabled: true,
                target: Target::External,
                mapping_id: String::new(),
                external: String::new(),
                request_script: String::new(),
                response_script: String::new(),
            },
            saving: false,
            autosave: Autosave::new(window),
            reload_due: false,
            _subscriptions: subscriptions,
        };
        this.reload(window, cx);
        this
    }

    /// Reads the rules again. The workspace snapshot only says how many there are.
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        if model.workspace_id().is_empty() {
            return;
        }
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "kind": "proxyRules" });
        call_in(&client, window, cx, "entity.list", payload, |this, outcome, window, cx| {
            match outcome {
                Ok(result) => {
                    this.rules = result
                        .get("entities")
                        .and_then(Value::as_array)
                        .map(|rules| rules.iter().filter_map(|rule| rule.as_object().cloned()).collect())
                        .unwrap_or_default();
                    this.rules.sort_by_key(|rule| text(rule, "name").to_lowercase());
                    this.loaded = true;
                    this.follow_rules(window, cx);
                }
                Err(error) => this.report("proxy_rules.error.read", describe(&error), cx),
            }
            cx.notify();
        });
    }

    /// Rules that are deleted and still waiting to be confirmed or reverted. `entity.list` does not return
    /// them, so they come from the snapshot.
    fn deleted_rules(&self, cx: &gpui_kit::App) -> Vec<crate::model::config::DeletedRule> {
        self.model
            .read(cx)
            .config()
            .deleted_rules
            .iter()
            .filter(|deleted| self.rule(&deleted.id).is_none())
            .cloned()
            .collect()
    }

    /// Rules a newer Bifurc than this one last saved. `entity.list` does not return them either: the engine
    /// lists them and does nothing else with them.
    fn locked_rules(&self, cx: &gpui_kit::App) -> Vec<crate::model::config::LockedRule> {
        let config = self.model.read(cx).config();
        config.locked_rules.iter().filter(|locked| self.rule(&locked.id).is_none()).cloned().collect()
    }

    fn rule(&self, id: &str) -> Option<&Map<String, Value>> {
        self.rules.iter().find(|rule| rule.get("id").and_then(Value::as_str) == Some(id))
    }

    /// Keeps the detail pane on a rule that exists. The controls are only written when the engine's copy
    /// differs from what they were last shown or saved as, and there is no edit here to lose: every
    /// autosave makes the engine's list change, and writing the controls would move the cursor under the user.
    fn follow_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let forced = std::mem::take(&mut self.reload_due);
        match self.selection.clone() {
            Selection::Draft => {}
            Selection::Saved(id) => match self.rule(&id).cloned() {
                Some(rule) => {
                    let differs = engine_fields(&rule) != self.baseline;
                    if forced || (differs && !self.saving && !self.is_dirty(cx)) {
                        self.show(&rule, window, cx);
                    }
                }
                None => self.select_first(window, cx),
            },
            Selection::None => self.select_first(window, cx),
        }
    }

    fn select_first(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.rules.first().cloned() {
            Some(rule) => {
                self.selection = Selection::Saved(text(&rule, "id"));
                self.show(&rule, window, cx);
            }
            None => self.selection = Selection::None,
        }
    }

    fn fields(&self, cx: &gpui_kit::App) -> Fields {
        let read = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        Fields {
            name: read(&self.name),
            pattern: read(&self.pattern),
            use_regex: self.use_regex,
            enabled: self.enabled,
            target: self.target,
            mapping_id: self.mapping_id.clone(),
            external: read(&self.external),
            request_script: self.request_script.value(cx),
            response_script: self.response_script.value(cx),
        }
    }

    pub fn is_dirty(&self, cx: &gpui_kit::App) -> bool {
        self.selection != Selection::None && self.fields(cx) != self.baseline
    }

    /// The id of the selected rule, once it is saved.
    fn saved_id(&self) -> Option<&str> {
        match &self.selection {
            Selection::Saved(id) => Some(id),
            _ => None,
        }
    }

    /// The selected rule, for the window's breadcrumb. The shell shows this screen's name field there.
    pub fn active_target(&self, cx: &gpui_kit::App) -> Option<ActiveTarget> {
        if self.selection == Selection::None {
            return None;
        }
        let name = self.name.read(cx).value();
        let title = if name.is_empty() { DEFAULT_NAME.into() } else { name };
        Some(ActiveTarget::named(title, self.saved_id().map(str::to_owned), self.name.clone()))
    }

    /// A saved rule is written a moment after each change; a draft waits for Save.
    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        if self.saved_id().is_some() {
            self.autosave.schedule(cx, |this, window, cx| this.save(SaveMode::Auto, window, cx));
        }
    }

    /// The name was confirmed, by Enter or by leaving the field. Naming a draft is what creates it, when it
    /// has what it needs to be created.
    fn commit_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selection == Selection::Draft && self.fields(cx).name != self.baseline.name {
            self.save(SaveMode::Auto, window, cx);
        }
    }

    /// Writes an edit that is still waiting, now: another rule is about to be shown.
    fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saved_id().is_some() {
            self.autosave.cancel();
            self.save(SaveMode::Auto, window, cx);
        }
    }

    /// Fills the detail pane from a rule.
    fn show(&mut self, rule: &Map<String, Value>, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |s, cx| s.set_value(text(rule, "name"), window, cx));
        self.pattern.update(cx, |s, cx| s.set_value(text(rule, "pattern"), window, cx));
        self.external.update(cx, |s, cx| s.set_value(text(rule, "targetExternal"), window, cx));
        self.use_regex = flag(rule, "useRegex");
        self.enabled = rule.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        self.target = if text(rule, "targetType") == "mapping" { Target::Mapping } else { Target::External };
        self.mapping_id = text(rule, "targetMappingId");
        self.request_script.set_value(text(rule, "requestScript"), window, cx);
        self.response_script.set_value(text(rule, "responseScript"), window, cx);
        self.baseline = self.fields(cx);
        cx.notify();
    }

    /// Moves the detail pane to another rule. An edit of a saved rule is written on the way out; a draft, or
    /// an edit that cannot be written, is asked about first.
    fn go_to(&mut self, selection: Selection, window: &mut Window, cx: &mut Context<Self>) {
        if selection == self.selection {
            return;
        }
        let writable = self.saved_id().is_some() && !self.saving && problem_of(&self.fields(cx)).is_none();
        if self.is_dirty(cx) && writable {
            self.flush(window, cx);
            self.open(selection, window, cx);
        } else if self.is_dirty(cx) {
            let name = self.baseline.name.clone();
            confirm_danger(
                cx.weak_entity(),
                window,
                cx,
                t_with("proxy_rules.confirm.discard.title", &[("name", &name)]),
                t("proxy_rules.confirm.discard.consequence"),
                t("proxy_rules.confirm.discard.ok"),
                move |this, window, cx| this.open(selection.clone(), window, cx),
            );
        } else {
            self.open(selection, window, cx);
        }
    }

    /// Selects the rule `by` places away from the selected one, wrapping around.
    pub fn step(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.rules.len() as isize;
        if len == 0 {
            return;
        }
        let at = self.saved_id().and_then(|id| self.rules.iter().position(|r| text(r, "id") == id));
        let next = at.map_or(0, |ix| (ix as isize + by).rem_euclid(len) as usize);
        let id = text(&self.rules[next], "id");
        self.go_to(Selection::Saved(id), window, cx);
    }

    /// Starts a rule that Save will create.
    pub fn new_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to(Selection::Draft, window, cx);
    }

    fn open(&mut self, selection: Selection, window: &mut Window, cx: &mut Context<Self>) {
        match &selection {
            Selection::Saved(id) => {
                if let Some(rule) = self.rule(id).cloned() {
                    self.show(&rule, window, cx);
                }
            }
            Selection::Draft => {
                let mut rule = Map::new();
                rule.insert("name".into(), json!(DEFAULT_NAME));
                rule.insert("targetType".into(), json!("external"));
                rule.insert("enabled".into(), json!(true));
                self.show(&rule, window, cx);
            }
            Selection::None => {}
        }
        self.selection = selection;
        cx.notify();
    }

    /// `what` is a key whose text takes the engine's `{message}`.
    fn report(&mut self, what: &'static str, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(t_with(what, &[("message", &message)]), cx));
    }

    /// What the footer of the detail pane asks for.
    pub fn entity_action(&mut self, action: EntityAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            EntityAction::Save => self.save(SaveMode::Explicit, window, cx),
            EntityAction::Sync => {
                if let Some(id) = self.saved_id().map(str::to_owned) {
                    self.model.update(cx, |model, cx| model.sync_entity(&id, cx));
                }
            }
            EntityAction::Revert => {
                if let Some(id) = self.saved_id().map(str::to_owned) {
                    let name = self.baseline.name.clone();
                    confirm_danger(
                        cx.weak_entity(),
                        window,
                        cx,
                        t_with("proxy_rules.confirm.revert.title", &[("name", &name)]),
                        t("proxy_rules.confirm.revert.consequence"),
                        t("proxy_rules.button.revert"),
                        move |this, _, cx| this.model.update(cx, |model, cx| model.revert_entity(&id, cx)),
                    );
                }
            }
            EntityAction::Delete => self.confirm_delete(window, cx),
            EntityAction::Close | EntityAction::Move(_) => {}
        }
    }

    /// Creates the rule if it is a draft, or writes the edit over the saved one. A write that is asked for
    /// while another is in flight is not lost: the one in flight looks for a newer edit when it lands.
    fn save(&mut self, mode: SaveMode, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::{WindowExt as _, notification::Notification};
        if self.saving || self.selection == Selection::None {
            return;
        }
        let fields = self.fields(cx);
        let creating = self.selection == Selection::Draft;
        if !creating && fields == self.baseline {
            return;
        }
        if let Some(problem) = problem_of(&fields) {
            if mode == SaveMode::Explicit {
                window.push_notification(
                    Notification::warning(problem).title(t("proxy_rules.problem.cant_save_yet")),
                    cx,
                );
            }
            cx.notify();
            return;
        }

        let mut entity = match &self.selection {
            Selection::Saved(id) => self.rule(id).cloned().unwrap_or_default(),
            _ => Map::new(),
        };
        entity.insert("name".into(), json!(if fields.name.is_empty() { DEFAULT_NAME } else { fields.name.as_str() }));
        entity.insert("pattern".into(), json!(fields.pattern));
        entity.insert("useRegex".into(), json!(fields.use_regex));
        entity.insert("enabled".into(), json!(fields.enabled));
        entity.insert("targetType".into(), json!(fields.target.wire()));
        entity.insert("targetMappingId".into(), json!(fields.mapping_id));
        entity.insert("targetExternal".into(), json!(fields.external));
        entity.insert("requestScript".into(), json!(fields.request_script));
        entity.insert("responseScript".into(), json!(fields.response_script));

        let model = self.model.read(cx);
        let client = model.client().clone();
        let (action, payload) = if creating {
            entity.insert("workspaceId".into(), json!(model.workspace_id()));
            ("entity.create", json!({ "kind": "proxyRules", "workspaceId": model.workspace_id(), "entity": entity }))
        } else {
            ("entity.update", json!({ "kind": "proxyRules", "entity": entity }))
        };
        self.saving = true;
        cx.notify();
        let started_on = self.selection.clone();
        call_in(&client, window, cx, action, payload, move |this, outcome, window, cx| {
            this.saving = false;
            match failure(&outcome) {
                Some(message) => {
                    this.report("proxy_rules.error.save", message, cx);
                    cx.notify();
                    return;
                }
                None => {
                    // The controls are left alone: they may hold what was typed while this was in flight. What
                    // was sent is the new baseline, so that newer text still reads as an edit. If another rule
                    // was selected meanwhile, the controls and baseline are its, and are not touched.
                    if this.selection == started_on {
                        this.baseline = fields;
                        if creating
                            && let Some(id) =
                                outcome.ok().and_then(|r| r.get("id").and_then(Value::as_str).map(str::to_owned))
                        {
                            this.selection = Selection::Saved(id);
                        }
                    }
                    this.reload(window, cx);
                }
            }
            // An edit made since this write was sent is the next write. (`changed` schedules it.)
            this.changed(cx);
        });
    }

    fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Selection::Saved(id) = self.selection.clone() else {
            // A draft has nothing stored to delete; leaving it is the whole of removing it.
            self.selection = Selection::None;
            self.select_first(window, cx);
            cx.notify();
            return;
        };
        let name = self.rule(&id).map(|rule| text(rule, "name")).unwrap_or_default();
        let consequence = delete_consequence(self.model.read(cx).git().entity_state(&id));
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            t_with("proxy_rules.confirm.delete.title", &[("name", &name)]),
            consequence,
            t("proxy_rules.button.delete"),
            move |this, _, cx| {
                let client = this.model.read(cx).client().clone();
                this.autosave.cancel();
                this.selection = Selection::None;
                call(&client, cx, "entity.delete", json!({ "kind": "proxyRules", "id": id }), |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.report("proxy_rules.error.delete", message, cx);
                    }
                });
            },
        );
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let model = self.model.clone();
        let rows = self.rules.iter().map(|rule| {
            let id = text(rule, "id");
            let (select_id, toggle_id) = (id.clone(), id.clone());
            let toggle_model = model.clone();
            ListItem::new(gpui_kit::ElementId::Name(format!("rule-{id}").into()))
                .selected(self.selection == Selection::Saved(id.clone()))
                .px_2()
                .py_1()
                .text_sm()
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().overflow_hidden().text_ellipsis().child(text(rule, "name")))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .font_family(theme.mono_font_family.clone())
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(text(rule, "pattern")),
                                ),
                        )
                        .child(
                            Switch::new(gpui_kit::ElementId::Name(format!("rule-enabled-{id}").into()))
                                .checked(rule.get("enabled").and_then(Value::as_bool).unwrap_or(true))
                                .small()
                                .on_change(move |checked: &bool, _, cx| {
                                    // Turning a rule on or off is not selecting it.
                                    cx.stop_propagation();
                                    let checked = *checked;
                                    toggle_model.update(cx, |m, cx| m.set_enabled("rules", &toggle_id, checked, cx));
                                }),
                        ),
                )
                .on_click(
                    cx.listener(move |this, _, window, cx| this.go_to(Selection::Saved(select_id.clone()), window, cx)),
                )
        });
        // Deleted rules stay in place, struck through. Opening one asks whether to revert or confirm.
        let deleted_rows = self.deleted_rules(cx).into_iter().map(|deleted| {
            let name = if deleted.name.is_empty() { deleted.pattern.clone() } else { deleted.name.clone() };
            let (id, label) = (deleted.id.clone(), name.clone());
            ListItem::new(gpui_kit::ElementId::Name(format!("rule-deleted-{id}").into()))
                .px_2()
                .py_1()
                .text_sm()
                .child(
                    div()
                        .w_full()
                        .overflow_hidden()
                        .text_ellipsis()
                        .line_through()
                        .text_color(theme.muted_foreground)
                        .child(label),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    let target = Deleted::Entity { kind: "proxyRules", id: id.clone() };
                    resolve_deleted(this.model.clone(), target, name.clone(), window, cx);
                }))
        });
        // Rules a newer Bifurc saved stay in place too, marked with the sign for an update. Opening one says
        // which version it needs.
        let locked_rows = self.locked_rules(cx).into_iter().map(|locked| {
            let (name, version) = (locked.name.clone(), locked.version.clone());
            ListItem::new(gpui_kit::ElementId::Name(format!("rule-locked-{}", locked.id).into()))
                .px_2()
                .py_1()
                .text_sm()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_color(theme.muted_foreground)
                        .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().child(locked.name))
                        .child(Icon::new(IconName::ArrowUp).small()),
                )
                .on_click(move |_, window, cx| needs_update(name.clone(), version.clone(), window, cx))
        });
        list_rows("rule-list").children(rows).children(deleted_rows).children(locked_rows).when(
            self.selection == Selection::Draft,
            |this| {
                this.child(
                    ListItem::new("rule-draft")
                        .selected(true)
                        .px_2()
                        .py_1()
                        .text_sm()
                        .child(div().text_color(theme.muted_foreground).child(t("proxy_rules.list.draft"))),
                )
            },
        )
    }

    fn render_target(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mappings: Vec<(String, String)> = self
            .model
            .read(cx)
            .config()
            .mappings
            .iter()
            .filter(|m| !m.deleted)
            .map(|m| (m.id.clone(), m.domain.clone()))
            .collect();
        let chosen =
            mappings.iter().find(|(id, _)| *id == self.mapping_id).map(|(_, domain)| domain.clone()).unwrap_or_else(
                || {
                    if mappings.is_empty() {
                        t("proxy_rules.target.no_mappings").to_owned()
                    } else {
                        t("proxy_rules.target.choose_mapping").to_owned()
                    }
                },
            );
        let this = cx.weak_entity();
        let current = self.mapping_id.clone();

        h_flex()
            .gap_3()
            .child(
                TabBar::new("rule-target")
                    .segmented()
                    .small()
                    .selected_index(if self.target == Target::Mapping { 0 } else { 1 })
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        this.target = if *ix == 0 { Target::Mapping } else { Target::External };
                        this.changed(cx);
                    }))
                    .child(Tab::new().label(t("proxy_rules.target.mapping")))
                    .child(Tab::new().label(t("proxy_rules.target.external"))),
            )
            .child(
                div().flex_1().min_w_0().child(match self.target {
                    Target::External => Input::new(&self.external).small().into_any_element(),
                    Target::Mapping => Button::new("rule-mapping")
                        .small()
                        .outline()
                        .label(chosen)
                        .icon(IconName::ChevronDown)
                        .disabled(mappings.is_empty())
                        .dropdown_menu(move |menu, _, _| {
                            mappings.iter().fold(menu, |menu, (id, domain)| {
                                let (this, id) = (this.clone(), id.clone());
                                menu.item(PopupMenuItem::new(domain.clone()).checked(id == current).on_click(
                                    move |_, _, cx| {
                                        this.update(cx, |this, cx| {
                                            this.mapping_id = id.clone();
                                            this.changed(cx);
                                        })
                                        .ok();
                                    },
                                ))
                            })
                        })
                        .into_any_element(),
                }),
            )
            .text_color(theme.foreground)
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        if self.selection == Selection::None {
            return if self.loaded {
                page::empty_with_action(
                    t("proxy_rules.empty.title"),
                    t("proxy_rules.empty.description"),
                    Button::new("empty-new-rule")
                        .label(t("proxy_rules.button.new_rule"))
                        .on_click(cx.listener(|this, _, window, cx| this.go_to(Selection::Draft, window, cx))),
                )
            } else {
                page::loading(t("proxy_rules.page.loading"))
            };
        }
        let theme = cx.theme().clone();
        let dirty = self.is_dirty(cx);
        let saved_id = self.saved_id().map(str::to_owned);
        let saved = saved_id.is_some();
        let problem = (saved && dirty)
            .then(|| problem_of(&self.fields(cx)))
            .flatten()
            .map(|problem| SharedString::from(t_with("proxy_rules.problem.not_saved", &[("problem", &problem)])));
        let bar = EntityBar {
            saving: self.saving,
            unsaved: saved && dirty,
            problem,
            closable: false,
            deletable: true,
            ..EntityBar::of(self.model.read(cx), saved_id.as_deref())
        };
        let editor = if self.script_ix == 0 { &self.request_script } else { &self.response_script };
        let label =
            |text: &'static str| div().w_24().flex_none().text_sm().text_color(theme.muted_foreground).child(text);

        v_flex()
            .size_full()
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        v_flex()
                            .flex_none()
                            .gap_3()
                            .p_3()
                            .child(
                                h_flex().gap_3().child(
                                    Switch::new("rule-enabled")
                                        .checked(self.enabled)
                                        .small()
                                        .label(t("proxy_rules.label.enabled"))
                                        .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                            this.enabled = *checked;
                                            this.changed(cx);
                                        })),
                                ),
                            )
                            .child(
                                h_flex()
                                    .gap_3()
                                    .child(label(t("proxy_rules.label.matches")))
                                    .child(div().flex_1().min_w_0().child(Input::new(&self.pattern).small()))
                                    .child(
                                        Checkbox::new("rule-regex")
                                            .checked(self.use_regex)
                                            .small()
                                            .label(t("proxy_rules.label.regular_expression"))
                                            .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                                this.use_regex = *checked;
                                                this.changed(cx);
                                            })),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_3()
                                    .child(label(t("proxy_rules.label.send_to")))
                                    .child(div().flex_1().min_w_0().child(self.render_target(cx))),
                            ),
                    )
                    .child(
                        div().flex_none().px_3().child(
                            TabBar::new("rule-scripts")
                                .underline()
                                .small()
                                .selected_index(self.script_ix)
                                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                    this.script_ix = *ix;
                                    cx.notify();
                                }))
                                .child(Tab::new().label(t("proxy_rules.script.request_tab")))
                                .child(Tab::new().label(t("proxy_rules.script.response_tab"))),
                        ),
                    )
                    .child(div().flex_1().min_h_0().child(Editor::new(editor.state()).bordered(false).h_full())),
            )
            .child(entity_bar(bar, cx, |this, action, window, cx| this.entity_action(action, window, cx)))
            .into_any_element()
    }
}

impl Render for ProxyRulesScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = page::header(
            t("proxy_rules.page.title"),
            t("proxy_rules.page.description"),
            Button::new("new-rule")
                .small()
                .icon(IconName::Plus)
                .label(t("proxy_rules.button.new_rule"))
                .on_click(cx.listener(|this, _, window, cx| this.go_to(Selection::Draft, window, cx))),
            cx,
        );
        let has_list = !self.rules.is_empty()
            || !self.deleted_rules(cx).is_empty()
            || !self.locked_rules(cx).is_empty()
            || self.selection == Selection::Draft;
        let list = has_list.then(|| self.render_list(cx).into_any_element());
        let detail = self.render_detail(cx);
        let body = master_detail(list, detail, cx);
        page::screen(header, body, cx)
    }
}
