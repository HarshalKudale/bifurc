//! A flow as it is stored: nodes, the edges between their ports, and the inputs a run starts with.
//!
//! A node is one flat record whatever its type; the fields a type does not use stay at their defaults. What a
//! client adds for itself (a node's position on the canvas) is kept in the file and ignored here.

use serde::Deserialize;
use serde_json::Value;

use crate::{
    kernel::kinds::to_engine_kind,
    protocol::{EngineError, Result},
};

/// The port a node without a choice leaves by.
pub const PORT_OUT: &str = "out";
pub const PORT_SUCCESS: &str = "success";
pub const PORT_ERROR: &str = "error";
pub const PORT_MESSAGE: &str = "message";
pub const PORT_CLOSED: &str = "closed";
/// The port of a condition none of whose branches matched.
pub const PORT_ELSE: &str = "else";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
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
    /// The kinds of saved request a node of this type may name, by their protocol names.
    pub fn request_kinds(self) -> &'static [&'static str] {
        match self {
            NodeKind::Request => &["requests", "graphqlRequests", "soapRequests", "grpcRequests"],
            NodeKind::Stream => &["sseRequests", "wsConnections", "socketioRequests", "grpcRequests"],
            NodeKind::Mcp => &["mcpRequests"],
            NodeKind::Start | NodeKind::Condition | NodeKind::Delay => &[],
        }
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
    /// Values for the request's `{{variables}}`, over the environment's.
    #[serde(default)]
    pub variables: Vec<Pair>,
    /// Whether the saved request's own post script runs once it has answered. Off, the node's `after_execute`
    /// runs in its place: never both. A request's pre script never runs in a flow, its tests always do.
    #[serde(default = "yes")]
    pub run_post_script: bool,
    /// A script run over what the node produced before that is handed on: it reads and replaces `lp.output`.
    #[serde(default)]
    pub after_execute: String,
    /// Whether an answer outside 2xx (or a gRPC status other than OK) leaves by the error port.
    #[serde(default = "yes")]
    pub fail_on_status: bool,
    #[serde(default)]
    pub fail_on_tests: bool,
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
    #[serde(default)]
    pub arguments: Vec<Pair>,

    #[serde(default)]
    pub branches: Vec<Branch>,

    #[serde(default)]
    pub delay_ms: u64,
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

    /// Whether the saved request's post script is the one that runs. Only a REST request has scripts.
    pub fn runs_post_script(&self) -> bool {
        self.run_post_script && self.request_kind == "requests"
    }

    /// The ports the node can leave by: id and label.
    pub fn ports(&self) -> Vec<(String, String)> {
        let fixed =
            |ports: &[(&str, &str)]| ports.iter().map(|(id, label)| (id.to_string(), label.to_string())).collect();
        match self.kind {
            NodeKind::Start | NodeKind::Delay => fixed(&[(PORT_OUT, "Next")]),
            NodeKind::Request | NodeKind::Mcp => fixed(&[(PORT_SUCCESS, "Success"), (PORT_ERROR, "Error")]),
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
        Flow::deserialize(value)
            .map_err(|error| EngineError::bad_request(format!("This flow cannot be read: {error}.")))
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// The edges leaving `node` by `port`, in the order they were drawn.
    pub fn edges_from<'a>(&'a self, node: &'a str, port: &'a str) -> impl Iterator<Item = &'a Edge> {
        self.edges.iter().filter(move |edge| edge.from == node && edge.port == port)
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
            ],
            "edges": [{ "id": "e1", "from": "start", "port": "out", "to": "call" }],
        }))
        .unwrap();
        let call = flow.node("call").unwrap();
        assert!(call.run_post_script && call.fail_on_status && !call.fail_on_tests);
        assert!(!call.runs_post_script(), "only a REST request has a post script to run");
        assert_eq!(call.store_kind(), "sockets");
        assert_eq!(call.label(), "call");
        let ports: Vec<String> = flow.node("check").unwrap().ports().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ports, ["ok", "else"]);
        assert_eq!(flow.node("check").unwrap().branches[0].matching, Match::Any);
        assert_eq!(flow.edges_from("start", "out").count(), 1);
    }

    #[test]
    fn an_unknown_node_type_is_refused() {
        let error = Flow::parse(&json!({ "nodes": [{ "id": "a", "type": "teleport" }] })).unwrap_err();
        assert!(error.message.contains("cannot be read"), "{}", error.message);
    }
}
