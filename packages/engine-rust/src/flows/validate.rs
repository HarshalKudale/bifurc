//! What is wrong with a flow before it runs, and what a client needs to draw it: each node's ports, the
//! request it names and the inputs it takes, with where each gets its value.

use std::collections::{HashMap, HashSet};

use regress::Regex;
use serde_json::{Map, Value, json};

use super::{
    model::{Edge, Flow, Node, NodeKind, Rule},
    reference::references,
};
use crate::{
    env::load_merged_environment,
    rest::pipeline::env_to_map,
    utils::json::str_of,
    workspace::config::load_entity,
};

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

/// Every node that runs after each node: what can be reached from it by the connections that make a node
/// run. A wire carries a value and makes nothing run.
fn descendants(flow: &Flow) -> HashMap<&str, HashSet<&str>> {
    let mut next: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in flow.edges.iter().filter(|edge| !edge.is_wire()) {
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
    for rule in node.branches.iter().flat_map(|branch| &branch.rules).chain(&node.rules) {
        out.push(("a rule".to_string(), rule.value.as_str()));
    }
    out
}

fn is_variable_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn check_variables(report: &mut Report, node: &Node) {
    for pair in node.variables.iter().filter(|pair| !is_variable_name(&pair.name)) {
        report.node(node, format!("\"{}\" is not a variable name: use letters, digits and underscores.", pair.name));
    }
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
    check_variables(report, node);
}

/// A rule reads what the node was handed, an earlier node's output or an input of the run.
fn check_path(report: &mut Report, node: &Node, path: &str) {
    let root = path.split(['.', '[']).next().unwrap_or_default();
    if !matches!(root, "input" | "nodes" | "flow") {
        report.node(node, format!("\"{path}\" is not a value that can be read: start with input, nodes or flow."));
    }
}

/// A node that reads what the node before it produced needs one, and what it decides needs somewhere to go.
fn check_in_the_middle(report: &mut Report, flow: &Flow, node: &Node, what: &str) {
    if !flow.edges.iter().any(|edge| edge.to == node.id && !edge.is_wire()) {
        report.node(node, format!("{what} cannot start a flow: connect a node to it."));
    }
    if !flow.edges.iter().any(|edge| edge.from == node.id) {
        report.node(node, format!("{what} cannot end a flow: connect at least one of its outputs."));
    }
}

fn check_rules(report: &mut Report, node: &Node, rules: &[Rule]) {
    for rule in rules {
        check_path(report, node, &rule.path);
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
        check_rules(report, node, &branch.rules);
    }
    check_in_the_middle(report, flow, node, "A condition");
}

/// What a connection is wrong about, if anything. `reach` is what runs after each node.
fn check_edge(flow: &Flow, edge: &Edge, reach: &HashMap<&str, HashSet<&str>>) -> Option<String> {
    let (Some(from), Some(to)) = (flow.node(&edge.from), flow.node(&edge.to)) else {
        return Some("This connection is attached to a node that is gone.".to_string());
    };
    if !from.ports().iter().any(|(port, _)| *port == edge.port) {
        return Some(format!("\"{}\" has no output \"{}\".", from.label(), edge.port));
    }
    if to.kind == NodeKind::Start {
        return Some("Nothing can lead into the start node.".to_string());
    }
    if !edge.is_wire() {
        return match (from.kind.is_constant(), to.kind.is_constant()) {
            (true, _) => Some(format!("\"{}\" only holds a value: wire it into an input of a node.", from.label())),
            (_, true) => Some(format!("\"{}\" only holds a value: nothing runs it.", to.label())),
            _ => None,
        };
    }
    let takes_inputs =
        matches!(to.kind, NodeKind::Request | NodeKind::Stream | NodeKind::Mcp | NodeKind::Evaluate);
    if !takes_inputs {
        return Some(format!("\"{}\" has no inputs to wire a value into.", to.label()));
    }
    let known = matches!(from.kind, NodeKind::Data | NodeKind::Sample);
    let runs_before = reach.get(from.id.as_str()).is_some_and(|after| after.contains(to.id.as_str()));
    (!known && !runs_before).then(|| {
        format!(
            "\"{}\" is wired to \"{}\", which does not run before it: connect them so that it does.",
            to.label(),
            from.label()
        )
    })
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

    let reach = descendants(flow);
    for edge in &flow.edges {
        if let Some(message) = check_edge(flow, edge, &reach) {
            report.edge(&edge.id, message);
        }
    }

    if let Some(node) =
        flow.nodes.iter().find(|node| reach.get(node.id.as_str()).is_some_and(|set| set.contains(node.id.as_str())))
    {
        report.node(
            node,
            "This node leads back to itself. To go round, use a Repeat or For each node; to try again, a node's retries.",
        );
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
            NodeKind::Validate => {
                if node.rules.is_empty() {
                    report.node(node, "A validate node needs at least one check.");
                }
                check_rules(&mut report, node, &node.rules);
                check_in_the_middle(&mut report, flow, node, "A validate node");
            }
            NodeKind::Evaluate => {
                if node.expression.trim().is_empty() {
                    report.node(node, "Write the expression this node works out.");
                }
                check_variables(&mut report, node);
            }
            NodeKind::Repeat => {
                if node.count == 0 {
                    report.warn(node, "It repeats 0 times, so what follows Each never runs.");
                }
            }
            NodeKind::ForEach => {
                if !node.path.trim().is_empty() {
                    check_path(&mut report, node, &node.path);
                }
            }
            NodeKind::Delay | NodeKind::Or | NodeKind::Display | NodeKind::Data | NodeKind::Sample => {}
        }
        let runs = node.kind == NodeKind::Start || started.contains(node.id.as_str());
        if !runs && !node.kind.is_constant() {
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

/// The inputs a node takes, each with where its value comes from as the flow stands: `wired` (with the node
/// and the part of its output), `static` (typed in for the node), `input` (an input of the run), `env` (the
/// environment in force, with the value there) or `unset`. A wire into an input the node no longer takes is
/// listed too, so that it can be seen and removed.
fn inputs_of(flow: &Flow, node: &Node, variables: &[String], env: &Value) -> Vec<Value> {
    let typed = |pairs: &[super::model::Pair], name: &str| {
        pairs.iter().find(|pair| pair.name == name).map(|pair| pair.value.clone())
    };
    // The id of each input, with what was typed in for it.
    let mut listed: Vec<(String, Option<String>)> = Vec::new();
    let takes_variables =
        matches!(node.kind, NodeKind::Request | NodeKind::Stream | NodeKind::Mcp | NodeKind::Evaluate);
    if takes_variables {
        let names = variables.iter().chain(node.variables.iter().map(|pair| &pair.name));
        for name in names {
            let id = format!("var:{name}");
            if !listed.iter().any(|(known, _)| *known == id) {
                listed.push((id, typed(&node.variables, name)));
            }
        }
    }
    if node.kind == NodeKind::Mcp {
        let given = |pair: &super::model::Pair| Some(pair.value.clone()).filter(|value| !value.is_empty());
        listed.extend(node.arguments.iter().map(|pair| (format!("arg:{}", pair.name), given(pair))));
    }
    let sends = matches!(node.request_kind.as_str(), "wsConnections" | "socketioRequests" | "grpcRequests");
    if node.kind == NodeKind::Stream && sends {
        listed.push(("message".to_string(), Some(node.message.clone()).filter(|text| !text.is_empty())));
    }
    for wire in flow.wires_into(&node.id) {
        if !listed.iter().any(|(known, _)| *known == wire.input) {
            listed.push((wire.input.clone(), None));
        }
    }

    listed
        .into_iter()
        .map(|(id, typed)| {
            let name = id.split_once(':').map_or(id.as_str(), |(_, name)| name).to_string();
            let wire = flow.wires_into(&node.id).find(|wire| wire.input == id);
            let from_run = flow.inputs.iter().find(|input| input.name == name).filter(|_| id.starts_with("var:"));
            let from_env = env.get(&name).filter(|_| id.starts_with("var:") && node.kind != NodeKind::Evaluate);
            let (source, value) = match (wire, typed, from_run, from_env) {
                (Some(_), ..) => ("wired", Value::Null),
                (None, Some(text), ..) => ("static", json!(text)),
                (None, None, Some(input), _) => ("input", json!(input.value)),
                (None, None, None, Some(value)) => ("env", value.clone()),
                (None, None, None, None) => ("unset", Value::Null),
            };
            json!({
                "id": id, "name": name, "source": source, "value": value,
                "from": wire.map(|wire| json!({
                    "edgeId": wire.id, "nodeId": wire.from, "path": wire.path,
                    "name": flow.node(&wire.from).map(Node::label),
                })),
            })
        })
        .collect()
}

/// The answer of `flow.validate`: the problems, whether the flow can run, and what each node offers.
pub fn describe(ws_id: &str, flow: &Flow) -> Value {
    let problems = check(ws_id, flow);
    let env = env_to_map(load_merged_environment(ws_id).as_ref());
    let mut nodes = Map::new();
    for node in &flow.nodes {
        let entity =
            (!node.request_id.is_empty()).then(|| load_entity(ws_id, node.store_kind(), &node.request_id)).flatten();
        let ports: Vec<Value> = node.ports().into_iter().map(|(id, name)| json!({ "id": id, "name": name })).collect();
        let variables = entity.as_ref().map(variables_of).unwrap_or_default();
        nodes.insert(
            node.id.clone(),
            json!({
                "ports": ports,
                "request": entity.as_ref().map(|entity| request_summary(node, entity)),
                "inputs": inputs_of(flow, node, &variables, &env),
                "variables": variables,
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

    fn wire(from: &str, port: &str, to: &str, input: &str) -> Value {
        json!({ "id": format!("{from}-{to}-{input}"), "from": from, "port": port, "to": to, "input": input, "path": ["body", "id"] })
    }

    #[test]
    fn a_sound_flow_has_no_problems_and_names_what_its_request_takes() {
        let _root = scratch_data_root();
        saved_request("r1", "http://{{host}}/users/{{id}}");
        let flow = Flow::parse(&json!({
            "inputs": [{ "name": "host", "value": "api.test" }],
            "nodes": [
                { "id": "start", "type": "start" },
                request("a"),
                { "id": "check", "type": "condition", "branches": [
                    { "id": "ok", "rules": [{ "path": "input.status", "op": "equals", "value": "200" }] },
                ] },
                { "id": "b", "type": "request", "requestKind": "requests", "requestId": "r1",
                  "variables": [{ "name": "token", "value": "t-1" }] },
                { "id": "seven", "type": "data", "value": 7 },
            ],
            "edges": [
                edge("start", "out", "a"), edge("a", "success", "check"), edge("check", "ok", "b"),
                wire("a", "success", "b", "var:id"), wire("seven", "out", "b", "var:gone"),
            ],
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

        let sources = |node: &str| -> Vec<(String, String)> {
            described["nodes"][node]["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|input| (str_of(input, "id").to_string(), str_of(input, "source").to_string()))
                .collect()
        };
        let pair = |id: &str, source: &str| (id.to_string(), source.to_string());
        assert_eq!(sources("a"), [pair("var:host", "input"), pair("var:id", "unset"), pair("var:token", "unset")]);
        assert_eq!(
            sources("b"),
            [pair("var:host", "input"), pair("var:id", "wired"), pair("var:token", "static"), pair("var:gone", "wired")]
        );
        let wired = &described["nodes"]["b"]["inputs"][1];
        assert_eq!((&wired["from"]["nodeId"], &wired["from"]["path"]), (&json!("a"), &json!(["body", "id"])));
        assert_eq!(described["nodes"]["b"]["inputs"][0]["value"], "api.test");
        assert!(sources("check").is_empty());
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
            "nodes": [
                { "id": "start", "type": "start" }, request("a"), request("a"), { "id": "bad id", "type": "delay" },
                request("late"), { "id": "wait", "type": "delay" }, { "id": "seven", "type": "data", "value": 7 },
            ],
            "edges": [
                edge("start", "nope", "a"), edge("a", "success", "start"), edge("a", "success", "gone"),
                edge("a", "success", "late"), wire("late", "success", "a", "var:id"), wire("a", "success", "wait", "var:id"),
                edge("seven", "out", "a"), edge("a", "success", "seven"),
            ],
        }));
        for expected in [
            "share the id",
            "not a node id",
            "has no output \"nope\"",
            "lead into the start",
            "node that is gone",
            "is wired to \"late\", which does not run before it",
            "\"wait\" has no inputs",
            "\"seven\" only holds a value: wire it",
            "\"seven\" only holds a value: nothing runs it",
        ] {
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
                { "id": "checks", "type": "validate" },
                { "id": "sum", "type": "evaluate" },
                { "id": "each", "type": "forEach", "path": "body.items" },
            ],
            "edges": [edge("start", "out", "early"), edge("early", "success", "late")],
        }));
        for expected in [
            "No saved request is chosen",
            "no longer exists",
            "cannot be used by this node",
            "Name the tool to call",
            "A condition cannot start a flow",
            "A condition cannot end a flow",
            "has no rules",
            "empty, taken or not allowed",
            "not a variable name",
            "reads \"late\", which does not run before",
            "needs at least one check",
            "A validate node cannot start a flow",
            "Write the expression",
            "\"body.items\" is not a value that can be read",
        ] {
            assert!(found.iter().any(|m| m.contains(expected)), "{expected}: {found:?}");
        }
        let flow = Flow::parse(&json!({
            "nodes": [{ "id": "start", "type": "start" }, request("a"), { "id": "seven", "type": "data" }],
            "edges": [],
        }))
        .unwrap();
        let warnings: Vec<String> = check(WS, &flow).into_iter().filter(|p| !p.is_error).map(|p| p.message).collect();
        assert_eq!(warnings.len(), 2, "a data node is not expected to run: {warnings:?}");
    }
}
