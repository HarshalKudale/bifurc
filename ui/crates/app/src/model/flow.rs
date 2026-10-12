//! A flow as the Flows screen edits it, and what the engine says about it: what `flow.validate` found and
//! how its latest run went.
//!
//! The document is the `flows` entity as it is stored. Fields this version does not know are kept as they
//! came, so a flow a newer Bifurc touched is not trimmed by being opened here. A field left at its default
//! is not written, which keeps the file to what was chosen.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NodeKind {
    Start,
    Request,
    Stream,
    Mcp,
    Condition,
    If,
    Validate,
    Evaluate,
    Delay,
    Or,
    Repeat,
    ForEach,
    Display,
    Data,
    Sample,
}

impl NodeKind {
    /// The kinds a node can be added as, in the order a menu lists them. A flow has its one start node.
    pub const ADDABLE: [NodeKind; 14] = [
        NodeKind::Request,
        NodeKind::Stream,
        NodeKind::Mcp,
        NodeKind::If,
        NodeKind::Condition,
        NodeKind::Validate,
        NodeKind::Evaluate,
        NodeKind::Delay,
        NodeKind::Or,
        NodeKind::Repeat,
        NodeKind::ForEach,
        NodeKind::Display,
        NodeKind::Data,
        NodeKind::Sample,
    ];

    pub fn label(self) -> &'static str {
        match self {
            NodeKind::Start => "Start",
            NodeKind::Request => "Request",
            NodeKind::Stream => "Stream",
            NodeKind::Mcp => "MCP call",
            NodeKind::Condition => "Condition",
            NodeKind::If => "If",
            NodeKind::Validate => "Validate",
            NodeKind::Evaluate => "Evaluate",
            NodeKind::Delay => "Delay",
            NodeKind::Or => "Or",
            NodeKind::Repeat => "Repeat",
            NodeKind::ForEach => "For each",
            NodeKind::Display => "Display",
            NodeKind::Data => "Data",
            NodeKind::Sample => "Sample response",
        }
    }

    /// What a new node of this kind is called in references until it is given a name.
    fn id_stem(self) -> &'static str {
        match self {
            NodeKind::Start => "start",
            NodeKind::Request => "request",
            NodeKind::Stream => "stream",
            NodeKind::Mcp => "mcp",
            NodeKind::Condition => "condition",
            NodeKind::If => "if",
            NodeKind::Validate => "validate",
            NodeKind::Evaluate => "evaluate",
            NodeKind::Delay => "delay",
            NodeKind::Or => "or",
            NodeKind::Repeat => "repeat",
            NodeKind::ForEach => "each",
            NodeKind::Display => "display",
            NodeKind::Data => "data",
            NodeKind::Sample => "sample",
        }
    }

    /// Whether a node of this kind is run by a connection into it. A data node only holds a value, and
    /// nothing leads into the start.
    pub fn is_run(self) -> bool {
        !matches!(self, NodeKind::Start | NodeKind::Data)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pair {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub path: String,
    pub op: String,
    pub value: String,
}

impl Rule {
    /// What a new rule asks until it is changed.
    pub fn status_is_200() -> Self {
        Self { path: "input.status".to_owned(), op: "equals".to_owned(), value: "200".to_owned() }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Branch {
    pub id: String,
    pub name: String,
    /// `all` or `any`.
    #[serde(rename = "match")]
    pub matching: String,
    pub rules: Vec<Rule>,
}

fn yes() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowNode {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type")]
    pub kind: NodeKind,
    /// Where the node is on the canvas.
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    /// Display: how wide the node was made, and how high what it shows. 0 is the size it has by itself.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub width: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub height: f32,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub request_kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub request_id: String,
    /// What was typed in for the node's variables. An evaluate node's are the names its expression reads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<Pair>,
    /// Whether the saved request's scripts run, whether its tests do, and whether a failed test is let pass.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub execute_scripts: bool,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub execute_tests: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub ignore_test_errors: bool,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub fail_on_status: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub retries: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub retry_delay_ms: u64,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub event: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub max_messages: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub idle_timeout_ms: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub listen_ms: u64,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub call: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<Pair>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<Branch>,

    #[serde(default, skip_serializing_if = "is_zero")]
    pub delay_ms: u64,

    /// Validate: the checks, all of which must hold.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    /// Evaluate: the expression worked out.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub expression: String,
    /// Repeat: how many turns.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub count: u64,
    /// For each: where the list is in what the node was handed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    /// Data and sample: what the node holds.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub value: Value,
    /// Display: how what it was handed is shown: `json`, `text` or `table`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub format: String,

    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl FlowNode {
    fn new(kind: NodeKind, id: String, x: f32, y: f32) -> Self {
        let mut node = Self {
            id,
            name: String::new(),
            kind,
            x,
            y,
            width: 0.0,
            height: 0.0,
            request_kind: String::new(),
            request_id: String::new(),
            variables: Vec::new(),
            execute_scripts: true,
            execute_tests: true,
            ignore_test_errors: false,
            fail_on_status: true,
            retries: 0,
            retry_delay_ms: 0,
            message: String::new(),
            event: String::new(),
            max_messages: 0,
            idle_timeout_ms: 0,
            listen_ms: 0,
            call: String::new(),
            target: String::new(),
            arguments: Vec::new(),
            branches: Vec::new(),
            delay_ms: 0,
            rules: Vec::new(),
            expression: String::new(),
            count: 0,
            path: String::new(),
            value: Value::Null,
            format: String::new(),
            extra: Map::new(),
        };
        match kind {
            NodeKind::Mcp => node.call = "tool".to_owned(),
            NodeKind::Delay => node.delay_ms = 1000,
            NodeKind::Repeat => node.count = 3,
            NodeKind::Condition => node.branches.push(Branch {
                id: "match".to_owned(),
                name: "Match".to_owned(),
                matching: "all".to_owned(),
                rules: vec![Rule::status_is_200()],
            }),
            NodeKind::Validate => node.rules.push(Rule::status_is_200()),
            NodeKind::Evaluate => node.expression = "input".to_owned(),
            NodeKind::If => node.expression = "input.status === 200".to_owned(),
            NodeKind::Data => node.value = Value::String(String::new()),
            NodeKind::Sample => node.value = serde_json::json!({ "status": 200, "headers": {}, "body": "" }),
            _ => {}
        }
        node
    }

    /// The name on the node: its own, or its kind's when it has none.
    pub fn title(&self) -> &str {
        if self.name.is_empty() { self.kind.label() } else { &self.name }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlowEdge {
    pub id: String,
    pub from: String,
    pub port: String,
    pub to: String,
    /// The input of `to` this is wired into. Empty, the connection is what makes `to` run.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub input: String,
    /// Wired: the keys and indexes that lead to the part of the output that is taken. Empty is all of it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<Value>,
}

impl FlowEdge {
    pub fn is_wire(&self) -> bool {
        !self.input.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FlowSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "is_false")]
    pub persist_variables: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FlowDoc {
    pub id: String,
    pub name: String,
    pub inputs: Vec<Pair>,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    pub settings: FlowSettings,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl FlowDoc {
    /// What a new flow holds: the node every run starts at.
    pub fn blank(name: &str) -> Self {
        let start = FlowNode::new(NodeKind::Start, "start".to_owned(), 60.0, 120.0);
        Self { name: name.to_owned(), nodes: vec![start], ..Self::default() }
    }

    pub fn from_value(entity: &Value) -> Result<Self, String> {
        Self::deserialize(entity).map_err(|error| error.to_string())
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn node(&self, id: &str) -> Option<&FlowNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn node_mut(&mut self, id: &str) -> Option<&mut FlowNode> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }

    fn free_node_id(&self, kind: NodeKind) -> String {
        let stem = kind.id_stem();
        (1..)
            .map(|n| if n == 1 { stem.to_owned() } else { format!("{stem}_{n}") })
            .find(|id| self.node(id).is_none())
            .expect("an unused id")
    }

    /// Add a node of `kind` at a place on the canvas and return its id: the kind's word, numbered when taken.
    pub fn add_node(&mut self, kind: NodeKind, x: f32, y: f32) -> String {
        let id = self.free_node_id(kind);
        self.nodes.push(FlowNode::new(kind, id.clone(), x, y));
        id
    }

    /// Add a copy of a node beside it, without its connections, and return the copy's id. The start node
    /// is the only one a flow has.
    pub fn duplicate_node(&mut self, id: &str) -> Option<String> {
        let mut copy = self.node(id).filter(|node| node.kind != NodeKind::Start)?.clone();
        copy.id = self.free_node_id(copy.kind);
        (copy.x, copy.y) = (copy.x + 32.0, copy.y + 32.0);
        let id = copy.id.clone();
        self.nodes.push(copy);
        Some(id)
    }

    /// Remove a node and every connection to or from it. The start node stays.
    pub fn remove_node(&mut self, id: &str) {
        if self.node(id).is_some_and(|node| node.kind != NodeKind::Start) {
            self.nodes.retain(|node| node.id != id);
            self.edges.retain(|edge| edge.from != id && edge.to != id);
        }
    }

    fn push_edge(&mut self, from: &str, port: &str, to: &str, input: &str) {
        let id = (1..)
            .map(|n| format!("e{n}"))
            .find(|id| !self.edges.iter().any(|edge| edge.id == *id))
            .expect("an unused id");
        self.edges.push(FlowEdge {
            id,
            from: from.to_owned(),
            port: port.to_owned(),
            to: to.to_owned(),
            input: input.to_owned(),
            path: Vec::new(),
        });
    }

    /// Whether a connection from `from` may arrive at `to`: two nodes of the flow, not the same one, and not
    /// the start, which nothing leads into.
    fn joins(&self, from: &str, to: &str) -> bool {
        from != to && self.node(from).is_some() && self.node(to).is_some_and(|node| node.kind != NodeKind::Start)
    }

    /// Make `to` run when `from` leaves by `port`. Connecting twice, a node to itself, or a node that only
    /// holds a value changes nothing.
    pub fn connect(&mut self, from: &str, port: &str, to: &str) -> bool {
        let runs = |id: &str| self.node(id).is_some_and(|node| node.kind != NodeKind::Data);
        let exists = self.edges.iter().any(|edge| !edge.is_wire() && edge.from == from && edge.port == port && edge.to == to);
        if exists || !self.joins(from, to) || !runs(from) || !runs(to) {
            return false;
        }
        self.push_edge(from, port, to, "");
        true
    }

    /// Wire what `from` hands on by `port` into the input `input` of `to`, in place of what was wired there.
    /// A wire only carries a value: when nothing runs `to` yet and `from` is a node that runs, the connection
    /// that runs `to` after it is made as well.
    pub fn wire(&mut self, from: &str, port: &str, to: &str, input: &str) -> bool {
        if !self.joins(from, to) || input.is_empty() {
            return false;
        }
        self.edges.retain(|edge| !(edge.to == to && edge.input == input));
        self.push_edge(from, port, to, input);
        if !self.edges.iter().any(|edge| edge.to == to && !edge.is_wire()) {
            self.connect(from, port, to);
        }
        true
    }

    pub fn disconnect(&mut self, edge_id: &str) {
        self.edges.retain(|edge| edge.id != edge_id);
    }

    /// What a run starts with when it is given nothing: the flow's inputs by name.
    pub fn default_inputs(&self) -> Value {
        Value::Object(self.inputs.iter().map(|input| (input.name.clone(), Value::String(input.value.clone()))).collect())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub node_id: Option<String>,
    pub edge_id: Option<String>,
    pub is_error: bool,
    pub message: String,
}

/// Where a wired input's value comes from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WireFrom {
    pub edge_id: String,
    pub node_id: String,
    pub node_name: String,
    pub path: Vec<Value>,
}

/// One of the inputs a node takes, as the engine lists it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InputInfo {
    /// `var:<name>`, `arg:<name>` or `message`: what a wire names.
    pub id: String,
    pub name: String,
    /// `wired`, `static`, `input`, `env` or `unset`.
    pub source: String,
    /// What it resolves to as the flow stands, when that is known before a run.
    pub value: Value,
    pub from: Option<WireFrom>,
}

/// What the engine says a node offers: its ports, its inputs and the request it names.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeInfo {
    pub ports: Vec<(String, String)>,
    pub inputs: Vec<InputInfo>,
    pub request_name: Option<String>,
    pub method: Option<String>,
    pub address: Option<String>,
}

/// The answer of `flow.validate`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Described {
    pub ok: bool,
    pub problems: Vec<Problem>,
    pub nodes: HashMap<String, NodeInfo>,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).filter(|text| !text.is_empty()).map(str::to_owned)
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

impl Described {
    pub fn from_value(answer: &Value) -> Self {
        let problems = list(answer, "problems")
            .iter()
            .map(|problem| Problem {
                node_id: text(problem, "nodeId"),
                edge_id: text(problem, "edgeId"),
                is_error: problem.get("severity").and_then(Value::as_str) == Some("error"),
                message: text(problem, "message").unwrap_or_default(),
            })
            .collect();
        let input = |input: &Value| {
            let from = input.get("from").filter(|from| from.is_object()).map(|from| WireFrom {
                edge_id: text(from, "edgeId").unwrap_or_default(),
                node_id: text(from, "nodeId").unwrap_or_default(),
                node_name: text(from, "name").unwrap_or_default(),
                path: list(from, "path").to_vec(),
            });
            Some(InputInfo {
                id: text(input, "id")?,
                name: text(input, "name").unwrap_or_default(),
                source: text(input, "source").unwrap_or_default(),
                value: input.get("value").cloned().unwrap_or(Value::Null),
                from,
            })
        };
        let nodes = answer
            .get("nodes")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(id, node)| {
                let ports = list(node, "ports")
                    .iter()
                    .filter_map(|port| Some((text(port, "id")?, text(port, "name").unwrap_or_default())))
                    .collect();
                let request = node.get("request").unwrap_or(&Value::Null);
                let info = NodeInfo {
                    ports,
                    inputs: list(node, "inputs").iter().filter_map(input).collect(),
                    request_name: text(request, "name"),
                    method: text(request, "method"),
                    address: text(request, "address"),
                };
                (id.clone(), info)
            })
            .collect();
        Self { ok: answer.get("ok").and_then(Value::as_bool).unwrap_or(false), problems, nodes }
    }

    pub fn has_error(&self, node_id: &str) -> bool {
        self.problems.iter().any(|problem| problem.is_error && problem.node_id.as_deref() == Some(node_id))
    }

    /// What is wrong with a node, one problem a line.
    pub fn problems_of(&self, node_id: &str) -> String {
        let of_node = self.problems.iter().filter(|problem| problem.node_id.as_deref() == Some(node_id));
        of_node.map(|problem| problem.message.as_str()).collect::<Vec<_>>().join("\n")
    }
}

/// How a test of a request, or a check of a validate node, came out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub error: Option<String>,
}

/// How one node fared in a run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeRun {
    /// `idle`, `running`, `listening`, `succeeded`, `failed` or `stopped`.
    pub status: String,
    pub runs: u64,
    pub port: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
    /// How many messages a stream node handed on, or how many turns a loop took.
    pub handed_on: u64,
    /// What the node was handed, what was wired into its inputs, what last left by each of its ports and its
    /// tests, when the whole run was read (`flow.status`).
    pub input: Value,
    pub inputs: Value,
    pub outputs: Value,
    pub tests: Vec<TestResult>,
}

impl NodeRun {
    /// What last left the node by `port`, if anything did.
    pub fn output(&self, port: &str) -> Option<&Value> {
        self.outputs.get(port)
    }
}

/// A flow's latest run, as its events and `flow.status` tell it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunView {
    pub run_id: String,
    /// `running`, `succeeded`, `failed` or `cancelled`.
    pub status: String,
    pub error: Option<String>,
    pub nodes: HashMap<String, NodeRun>,
    pub edges: HashMap<String, u64>,
    pub total_tests: u64,
    pub failed_tests: u64,
    pub duration_ms: Option<u64>,
}

impl RunView {
    pub fn started(run_id: String) -> Self {
        Self { run_id, status: "running".to_owned(), ..Self::default() }
    }

    pub fn is_running(&self) -> bool {
        self.status == "running"
    }

    fn apply_nodes(&mut self, payload: &Value, whole: bool) {
        for node in list(payload, "nodes") {
            let Some(id) = text(node, "id") else { continue };
            let entry = self.nodes.entry(id).or_default();
            entry.status = text(node, "status").unwrap_or_else(|| "idle".to_owned());
            entry.runs = node.get("runs").and_then(Value::as_u64).unwrap_or(0);
            entry.port = text(node, "port");
            entry.error = text(node, "error");
            entry.duration_ms = node.get("durationMs").and_then(Value::as_u64);
            entry.handed_on = node.get("handedOn").and_then(Value::as_u64).unwrap_or(0);
            if whole {
                let part = |key: &str| node.get(key).cloned().unwrap_or(Value::Null);
                (entry.input, entry.inputs, entry.outputs) = (part("input"), part("inputs"), part("outputs"));
                entry.tests = list(node, "tests")
                    .iter()
                    .map(|test| TestResult {
                        name: text(test, "name").unwrap_or_default(),
                        passed: test.get("passed").and_then(Value::as_bool).unwrap_or(false),
                        error: text(test, "error"),
                    })
                    .collect();
            }
        }
    }

    /// Take in an `event.flow.progress` payload. Returns the edges that were taken since the last one.
    pub fn apply_progress(&mut self, payload: &Value) -> Vec<String> {
        self.apply_nodes(payload, false);
        let mut taken = Vec::new();
        for edge in list(payload, "edges") {
            let (Some(id), Some(count)) = (text(edge, "id"), edge.get("count").and_then(Value::as_u64)) else {
                continue;
            };
            if self.edges.insert(id.clone(), count).unwrap_or(0) < count {
                taken.push(id);
            }
        }
        taken
    }

    /// A run read whole: the `report` of `event.flow.finished`, or the answer of `flow.status`.
    pub fn from_report(report: &Value) -> Option<Self> {
        let mut run = Self::started(text(report, "runId")?);
        run.status = text(report, "status").unwrap_or_default();
        run.error = text(report, "error");
        run.apply_nodes(report, true);
        run.apply_progress(&serde_json::json!({ "edges": report.get("edges") }));
        run.total_tests = report.get("totalTests").and_then(Value::as_u64).unwrap_or(0);
        run.failed_tests = report.get("failedTests").and_then(Value::as_u64).unwrap_or(0);
        let at = |key: &str| report.get(key).and_then(Value::as_i64);
        run.duration_ms = at("finishedAt").zip(at("startedAt")).map(|(end, start)| (end - start).max(0) as u64);
        Some(run)
    }
}

/// One line of a JSON value shown as a tree: a key with what it holds, as deep as its parents are open.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonRow {
    /// The keys and indexes that lead to the value, as a wire stores them.
    pub path: Vec<Value>,
    pub key: String,
    pub depth: usize,
    /// The value itself when it is a single one, or how much a list or an object holds.
    pub preview: String,
    /// `Some(open)` for a list or an object, which can be opened.
    pub open: Option<bool>,
}

/// What names a row in the set of open rows.
pub fn path_key(path: &[Value]) -> String {
    Value::Array(path.to_vec()).to_string()
}

/// A path as it reads: `body.items[0].id`.
pub fn path_label(path: &[Value]) -> String {
    let mut out = String::new();
    for part in path {
        match part {
            Value::Number(index) => out.push_str(&format!("[{index}]")),
            other => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(other.as_str().unwrap_or_default());
            }
        }
    }
    out
}

/// Text that holds a JSON list or object is shown as what it holds, the way the engine reads it when a path
/// goes through it: a response's body is text.
fn held(value: &Value) -> Option<Value> {
    let text = value.as_str()?.trim_start();
    (text.starts_with('{') || text.starts_with('[')).then(|| serde_json::from_str(text).ok()).flatten()
}

/// The rows of `value` as a tree in which only the rows named in `open` (by [`path_key`]) show what is in
/// them. No more than `limit` rows are made: a tree is a way to look, not a way to list a large response.
pub fn json_rows(value: &Value, open: &dyn Fn(&str) -> bool, limit: usize) -> Vec<JsonRow> {
    fn walk(value: &Value, path: &mut Vec<Value>, open: &dyn Fn(&str) -> bool, limit: usize, rows: &mut Vec<JsonRow>) {
        let parsed = held(value);
        let children: Vec<(Value, &Value)> = match parsed.as_ref().unwrap_or(value) {
            Value::Object(map) => map.iter().map(|(key, child)| (Value::String(key.clone()), child)).collect(),
            Value::Array(items) => items.iter().enumerate().map(|(index, child)| (Value::from(index), child)).collect(),
            _ => Vec::new(),
        };
        for (step, child) in children {
            if rows.len() >= limit {
                return;
            }
            path.push(step.clone());
            let inner = held(child);
            let (preview, container) = match inner.as_ref().unwrap_or(child) {
                Value::Object(map) => (format!("{{{}}}", map.len()), true),
                Value::Array(items) => (format!("[{}]", items.len()), true),
                Value::String(text) => (format!("\"{text}\""), false),
                other => (other.to_string(), false),
            };
            let is_open = container && open(&path_key(path));
            rows.push(JsonRow {
                path: path.clone(),
                key: step.as_str().map(str::to_owned).unwrap_or_else(|| step.to_string()),
                depth: path.len() - 1,
                preview,
                open: container.then_some(is_open),
            });
            if is_open {
                walk(child, path, open, limit, rows);
            }
            path.pop();
        }
    }
    let mut rows = Vec::new();
    walk(value, &mut Vec::new(), open, limit, &mut rows);
    rows
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_flow_is_written_back_as_it_was_read() {
        let stored = json!({
            "id": "f1", "name": "Checkout", "workspaceId": "ws1", "createdAt": 5,
            "inputs": [{ "name": "user", "value": "1" }],
            "nodes": [
                { "id": "start", "type": "start", "name": "", "x": 60.0, "y": 120.0 },
                { "id": "login", "type": "request", "name": "Login", "x": 300.0, "y": 120.0, "requestKind": "requests",
                  "requestId": "r1", "failOnStatus": false, "futureField": { "a": 1 } },
                { "id": "seven", "type": "data", "name": "", "x": 0.0, "y": 0.0, "value": 7 },
                { "id": "show", "type": "display", "name": "", "x": 0.0, "y": 0.0, "width": 480.0, "height": 320.0 },
            ],
            "edges": [
                { "id": "e1", "from": "start", "port": "out", "to": "login" },
                { "id": "e2", "from": "seven", "port": "out", "to": "login", "input": "var:n", "path": ["a", 0] },
            ],
            "settings": {},
        });
        let doc = FlowDoc::from_value(&stored).unwrap();
        assert_eq!(doc.to_value(), stored, "defaults are not written and unknown fields are kept");
        assert!(doc.node("login").unwrap().execute_scripts && doc.edges[1].is_wire());
        assert!(FlowDoc::from_value(&json!({ "nodes": [{ "id": "a", "type": "teleport" }] })).is_err());
    }

    #[test]
    fn nodes_and_connections_are_added_and_removed() {
        let mut doc = FlowDoc::blank("F");
        assert_eq!(doc.nodes[0].kind, NodeKind::Start);
        let first = doc.add_node(NodeKind::Request, 0.0, 0.0);
        let second = doc.add_node(NodeKind::Request, 0.0, 0.0);
        assert_eq!((first.as_str(), second.as_str()), ("request", "request_2"));
        let condition = doc.add_node(NodeKind::Condition, 0.0, 0.0);
        assert_eq!(doc.node(&condition).unwrap().branches.len(), 1);

        assert!(doc.connect("start", "out", "request"));
        assert!(!doc.connect("start", "out", "request"), "the same connection twice");
        assert!(!doc.connect("request", "success", "request"), "a node to itself");
        assert!(!doc.connect("request", "success", "gone"));
        assert!(!doc.connect("request", "success", "start"), "nothing leads into the start");
        assert!(doc.connect("request", "success", "request_2"));
        assert_eq!(doc.edges.iter().map(|edge| edge.id.as_str()).collect::<Vec<_>>(), ["e1", "e2"]);

        let copy = doc.duplicate_node("request").unwrap();
        assert_eq!(copy, "request_3");
        assert!(!doc.edges.iter().any(|edge| edge.from == copy || edge.to == copy), "a copy starts unconnected");
        assert!(doc.duplicate_node("start").is_none());

        doc.remove_node("request");
        assert!(doc.edges.is_empty(), "a removed node takes its connections with it");
        doc.remove_node("start");
        assert!(doc.node("start").is_some(), "the start node stays");
        assert!(doc.connect("start", "out", "request_2"));
        doc.disconnect("e1");
        assert!(doc.edges.is_empty());
    }

    #[test]
    fn a_wire_takes_the_place_of_the_last_one_and_brings_the_connection_that_runs_its_node() {
        let mut doc = FlowDoc::blank("F");
        let (login, user) = (doc.add_node(NodeKind::Request, 0.0, 0.0), doc.add_node(NodeKind::Request, 0.0, 0.0));
        let data = doc.add_node(NodeKind::Data, 0.0, 0.0);
        assert!(!doc.connect(&data, "out", &user), "a data node runs nothing");

        assert!(doc.wire(&data, "out", &user, "var:id"));
        assert_eq!(doc.edges.len(), 1, "a value does not run the node it is wired into");
        assert!(doc.wire(&login, "success", &user, "var:id"));
        let wires: Vec<&FlowEdge> = doc.edges.iter().filter(|edge| edge.is_wire()).collect();
        assert_eq!((wires.len(), wires[0].from.as_str()), (1, login.as_str()), "one wire into an input");
        assert!(doc.edges.iter().any(|edge| !edge.is_wire() && edge.from == login && edge.to == user));

        assert!(doc.wire(&login, "success", &user, "var:token"));
        assert_eq!(doc.edges.iter().filter(|edge| !edge.is_wire()).count(), 1, "it is run by one connection");
        assert!(!doc.wire(&login, "success", "start", "var:x") && !doc.wire(&login, "success", &user, ""));
    }

    #[test]
    fn what_the_engine_says_of_a_flow_is_read() {
        let described = Described::from_value(&json!({
            "ok": false,
            "problems": [
                { "nodeId": "a", "edgeId": null, "severity": "error", "message": "No saved request is chosen." },
                { "nodeId": "b", "edgeId": null, "severity": "warning", "message": "Never runs." },
            ],
            "nodes": { "a": {
                "ports": [{ "id": "success", "name": "Success" }, { "id": "error", "name": "Error" }],
                "request": { "id": "r1", "kind": "requests", "name": "Login", "method": "GET", "address": "http://x" },
                "inputs": [
                    { "id": "var:host", "name": "host", "source": "env", "value": "api.test", "from": null },
                    { "id": "var:id", "name": "id", "source": "wired", "value": null,
                      "from": { "edgeId": "e2", "nodeId": "login", "name": "Login", "path": ["body", "id"] } },
                ],
            } },
        }));
        assert!(!described.ok && described.has_error("a") && !described.has_error("b"));
        assert_eq!(described.problems_of("a"), "No saved request is chosen.");
        let info = &described.nodes["a"];
        assert_eq!(info.ports[1], ("error".to_owned(), "Error".to_owned()));
        assert_eq!((info.request_name.as_deref(), info.address.as_deref()), (Some("Login"), Some("http://x")));
        assert_eq!((info.inputs[0].source.as_str(), &info.inputs[0].value), ("env", &json!("api.test")));
        let from = info.inputs[1].from.as_ref().unwrap();
        assert_eq!((from.node_name.as_str(), path_label(&from.path)), ("Login", "body.id".to_owned()));
    }

    #[test]
    fn a_run_follows_its_progress_and_its_report() {
        let mut run = RunView::started("run1".to_owned());
        let taken = run.apply_progress(&json!({
            "nodes": [{ "id": "a", "status": "running", "runs": 1, "port": null, "error": null, "durationMs": null }],
            "edges": [{ "id": "e1", "count": 1 }],
        }));
        assert_eq!(taken, ["e1"]);
        assert!(run.is_running() && run.nodes["a"].status == "running");
        assert!(
            run.apply_progress(&json!({ "edges": [{ "id": "e1", "count": 1 }] })).is_empty(),
            "nothing new was taken"
        );
        assert_eq!(run.apply_progress(&json!({ "edges": [{ "id": "e1", "count": 3 }] })), ["e1"]);

        let report = json!({
            "runId": "run1", "status": "failed", "error": "a: HTTP 503", "startedAt": 1000, "finishedAt": 1250,
            "totalTests": 2, "failedTests": 1,
            "nodes": [{ "id": "a", "status": "failed", "runs": 1, "port": "error", "error": "HTTP 503", "durationMs": 40,
                        "input": { "user": "1" }, "inputs": { "var:id": 7 }, "output": { "status": 503 },
                        "outputs": { "error": { "status": 503 } },
                        "tests": [{ "name": "is 200", "passed": false, "error": "got 503" }] }],
            "edges": [{ "id": "e1", "count": 1 }],
        });
        let run = RunView::from_report(&report).unwrap();
        assert_eq!((run.status.as_str(), run.duration_ms, run.failed_tests), ("failed", Some(250), 1));
        let node = &run.nodes["a"];
        assert_eq!((node.output("error").unwrap()["status"].clone(), node.output("success")), (json!(503), None));
        assert_eq!((&node.inputs["var:id"], node.tests[0].error.as_deref()), (&json!(7), Some("got 503")));
        assert_eq!(run.edges["e1"], 1);
        assert!(RunView::from_report(&Value::Null).is_none());
    }

    #[test]
    fn a_value_is_listed_as_a_tree_that_opens_where_it_is_asked_to() {
        let value = json!({ "status": 200, "body": r#"{"items":[{"id":"a"}],"ok":true}"#, "note": "{not json" });
        let closed = json_rows(&value, &|_| false, 100);
        let shown: Vec<(&str, &str, Option<bool>)> =
            closed.iter().map(|row| (row.key.as_str(), row.preview.as_str(), row.open)).collect();
        assert_eq!(
            shown,
            [("status", "200", None), ("body", "{2}", Some(false)), ("note", "\"{not json\"", None)],
            "text that holds JSON is shown as what it holds"
        );

        let open = [path_key(&[json!("body")]), path_key(&[json!("body"), json!("items")])];
        let rows = json_rows(&value, &|key| open.iter().any(|known| known == key), 100);
        let keys: Vec<(&str, usize)> = rows.iter().map(|row| (row.key.as_str(), row.depth)).collect();
        assert_eq!(keys, [("status", 0), ("body", 0), ("items", 1), ("0", 2), ("ok", 1), ("note", 0)]);
        assert_eq!(path_label(&rows[3].path), "body.items[0]");
        assert_eq!(json_rows(&value, &|_| true, 2).len(), 2, "no more rows than asked for");
    }
}
