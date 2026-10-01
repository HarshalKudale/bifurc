//! The Proxy rules screen: which traffic through the forward proxy is sent somewhere else.
//!
//! A rule matches a URL pattern and sends the request to one of the workspace's mappings or to another
//! server, optionally running a script on the request and on the response. The list is read from the engine
//! (`entity.list`); the detail pane holds an *edit* of the selected rule, saved over the engine's copy by id.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
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
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::{Map, Value, json};

use crate::{
    app_model::{AppModel, ModelEvent},
    components::{Syntax, VarEditor, dialogs::confirm_danger, page},
    engine::{call, call_in, describe, failure},
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
    _subscriptions: Vec<Subscription>,
}

fn text(rule: &Map<String, Value>, key: &str) -> String {
    rule.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn flag(rule: &Map<String, Value>, key: &str) -> bool {
    rule.get(key).and_then(Value::as_bool).unwrap_or(false)
}

impl ProxyRulesScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Rule name"));
        let pattern = cx.new(|cx| InputState::new(window, cx).placeholder("api.example.com/v1"));
        let external = cx.new(|cx| InputState::new(window, cx).placeholder("http://localhost:9000"));
        let request_script =
            VarEditor::code(Syntax::JavaScript, "// Runs on the request before it is forwarded", window, cx);
        let response_script =
            VarEditor::code(Syntax::JavaScript, "// Runs on the response before it is returned", window, cx);

        let mut subscriptions = vec![cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| {
            if matches!(event, ModelEvent::ConfigChanged | ModelEvent::WorkspaceSwitched) {
                this.reload(window, cx);
            }
        })];
        for input in [&name, &pattern, &external] {
            subscriptions.push(cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
        }
        for editor in [&request_script, &response_script] {
            subscriptions.push(cx.subscribe(editor.state(), |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
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
                Err(error) => this.report("Couldn't read the proxy rules.", describe(&error), cx),
            }
            cx.notify();
        });
    }

    fn rule(&self, id: &str) -> Option<&Map<String, Value>> {
        self.rules.iter().find(|rule| rule.get("id").and_then(Value::as_str) == Some(id))
    }

    /// Keeps the detail pane on a rule that exists, refreshing it when there is no edit here to lose.
    fn follow_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selection.clone() {
            Selection::Draft => {}
            Selection::Saved(id) => match self.rule(&id).cloned() {
                Some(rule) if !self.is_dirty(cx) => self.show(&rule, window, cx),
                Some(_) => {}
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

    /// Moves the detail pane to another rule, asking first if that would lose an edit.
    fn go_to(&mut self, selection: Selection, window: &mut Window, cx: &mut Context<Self>) {
        if selection == self.selection {
            return;
        }
        if self.is_dirty(cx) {
            let name = self.baseline.name.clone();
            confirm_danger(
                cx.weak_entity(),
                window,
                cx,
                format!("Discard changes to “{name}”?"),
                "The edits to this rule have not been saved.",
                "Discard changes",
                move |this, window, cx| this.open(selection.clone(), window, cx),
            );
        } else {
            self.open(selection, window, cx);
        }
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

    fn report(&mut self, what: &str, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(format!("{what} {message}"), cx));
    }

    pub fn revert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.selection.clone();
        self.open(selection, window, cx);
    }

    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::{WindowExt as _, notification::Notification};
        if self.saving || self.selection == Selection::None {
            return;
        }
        let fields = self.fields(cx);
        let problem = if fields.pattern.is_empty() {
            Some("Enter the URL pattern this rule matches.")
        } else if fields.target == Target::Mapping && fields.mapping_id.is_empty() {
            Some("Choose the mapping to send matching requests to.")
        } else if fields.target == Target::External && fields.external.is_empty() {
            Some("Enter the server to send matching requests to.")
        } else {
            None
        };
        if let Some(problem) = problem {
            window.push_notification(Notification::warning(problem).title("Can't save yet"), cx);
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
        let creating = self.selection == Selection::Draft;
        let (action, payload) = if creating {
            entity.insert("workspaceId".into(), json!(model.workspace_id()));
            ("entity.create", json!({ "kind": "proxyRules", "workspaceId": model.workspace_id(), "entity": entity }))
        } else {
            ("entity.update", json!({ "kind": "proxyRules", "entity": entity }))
        };
        self.saving = true;
        cx.notify();
        call_in(&client, window, cx, action, payload, move |this, outcome, window, cx| {
            this.saving = false;
            match failure(&outcome) {
                Some(message) => this.report("Couldn't save the rule.", message, cx),
                None => {
                    // What was saved is the new baseline; the list that follows will agree with it.
                    this.baseline = fields;
                    if creating
                        && let Some(id) =
                            outcome.ok().and_then(|r| r.get("id").and_then(Value::as_str).map(str::to_owned))
                    {
                        this.selection = Selection::Saved(id);
                    }
                    this.reload(window, cx);
                }
            }
            cx.notify();
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
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            format!("Delete rule “{name}”?"),
            "Traffic it matched will go where it was originally headed.",
            "Delete",
            move |this, _, cx| {
                let client = this.model.read(cx).client().clone();
                this.selection = Selection::None;
                call(&client, cx, "entity.delete", json!({ "kind": "proxyRules", "id": id }), |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.report("Couldn't delete the rule.", message, cx);
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
        v_flex()
            .id("rule-list")
            .w_72()
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .p_2()
            .gap_1()
            .children(rows)
            .when(self.selection == Selection::Draft, |this| {
                this.child(
                    ListItem::new("rule-draft")
                        .selected(true)
                        .px_2()
                        .py_1()
                        .text_sm()
                        .child(div().text_color(theme.muted_foreground).child("New rule (not saved)")),
                )
            })
    }

    fn render_target(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mappings: Vec<(String, String)> =
            self.model.read(cx).config().mappings.iter().map(|m| (m.id.clone(), m.domain.clone())).collect();
        let chosen =
            mappings.iter().find(|(id, _)| *id == self.mapping_id).map(|(_, domain)| domain.clone()).unwrap_or_else(
                || if mappings.is_empty() { "No mappings yet".to_owned() } else { "Choose a mapping".to_owned() },
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
                        cx.notify();
                    }))
                    .child(Tab::new().label("A mapping"))
                    .child(Tab::new().label("Another server")),
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
                                            cx.notify();
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
                    "No proxy rules",
                    "A rule sends traffic that matches a pattern to a mapping or to another server.",
                    Button::new("empty-new-rule")
                        .label("New rule")
                        .on_click(cx.listener(|this, _, window, cx| this.go_to(Selection::Draft, window, cx))),
                )
            } else {
                page::loading("Loading rules…")
            };
        }
        let theme = cx.theme().clone();
        let dirty = self.is_dirty(cx);
        let is_draft = self.selection == Selection::Draft;
        let editor = if self.script_ix == 0 { &self.request_script } else { &self.response_script };
        let label =
            |text: &'static str| div().w_24().flex_none().text_sm().text_color(theme.muted_foreground).child(text);

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
                    .child(div().flex_1().min_w_0().child(Input::new(&self.name).small().appearance(false)))
                    .child(Switch::new("rule-enabled").checked(self.enabled).small().label("Enabled").on_change(
                        cx.listener(|this, checked: &bool, _, cx| {
                            this.enabled = *checked;
                            cx.notify();
                        }),
                    ))
                    .child(
                        Button::new("delete-rule")
                            .ghost()
                            .small()
                            .icon(IconName::Delete)
                            .tooltip(if is_draft { "Discard this new rule" } else { "Delete this rule…" })
                            .on_click(cx.listener(|this, _, window, cx| this.confirm_delete(window, cx))),
                    )
                    .child(
                        Button::new("revert-rule")
                            .ghost()
                            .small()
                            .label("Revert")
                            .disabled(!dirty)
                            .on_click(cx.listener(|this, _, window, cx| this.revert(window, cx))),
                    )
                    .child(
                        Button::new("save-rule")
                            .small()
                            .label("Save")
                            .loading(self.saving)
                            .disabled(!dirty && !is_draft)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
            .child(
                v_flex()
                    .flex_none()
                    .gap_3()
                    .p_3()
                    .child(
                        h_flex()
                            .gap_3()
                            .child(label("Matches"))
                            .child(div().flex_1().min_w_0().child(Input::new(&self.pattern).small()))
                            .child(
                                Checkbox::new("rule-regex")
                                    .checked(self.use_regex)
                                    .small()
                                    .label("Regular expression")
                                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                        this.use_regex = *checked;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(label("Send to"))
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
                        .child(Tab::new().label("Request script"))
                        .child(Tab::new().label("Response script")),
                ),
            )
            .child(div().flex_1().min_h_0().child(Editor::new(editor.state()).bordered(false).h_full()))
            .into_any_element()
    }
}

impl Render for ProxyRulesScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = page::header(
            "Proxy rules",
            "Send traffic through the forward proxy that matches a pattern to a mapping or to another server.",
            Button::new("new-rule")
                .small()
                .icon(IconName::Plus)
                .label("New rule")
                .on_click(cx.listener(|this, _, window, cx| this.go_to(Selection::Draft, window, cx))),
            cx,
        );
        let has_list = !self.rules.is_empty() || self.selection == Selection::Draft;
        let body = h_flex()
            .size_full()
            .items_stretch()
            .when(has_list, |this| this.child(self.render_list(cx)))
            .child(div().flex_1().min_w_0().child(self.render_detail(cx)));
        page::screen(header, body, cx)
    }
}
