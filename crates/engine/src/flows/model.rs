//! A flow as it is stored: nodes, the edges between their ports, and the inputs a run starts with.
//!
//! A node is one flat record whatever its type; the fields a type does not use stay at their defaults. What a
//! client adds for itself (a node's position on the canvas) is kept in the file and ignored here.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{kernel::kinds::to_engine_kind, protocol::Result};

/// The port a node without a choice leaves by.
pub const PORT_OUT: &str = "out";
pub const PORT_SUCCESS: &str = "success";
pub const PORT_ERROR: &str = "error";
pub const PORT_MESSAGE: &str = "message";
pub const PORT_CLOSED: &str = "closed";
/// The port of a condition none of whose branches matched.
pub const PORT_ELSE: &str = "else";
pub const PORT_PASS: &str = "pass";
/// The ports of a loop: `each` once per turn, `done` when the turns are over.
pub const PORT_EACH: &str = "each";
pub const PORT_DONE: &str = "done";
/// The ports of an if node: `true` when its expression holds, `false` when it does not.
pub const PORT_TRUE: &str = "true";
pub const PORT_FALSE: &str = "false";

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
    Mail,
}

impl NodeKind {
    /// The kinds of saved request a node of this type may name, by their protocol names.
    pub fn request_kinds(self) -> &'static [&'static str] {
        match self {
            // A request node over a streaming request connects, hands on the first message and disconnects.
            NodeKind::Request => &[
                "requests",
                "graphqlRequests",
                "soapRequests",
                "grpcRequests",
                "sseRequests",
                "wsConnections",
                "socketioRequests",
            ],
            NodeKind::Stream => &["sseRequests", "wsConnections", "socketioRequests", "grpcRequests"],
            NodeKind::Mcp => &["mcpRequests"],
            _ => &[],
        }
    }

    /// A node that only holds a value: it never runs, and what it holds is wired into other nodes.
    pub fn is_constant(self) -> bool {
        self == NodeKind::Data
    }
}

/// A name with the text given for it: a flow input and its default, a variable a node binds, an argument.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Pair {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Match {
    #[default]
    All,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Op {
    #[default]
    Equals,
    NotEquals,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    Matches,
    GreaterThan,
    GreaterOrEqual,
    LessThan,
    LessOrEqual,
    Exists,
    NotExists,
    IsEmpty,
    NotEmpty,
    /// The value is of the type named: `string`, `number`, `boolean`, `object`, `array` or `null`.
    IsType,
}

/// One comparison of a condition: what `path` (`input.status`) holds against `value`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub path: String,
    pub op: Op,
    pub value: String,
}

/// One way out of a condition. Its id is the port's.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Branch {
    pub id: String,
    pub name: String,
    #[serde(rename = "match")]
    pub matching: Match,
    pub rules: Vec<Rule>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    /// What the flow's references name the node by (`nodes.login.body`); it never changes.
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type")]
    pub kind: NodeKind,

    /// The saved request a request, stream or MCP node runs, by its protocol kind and id.
    #[serde(default)]
    pub request_kind: String,
    #[serde(default)]
    pub request_id: String,
    /// Values typed in for the request's `{{variables}}`, over the environment's and under what is wired. For
    /// an evaluate node, the names its expression reads as `vars.<name>`.
    #[serde(default)]
    pub variables: Vec<Pair>,
    /// Whether the saved request's pre and post scripts run, and whether its tests do.
    #[serde(default = "yes")]
    pub execute_scripts: bool,
    #[serde(default = "yes")]
    pub execute_tests: bool,
    /// Whether a failed test is let pass. Off, it sends the node out by its error port.
    #[serde(default)]
    pub ignore_test_errors: bool,
    /// Whether an answer outside 2xx (or a gRPC status other than OK) leaves by the error port.
    #[serde(default = "yes")]
    pub fail_on_status: bool,
    #[serde(default)]
    pub retries: u32,
    #[serde(default)]
    pub retry_delay_ms: u64,

    /// Stream: what is sent once the connection is open and each time the flow reaches the node again.
    #[serde(default)]
    pub message: String,
    /// Stream: the Socket.IO event `message` is emitted as.
    #[serde(default)]
    pub event: String,
    /// Stream: stop listening after this many messages, this long without one, or this long in all. 0 = no limit.
    #[serde(default)]
    pub max_messages: u64,
    #[serde(default)]
    pub idle_timeout_ms: u64,
    #[serde(default)]
    pub listen_ms: u64,

    /// MCP: `tool`, `prompt` or `resource`, and the name (or URI) of the one called.
    #[serde(default)]
    pub call: String,
    #[serde(default)]
    pub target: String,
    /// MCP: what the call is given. Mail: `to`, the address (or part of one) the mail is for.
    #[serde(default)]
    pub arguments: Vec<Pair>,
    /// Mail: how long to wait for the mail; 0 is 30 seconds.
    #[serde(default)]
    pub timeout_ms: u64,

    #[serde(default)]
    pub branches: Vec<Branch>,

    #[serde(default)]
    pub delay_ms: u64,

    /// Validate: the checks, all of which must hold.
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Evaluate and if: a JavaScript expression over `input` (what the node was handed) and `vars`.
    #[serde(default)]
    pub expression: String,
    /// Repeat: how many turns.
    #[serde(default)]
    pub count: u64,
    /// For each: where the list is in what the node was handed (`input.body.items`).
    #[serde(default)]
    pub path: String,
    /// Data and sample: what the node holds.
    #[serde(default)]
    pub value: Value,
}

impl Node {
    /// What the node is called in messages: its name, or its id when it has none.
    pub fn label(&self) -> &str {
        if self.name.trim().is_empty() { &self.id } else { &self.name }
    }

    /// The store's kind of the saved request the node names.
    pub fn store_kind(&self) -> &str {
        to_engine_kind(&self.request_kind)
    }

    /// The ports the node can leave by: id and label.
    pub fn ports(&self) -> Vec<(String, String)> {
        let fixed =
            |ports: &[(&str, &str)]| ports.iter().map(|(id, label)| (id.to_string(), label.to_string())).collect();
        match self.kind {
            NodeKind::Start | NodeKind::Delay | NodeKind::Or | NodeKind::Display | NodeKind::Sample => {
                fixed(&[(PORT_OUT, "Next")])
            }
            NodeKind::Data => fixed(&[(PORT_OUT, "Value")]),
            NodeKind::Request | NodeKind::Mcp | NodeKind::Mail => fixed(&[(PORT_SUCCESS, "Success"), (PORT_ERROR, "Error")]),
            NodeKind::Validate => fixed(&[(PORT_PASS, "Pass"), (PORT_ERROR, "Fail")]),
            NodeKind::Evaluate => fixed(&[(PORT_OUT, "Result"), (PORT_ERROR, "Error")]),
            NodeKind::If => fixed(&[(PORT_TRUE, "True"), (PORT_FALSE, "False")]),
            NodeKind::Repeat | NodeKind::ForEach => fixed(&[(PORT_EACH, "Each"), (PORT_DONE, "Done")]),
            NodeKind::Stream => fixed(&[(PORT_MESSAGE, "Message"), (PORT_CLOSED, "Closed"), (PORT_ERROR, "Error")]),
            NodeKind::Condition => {
                let mut ports: Vec<(String, String)> = self
                    .branches
                    .iter()
                    .map(|branch| {
                        let label = if branch.name.trim().is_empty() { &branch.id } else { &branch.name };
                        (branch.id.clone(), label.clone())
                    })
                    .collect();
                ports.push((PORT_ELSE.to_string(), "Else".to_string()));
                ports
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Edge {
    pub id: String,
    pub from: String,
    #[serde(default)]
    pub port: String,
    pub to: String,
    /// The input of `to` this is wired into: `var:<name>`, `arg:<name>` or `message`. Empty, the connection is
    /// what makes `to` run; with a name it only carries a value, and running is left to another connection.
    #[serde(default)]
    pub input: String,
    /// Wired: the part of what `from` produced that is taken, as the keys and indexes that lead to it. Empty
    /// is all of it.
    #[serde(default)]
    pub path: Vec<Value>,
}

impl Edge {
    pub fn is_wire(&self) -> bool {
        !self.input.is_empty()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// How long a run may take before it is failed; `None` is the engine's default, 0 no limit.
    pub timeout_ms: Option<u64>,
    /// Whether the variables scripts set are written to the environment, as they are when a request is sent by
    /// hand. Off, they live for the run only.
    pub persist_variables: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Flow {
    pub id: String,
    pub name: String,
    pub inputs: Vec<Pair>,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub settings: Settings,
}

impl Flow {
    pub fn parse(value: &Value) -> Result<Flow> {
        Flow::deserialize(value).map_err(|error| {
            super::said(
                "flow.unreadable",
                json!({ "detail": error.to_string() }),
                format!("This flow cannot be read: {error}."),
            )
        })
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// The connections that make another node run when `node` leaves by `port`, in the order they were drawn.
    pub fn edges_from<'a>(&'a self, node: &'a str, port: &'a str) -> impl Iterator<Item = &'a Edge> {
        self.edges.iter().filter(move |edge| edge.from == node && edge.port == port && !edge.is_wire())
    }

    /// What is wired into the inputs of `node`.
    pub fn wires_into<'a>(&'a self, node: &'a str) -> impl Iterator<Item = &'a Edge> {
        self.edges.iter().filter(move |edge| edge.to == node && edge.is_wire())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_node_keeps_its_defaults_and_ignores_what_a_client_added() {
        let flow = Flow::parse(&json!({
            "name": "F",
            "nodes": [
                { "id": "start", "type": "start", "x": 10, "y": 20 },
                { "id": "call", "type": "request", "requestKind": "wsConnections", "requestId": "r1" },
                { "id": "check", "type": "condition", "branches": [{ "id": "ok", "name": "OK", "match": "any" }] },
                { "id": "each", "type": "forEach", "path": "input.items" },
            ],
            "edges": [
                { "id": "e1", "from": "start", "port": "out", "to": "call" },
                { "id": "e2", "from": "start", "port": "out", "to": "call", "input": "var:user", "path": ["users", 0] },
            ],
        }))
        .unwrap();
        let call = flow.node("call").unwrap();
        assert!(call.execute_scripts && call.execute_tests && call.fail_on_status && !call.ignore_test_errors);
        assert_eq!(call.store_kind(), "sockets");
        assert_eq!(call.label(), "call");
        let ports: Vec<String> = flow.node("check").unwrap().ports().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ports, ["ok", "else"]);
        assert_eq!(flow.node("check").unwrap().branches[0].matching, Match::Any);
        assert_eq!(flow.node("each").unwrap().ports()[0].0, PORT_EACH);
        assert_eq!(flow.edges_from("start", "out").count(), 1, "a wire does not make its node run");
        assert_eq!(flow.wires_into("call").map(|wire| wire.input.as_str()).collect::<Vec<_>>(), ["var:user"]);
    }

    #[test]
    fn an_unknown_node_type_is_refused() {
        let error = Flow::parse(&json!({ "nodes": [{ "id": "a", "type": "teleport" }] })).unwrap_err();
        assert!(error.message.contains("cannot be read"), "{}", error.message);
    }
}
