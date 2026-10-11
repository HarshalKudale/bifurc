//! What is wrong with a flow before it runs, and what a client needs to draw it: each node's ports, the
//! request it names and the variables that request takes.

use std::collections::{HashMap, HashSet};

use regress::Regex;
use serde_json::{Map, Value, json};

use super::{
    model::{Flow, Node, NodeKind},
    reference::references,
};
use crate::{utils::json::str_of, workspace::config::load_entity};

#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub node_id: Option<String>,
    pub edge_id: Option<String>,
    /// An error keeps the flow from running; a warning does not.
    pub is_error: bool,
    pub message: String,
}

impl Problem {
    fn to_value(&self) -> Value {
        json!({
            "nodeId": self.node_id, "edgeId": self.edge_id,
            "severity": if self.is_error { "error" } else { "warning" }, "message": self.message,
        })
    }
}

#[derive(Default)]
struct Report(Vec<Problem>);

impl Report {
    fn node(&mut self, node: &Node, message: impl Into<String>) {
        self.0.push(Problem { node_id: Some(node.id.clone()), edge_id: None, is_error: true, message: message.into() });
    }

    fn warn(&mut self, node: &Node, message: impl Into<String>) {
        self.0.push(Problem {
            node_id: Some(node.id.clone()),
            edge_id: None,
            is_error: false,
            message: message.into(),
        });
    }

    fn edge(&mut self, edge_id: &str, message: impl Into<String>) {
        self.0.push(Problem {
            node_id: None,
            edge_id: Some(edge_id.to_string()),
            is_error: true,
            message: message.into(),
        });
    }

    fn flow(&mut self, message: impl Into<String>) {
        self.0.push(Problem { node_id: None, edge_id: None, is_error: true, message: message.into() });
    }
}

fn is_id(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Every node that can be reached from each node by following edges.
fn descendants(flow: &Flow) -> HashMap<&str, HashSet<&str>> {
    let mut next: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &flow.edges {
        next.entry(edge.from.as_str()).or_default().push(edge.to.as_str());
    }
    flow.nodes
        .iter()
        .map(|node| {
            let mut seen: HashSet<&str> = HashSet::new();
            let mut todo: Vec<&str> = next.get(node.id.as_str()).cloned().unwrap_or_default();
            while let Some(id) = todo.pop() {
                if seen.insert(id) {
                    todo.extend(next.get(id).into_iter().flatten());
                }
            }
            (node.id.as_str(), seen)
        })
        .collect()
}

/// Every text of a node that may hold references, with what it is called in a message.
fn templates(node: &Node) -> Vec<(String, &str)> {
    let mut out: Vec<(String, &str)> = Vec::new();
    out.extend(node.variables.iter().map(|pair| (format!("the variable \"{}\"", pair.name), pair.value.as_str())));
    out.extend(node.arguments.iter().map(|pair| (format!("the argument \"{}\"", pair.name), pair.value.as_str())));
    out.push(("the message".to_string(), node.message.as_str()));
    out.push(("the event".to_string(), node.event.as_str()));
    out.push(("the target".to_string(), node.target.as_str()));
    for rule in node.branches.iter().flat_map(|branch| &branch.rules) {
        out.push(("a rule".to_string(), rule.value.as_str()));
    }
    out
}

fn check_request(report: &mut Report, ws_id: &str, node: &Node) {
    if node.request_id.is_empty() {
        return report.node(node, "No saved request is chosen.");
    }
    if !node.kind.request_kinds().contains(&node.request_kind.as_str()) {
        return report
            .node(node, format!("A request of the kind \"{}\" cannot be used by this node.", node.request_kind));
    }
    if load_entity(ws_id, node.store_kind(), &node.request_id).is_none() {
        report.node(node, "The saved request this node runs no longer exists.");
    }
    for pair in &node.variables {
        if pair.name.is_empty() || !pair.name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            report
                .node(node, format!("\"{}\" is not a variable name: use letters, digits and underscores.", pair.name));
        }
    }
}

fn check_condition(report: &mut Report, flow: &Flow, node: &Node) {
    if node.branches.is_empty() {
        report.node(node, "A condition needs at least one branch.");
    }
    let mut seen = HashSet::new();
    for branch in &node.branches {
        if !is_id(&branch.id) || branch.id == super::model::PORT_ELSE || !seen.insert(branch.id.as_str()) {
            report.node(node, format!("The branch id \"{}\" is empty, taken or not allowed.", branch.id));
        }
        if branch.rules.is_empty() {
            report.node(node, format!("The branch \"{}\" has no rules, so it is never taken.", branch.name));
        }
        for rule in &branch.rules {
            let root = rule.path.split(['.', '[']).next().unwrap_or_default();
            if !matches!(root, "input" | "nodes" | "flow") {
                report.node(
                    node,
                    format!("\"{}\" is not a value a rule can read: start with input, nodes or flow.", rule.path),
                );
            }
        }
    }
    if !flow.edges.iter().any(|edge| edge.to == node.id) {
        report.node(node, "A condition cannot start a flow: connect a node to it.");
    }
    if !flow.edges.iter().any(|edge| edge.from == node.id) {
        report.node(node, "A condition cannot end a flow: connect at least one of its branches.");
    }
}

/// The problems of `flow` in the workspace `ws_id`. A flow with an error among them is not run.
pub fn check(ws_id: &str, flow: &Flow) -> Vec<Problem> {
    let mut report = Report::default();

    let mut ids = HashSet::new();
    for node in &flow.nodes {
        if !is_id(&node.id) {
            report.node(node, format!("\"{}\" is not a node id: use letters, digits, - and _.", node.id));
        } else if !ids.insert(node.id.as_str()) {
            report.node(node, format!("Two nodes share the id \"{}\".", node.id));
        }
    }
    let starts: Vec<&Node> = flow.nodes.iter().filter(|node| node.kind == NodeKind::Start).collect();
    if starts.len() != 1 {
        report.flow(format!("A flow has exactly one start node; this one has {}.", starts.len()));
    }

    for edge in &flow.edges {
        match (flow.node(&edge.from), flow.node(&edge.to)) {
            (Some(from), Some(to)) => {
                if !from.ports().iter().any(|(port, _)| *port == edge.port) {
                    report.edge(&edge.id, format!("\"{}\" has no output \"{}\".", from.label(), edge.port));
                }
                if to.kind == NodeKind::Start {
                    report.edge(&edge.id, "Nothing can lead into the start node.");
                }
            }
            _ => report.edge(&edge.id, "This connection is attached to a node that is gone."),
        }
    }

    let reach = descendants(flow);
    if let Some(node) =
        flow.nodes.iter().find(|node| reach.get(node.id.as_str()).is_some_and(|set| set.contains(node.id.as_str())))
    {
        report.node(node, "This node leads back to itself. A flow cannot loop; use a node's retries to try again.");
    }
    let started: HashSet<&str> =
        starts.first().and_then(|start| reach.get(start.id.as_str())).cloned().unwrap_or_default();

    for node in &flow.nodes {
        match node.kind {
            NodeKind::Start => {
                if !flow.edges.iter().any(|edge| edge.from == node.id) {
                    report.warn(node, "Nothing is connected to the start, so a run does nothing.");
                }
            }
            NodeKind::Request | NodeKind::Stream => check_request(&mut report, ws_id, node),
            NodeKind::Mcp => {
                check_request(&mut report, ws_id, node);
                if !matches!(node.call.as_str(), "tool" | "prompt" | "resource") {
                    report.node(node, "Choose what to call: a tool, a prompt or a resource.");
                } else if node.target.trim().is_empty() {
                    report.node(node, format!("Name the {} to call.", node.call));
                }
            }
            NodeKind::Condition => check_condition(&mut report, flow, node),
            NodeKind::Delay => {}
        }
        if node.kind != NodeKind::Start && !started.contains(node.id.as_str()) {
            report.warn(node, "This node is not connected to the start, so it never runs.");
        }
        for (what, template) in templates(node) {
            for path in references(template) {
                let Some(other) =
                    path.strip_prefix("nodes.").map(|rest| rest.split(['.', '[']).next().unwrap_or_default())
                else {
                    continue;
                };
                let runs_before = reach.get(other).is_some_and(|after| after.contains(node.id.as_str()));
                if !runs_before {
                    report.node(node, format!("{what} reads \"{other}\", which does not run before this node."));
                }
            }
        }
    }
    report.0
}

/// The names of the `{{variables}}` a saved request takes, in the order it first uses them.
fn variables_of(entity: &Value) -> Vec<String> {
    let pattern = Regex::new(r"\{\{(\w+)\}\}").expect("a valid pattern");
    let text = entity.to_string();
    let mut names: Vec<String> = Vec::new();
    for found in pattern.find_iter(&text) {
        let name = found.group(1).map(|range| &text[range]).unwrap_or_default();
        if !names.iter().any(|known| known == name) {
            names.push(name.to_string());
        }
    }
    names
}

/// What a listing shows of a saved request: its name and where it goes.
fn request_summary(node: &Node, entity: &Value) -> Value {
    let address =
        ["url", "endpointUrl", "serverAddress"].iter().map(|key| str_of(entity, key)).find(|text| !text.is_empty());
    json!({
        "id": node.request_id, "kind": node.request_kind, "name": str_of(entity, "name"),
        "method": entity.get("method").cloned().unwrap_or(Value::Null), "address": address,
    })
}

/// The answer of `flow.validate`: the problems, whether the flow can run, and what each node offers.
pub fn describe(ws_id: &str, flow: &Flow) -> Value {
    let problems = check(ws_id, flow);
    let mut nodes = Map::new();
    for node in &flow.nodes {
        let entity =
            (!node.request_id.is_empty()).then(|| load_entity(ws_id, node.store_kind(), &node.request_id)).flatten();
        let ports: Vec<Value> = node.ports().into_iter().map(|(id, name)| json!({ "id": id, "name": name })).collect();
        nodes.insert(
            node.id.clone(),
            json!({
                "ports": ports,
                "request": entity.as_ref().map(|entity| request_summary(node, entity)),
                "variables": entity.as_ref().map(variables_of).unwrap_or_default(),
            }),
        );
    }
    json!({
        "ok": !problems.iter().any(|problem| problem.is_error),
        "problems": problems.iter().map(Problem::to_value).collect::<Vec<_>>(),
        "nodes": nodes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{testing::scratch_data_root, workspace::fs::write_entity};

    const WS: &str = "ws1";

    fn errors(flow: Value) -> Vec<String> {
        let flow = Flow::parse(&flow).unwrap();
        check(WS, &flow).into_iter().filter(|problem| problem.is_error).map(|problem| problem.message).collect()
    }

    fn saved_request(id: &str, url: &str) {
        let folder = crate::workspace::fs::create_folder(WS, "requests", "F", None, false).unwrap().0;
        let entity = json!({
            "id": id, "name": "Get user", "method": "GET", "url": url, "headers": { "Authorization": "Bearer {{token}}" },
            "body": "", "workspaceId": WS, "createdAt": 0,
        });
        write_entity(WS, "requests", id, &entity, Some(folder.as_str())).unwrap();
    }

    fn request(id: &str) -> Value {
        json!({ "id": id, "type": "request", "requestKind": "requests", "requestId": "r1" })
    }

    fn edge(from: &str, port: &str, to: &str) -> Value {
        json!({ "id": format!("{from}-{to}"), "from": from, "port": port, "to": to })
    }

    #[test]
    fn a_sound_flow_has_no_problems_and_names_what_its_request_takes() {
        let _root = scratch_data_root();
        saved_request("r1", "http://{{host}}/users/{{id}}");
        let flow = Flow::parse(&json!({
            "nodes": [
                { "id": "start", "type": "start" },
                request("a"),
                { "id": "check", "type": "condition", "branches": [
                    { "id": "ok", "rules": [{ "path": "input.status", "op": "equals", "value": "200" }] },
                ] },
                { "id": "b", "type": "request", "requestKind": "requests", "requestId": "r1",
                  "variables": [{ "name": "id", "value": "{{nodes.a.body.id}}" }] },
            ],
            "edges": [edge("start", "out", "a"), edge("a", "success", "check"), edge("check", "ok", "b")],
        }))
        .unwrap();
        let described = describe(WS, &flow);
        assert_eq!(described["problems"], json!([]));
        assert_eq!(described["ok"], true);
        assert_eq!(described["nodes"]["a"]["variables"], json!(["host", "id", "token"]));
        assert_eq!(described["nodes"]["a"]["request"]["name"], "Get user");
        assert_eq!(described["nodes"]["a"]["request"]["address"], "http://{{host}}/users/{{id}}");
        assert_eq!(
            described["nodes"]["check"]["ports"],
            json!([{ "id": "ok", "name": "ok" }, { "id": "else", "name": "Else" }])
        );
        assert_eq!(described["nodes"]["start"]["request"], Value::Null);
    }

    #[test]
    fn the_shape_of_the_graph_is_checked() {
        let _root = scratch_data_root();
        saved_request("r1", "http://x.test/");
        let none = errors(json!({ "nodes": [request("a")], "edges": [] }));
        assert!(none.iter().any(|m| m.contains("exactly one start node")), "{none:?}");

        let looped = errors(json!({
            "nodes": [{ "id": "start", "type": "start" }, request("a"), request("b")],
            "edges": [edge("start", "out", "a"), edge("a", "success", "b"), edge("b", "success", "a")],
        }));
        assert!(looped.iter().any(|m| m.contains("leads back to itself")), "{looped:?}");

        let wired = errors(json!({
            "nodes": [{ "id": "start", "type": "start" }, request("a"), request("a"), { "id": "bad id", "type": "delay" }],
            "edges": [edge("start", "nope", "a"), edge("a", "success", "start"), edge("a", "success", "gone")],
        }));
        for expected in
            ["share the id", "not a node id", "has no output \"nope\"", "lead into the start", "node that is gone"]
        {
            assert!(wired.iter().any(|m| m.contains(expected)), "{expected}: {wired:?}");
        }
    }

    #[test]
    fn nodes_are_checked_for_what_they_need() {
        let _root = scratch_data_root();
        saved_request("r1", "http://x.test/");
        let found = errors(json!({
            "nodes": [
                { "id": "start", "type": "start" },
                { "id": "none", "type": "request" },
                { "id": "gone", "type": "request", "requestKind": "requests", "requestId": "missing" },
                { "id": "kind", "type": "stream", "requestKind": "requests", "requestId": "r1" },
                { "id": "tool", "type": "mcp", "requestKind": "mcpRequests", "requestId": "m1", "call": "tool" },
                { "id": "first", "type": "condition", "branches": [{ "id": "else", "rules": [] }] },
                { "id": "early", "type": "request", "requestKind": "requests", "requestId": "r1",
                  "variables": [{ "name": "a b", "value": "{{nodes.late.body}}" }] },
                request("late"),
            ],
            "edges": [edge("start", "out", "early"), edge("early", "success", "late")],
        }));
        for expected in [
            "No saved request is chosen",
            "no longer exists",
            "cannot be used by this node",
            "Name the tool to call",
            "cannot start a flow",
            "cannot end a flow",
            "has no rules",
            "empty, taken or not allowed",
            "not a variable name",
            "reads \"late\", which does not run before",
        ] {
            assert!(found.iter().any(|m| m.contains(expected)), "{expected}: {found:?}");
        }
        let flow =
            Flow::parse(&json!({ "nodes": [{ "id": "start", "type": "start" }, request("a")], "edges": [] })).unwrap();
        let warnings: Vec<String> = check(WS, &flow).into_iter().filter(|p| !p.is_error).map(|p| p.message).collect();
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }
}
