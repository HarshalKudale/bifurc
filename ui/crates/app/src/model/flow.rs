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
    Delay,
}

impl NodeKind {
    pub fn label(self) -> &'static str {
        match self {
            NodeKind::Start => "Start",
            NodeKind::Request => "Request",
            NodeKind::Stream => "Stream",
            NodeKind::Mcp => "MCP call",
            NodeKind::Condition => "Condition",
            NodeKind::Delay => "Delay",
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
            NodeKind::Delay => "delay",
        }
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

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub request_kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub request_id: String,
    /// Whether the saved request's own post script runs. Off, `after_execute` runs in its place.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub run_post_script: bool,
    /// The script run over what the node produced, before that is handed on.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub after_execute: String,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub fail_on_status: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub fail_on_tests: bool,
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
            request_kind: String::new(),
            request_id: String::new(),
            run_post_script: true,
            after_execute: String::new(),
            fail_on_status: true,
            fail_on_tests: false,
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
            extra: Map::new(),
        };
        match kind {
            NodeKind::Mcp => node.call = "tool".to_owned(),
            NodeKind::Delay => node.delay_ms = 1000,
            NodeKind::Condition => node.branches.push(Branch {
                id: "match".to_owned(),
                name: "Match".to_owned(),
                matching: "all".to_owned(),
                rules: vec![Rule { path: "input.status".to_owned(), op: "equals".to_owned(), value: "200".to_owned() }],
            }),
            NodeKind::Start | NodeKind::Request | NodeKind::Stream => {}
        }
        node
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlowEdge {
    pub id: String,
    pub from: String,
    pub port: String,
    pub to: String,
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

    /// Add a node of `kind` at a place on the canvas and return its id: the kind's word, numbered when taken.
    pub fn add_node(&mut self, kind: NodeKind, x: f32, y: f32) -> String {
        let stem = kind.id_stem();
        let id = (1..)
            .map(|n| if n == 1 { stem.to_owned() } else { format!("{stem}_{n}") })
            .find(|id| self.node(id).is_none())
            .expect("an unused id");
        self.nodes.push(FlowNode::new(kind, id.clone(), x, y));
        id
    }

    /// Remove a node and every connection to or from it. The start node stays.
    pub fn remove_node(&mut self, id: &str) {
        if self.node(id).is_some_and(|node| node.kind != NodeKind::Start) {
            self.nodes.retain(|node| node.id != id);
            self.edges.retain(|edge| edge.from != id && edge.to != id);
        }
    }

    /// Connect a node's port to another node. Connecting twice, or a node to itself, changes nothing.
    pub fn connect(&mut self, from: &str, port: &str, to: &str) -> bool {
        let exists = self.edges.iter().any(|edge| edge.from == from && edge.port == port && edge.to == to);
        if from == to || exists || self.node(from).is_none() || self.node(to).is_none() {
            return false;
        }
        let id = (1..)
            .map(|n| format!("e{n}"))
            .find(|id| !self.edges.iter().any(|edge| edge.id == *id))
            .expect("an unused id");
        self.edges.push(FlowEdge { id, from: from.to_owned(), port: port.to_owned(), to: to.to_owned() });
        true
    }

    pub fn disconnect(&mut self, edge_id: &str) {
        self.edges.retain(|edge| edge.id != edge_id);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub node_id: Option<String>,
    pub is_error: bool,
    pub message: String,
}

/// What the engine says a node offers: its ports, and the request it names.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeInfo {
    pub ports: Vec<(String, String)>,
    pub request_name: Option<String>,
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
                is_error: problem.get("severity").and_then(Value::as_str) == Some("error"),
                message: text(problem, "message").unwrap_or_default(),
            })
            .collect();
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
                let info = NodeInfo { ports, request_name: text(request, "name"), address: text(request, "address") };
                (id.clone(), info)
            })
            .collect();
        Self { ok: answer.get("ok").and_then(Value::as_bool).unwrap_or(false), problems, nodes }
    }

    pub fn has_error(&self, node_id: &str) -> bool {
        self.problems.iter().any(|problem| problem.is_error && problem.node_id.as_deref() == Some(node_id))
    }
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
    /// What the node was handed and what it produced, when the whole run was read (`flow.status`).
    pub input: Value,
    pub output: Value,
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
            if whole {
                entry.input = node.get("input").cloned().unwrap_or(Value::Null);
                entry.output = node.get("output").cloned().unwrap_or(Value::Null);
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
            ],
            "edges": [{ "id": "e1", "from": "start", "port": "out", "to": "login" }],
            "settings": {},
        });
        let doc = FlowDoc::from_value(&stored).unwrap();
        assert_eq!(doc.to_value(), stored, "defaults are not written and unknown fields are kept");
        assert!(doc.node("login").unwrap().run_post_script);
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
        assert!(doc.connect("request", "success", "request_2"));
        assert_eq!(doc.edges.iter().map(|edge| edge.id.as_str()).collect::<Vec<_>>(), ["e1", "e2"]);

        doc.remove_node("request");
        assert!(doc.edges.is_empty(), "a removed node takes its connections with it");
        doc.remove_node("start");
        assert!(doc.node("start").is_some(), "the start node stays");
        assert!(doc.connect("start", "out", "request_2"));
        doc.disconnect("e1");
        assert!(doc.edges.is_empty());
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
                "variables": ["host"],
            } },
        }));
        assert!(!described.ok && described.has_error("a") && !described.has_error("b"));
        let info = &described.nodes["a"];
        assert_eq!(info.ports[1], ("error".to_owned(), "Error".to_owned()));
        assert_eq!((info.request_name.as_deref(), info.address.as_deref()), (Some("Login"), Some("http://x")));
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
                        "input": { "user": "1" }, "output": { "status": 503 } }],
            "edges": [{ "id": "e1", "count": 1 }],
        });
        let run = RunView::from_report(&report).unwrap();
        assert_eq!((run.status.as_str(), run.duration_ms, run.failed_tests), ("failed", Some(250), 1));
        assert_eq!(run.nodes["a"].output["status"], 503);
        assert_eq!(run.edges["e1"], 1);
        assert!(RunView::from_report(&Value::Null).is_none());
    }
}
