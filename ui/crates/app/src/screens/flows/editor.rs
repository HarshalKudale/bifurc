//! The controls of the node that is selected, drawn inside the node itself: what it runs, what it compares,
//! how long it waits.
//!
//! The editor holds an edit of one node. Its fields are written when a node is selected and never after, so
//! typing is not disturbed by the flow being saved; every change is reported whole as
//! [`EditorEvent::Changed`] and the screen puts it in the flow. A part of a response is chosen by clicking
//! through a tree of what the node is handed, never by typing a path.

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use gpui_kit::component::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    combobox::{Combobox, ComboboxEvent, ComboboxState},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    searchable_list::{SearchableGroup, SearchableListItem, SearchableVec},
    select::{Select, SelectEvent, SelectState},
    switch::Switch,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    rems,
};
use serde_json::{Value, json};

use super::json_view::{Pick, Toggle, flip, json_tree};
use crate::{
    app_model::{AppModel, ModelEvent},
    components::{KeyValueEvent, KeyValueTable},
    engine::{call_in, describe},
    model::{
        ConfigSnapshot, RequestKind,
        config::RequestStub,
        flow::{Branch, FlowNode, NodeKind, Pair, Rule},
        mcp::{ArgumentInput, McpArgument, McpItem, McpView},
        socket::SocketState,
        tree::folder_choices,
    },
};

/// The comparisons a rule can make: the name the flow stores, and how it reads.
const OPS: [(&str, &str); 16] = [
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
    ("isType", "is of the type"),
];

pub fn op_label(op: &str) -> &'static str {
    OPS.iter().find(|(name, _)| *name == op).map_or("equals", |(_, label)| label)
}

/// Whether a comparison is made against a value, or only asks about the one it reads.
pub fn op_takes_value(op: &str) -> bool {
    !matches!(op, "exists" | "notExists" | "isEmpty" | "notEmpty")
}

/// A rule in a line: `input.status equals 200`.
pub fn rule_summary(rule: &Rule) -> String {
    let value = if op_takes_value(&rule.op) { rule.value.as_str() } else { "" };
    format!("{} {} {value}", rule.path, op_label(&rule.op)).trim_end().to_owned()
}

/// The request types a node of this kind can run.
fn request_kinds(kind: NodeKind) -> &'static [RequestKind] {
    match kind {
        NodeKind::Request => &[
            RequestKind::Rest,
            RequestKind::GraphQl,
            RequestKind::Soap,
            RequestKind::Grpc,
            RequestKind::Sse,
            RequestKind::WebSocket,
            RequestKind::SocketIo,
        ],
        NodeKind::Stream => &[RequestKind::Sse, RequestKind::WebSocket, RequestKind::SocketIo, RequestKind::Grpc],
        NodeKind::Mcp => &[RequestKind::Mcp],
        _ => &[],
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

/// What the server of an MCP node declares, by name alone: what one does is said under the one chosen.
type ToolList = SearchableVec<SharedString>;

/// How an argument of an MCP node is given: typed, or chosen from the values the server limits it to.
enum ArgumentField {
    Text(Entity<InputState>),
    Choice { state: Entity<SelectState<Vec<String>>>, required: bool },
}

/// The values an argument is chosen from, when it is one that is chosen: those its schema limits it to, or
/// true and false. What is held and is not one of them (a `{{reference}}`) stays typed, so it is not lost.
fn argument_choices(argument: &McpArgument, held: &str) -> Option<Vec<String>> {
    let choices = match argument.input() {
        ArgumentInput::Choice => argument.choices.clone(),
        ArgumentInput::Flag => vec!["true".to_owned(), "false".to_owned()],
        ArgumentInput::Json | ArgumentInput::Text => return None,
    };
    (held.is_empty() || choices.iter().any(|choice| choice == held)).then_some(choices)
}

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

/// A part of what a node is handed, as a rule names it: `input.body.items[0].id`.
// shortcut: a key that holds a dot or a bracket cannot be named this way, the engine would read it as two
// steps; store a rule's path as a list of steps, as a wire does, if such a key needs comparing.
pub fn rule_path(path: &[Value]) -> String {
    let rest = crate::model::flow::path_label(path);
    match rest.starts_with('[') || rest.is_empty() {
        true => format!("input{rest}"),
        false => format!("input.{rest}"),
    }
}

/// The types a data node's value can be given as.
const DATA_TYPES: [(&str, &str); 4] = [("text", "Text"), ("number", "Number"), ("boolean", "Boolean"), ("json", "JSON")];

fn data_type_of(value: &Value) -> &'static str {
    match value {
        Value::String(_) => "text",
        Value::Number(_) => "number",
        Value::Bool(_) => "boolean",
        _ => "json",
    }
}

fn data_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// What a data node holds when `text` is typed for a value of the type `data_type`. Text that is not such a
/// value yet (half a number, JSON still being written) leaves what was held.
fn data_value(data_type: &str, text: &str, held: &Value) -> Value {
    match data_type {
        "text" => Value::String(text.to_owned()),
        "number" => serde_json::from_str::<Value>(text.trim()).ok().filter(Value::is_number).unwrap_or_else(|| held.clone()),
        "json" => serde_json::from_str(text).unwrap_or_else(|_| held.clone()),
        _ => held.clone(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EditorEvent {
    /// The node as it now is.
    Changed(Box<FlowNode>),
    /// The flow's inputs, which the start node shows.
    Inputs(Vec<Pair>),
}

/// The fields a node keeps as a count or a time in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Number {
    Delay,
    Count,
    Retries,
    RetryDelay,
    MaxMessages,
    IdleTimeout,
    Listen,
}

impl Number {
    const ALL: [Number; 7] = [
        Number::Delay,
        Number::Count,
        Number::Retries,
        Number::RetryDelay,
        Number::MaxMessages,
        Number::IdleTimeout,
        Number::Listen,
    ];

    fn label(self) -> &'static str {
        match self {
            Number::Delay => "Wait (ms)",
            Number::Count => "Times",
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
            Number::Count => node.count,
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
            Number::Count => node.count = value.min(10_000),
            Number::Retries => node.retries = value.min(20) as u32,
            Number::RetryDelay => node.retry_delay_ms = value,
            Number::MaxMessages => node.max_messages = value,
            Number::IdleTimeout => node.idle_timeout_ms = value,
            Number::Listen => node.listen_ms = value,
        }
    }
}

/// The rule, or the list, whose part of the response is being chosen. A validate node's rules are in no
/// branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Choosing {
    Rule { branch: Option<usize>, rule: usize },
    List,
}

fn rules_mut(node: &mut FlowNode, branch: Option<usize>) -> Option<&mut Vec<Rule>> {
    match branch {
        Some(index) => node.branches.get_mut(index).map(|branch| &mut branch.rules),
        None => Some(&mut node.rules),
    }
}

/// The arguments a node gives the tool or prompt it calls: one for each the server says it takes, in its
/// order, with what was already typed for it.
fn arguments_for(item: &McpItem, held: &[Pair]) -> Vec<Pair> {
    let typed = |name: &str| held.iter().find(|pair| pair.name == name).map(|pair| pair.value.clone());
    item.arguments
        .iter()
        .map(|argument| Pair { name: argument.name.clone(), value: typed(&argument.name).unwrap_or_default() })
        .collect()
}

/// What the server takes for an argument, as its empty field says it: its type, whether it must be given,
/// and what is used when it is not.
fn argument_hint(argument: &McpArgument) -> String {
    let default = argument.default.as_ref().map(|default| format!("default {default}")).unwrap_or_default();
    [argument.kind.as_str(), if argument.required { "required" } else { "" }, default.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

pub struct NodeEditor {
    model: Entity<AppModel>,
    node: Option<FlowNode>,
    /// What the node is handed, as far as is known before it runs again: what its last run was, or what the
    /// node before it last produced or holds.
    handed: Value,
    name: Entity<InputState>,
    message: Entity<InputState>,
    event: Entity<InputState>,
    target: Entity<InputState>,
    expression: Entity<InputState>,
    data: Entity<InputState>,
    data_type: &'static str,
    numbers: Vec<(Number, Entity<InputState>)>,
    /// The saved requests the node can run, and the list as it was last handed to the picker.
    requests: Entity<ComboboxState<RequestList>>,
    listed: Vec<(String, Vec<RequestItem>)>,
    /// What the node's server declares of the kind the node calls, and the names as they were last handed to
    /// the picker.
    tools: Entity<ComboboxState<ToolList>>,
    tools_listed: Vec<SharedString>,
    /// The field of each argument of an MCP node, by the argument's name. The canvas draws each in the row
    /// of the input it is.
    arguments: Vec<(String, ArgumentField)>,
    /// What the MCP server of the node declares, and the saved request it was asked of.
    mcp: McpView,
    mcp_of: String,
    variables: Entity<KeyValueTable>,
    inputs: Entity<KeyValueTable>,
    branch_names: Vec<Entity<InputState>>,
    /// The value field of every rule: a condition's by branch, a validate node's as its one list.
    rule_values: Vec<Vec<Entity<InputState>>>,
    choosing: Option<Choosing>,
    open: HashSet<String>,
    _rule_subscriptions: Vec<Subscription>,
    _argument_subscriptions: Vec<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EditorEvent> for NodeEditor {}

fn pairs_of(pairs: &[Pair]) -> Vec<(String, String)> {
    pairs.iter().map(|pair| (pair.name.clone(), pair.value.clone())).collect()
}

fn to_pairs(pairs: Vec<(String, String)>) -> Vec<Pair> {
    pairs.into_iter().map(|(name, value)| Pair { name, value }).collect()
}

impl NodeEditor {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let name = field("Name", window, cx);
        let message = field("What to send", window, cx);
        let event = field("Event name", window, cx);
        let target = field("Name of the tool, prompt or resource URI", window, cx);
        let expression = field("vars.total * 2", window, cx);
        let data = field("Value", window, cx);
        let numbers: Vec<(Number, Entity<InputState>)> =
            Number::ALL.iter().map(|number| (*number, field("0", window, cx))).collect();
        let requests =
            cx.new(|cx| ComboboxState::new(RequestList::new(Vec::new()), vec![], window, cx).searchable(true));
        let tools = cx.new(|cx| ComboboxState::new(ToolList::new(Vec::new()), vec![], window, cx).searchable(true));
        let variables = cx.new(|cx| KeyValueTable::new("Variable", "Value", window, cx));
        let inputs = cx.new(|cx| KeyValueTable::new("Input", "Default value", window, cx));

        let on_input = |this: &mut Self, _: Entity<InputState>, event: &InputEvent, cx: &mut Context<Self>| {
            if matches!(event, InputEvent::Change) {
                this.changed(cx);
            }
        };
        let mut subscriptions: Vec<Subscription> = [&name, &message, &event, &target, &expression, &data]
            .into_iter()
            .map(|input| cx.subscribe(input, on_input))
            .collect();
        subscriptions.extend(numbers.iter().map(|(_, input)| cx.subscribe(input, on_input)));
        subscriptions.push(cx.subscribe_in(
            &requests,
            window,
            |this, _, event: &ComboboxEvent<RequestList>, window, cx| {
                if let ComboboxEvent::Change(chosen) = event
                    && let Some((kind, id)) = chosen.first().cloned()
                {
                    this.edit(cx, |node| {
                        node.request_kind = kind.entity_kind().to_owned();
                        node.request_id = id;
                    });
                    this.attach_mcp(window, cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(&tools, window, |this, _, event: &ComboboxEvent<ToolList>, window, cx| {
            if let ComboboxEvent::Change(chosen) = event
                && let Some(name) = chosen.first()
                && this.node.as_ref().is_some_and(|node| node.target != name.as_ref())
            {
                this.call_item(name.to_string(), window, cx);
            }
        }));
        subscriptions.push(cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| {
            if let ModelEvent::McpStatus(payload) = event
                && !this.mcp_of.is_empty()
                && payload.get("connectionId").and_then(Value::as_str) == Some(this.mcp_of.as_str())
            {
                this.mcp.apply_status_event(payload);
                this.declared(window, cx);
            }
        }));
        // A request saved, renamed or deleted elsewhere while a node is selected is one the picker knows of.
        subscriptions.push(cx.observe_in(&model, window, |this, _, window, cx| this.list_requests(false, window, cx)));
        subscriptions.push(cx.subscribe(&variables, |this, _, _: &KeyValueEvent, cx| this.changed(cx)));
        subscriptions.push(cx.subscribe(&inputs, |_, table, _: &KeyValueEvent, cx| {
            let pairs = to_pairs(table.read(cx).pairs(cx));
            cx.emit(EditorEvent::Inputs(pairs));
        }));

        Self {
            model,
            node: None,
            handed: Value::Null,
            name,
            message,
            event,
            target,
            expression,
            data,
            data_type: "text",
            numbers,
            requests,
            listed: Vec::new(),
            tools,
            tools_listed: Vec::new(),
            arguments: Vec::new(),
            mcp: McpView::default(),
            mcp_of: String::new(),
            variables,
            inputs,
            branch_names: Vec::new(),
            rule_values: Vec::new(),
            choosing: None,
            open: HashSet::new(),
            _rule_subscriptions: Vec::new(),
            _argument_subscriptions: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn node_id(&self) -> Option<&str> {
        self.node.as_ref().map(|node| node.id.as_str())
    }

    /// The field the selected node's name is typed in. The canvas draws it in the node's header.
    pub fn name_field(&self) -> &Entity<InputState> {
        &self.name
    }

    /// What the value of an input of the selected node is given in, for the inputs that have one: the
    /// arguments of an MCP node. One the server limits to certain values is chosen from them.
    pub fn input_control(&self, input_id: &str) -> Option<AnyElement> {
        let name = input_id.strip_prefix("arg:")?;
        let (_, field) = self.arguments.iter().find(|(known, _)| known == name)?;
        Some(match field {
            ArgumentField::Text(field) => Input::new(field).xsmall().into_any_element(),
            ArgumentField::Choice { state, required } => Select::new(state)
                .xsmall()
                .w_full()
                .placeholder(if *required { "Choose one" } else { "Not set" })
                // An argument that need not be sent can be put back to nothing chosen.
                .cleanable(!required)
                .into_any_element(),
        })
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
            set(&self.expression, &node.expression);
            set(&self.data, &data_text(&node.value));
            for (number, input) in &self.numbers {
                let value = number.read(node);
                set(input, &if value == 0 { String::new() } else { value.to_string() });
            }
            self.data_type = data_type_of(&node.value);
            self.variables.update(cx, |table, cx| table.set_pairs(&pairs_of(&node.variables), window, cx));
            self.inputs.update(cx, |table, cx| table.set_pairs(&pairs_of(inputs), window, cx));
        }
        self.node = node;
        self.choosing = None;
        self.list_requests(true, window, cx);
        self.rebuild_rules(window, cx);
        self.rebuild_arguments(window, cx);
        // A new MCP node runs the one saved MCP request there is, when there is just one to choose.
        let only = match self.listed.as_slice() {
            [(_, items)] if items.len() == 1 => Some(items[0].key.clone()),
            _ => None,
        };
        if let Some((kind, id)) = only
            && self.node.as_ref().is_some_and(|node| node.kind == NodeKind::Mcp && node.request_id.is_empty())
        {
            self.edit(cx, |node| (node.request_kind, node.request_id) = (kind.entity_kind().to_owned(), id));
            self.list_requests(true, window, cx);
        }
        self.attach_mcp(window, cx);
        self.list_tools(true, window, cx);
        cx.notify();
    }

    /// Hand the picker what the node's server declares of the kind the node calls, with what the node calls
    /// chosen. Unless `anew`, a list that did not change is left alone, as a search may be typed in it.
    fn list_tools(&mut self, anew: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.node.as_ref().filter(|node| node.kind == NodeKind::Mcp) else { return };
        let names: Vec<SharedString> = self.mcp_items(&node.call).iter().map(|item| item.name.clone().into()).collect();
        let chosen = self.tools.read(cx).selected_value().unwrap_or_default();
        if !anew && names == self.tools_listed && chosen.as_ref() == node.target {
            return;
        }
        let target = [SharedString::from(node.target.clone())];
        self.tools_listed = names.clone();
        self.tools.update(cx, |picker, cx| {
            picker.set_items(ToolList::new(names), window, cx);
            picker.set_selected_values(&target, window, cx);
        });
    }

    /// What the node's server declares of the kind the node calls.
    fn mcp_items(&self, call: &str) -> &[McpItem] {
        match call {
            "tool" => &self.mcp.tools,
            "prompt" => &self.mcp.prompts,
            _ => &self.mcp.resources,
        }
    }

    /// Ask the engine what the MCP server of the node declares, connecting to it when nothing has yet. The
    /// connection is the saved request's own, the one its tab shows, so it is left open.
    fn attach_mcp(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.node.as_ref().filter(|node| node.kind == NodeKind::Mcp && !node.request_id.is_empty());
        let Some(id) = wanted.map(|node| node.request_id.clone()) else { return };
        if self.mcp_of != id {
            (self.mcp, self.mcp_of) = (McpView::default(), id.clone());
        }
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "connectionId": id });
        let (connect_client, connect) = (client.clone(), payload.clone());
        call_in(&client, window, cx, "mcp.status", payload, move |this, outcome, window, cx| {
            if this.mcp_of != id {
                return;
            }
            if let Ok(status) = outcome {
                this.mcp = McpView::from_status(&status);
            }
            if this.mcp.state.is_live() {
                return this.declared(window, cx);
            }
            this.mcp.state = SocketState::Connecting;
            cx.notify();
            // What the server declares comes with the status the engine reports once it is reached.
            call_in(&connect_client, window, cx, "mcp.connect", connect, move |this, outcome, _, cx| {
                let refused = match outcome {
                    Ok(answer) if answer.get("ok").and_then(Value::as_bool) == Some(false) => {
                        Some(answer.get("error").and_then(Value::as_str).unwrap_or("It refused.").to_owned())
                    }
                    Ok(_) => None,
                    Err(error) => Some(describe(&error)),
                };
                if let Some(error) = refused.filter(|_| this.mcp_of == id) {
                    (this.mcp.state, this.mcp.error) = (SocketState::Error, Some(error));
                    cx.notify();
                }
            });
        });
    }

    /// The server said what it declares: the node takes the arguments of what it calls.
    fn declared(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        self.list_tools(false, window, cx);
        let Some(node) = self.node.as_ref().filter(|node| node.kind == NodeKind::Mcp) else { return };
        let Some(item) = self.mcp_items(&node.call).iter().find(|item| item.name == node.target) else { return };
        let arguments = arguments_for(item, &node.arguments);
        if arguments != node.arguments {
            self.take_arguments(arguments, window, cx);
        } else {
            // The same arguments, now with what the server says each takes beside them.
            self.rebuild_arguments(window, cx);
        }
    }

    fn take_arguments(&mut self, arguments: Vec<Pair>, window: &mut Window, cx: &mut Context<Self>) {
        // What is typed in the fields goes into the node first, so that rebuilding them loses nothing.
        self.changed(cx);
        if let Some(node) = self.node.as_mut() {
            node.arguments = arguments;
        }
        self.rebuild_arguments(window, cx);
        self.changed(cx);
    }

    /// The node calls this tool, prompt or resource of its server, with the arguments it takes.
    fn call_item(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.node.as_ref() else { return };
        let item = self.mcp_items(&node.call).iter().find(|item| item.name == name);
        let arguments = item.map(|item| arguments_for(item, &node.arguments)).unwrap_or_default();
        self.target.update(cx, |state, cx| state.set_value(name, window, cx));
        self.take_arguments(arguments, window, cx);
        self.list_tools(false, window, cx);
    }

    /// Fresh fields for the arguments of the node, each saying what the server takes for it.
    fn rebuild_arguments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let node = self.node.as_ref().filter(|node| node.kind == NodeKind::Mcp);
        let held = node.map(|node| node.arguments.clone()).unwrap_or_default();
        let item =
            node.and_then(|node| self.mcp_items(&node.call).iter().find(|item| item.name == node.target)).cloned();
        let mut subscriptions = Vec::new();
        let mut fields = Vec::new();
        for pair in held {
            let argument = item.as_ref().and_then(|item| item.arguments.iter().find(|a| a.name == pair.name));
            if let Some(choices) = argument.and_then(|argument| argument_choices(argument, &pair.value)) {
                let state = cx.new(|cx| SelectState::new(choices, None, window, cx));
                if !pair.value.is_empty() {
                    state.update(cx, |state, cx| state.set_selected_value(&pair.value, window, cx));
                }
                subscriptions.push(cx.subscribe(&state, |this: &mut Self, _, _: &SelectEvent<Vec<String>>, cx| {
                    this.changed(cx)
                }));
                let required = argument.is_some_and(|argument| argument.required);
                fields.push((pair.name, ArgumentField::Choice { state, required }));
                continue;
            }
            let hint = argument.map(argument_hint).filter(|hint| !hint.is_empty()).unwrap_or_else(|| "value".to_owned());
            let field = cx.new(|cx| InputState::new(window, cx).placeholder(hint).default_value(pair.value));
            subscriptions.push(cx.subscribe(&field, |this: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.changed(cx);
                }
            }));
            fields.push((pair.name, ArgumentField::Text(field)));
        }
        self.arguments = fields;
        self._argument_subscriptions = subscriptions;
    }

    /// The selected node was changed on the canvas, outside these controls: a switch on the node itself.
    /// The fields hold nothing of that, so they stay as they are.
    pub fn follow(&mut self, node: FlowNode) {
        if self.node_id() == Some(node.id.as_str()) {
            self.node = Some(node);
        }
    }

    /// What the selected node is handed, for choosing a part of it.
    pub fn set_handed(&mut self, handed: Value, cx: &mut Context<Self>) {
        if self.handed != handed {
            self.handed = handed;
            cx.notify();
        }
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

    /// Fresh fields for the node's branches and rules: their number changed, or another node is shown.
    fn rebuild_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (branches, rules) = match &self.node {
            Some(node) => (node.branches.clone(), node.rules.clone()),
            None => (Vec::new(), Vec::new()),
        };
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
        self.branch_names = branches.iter().map(|branch| field(&branch.name, "Branch name", cx)).collect();
        let lists: Vec<&Vec<Rule>> = match self.node.as_ref().map(|node| node.kind) {
            Some(NodeKind::Validate) => vec![&rules],
            _ => branches.iter().map(|branch| &branch.rules).collect(),
        };
        self.rule_values =
            lists.into_iter().map(|rules| rules.iter().map(|rule| field(&rule.value, "Value", cx)).collect()).collect();
        self._rule_subscriptions = subscriptions;
    }

    /// Read the controls into the node and report it.
    fn changed(&mut self, cx: &mut Context<Self>) {
        let Some(mut node) = self.node.clone() else { return };
        let read = |input: &Entity<InputState>| input.read(cx).value().to_string();
        node.name = read(&self.name).trim().to_owned();
        node.message = read(&self.message);
        node.event = read(&self.event).trim().to_owned();
        node.target = read(&self.target).trim().to_owned();
        node.expression = read(&self.expression);
        for (number, input) in &self.numbers {
            number.write(&mut node, read(input).trim().parse().unwrap_or(0));
        }
        match node.kind {
            NodeKind::Mcp => {
                for pair in &mut node.arguments {
                    pair.value = match self.arguments.iter().find(|(name, _)| *name == pair.name) {
                        Some((_, ArgumentField::Text(field))) => read(field),
                        Some((_, ArgumentField::Choice { state, .. })) => {
                            state.read(cx).selected_value().cloned().unwrap_or_default()
                        }
                        None => continue,
                    };
                }
            }
            NodeKind::Evaluate | NodeKind::If => node.variables = to_pairs(self.variables.read(cx).pairs(cx)),
            NodeKind::Data => node.value = data_value(self.data_type, &read(&self.data), &node.value),
            _ => {}
        }
        for (branch, name) in node.branches.iter_mut().zip(&self.branch_names) {
            branch.name = read(name).trim().to_owned();
        }
        let validates = node.kind == NodeKind::Validate;
        for (index, values) in self.rule_values.iter().enumerate() {
            let Some(rules) = rules_mut(&mut node, (!validates).then_some(index)) else { continue };
            for (rule, value) in rules.iter_mut().zip(values) {
                rule.value = read(value);
            }
        }
        self.node = Some(node.clone());
        cx.emit(EditorEvent::Changed(Box::new(node)));
        cx.notify();
    }

    /// Change something of the node that no field holds: a switch, a choice, the number of its branches.
    fn edit(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut FlowNode)) {
        if let Some(node) = self.node.as_mut() {
            change(node);
            self.changed(cx);
        }
    }

    /// Like [`Self::edit`], for a change after which the rule fields no longer line up with the rules.
    fn restructure(&mut self, window: &mut Window, cx: &mut Context<Self>, change: impl FnOnce(&mut FlowNode)) {
        // What is typed in the fields goes into the node first, so that rebuilding them loses nothing.
        self.changed(cx);
        self.choosing = None;
        if let Some(node) = self.node.as_mut() {
            change(node);
        }
        self.rebuild_rules(window, cx);
        self.changed(cx);
    }

    fn label(text: &'static str, cx: &App) -> AnyElement {
        div().text_color(cx.theme().muted_foreground).child(text).into_any_element()
    }

    fn labelled(label: &'static str, control: impl IntoElement, cx: &App) -> AnyElement {
        v_flex().gap_1().child(Self::label(label, cx)).child(control).into_any_element()
    }

    fn render_request_picker(&self, node: &FlowNode) -> AnyElement {
        let none = match node.kind {
            NodeKind::Stream => "No saved stream, WebSocket, Socket.IO or gRPC request yet",
            NodeKind::Mcp => "No saved MCP request yet",
            _ => "No saved request yet",
        };
        Combobox::new(&self.requests)
            .small()
            .placeholder(if self.listed.is_empty() { none } else { "Choose a saved request" })
            .search_placeholder("Search by name, folder or address")
            .disabled(self.listed.is_empty())
            .w_full()
            .into_any_element()
    }

    /// What the node calls: chosen from what its server declares, or typed while the server is not reached.
    fn render_mcp_target(&self, node: &FlowNode, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme().clone();
        let what = match node.call.as_str() {
            "tool" => "tool",
            "prompt" => "prompt",
            _ => "resource",
        };
        let note = |text: String| div().text_color(theme.muted_foreground).child(text).into_any_element();
        if node.request_id.is_empty() {
            return vec![note(format!("Choose the saved MCP request whose server has the {what}."))];
        }
        let items = self.mcp_items(&node.call);
        if items.is_empty() {
            let state = match (self.mcp.state, &self.mcp.error) {
                (SocketState::Connected, _) => format!("Its server declares no {what}."),
                (SocketState::Connecting, _) => "Connecting to its server to list what it has…".to_owned(),
                (_, Some(error)) => format!("Its server was not reached: {error}"),
                _ => "Its server is not connected.".to_owned(),
            };
            return vec![v_flex().gap_1().child(Input::new(&self.target).small()).child(note(state)).into_any_element()];
        }
        // The list is of names alone; what the one chosen does is said under it.
        let description = items.iter().find(|item| item.name == node.target).map(|item| item.description.trim().to_owned());
        let picker = Combobox::new(&self.tools)
            .small()
            .placeholder(format!("Choose a {what}"))
            .search_placeholder(format!("Search the {what}s"))
            .w_full()
            .into_any_element();
        let described = description.filter(|text| !text.is_empty()).map(|text| {
            div()
                .id("flow-mcp-description")
                // Not to be shrunk: the pickers ask for all the height there is, and what can scroll gives way.
                .flex_none()
                .max_h(rems(6.0))
                .overflow_y_scroll()
                .text_color(theme.muted_foreground)
                .child(text)
                .into_any_element()
        });
        [picker].into_iter().chain(described).collect()
    }

    fn render_numbers(&self, numbers: &[Number], cx: &Context<Self>) -> AnyElement {
        let fields = numbers.iter().map(|number| {
            let input = &self.numbers.iter().find(|(known, _)| known == number).expect("every number has a field").1;
            div().flex_1().min_w_0().child(Self::labelled(number.label(), Input::new(input).small(), cx))
        });
        h_flex().gap_2().items_end().children(fields).into_any_element()
    }

    fn render_choice(
        &self,
        id: &'static str,
        choices: &[(&'static str, &'static str)],
        chosen: &str,
        cx: &mut Context<Self>,
        choose: fn(&mut Self, &'static str, &mut Window, &mut Context<Self>),
    ) -> AnyElement {
        let buttons = choices.iter().map(|(value, label)| {
            let value = *value;
            Button::new(ElementId::Name(format!("{id}-{value}").into()))
                .xsmall()
                .outline()
                .selected(value == chosen)
                .label(*label)
                .on_click(cx.listener(move |this, _, window, cx| choose(this, value, window, cx)))
        });
        h_flex().gap_1().children(buttons).into_any_element()
    }

    /// The tree a part of what the node is handed is chosen from, under the rule or the list it is for.
    fn render_chooser(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let this = cx.weak_entity();
        let toggle: Toggle = {
            let this = this.clone();
            Rc::new(move |key, _, cx| {
                this.update(cx, |this, cx| {
                    flip(&mut this.open, key);
                    cx.notify();
                })
                .ok();
            })
        };
        let pick: Pick = Rc::new(move |path, window, cx| {
            this.update(cx, |this, cx| this.chose(rule_path(&path), window, cx)).ok();
        });
        let body = match self.handed.is_null() {
            true => div()
                .px_2()
                .py_1()
                .text_color(theme.muted_foreground)
                .child("Run the flow once, or put a Sample response node before this one, to choose from what it is handed.")
                .into_any_element(),
            false => json_tree("choose", &self.handed, &self.open, toggle, Some(pick), cx).into_any_element(),
        };
        v_flex()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .child(
                h_flex().px_2().pt_1().justify_between().child(Self::label("Click the part to read", cx)).child(
                    Button::new("flow-choose-all").xsmall().ghost().label("All of it").on_click(
                        cx.listener(|this, _, window, cx| this.chose("input".to_owned(), window, cx)),
                    ),
                ),
            )
            .child(body)
            .into_any_element()
    }

    /// A part was chosen for the rule or the list that asked.
    fn chose(&mut self, path: String, _: &mut Window, cx: &mut Context<Self>) {
        let Some(choosing) = self.choosing.take() else { return };
        self.edit(cx, |node| match choosing {
            Choosing::List => node.path = path,
            Choosing::Rule { branch, rule } => {
                if let Some(rule) = rules_mut(node, branch).and_then(|rules| rules.get_mut(rule)) {
                    rule.path = path;
                }
            }
        });
    }

    fn render_path_button(&self, id: String, path: &str, choosing: Choosing, cx: &mut Context<Self>) -> AnyElement {
        Button::new(ElementId::Name(id.into()))
            .xsmall()
            .outline()
            .selected(self.choosing == Some(choosing))
            .label(if path.is_empty() { "input".to_owned() } else { path.to_owned() })
            .icon(IconName::ChevronDown)
            .tooltip("Choose the part of the response to read")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.choosing = (this.choosing != Some(choosing)).then_some(choosing);
                cx.notify();
            }))
            .into_any_element()
    }

    /// The rules of one branch, or of a validate node (`branch` is `None`), with the button that adds one.
    fn render_rules(&self, branch: Option<usize>, rules: &[Rule], cx: &mut Context<Self>) -> Vec<AnyElement> {
        let tag = branch.map_or("v".to_owned(), |index| index.to_string());
        let values = self.rule_values.get(branch.unwrap_or(0)).map(Vec::as_slice).unwrap_or(&[]);
        let mut rows: Vec<AnyElement> = Vec::new();
        for (index, (rule, value)) in rules.iter().zip(values).enumerate() {
            let choosing = Choosing::Rule { branch, rule: index };
            let this = cx.weak_entity();
            let current = rule.op.clone();
            let op = Button::new(ElementId::Name(format!("flow-rule-op-{tag}-{index}").into()))
                .xsmall()
                .outline()
                .label(op_label(&rule.op))
                .icon(IconName::ChevronDown)
                .dropdown_menu(move |menu, _, _| {
                    OPS.iter().fold(menu, |menu, (name, label)| {
                        let this = this.clone();
                        menu.item(PopupMenuItem::new(*label).checked(*name == current).on_click(move |_, _, cx| {
                            this.update(cx, |this, cx| {
                                this.edit(cx, |node| {
                                    if let Some(rule) = rules_mut(node, branch).and_then(|rules| rules.get_mut(index)) {
                                        rule.op = name.to_string();
                                    }
                                })
                            })
                            .ok();
                        }))
                    })
                });
            rows.push(
                h_flex()
                    .gap_1()
                    .child(self.render_path_button(format!("flow-rule-path-{tag}-{index}"), &rule.path, choosing, cx))
                    .child(op)
                    .child(div().flex_1())
                    .child(
                        Button::new(ElementId::Name(format!("flow-rule-remove-{tag}-{index}").into()))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Close)
                            .tooltip("Remove this rule")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.restructure(window, cx, |node| {
                                    if let Some(rules) = rules_mut(node, branch).filter(|rules| index < rules.len()) {
                                        rules.remove(index);
                                    }
                                })
                            })),
                    )
                    .into_any_element(),
            );
            if op_takes_value(&rule.op) {
                rows.push(Input::new(value).small().into_any_element());
            }
            if self.choosing == Some(choosing) {
                rows.push(self.render_chooser(cx));
            }
        }
        rows.push(
            Button::new(ElementId::Name(format!("flow-rule-add-{tag}").into()))
                .xsmall()
                .ghost()
                .icon(IconName::Plus)
                .label("Add a rule")
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.restructure(window, cx, |node| {
                        if let Some(rules) = rules_mut(node, branch) {
                            rules.push(Rule { value: String::new(), ..Rule::status_is_200() });
                        }
                    })
                }))
                .into_any_element(),
        );
        rows
    }

    fn render_branches(&self, node: &FlowNode, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let mut branches: Vec<AnyElement> = Vec::new();
        for (index, (branch, name)) in node.branches.iter().zip(&self.branch_names).enumerate() {
            let any = branch.matching == "any";
            branches.push(
                v_flex()
                    .gap_2()
                    .p_2()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .child(
                        h_flex().gap_1().child(div().flex_1().min_w_0().child(Input::new(name).small())).child(
                            Button::new(ElementId::Name(format!("flow-branch-remove-{index}").into()))
                                .xsmall()
                                .ghost()
                                .icon(IconName::Delete)
                                .tooltip("Remove this branch and its connections")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.restructure(window, cx, |node| {
                                        if index < node.branches.len() {
                                            node.branches.remove(index);
                                        }
                                    })
                                })),
                        ),
                    )
                    .child(
                        Switch::new(ElementId::Name(format!("flow-branch-any-{index}").into()))
                            .small()
                            .checked(any)
                            .label("Any one rule is enough")
                            .on_change(cx.listener(move |this, checked: &bool, _, cx| {
                                let matching = if *checked { "any" } else { "all" };
                                this.edit(cx, |node| {
                                    if let Some(branch) = node.branches.get_mut(index) {
                                        branch.matching = matching.to_owned();
                                    }
                                });
                            })),
                    )
                    .children(self.render_rules(Some(index), &branch.rules, cx))
                    .into_any_element(),
            );
        }
        v_flex()
            .gap_2()
            .child(Self::label("Tried in this order; the first that holds is taken", cx))
            .children(branches)
            .child(
                Button::new("flow-branch-add").xsmall().outline().icon(IconName::Plus).label("Add a branch").on_click(
                    cx.listener(|this, _, window, cx| {
                        this.restructure(window, cx, |node| {
                            let id = (1..)
                                .map(|n| format!("branch_{n}"))
                                .find(|id| !node.branches.iter().any(|branch| branch.id == *id))
                                .expect("an unused id");
                            node.branches.push(Branch {
                                id,
                                name: "Branch".to_owned(),
                                matching: "all".to_owned(),
                                rules: Vec::new(),
                            });
                        })
                    }),
                ),
            )
            .into_any_element()
    }

    fn render_data(&self, node: &FlowNode, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let types = self.render_choice("flow-data-type", &DATA_TYPES, self.data_type, cx, |this, chosen, window, cx| {
            // The value is kept when it is one of the new type, and starts over when it is not.
            let text = this.data.read(cx).value().to_string();
            let value = match chosen {
                "text" => Value::String(text),
                "number" => data_value("number", &text, &Value::from(0)),
                "boolean" => Value::Bool(text.trim() == "true"),
                _ => data_value("json", &text, &serde_json::json!({})),
            };
            this.data_type = chosen;
            this.data.update(cx, |state, cx| state.set_value(data_text(&value), window, cx));
            this.edit(cx, |node| node.value = value);
        });
        let value = match self.data_type {
            "boolean" => Switch::new("flow-data-boolean")
                .small()
                .checked(node.value == Value::Bool(true))
                .label(if node.value == Value::Bool(true) { "true" } else { "false" })
                .on_change(cx.listener(|this, checked: &bool, _, cx| {
                    let checked = *checked;
                    this.edit(cx, |node| node.value = Value::Bool(checked));
                }))
                .into_any_element(),
            _ => Input::new(&self.data).small().into_any_element(),
        };
        let typed = self.data.read(cx).value().to_string();
        let unread = matches!(self.data_type, "number" | "json") && data_text(&node.value) != typed.trim();
        let mut out = vec![types, value];
        if unread {
            let what = if self.data_type == "number" { "a number" } else { "JSON" };
            out.push(
                div()
                    .text_color(cx.theme().warning)
                    .child(format!("This is not {what} yet. The node holds what it held before."))
                    .into_any_element(),
            );
        }
        out
    }

    /// Put the JSON that is on the clipboard in a sample node.
    fn paste_sample(&mut self, cx: &mut Context<Self>) {
        let pasted = cx.read_from_clipboard().and_then(|item| item.text());
        match pasted.as_deref().map(serde_json::from_str::<Value>) {
            Some(Ok(value)) => self.edit(cx, |node| node.value = value),
            _ => self.model.update(cx, |model, cx| {
                model.notice("The clipboard does not hold JSON. Copy a response, then paste it here.".to_owned(), cx)
            }),
        }
    }
}

impl Render for NodeEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(node) = self.node.clone() else {
            return div().into_any_element();
        };
        let mut sections: Vec<AnyElement> = Vec::new();
        match node.kind {
            NodeKind::Start => sections.push(Self::labelled(
                "Inputs a run can be given; a request reads one as {{name}}",
                div().h(rems(9.0)).child(self.inputs.clone()),
                cx,
            )),
            NodeKind::Request => {
                sections.push(self.render_request_picker(&node));
                sections.push(
                    Checkbox::new("flow-fail-status")
                        .small()
                        .checked(node.fail_on_status)
                        .label("An answer that is not a success leaves by Error")
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            let checked = *checked;
                            this.edit(cx, |node| node.fail_on_status = checked);
                        }))
                        .into_any_element(),
                );
                sections.push(self.render_numbers(&[Number::Retries, Number::RetryDelay], cx));
            }
            NodeKind::Stream => {
                sections.push(self.render_request_picker(&node));
                if node.request_kind == RequestKind::SocketIo.entity_kind() {
                    sections.push(Self::labelled("Event to emit", Input::new(&self.event).small(), cx));
                }
                if node.request_kind != RequestKind::Sse.entity_kind() {
                    sections.push(Self::labelled(
                        "Message sent once connected, unless one is wired",
                        Input::new(&self.message).small(),
                        cx,
                    ));
                }
                sections.push(self.render_numbers(&[Number::MaxMessages, Number::IdleTimeout], cx));
                sections.push(self.render_numbers(&[Number::Listen], cx));
            }
            NodeKind::Mcp => {
                sections.push(self.render_request_picker(&node));
                let calls = [("tool", "Tool"), ("prompt", "Prompt"), ("resource", "Resource")];
                sections.push(self.render_choice("flow-mcp-call", &calls, &node.call, cx, |this, call, window, cx| {
                    // Another kind of call is of another thing: what was named, and its arguments, go.
                    if this.node.as_ref().is_some_and(|node| node.call != call) {
                        this.edit(cx, |node| node.call = call.to_owned());
                        this.call_item(String::new(), window, cx);
                    }
                }));
                sections.extend(self.render_mcp_target(&node, cx));
                sections.push(self.render_numbers(&[Number::Retries, Number::RetryDelay], cx));
            }
            NodeKind::Condition => sections.push(self.render_branches(&node, cx)),
            NodeKind::Validate => {
                sections.push(Self::label("Every check must hold", cx));
                sections.extend(self.render_rules(None, &node.rules, cx));
            }
            NodeKind::Evaluate | NodeKind::If => {
                sections.push(Self::labelled(
                    match node.kind {
                        NodeKind::If => "JavaScript over input and vars: what holds leaves by True, anything else by False",
                        _ => "JavaScript over input, what the node is handed, and vars",
                    },
                    Input::new(&self.expression).small(),
                    cx,
                ));
                sections.push(Self::labelled(
                    "Variables: type a value, or wire one in below",
                    div().h(rems(8.0)).child(self.variables.clone()),
                    cx,
                ));
            }
            NodeKind::Delay => sections.push(self.render_numbers(&[Number::Delay], cx)),
            NodeKind::Repeat => sections.push(self.render_numbers(&[Number::Count], cx)),
            NodeKind::ForEach => {
                sections.push(Self::labelled(
                    "The list to go through",
                    self.render_path_button("flow-list-path".to_owned(), &node.path, Choosing::List, cx),
                    cx,
                ));
                if self.choosing == Some(Choosing::List) {
                    sections.push(self.render_chooser(cx));
                }
            }
            NodeKind::Display => {
                let formats = [("json", "JSON"), ("text", "Text"), ("table", "Table")];
                let chosen = if node.format.is_empty() { "json" } else { node.format.as_str() };
                sections.push(self.render_choice("flow-display-format", &formats, chosen, cx, |this, format, _, cx| {
                    this.edit(cx, |node| node.format = format.to_owned())
                }));
            }
            NodeKind::Data => sections.extend(self.render_data(&node, cx)),
            NodeKind::Sample => sections.push(
                Button::new("flow-sample-paste")
                    .xsmall()
                    .outline()
                    .icon(IconName::Copy)
                    .label("Paste JSON")
                    .tooltip("Replace the sample with the JSON on the clipboard")
                    .on_click(cx.listener(|this, _, _, cx| this.paste_sample(cx)))
                    .into_any_element(),
            ),
            NodeKind::Or => {}
        }
        if sections.is_empty() {
            return div().into_any_element();
        }
        v_flex().px_3().pb_2().gap_2().children(sections).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

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
                stub(RequestKind::Mcp, "m1", "Tools", Some("u")),
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
        assert_eq!(names(NodeKind::Stream).len(), 1, "a gRPC request can be listened to as well");
        assert_eq!(names(NodeKind::Mcp), [("Users".to_owned(), vec!["Tools".to_owned()])]);
        assert!(names(NodeKind::Delay).is_empty());

        let groups = request_groups(&config, NodeKind::Request);
        let ban = &groups[2].1[0];
        assert_eq!((ban.tag.as_ref(), ban.key.clone()), ("gRPC", (RequestKind::Grpc, "g1".to_owned())));
        assert!(ban.matches("admin") && ban.matches("API.test/g1") && ban.matches("ban") && !ban.matches("feed"));
    }

    #[test]
    fn a_node_takes_the_arguments_of_what_it_calls_and_keeps_what_was_typed() {
        let mut view = McpView::default();
        view.apply_status_event(&json!({ "status": "connected", "tools": [{ "name": "echo", "inputSchema": {
            "required": ["message"],
            "properties": { "message": { "type": "string" }, "count": { "type": "integer", "default": 1 } } } }] }));
        let pair = |name: &str, value: &str| Pair { name: name.to_owned(), value: value.to_owned() };
        let echo = &view.tools[0];
        assert_eq!(arguments_for(echo, &[pair("count", "3"), pair("gone", "x")]), [pair("message", ""), pair("count", "3")]);
        let hints: Vec<String> = echo.arguments.iter().map(argument_hint).collect();
        assert_eq!(hints, ["string, required", "integer, default 1"]);
    }

    #[test]
    fn an_argument_the_server_limits_is_chosen_unless_what_is_held_is_not_a_choice() {
        let mut view = McpView::default();
        view.apply_status_event(&json!({ "status": "connected", "tools": [{ "name": "list", "inputSchema": {
            "properties": {
                "type": { "type": "string", "enum": ["rest", "grpc"] },
                "all": { "type": "boolean" },
                "name": { "type": "string" } } } }] }));
        let tool = &view.tools[0];
        let of = |name: &str, held: &str| argument_choices(tool.arguments.iter().find(|a| a.name == name).unwrap(), held);
        let choices = |values: &[&str]| Some(values.iter().map(|value| value.to_string()).collect::<Vec<_>>());
        assert_eq!((of("type", ""), of("type", "grpc")), (choices(&["rest", "grpc"]), choices(&["rest", "grpc"])));
        assert_eq!(of("type", "{{input.kind}}"), None, "a reference stays typed");
        assert_eq!((of("all", "true"), of("name", "")), (choices(&["true", "false"]), None));
    }

    #[test]
    fn a_rule_reads_the_part_that_was_clicked() {
        assert_eq!(rule_path(&[json!("body"), json!("items"), json!(0), json!("id")]), "input.body.items[0].id");
        assert_eq!(rule_path(&[json!(2)]), "input[2]");
        assert_eq!(rule_path(&[]), "input");
        let rule = Rule { path: "input.status".to_owned(), op: "notExists".to_owned(), value: "x".to_owned() };
        assert_eq!(rule_summary(&rule), "input.status does not exist");
        assert_eq!(rule_summary(&Rule::status_is_200()), "input.status equals 200");
    }

    #[test]
    fn a_data_node_holds_what_was_typed_once_it_is_a_value_of_its_type() {
        let held = json!(7);
        assert_eq!(data_value("text", "12", &held), json!("12"));
        assert_eq!(data_value("number", " 12.5 ", &held), json!(12.5));
        assert_eq!(data_value("number", "12.", &held), held, "half a number leaves what was held");
        assert_eq!(data_value("number", "\"12\"", &held), held);
        assert_eq!(data_value("json", r#"{"a":[1]}"#, &held), json!({ "a": [1] }));
        assert_eq!(data_value("json", "{", &held), held);
        assert_eq!((data_type_of(&json!("a")), data_type_of(&json!([1])), data_type_of(&json!(true))), ("text", "json", "boolean"));
    }
}
