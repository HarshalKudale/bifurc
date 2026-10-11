//! The panel beside the canvas: what the selected node does, where its ports lead, and how it fared in the
//! run shown.
//!
//! The inspector holds an edit of one node. Its controls are written when a node is selected and never
//! after, so typing is not disturbed by the flow being saved; every change is reported whole as
//! [`InspectorEvent::Changed`] and the screen puts it in the flow.

use std::collections::HashMap;

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    combobox::{Combobox, ComboboxEvent, ComboboxState},
    h_flex,
    input::{Editor, Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    searchable_list::{SearchableGroup, SearchableListItem, SearchableVec},
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Context, ElementId, Entity, EventEmitter,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, px,
};

use crate::{
    app_model::AppModel,
    components::{BodyPane, KeyValueEvent, KeyValueTable, Syntax, VarEditor, page},
    model::{
        ConfigSnapshot, RequestKind,
        config::RequestStub,
        flow::{Branch, FlowNode, NodeInfo, NodeKind, NodeRun, Pair, Rule},
        tree::folder_choices,
    },
};

/// The comparisons a rule can make: the name the flow stores, and how it reads.
const OPS: [(&str, &str); 15] = [
    ("equals", "equals"),
    ("notEquals", "does not equal"),
    ("contains", "contains"),
    ("notContains", "does not contain"),
    ("startsWith", "starts with"),
    ("endsWith", "ends with"),
    ("matches", "matches the pattern"),
    ("greaterThan", "is greater than"),
    ("greaterOrEqual", "is at least"),
    ("lessThan", "is less than"),
    ("lessOrEqual", "is at most"),
    ("exists", "exists"),
    ("notExists", "does not exist"),
    ("isEmpty", "is empty"),
    ("notEmpty", "is not empty"),
];

fn op_label(op: &str) -> &'static str {
    OPS.iter().find(|(name, _)| *name == op).map_or("equals", |(_, label)| label)
}

/// Whether a comparison is made against a value, or only asks about the one it reads.
fn op_takes_value(op: &str) -> bool {
    !matches!(op, "exists" | "notExists" | "isEmpty" | "notEmpty")
}

/// The request types a node of this kind can run.
fn request_kinds(kind: NodeKind) -> &'static [RequestKind] {
    match kind {
        NodeKind::Request => &[RequestKind::Rest, RequestKind::GraphQl, RequestKind::Soap, RequestKind::Grpc],
        NodeKind::Stream => &[RequestKind::Sse, RequestKind::WebSocket, RequestKind::SocketIo, RequestKind::Grpc],
        NodeKind::Mcp => &[RequestKind::Mcp],
        NodeKind::Start | NodeKind::Condition | NodeKind::Delay => &[],
    }
}

/// A saved request as the picker lists it.
#[derive(Debug, Clone, PartialEq)]
struct RequestItem {
    key: (RequestKind, String),
    name: SharedString,
    /// Its method, or its type when it has none.
    tag: SharedString,
    /// What a search also finds it by: its folder and where it goes.
    found_by: String,
}

impl SearchableListItem for RequestItem {
    type Value = (RequestKind, String);

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.key
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        self.name.to_lowercase().contains(&query) || self.found_by.contains(&query)
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .w_full()
            .gap_3()
            .justify_between()
            .child(div().min_w_0().overflow_hidden().text_ellipsis().child(self.name.clone()))
            .child(div().flex_none().text_xs().text_color(cx.theme().muted_foreground).child(self.tag.clone()))
    }
}

type RequestList = SearchableVec<SearchableGroup<RequestItem>>;

/// The saved requests a node of `kind` can run, under the folder each is kept in: the folders in the order
/// of their paths, the requests of a folder by name.
fn request_groups(config: &ConfigSnapshot, kind: NodeKind) -> Vec<(String, Vec<RequestItem>)> {
    let allowed = request_kinds(kind);
    let paths: HashMap<String, String> = folder_choices(&config.request_folder_rows()).into_iter().collect();
    let mut groups: Vec<(String, Vec<RequestItem>)> = Vec::new();
    let usable =
        |request: &&RequestStub| allowed.contains(&request.kind) && !request.deleted && request.locked.is_none();
    for request in config.requests.iter().filter(usable) {
        // A request whose folder is gone is still one a node can run.
        let path = request.folder_id.as_deref().and_then(|id| paths.get(id)).map_or("No folder", String::as_str);
        let item = RequestItem {
            key: (request.kind, request.id.clone()),
            name: request.name.clone().into(),
            tag: request.method.clone().unwrap_or_else(|| request.kind.label().to_owned()).into(),
            found_by: format!("{path} {} {}", request.target, request.kind.label()).to_lowercase(),
        };
        match groups.iter_mut().find(|(known, _)| known == path) {
            Some((_, items)) => items.push(item),
            None => groups.push((path.to_owned(), vec![item])),
        }
    }
    groups.sort_by_key(|(path, _)| path.to_lowercase());
    for (_, items) in &mut groups {
        items.sort_by_key(|item| item.name.to_lowercase());
    }
    groups
}

#[derive(Debug, Clone, PartialEq)]
pub enum InspectorEvent {
    /// The node as it now is.
    Changed(Box<FlowNode>),
    /// The flow's inputs, which the start node shows.
    Inputs(Vec<Pair>),
    Delete(String),
    /// Remove the connection with this id.
    Disconnect(String),
}

/// The fields a node keeps as a count or a time in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Number {
    Delay,
    Retries,
    RetryDelay,
    MaxMessages,
    IdleTimeout,
    Listen,
}

impl Number {
    const ALL: [Number; 6] =
        [Number::Delay, Number::Retries, Number::RetryDelay, Number::MaxMessages, Number::IdleTimeout, Number::Listen];

    fn label(self) -> &'static str {
        match self {
            Number::Delay => "Wait (ms)",
            Number::Retries => "Retries",
            Number::RetryDelay => "Wait between tries (ms)",
            Number::MaxMessages => "Stop after messages",
            Number::IdleTimeout => "Stop when idle for (ms)",
            Number::Listen => "Listen for at most (ms)",
        }
    }

    fn read(self, node: &FlowNode) -> u64 {
        match self {
            Number::Delay => node.delay_ms,
            Number::Retries => node.retries as u64,
            Number::RetryDelay => node.retry_delay_ms,
            Number::MaxMessages => node.max_messages,
            Number::IdleTimeout => node.idle_timeout_ms,
            Number::Listen => node.listen_ms,
        }
    }

    fn write(self, node: &mut FlowNode, value: u64) {
        match self {
            Number::Delay => node.delay_ms = value,
            Number::Retries => node.retries = value.min(20) as u32,
            Number::RetryDelay => node.retry_delay_ms = value,
            Number::MaxMessages => node.max_messages = value,
            Number::IdleTimeout => node.idle_timeout_ms = value,
            Number::Listen => node.listen_ms = value,
        }
    }
}

struct RuleRow {
    path: Entity<InputState>,
    value: Entity<InputState>,
}

struct BranchRow {
    name: Entity<InputState>,
    rules: Vec<RuleRow>,
}

pub struct Inspector {
    model: Entity<AppModel>,
    node: Option<FlowNode>,
    info: NodeInfo,
    /// The node's connections: id, and what it reads as (`Success → Get user`).
    edges: Vec<(String, SharedString)>,
    run: Option<NodeRun>,
    /// The window the panel is in. A run's report carries none, and showing what a node produced needs one.
    window: AnyWindowHandle,
    name: Entity<InputState>,
    message: Entity<InputState>,
    event: Entity<InputState>,
    target: Entity<InputState>,
    numbers: Vec<(Number, Entity<InputState>)>,
    /// The saved requests the node can run, and the list as it was last handed to the picker.
    requests: Entity<ComboboxState<RequestList>>,
    listed: Vec<(String, Vec<RequestItem>)>,
    after_execute: VarEditor,
    /// What the node produced and what it was handed in the run shown.
    produced: Entity<BodyPane>,
    handed: Entity<BodyPane>,
    arguments: Entity<KeyValueTable>,
    inputs: Entity<KeyValueTable>,
    branches: Vec<BranchRow>,
    _branch_subscriptions: Vec<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<InspectorEvent> for Inspector {}

fn pairs_of(pairs: &[Pair]) -> Vec<(String, String)> {
    pairs.iter().map(|pair| (pair.name.clone(), pair.value.clone())).collect()
}

fn to_pairs(pairs: Vec<(String, String)>) -> Vec<Pair> {
    pairs.into_iter().map(|(name, value)| Pair { name, value }).collect()
}

impl Inspector {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let name = field("Name", window, cx);
        let message = field("What to send; {{input.data}} is what the node was handed", window, cx);
        let event = field("Event name", window, cx);
        let target = field("Name of the tool, prompt or resource URI", window, cx);
        let numbers: Vec<(Number, Entity<InputState>)> =
            Number::ALL.iter().map(|number| (*number, field("0", window, cx))).collect();
        let requests =
            cx.new(|cx| ComboboxState::new(RequestList::new(Vec::new()), vec![], window, cx).searchable(true));
        let after_execute = VarEditor::code(
            Syntax::JavaScript,
            "// lp.output is what this node produced: change it, or replace it.\n// lp.environment.set(\"token\", JSON.parse(lp.output.body).token);",
            window,
            cx,
        );
        let produced = cx.new(|cx| BodyPane::fitted("", window, cx));
        let handed = cx.new(|cx| BodyPane::fitted("", window, cx));
        let arguments = cx.new(|cx| KeyValueTable::new("Argument", "Value", window, cx));
        let inputs = cx.new(|cx| KeyValueTable::new("Input", "Default value", window, cx));

        let on_input = |this: &mut Self, _: Entity<InputState>, event: &InputEvent, cx: &mut Context<Self>| {
            if matches!(event, InputEvent::Change) {
                this.changed(cx);
            }
        };
        let mut subscriptions: Vec<Subscription> =
            [&name, &message, &event, &target].into_iter().map(|input| cx.subscribe(input, on_input)).collect();
        subscriptions.extend(numbers.iter().map(|(_, input)| cx.subscribe(input, on_input)));
        subscriptions.push(cx.subscribe(after_execute.state(), |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.changed(cx);
            }
        }));
        subscriptions.push(cx.subscribe(&requests, |this, _, event: &ComboboxEvent<RequestList>, cx| {
            if let ComboboxEvent::Change(chosen) = event
                && let Some((kind, id)) = chosen.first().cloned()
            {
                this.edit(cx, |node| {
                    node.request_kind = kind.entity_kind().to_owned();
                    node.request_id = id;
                });
            }
        }));
        // A request saved, renamed or deleted elsewhere while the panel is open is one the picker knows of.
        subscriptions.push(cx.observe_in(&model, window, |this, _, window, cx| this.list_requests(false, window, cx)));
        subscriptions.push(cx.subscribe(&arguments, |this, _, _: &KeyValueEvent, cx| this.changed(cx)));
        subscriptions.push(cx.subscribe(&inputs, |_, table, _: &KeyValueEvent, cx| {
            let pairs = to_pairs(table.read(cx).pairs(cx));
            cx.emit(InspectorEvent::Inputs(pairs));
        }));

        Self {
            model,
            node: None,
            info: NodeInfo::default(),
            edges: Vec::new(),
            run: None,
            window: window.window_handle(),
            name,
            message,
            event,
            target,
            numbers,
            requests,
            listed: Vec::new(),
            after_execute,
            produced,
            handed,
            arguments,
            inputs,
            branches: Vec::new(),
            _branch_subscriptions: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn node_id(&self) -> Option<&str> {
        self.node.as_ref().map(|node| node.id.as_str())
    }

    /// Show `node` in the controls, as the flow has it. `inputs` are the flow's, which the start node edits.
    pub fn show(&mut self, node: Option<FlowNode>, inputs: &[Pair], window: &mut Window, cx: &mut Context<Self>) {
        if let Some(node) = &node {
            let mut set = |input: &Entity<InputState>, value: &str| {
                input.update(cx, |state, cx| state.set_value(value.to_owned(), window, cx));
            };
            set(&self.name, &node.name);
            set(&self.message, &node.message);
            set(&self.event, &node.event);
            set(&self.target, &node.target);
            for (number, input) in &self.numbers {
                let value = number.read(node);
                set(input, &if value == 0 { String::new() } else { value.to_string() });
            }
            self.after_execute.set_value(node.after_execute.clone(), window, cx);
            self.arguments.update(cx, |table, cx| table.set_pairs(&pairs_of(&node.arguments), window, cx));
            self.inputs.update(cx, |table, cx| table.set_pairs(&pairs_of(inputs), window, cx));
        }
        self.node = node;
        self.list_requests(true, window, cx);
        self.rebuild_branches(window, cx);
        cx.notify();
    }

    /// Hand the picker the requests the node can run, as the workspace has them now, with the node's own
    /// chosen. Unless `anew`, a list that did not change is left alone: a search being typed is not thrown
    /// away because something else was saved.
    fn list_requests(&mut self, anew: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = &self.node else { return };
        let groups = request_groups(self.model.read(cx).config(), node.kind);
        if !anew && groups == self.listed {
            return;
        }
        let chosen: Vec<(RequestKind, String)> = RequestKind::from_entity_kind(&node.request_kind)
            .map(|kind| (kind, node.request_id.clone()))
            .into_iter()
            .collect();
        self.listed = groups.clone();
        let list: Vec<SearchableGroup<RequestItem>> =
            groups.into_iter().map(|(folder, items)| SearchableGroup::new(folder).items(items)).collect();
        self.requests.update(cx, |picker, cx| {
            picker.set_items(RequestList::new(list), window, cx);
            picker.set_selected_values(&chosen, window, cx);
        });
    }

    /// Put what the node produced and was handed in the run shown into their panes.
    fn fill_run(&mut self, cx: &mut Context<Self>) {
        let window = self.window;
        cx.spawn(async move |this, cx| {
            window
                .update(cx, |_, window, cx| {
                    this.update(cx, |this, cx| {
                        let text = |value: Option<&serde_json::Value>| {
                            value.and_then(|value| serde_json::to_string_pretty(value).ok()).unwrap_or_default()
                        };
                        let run = this.run.as_ref();
                        let (output, input) = (text(run.map(|run| &run.output)), text(run.map(|run| &run.input)));
                        this.produced.update(cx, |pane, cx| pane.set(output, None, Syntax::Json, window, cx));
                        this.handed.update(cx, |pane, cx| pane.set(input, None, Syntax::Json, window, cx));
                    })
                    .ok();
                })
                .ok();
        })
        .detach();
    }

    /// What surrounds the node: what the engine says it offers, its connections and its part in the run.
    pub fn set_context(
        &mut self,
        info: NodeInfo,
        edges: Vec<(String, SharedString)>,
        run: Option<NodeRun>,
        cx: &mut Context<Self>,
    ) {
        if self.info != info || self.edges != edges || self.run != run {
            let shown = |run: &Option<NodeRun>| run.as_ref().map(|run| (run.output.clone(), run.input.clone()));
            let refill = shown(&self.run) != shown(&run);
            (self.info, self.edges, self.run) = (info, edges, run);
            if refill {
                self.fill_run(cx);
            }
            cx.notify();
        }
    }

    /// Fresh fields for the node's branches and their rules: their number changed, or another node is shown.
    fn rebuild_branches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let branches: Vec<Branch> = self.node.as_ref().map(|node| node.branches.clone()).unwrap_or_default();
        let mut subscriptions = Vec::new();
        let mut field = |value: &str, placeholder: &'static str, cx: &mut Context<Self>| {
            let input =
                cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value.to_owned()));
            subscriptions.push(cx.subscribe(&input, |this: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.changed(cx);
                }
            }));
            input
        };
        self.branches = branches
            .iter()
            .map(|branch| BranchRow {
                name: field(&branch.name, "Branch name", cx),
                rules: branch
                    .rules
                    .iter()
                    .map(|rule| RuleRow {
                        path: field(&rule.path, "input.status", cx),
                        value: field(&rule.value, "Value", cx),
                    })
                    .collect(),
            })
            .collect();
        self._branch_subscriptions = subscriptions;
    }

    /// Read the controls into the node and report it.
    fn changed(&mut self, cx: &mut Context<Self>) {
        let Some(mut node) = self.node.clone() else { return };
        let read = |input: &Entity<InputState>| input.read(cx).value().to_string();
        node.name = read(&self.name).trim().to_owned();
        node.message = read(&self.message);
        node.event = read(&self.event).trim().to_owned();
        node.target = read(&self.target).trim().to_owned();
        for (number, input) in &self.numbers {
            number.write(&mut node, read(input).trim().parse().unwrap_or(0));
        }
        node.after_execute = self.after_execute.value(cx);
        node.arguments = to_pairs(self.arguments.read(cx).pairs(cx));
        for (branch, row) in node.branches.iter_mut().zip(&self.branches) {
            branch.name = read(&row.name).trim().to_owned();
            for (rule, fields) in branch.rules.iter_mut().zip(&row.rules) {
                rule.path = read(&fields.path).trim().to_owned();
                rule.value = read(&fields.value);
            }
        }
        self.node = Some(node.clone());
        cx.emit(InspectorEvent::Changed(Box::new(node)));
        cx.notify();
    }

    /// Change something of the node that no field holds: a switch, a choice, the number of its branches.
    fn edit(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut FlowNode)) {
        if let Some(node) = self.node.as_mut() {
            change(node);
            self.changed(cx);
        }
    }

    /// Like [`Self::edit`], for a change after which the branch fields no longer line up with the branches.
    fn restructure(&mut self, window: &mut Window, cx: &mut Context<Self>, change: impl FnOnce(&mut FlowNode)) {
        // What is typed in the fields goes into the node first, so that rebuilding them loses nothing.
        self.changed(cx);
        if let Some(node) = self.node.as_mut() {
            change(node);
        }
        self.rebuild_branches(window, cx);
        self.changed(cx);
    }

    fn labelled(label: &'static str, control: impl IntoElement, cx: &Context<Self>) -> AnyElement {
        v_flex().gap_1().child(page::section_label(label, cx)).child(control).into_any_element()
    }

    fn render_request_picker(&self, node: &FlowNode, cx: &mut Context<Self>) -> AnyElement {
        let none = match node.kind {
            NodeKind::Stream => "No saved stream, WebSocket, Socket.IO or gRPC request yet",
            NodeKind::Mcp => "No saved MCP request yet",
            _ => "No saved request yet",
        };
        let picker = Combobox::new(&self.requests)
            .small()
            .placeholder(if self.listed.is_empty() { none } else { "Choose a saved request" })
            .search_placeholder("Search by name, folder or address")
            .disabled(self.listed.is_empty())
            .w_full();
        let theme = cx.theme();
        v_flex()
            .gap_1()
            .child(page::section_label("Runs the saved request", cx))
            .child(picker)
            .when_some(self.info.address.clone(), |this, address| {
                this.child(
                    div().text_xs().text_color(theme.muted_foreground).overflow_hidden().text_ellipsis().child(address),
                )
            })
            .into_any_element()
    }

    /// What happens to the node's output before the next node gets it: the saved request's own post script,
    /// or a script of this node. Never both.
    fn render_after_execute(&self, node: &FlowNode, cx: &mut Context<Self>) -> AnyElement {
        // Only a REST request has scripts of its own.
        let rest = node.request_kind == RequestKind::Rest.entity_kind();
        let post = rest && node.run_post_script;
        let switch = rest.then(|| {
            Self::render_switch(
                "flow-post-script",
                "Execute the request's post script",
                node.run_post_script,
                cx,
                |node, on| node.run_post_script = on,
            )
        });
        let hint = match (post, rest) {
            (true, _) => {
                "The saved request's post script runs, and its tests. Turn this off to run a script of this node instead."
            }
            (false, true) => {
                "Runs when the node has its output, before the next node is handed it. lp.output is that output; lp.input, lp.nodes.<node>, lp.flow.<input> and lp.environment are there to read. The request's tests still run; its pre script never does in a flow."
            }
            (false, false) => {
                "Runs each time the node has an output, before the next node is handed it. lp.output is that output; lp.input, lp.nodes.<node>, lp.flow.<input> and lp.environment are there to read."
            }
        };
        let theme = cx.theme();
        v_flex()
            .gap_2()
            .child(page::section_label("After execute", cx))
            .children(switch)
            .child(div().text_xs().text_color(theme.muted_foreground).child(hint))
            .when(!post, |this| this.child(Editor::new(self.after_execute.state()).h(px(200.0))))
            .into_any_element()
    }

    fn render_number(&self, number: Number, cx: &Context<Self>) -> AnyElement {
        let input = &self.numbers.iter().find(|(known, _)| *known == number).expect("every number has a field").1;
        Self::labelled(number.label(), Input::new(input).small(), cx)
    }

    fn render_switch(
        id: &'static str,
        label: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        set: fn(&mut FlowNode, bool),
    ) -> AnyElement {
        Switch::new(id)
            .small()
            .checked(on)
            .label(label)
            .on_change(cx.listener(move |this, checked: &bool, _, cx| {
                let checked = *checked;
                this.edit(cx, |node| set(node, checked));
            }))
            .into_any_element()
    }

    fn render_branches(&self, node: &FlowNode, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let branches = node.branches.iter().zip(&self.branches).enumerate().map(|(index, (branch, row))| {
            let any = branch.matching == "any";
            let rules = branch.rules.iter().zip(&row.rules).enumerate().map(|(rule_index, (rule, fields))| {
                let this = cx.weak_entity();
                let current = rule.op.clone();
                let op = Button::new(ElementId::Name(format!("flow-rule-op-{index}-{rule_index}").into()))
                    .xsmall()
                    .outline()
                    .label(op_label(&rule.op))
                    .icon(IconName::ChevronDown)
                    .dropdown_menu(move |menu, _, _| {
                        OPS.iter().fold(menu, |menu, (name, label)| {
                            let this = this.clone();
                            menu.item(PopupMenuItem::new(*label).checked(*name == current).on_click(move |_, _, cx| {
                                this.update(cx, |this, cx| {
                                    this.edit(cx, |node| node.branches[index].rules[rule_index].op = name.to_string())
                                })
                                .ok();
                            }))
                        })
                    });
                v_flex()
                    .gap_1()
                    .child(
                        h_flex().gap_1().child(div().flex_1().min_w_0().child(Input::new(&fields.path).small())).child(
                            Button::new(ElementId::Name(format!("flow-rule-remove-{index}-{rule_index}").into()))
                                .xsmall()
                                .ghost()
                                .icon(IconName::Close)
                                .tooltip("Remove this rule")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.restructure(window, cx, |node| {
                                        node.branches[index].rules.remove(rule_index);
                                    })
                                })),
                        ),
                    )
                    .child(h_flex().gap_1().child(op).when(op_takes_value(&rule.op), |this| {
                        this.child(div().flex_1().min_w_0().child(Input::new(&fields.value).small()))
                    }))
                    .into_any_element()
            });
            let rules: Vec<AnyElement> = rules.collect();
            v_flex()
                .gap_2()
                .p_2()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .child(
                    h_flex().gap_1().child(div().flex_1().min_w_0().child(Input::new(&row.name).small())).child(
                        Button::new(ElementId::Name(format!("flow-branch-remove-{index}").into()))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Delete)
                            .tooltip("Remove this branch and its connections")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.restructure(window, cx, |node| {
                                    node.branches.remove(index);
                                })
                            })),
                    ),
                )
                .child(
                    Switch::new(ElementId::Name(format!("flow-branch-any-{index}").into()))
                        .small()
                        .checked(any)
                        .label("Taken when any one rule holds, not only when all do")
                        .on_change(cx.listener(move |this, checked: &bool, _, cx| {
                            let matching = if *checked { "any" } else { "all" };
                            this.edit(cx, |node| node.branches[index].matching = matching.to_owned());
                        })),
                )
                .children(rules)
                .child(
                    Button::new(ElementId::Name(format!("flow-rule-add-{index}").into()))
                        .xsmall()
                        .ghost()
                        .icon(IconName::Plus)
                        .label("Add a rule")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.restructure(window, cx, |node| {
                                node.branches[index].rules.push(Rule {
                                    path: "input.status".to_owned(),
                                    op: "equals".to_owned(),
                                    value: String::new(),
                                });
                            })
                        })),
                )
                .into_any_element()
        });
        let branches: Vec<AnyElement> = branches.collect();
        v_flex()
            .gap_2()
            .child(page::section_label("Branches, tried in this order; the first that holds is taken", cx))
            .children(branches)
            .child(
                Button::new("flow-branch-add").small().outline().icon(IconName::Plus).label("Add a branch").on_click(
                    cx.listener(|this, _, window, cx| {
                        this.restructure(window, cx, |node| {
                            let id = (1..)
                                .map(|n| format!("branch_{n}"))
                                .find(|id| !node.branches.iter().any(|branch| branch.id == *id))
                                .expect("an unused id");
                            node.branches.push(Branch { id, name: "Branch".to_owned(), matching: "all".to_owned(), rules: Vec::new() });
                        })
                    }),
                ),
            )
            .child(div().text_xs().text_color(theme.muted_foreground).child(
                "A rule reads a value by its path: input.status, input.body.items[0].id, input.headers.content-type, nodes.<node>.body.",
            ))
            .into_any_element()
    }

    fn render_connections(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let rows = self.edges.iter().map(|(id, label)| {
            let edge_id = id.clone();
            h_flex()
                .gap_2()
                .text_sm()
                .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().child(label.clone()))
                .child(
                    Button::new(ElementId::Name(format!("flow-edge-remove-{id}").into()))
                        .xsmall()
                        .ghost()
                        .icon(IconName::Close)
                        .tooltip("Remove this connection")
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(InspectorEvent::Disconnect(edge_id.clone())))),
                )
                .into_any_element()
        });
        let rows: Vec<AnyElement> = rows.collect();
        v_flex()
            .gap_1()
            .child(page::section_label("Connections", cx))
            .when(self.edges.is_empty(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("Drag from a dot on the node's right edge onto the node that runs next."),
                )
            })
            .children(rows)
            .into_any_element()
    }

    fn render_run(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let run = self.run.as_ref().filter(|run| run.status != "idle")?;
        let theme = cx.theme().clone();
        let mut summary = run.status.clone();
        if let Some(port) = &run.port {
            summary.push_str(&format!(", left by “{port}”"));
        }
        if run.runs > 1 {
            summary.push_str(&format!(", ran {} times", run.runs));
        }
        if let Some(took) = run.duration_ms {
            summary.push_str(&format!(", {took} ms"));
        }
        let block = |label: &'static str, pane: &Entity<BodyPane>| {
            v_flex().gap_1().child(page::section_label(label, cx)).child(
                div().rounded(theme.radius).border_1().border_color(theme.border).overflow_hidden().child(pane.clone()),
            )
        };
        Some(
            v_flex()
                .gap_2()
                .child(page::section_label("In the last run", cx))
                .child(div().text_sm().child(summary))
                .when_some(run.error.clone(), |this, error| {
                    this.child(div().text_sm().text_color(theme.danger).child(error))
                })
                .when(!run.output.is_null(), |this| this.child(block("What it produced", &self.produced)))
                .when(!run.input.is_null(), |this| this.child(block("What it was handed", &self.handed)))
                .into_any_element(),
        )
    }
}

impl Render for Inspector {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let Some(node) = self.node.clone() else {
            return div().into_any_element();
        };
        let mut sections: Vec<AnyElement> = Vec::new();
        sections.push(
            v_flex()
                .gap_1()
                .child(page::section_label(node.kind.label(), cx))
                .child(Input::new(&self.name).small())
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!("Later nodes read what this one produced as {{{{nodes.{}.…}}}}", node.id)),
                )
                .into_any_element(),
        );
        match node.kind {
            NodeKind::Start => sections.push(
                v_flex()
                    .gap_1()
                    .child(page::section_label("Inputs of the flow", cx))
                    .child(div().text_xs().text_color(theme.muted_foreground).child(
                        "A run can be given a value for each; requests read one as {{name}}, nodes as {{flow.name}}.",
                    ))
                    .child(div().h(px(180.0)).child(self.inputs.clone()))
                    .into_any_element(),
            ),
            NodeKind::Request => {
                sections.push(self.render_request_picker(&node, cx));
                let switches = v_flex()
                    .gap_2()
                    .child(Self::render_switch(
                        "flow-fail-status",
                        "An answer that is not a success leaves by Error",
                        node.fail_on_status,
                        cx,
                        |node, on| node.fail_on_status = on,
                    ))
                    .child(Self::render_switch(
                        "flow-fail-tests",
                        "A failed test leaves by Error",
                        node.fail_on_tests,
                        cx,
                        |node, on| node.fail_on_tests = on,
                    ));
                sections.push(switches.into_any_element());
                sections.push(self.render_number(Number::Retries, cx));
                sections.push(self.render_number(Number::RetryDelay, cx));
                sections.push(self.render_after_execute(&node, cx));
            }
            NodeKind::Stream => {
                sections.push(self.render_request_picker(&node, cx));
                if node.request_kind == RequestKind::SocketIo.entity_kind() {
                    sections.push(Self::labelled("Event to emit", Input::new(&self.event).small(), cx));
                }
                if node.request_kind != RequestKind::Sse.entity_kind() {
                    sections.push(Self::labelled(
                        "Message sent once connected, and each time the flow reaches this node again",
                        Input::new(&self.message).small(),
                        cx,
                    ));
                }
                for number in [Number::MaxMessages, Number::IdleTimeout, Number::Listen] {
                    sections.push(self.render_number(number, cx));
                }
                sections.push(self.render_after_execute(&node, cx));
            }
            NodeKind::Mcp => {
                sections.push(self.render_request_picker(&node, cx));
                let calls = h_flex().gap_1().children(["tool", "prompt", "resource"].map(|call| {
                    let chosen = node.call == call;
                    Button::new(call)
                        .small()
                        .when(chosen, |this| this.primary())
                        .when(!chosen, |this| this.outline())
                        .label(match call {
                            "tool" => "Tool",
                            "prompt" => "Prompt",
                            _ => "Resource",
                        })
                        .on_click(cx.listener(move |this, _, _, cx| this.edit(cx, |node| node.call = call.to_owned())))
                }));
                sections.push(Self::labelled("Calls", calls, cx));
                sections.push(Self::labelled("Named", Input::new(&self.target).small(), cx));
                if node.call != "resource" {
                    sections.push(Self::labelled("Arguments", div().h(px(150.0)).child(self.arguments.clone()), cx));
                }
                sections.push(self.render_number(Number::Retries, cx));
                sections.push(self.render_number(Number::RetryDelay, cx));
                sections.push(self.render_after_execute(&node, cx));
            }
            NodeKind::Condition => sections.push(self.render_branches(&node, cx)),
            NodeKind::Delay => sections.push(self.render_number(Number::Delay, cx)),
        }
        sections.push(self.render_connections(cx));
        sections.extend(self.render_run(cx));
        if node.kind != NodeKind::Start {
            let id = node.id.clone();
            sections.push(
                Button::new("flow-node-delete")
                    .small()
                    .danger()
                    .outline()
                    .icon(IconName::Delete)
                    .label("Remove this node")
                    .tooltip("Delete, with the node picked on the canvas")
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(InspectorEvent::Delete(id.clone()))))
                    .into_any_element(),
            );
        }
        v_flex()
            .id("flow-inspector")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .gap_4()
            // A section keeps its height when the panel is shorter than what it holds: the panel scrolls.
            .children(sections.into_iter().map(|section| div().flex_none().child(section)))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::config::FolderInfo;

    fn stub(kind: RequestKind, id: &str, name: &str, folder: Option<&str>) -> RequestStub {
        RequestStub {
            kind,
            id: id.to_owned(),
            name: name.to_owned(),
            folder_id: folder.map(str::to_owned),
            method: (kind == RequestKind::Rest).then(|| "GET".to_owned()),
            target: format!("http://api.test/{id}"),
            deleted: false,
            locked: None,
        }
    }

    #[test]
    fn requests_are_listed_under_their_folders_for_the_node_that_can_run_them() {
        let folder = |id: &str, name: &str, parent: Option<&str>| FolderInfo {
            id: id.to_owned(),
            name: name.to_owned(),
            parent_id: parent.map(str::to_owned),
            deleted: false,
        };
        let config = ConfigSnapshot {
            folders: vec![folder("u", "Users", None), folder("a", "Admin", Some("u")), folder("e", "Empty", None)],
            requests: vec![
                stub(RequestKind::Rest, "r2", "list", Some("u")),
                stub(RequestKind::Rest, "r1", "Create", Some("u")),
                stub(RequestKind::Grpc, "g1", "Ban", Some("a")),
                stub(RequestKind::Sse, "s1", "Feed", Some("u")),
                stub(RequestKind::Rest, "r3", "Stranded", Some("gone")),
                RequestStub { deleted: true, ..stub(RequestKind::Rest, "r4", "Deleted", Some("u")) },
            ],
            ..ConfigSnapshot::default()
        };
        let names = |kind: NodeKind| -> Vec<(String, Vec<String>)> {
            request_groups(&config, kind)
                .into_iter()
                .map(|(folder, items)| (folder, items.iter().map(|item| item.name.to_string()).collect()))
                .collect()
        };
        assert_eq!(
            names(NodeKind::Request),
            [
                ("No folder".to_owned(), vec!["Stranded".to_owned()]),
                ("Users".to_owned(), vec!["Create".to_owned(), "list".to_owned()]),
                ("Users / Admin".to_owned(), vec!["Ban".to_owned()]),
            ]
        );
        assert_eq!(names(NodeKind::Stream).len(), 2, "a gRPC request can be listened to as well");
        assert!(names(NodeKind::Mcp).is_empty());

        let groups = request_groups(&config, NodeKind::Request);
        let ban = &groups[2].1[0];
        assert_eq!((ban.tag.as_ref(), ban.key.clone()), ("gRPC", (RequestKind::Grpc, "g1".to_owned())));
        assert!(ban.matches("admin") && ban.matches("API.test/g1") && ban.matches("ban") && !ban.matches("feed"));
    }
}
