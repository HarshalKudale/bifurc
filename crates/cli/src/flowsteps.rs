//! A flow that is a straight line, read back as the steps it was written from: `flows show` prints them one
//! per line and `flows show --steps` as the step file `flows create --steps` takes.
//!
//! A flow is a line when it is a start node, then request nodes each followed by its `expect` checks, with
//! only success edges between them. Display nodes (where a failure is shown, or the end of the chain) are
//! ignored. What a step extracts is read from the wires that carry its response to later requests.

use serde_json::Value;

use crate::{format::js_template, steps::path_of};

/// One step of a linear flow.
#[derive(Debug, PartialEq)]
pub struct Line {
    pub method: String,
    pub address: String,
    pub name: String,
    pub vars: Vec<(String, String)>,
    pub extract: Vec<(String, String)>,
    pub expects: Vec<String>,
}

fn items<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

fn text(value: &Value, key: &str) -> String {
    js_template(value.get(key))
}

/// `body`, `items`, 0 as `.items[0]`; `None` for a path that does not start in the body.
fn extract_path(path: &[Value]) -> Option<String> {
    let (first, rest) = path.split_first()?;
    if first.as_str() != Some("body") {
        return None;
    }
    let mut out = String::new();
    for part in rest {
        match part {
            Value::String(key) => out.push_str(&format!(".{key}")),
            Value::Number(index) => out.push_str(&format!("[{index}]")),
            _ => return None,
        }
    }
    Some(if out.is_empty() { ".".to_string() } else { out })
}

/// The steps of `flow` when it is a line; `described` is what `flow.validate` says each node runs.
pub fn linear(flow: &Value, described: &Value) -> Option<Vec<Line>> {
    let nodes = items(flow, "nodes");
    let edges = items(flow, "edges");
    let node = |id: &str| nodes.iter().find(|node| text(node, "id") == id);
    let kind = |node: &Value| text(node, "type");
    let mut starts = nodes.iter().filter(|node| kind(node) == "start");
    let (start, None) = (starts.next()?, starts.next()) else { return None };

    let mut lines: Vec<Line> = Vec::new();
    // The request node of each step, to match the wires with.
    let mut requests: Vec<String> = Vec::new();
    let mut chain = vec![text(start, "id")];
    let mut at = start;
    loop {
        let success = match kind(at).as_str() {
            "start" | "evaluate" => "out",
            "request" => "success",
            "validate" => "pass",
            _ => return None,
        };
        let id = text(at, "id");
        let mut next = None;
        for edge in edges.iter().filter(|edge| edge.get("input").is_none_or(|input| input.as_str() == Some(""))) {
            if text(edge, "from") != id {
                continue;
            }
            let target = node(&text(edge, "to"))?;
            if kind(target) == "display" {
                continue;
            }
            if text(edge, "port") != success || next.replace(target).is_some() {
                return None;
            }
        }
        let Some(target) = next else { break };
        let target_id = text(target, "id");
        if chain.contains(&target_id) {
            return None;
        }
        match kind(target).as_str() {
            "request" => {
                let request = described.get(&target_id).and_then(|node| node.get("request")).filter(|r| r.is_object());
                let kind = text(target, "requestKind");
                let plain = !["sseRequests", "wsConnections", "socketioRequests"].contains(&kind.as_str());
                let sound = target.get("failOnStatus").and_then(Value::as_bool) != Some(false)
                    && target.get("executeScripts").and_then(Value::as_bool) != Some(false)
                    && target.get("executeTests").and_then(Value::as_bool) != Some(false)
                    && target.get("ignoreTestErrors").and_then(Value::as_bool) != Some(true)
                    && target.get("retries").and_then(Value::as_u64).unwrap_or(0) == 0;
                if !plain || !sound {
                    return None;
                }
                let name =
                    request.map(|r| text(r, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| text(target, "name"));
                lines.push(Line {
                    method: request.map(|r| text(r, "method")).unwrap_or_default(),
                    address: request.map(|r| text(r, "address")).unwrap_or_default(),
                    name,
                    vars: items(target, "variables").iter().map(|v| (text(v, "name"), text(v, "value"))).collect(),
                    extract: Vec::new(),
                    expects: Vec::new(),
                });
                requests.push(target_id.clone());
            }
            "validate" | "evaluate" => {
                let name = text(target, "name");
                let checks = if kind(target) == "validate" { items(target, "rules").len() } else { 1 };
                let expect = name.strip_prefix("expect ")?;
                if checks != 1 {
                    return None;
                }
                lines.last_mut()?.expects.push(expect.to_string());
            }
            _ => return None,
        }
        chain.push(target_id);
        at = target;
    }
    // Every node is the start, a step, or a display node.
    if nodes.iter().any(|node| !chain.contains(&text(node, "id")) && kind(node) != "display") {
        return None;
    }
    if lines.is_empty() {
        return None;
    }
    for wire in edges.iter().filter(|edge| edge.get("input").is_some_and(|input| input.as_str() != Some(""))) {
        let from = requests.iter().position(|id| *id == text(wire, "from"))?;
        let to = requests.iter().position(|id| *id == text(wire, "to"))?;
        let name = text(wire, "input").strip_prefix("var:")?.to_string();
        let path = extract_path(items(wire, "path"))?;
        if to <= from {
            return None;
        }
        if !lines[from].extract.contains(&(name.clone(), path.clone())) {
            lines[from].extract.push((name, path));
        }
    }
    Some(lines)
}

/// `1 POST /login  Login as admin  extract cartId=.id  expect status == 201`.
pub fn render(lines: &[Line]) -> Vec<String> {
    lines
        .iter()
        .enumerate()
        .map(|(at, line)| {
            let target = format!("{} {}", line.method, path_of(&line.address));
            let mut out = format!("{} {}  {}", at + 1, target.trim(), line.name);
            if !line.vars.is_empty() {
                let vars: Vec<String> = line.vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
                out += &format!("  vars {}", vars.join(", "));
            }
            if !line.extract.is_empty() {
                let pairs: Vec<String> = line.extract.iter().map(|(k, v)| format!("{k}={v}")).collect();
                out += &format!("  extract {}", pairs.join(", "));
            }
            if !line.expects.is_empty() {
                out += &format!("  expect {}", line.expects.join(", "));
            }
            out
        })
        .collect()
}

/// `text` as it can stand in a YAML flow collection: as it is when it is plain, else in JSON quotes. A number
/// or a boolean is quoted too unless `bare` allows it (a variable's value, which is text either way).
fn scalar(text: &str, bare: bool) -> String {
    let plain = text.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '.' || c == '/')
        && !text.ends_with(' ')
        && text.chars().all(|c| c.is_alphanumeric() || " _./-|=!<>".contains(c));
    let special = ["true", "false", "null", "yes", "no", "on", "off", "y", "n"].contains(&text.to_lowercase().as_str())
        || text.parse::<f64>().is_ok();
    if plain && (bare || !special) { text.to_string() } else { Value::String(text.to_string()).to_string() }
}

/// The step file of a linear flow.
pub fn yaml(name: &str, lines: &[Line]) -> String {
    let mut out = format!("name: {}\nsteps:\n", scalar(name, false));
    for line in lines {
        out += &format!("  - request: {}\n", scalar(&line.name, false));
        let map = |pairs: &[(String, String)], bare: bool| {
            let items: Vec<String> = pairs.iter().map(|(k, v)| format!("{k}: {}", scalar(v, bare))).collect();
            format!("{{ {} }}", items.join(", "))
        };
        if !line.vars.is_empty() {
            out += &format!("    vars: {}\n", map(&line.vars, true));
        }
        if !line.extract.is_empty() {
            out += &format!("    extract: {}\n", map(&line.extract, false));
        }
        if !line.expects.is_empty() {
            let expects: Vec<String> = line.expects.iter().map(|e| scalar(e, false)).collect();
            out += &format!("    expect: [ {} ]\n", expects.join(", "));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn request(id: &str, extra: Value) -> Value {
        let mut node = json!({"id": id, "name": id, "type": "request", "requestKind": "requests", "requestId": id});
        node.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        node
    }

    fn edge(id: &str, from: &str, port: &str, to: &str) -> Value {
        json!({"id": id, "from": from, "port": port, "to": to})
    }

    fn described() -> Value {
        json!({
            "s1": {"request": {"name": "Login", "method": "POST", "address": "{{baseUrl}}/login"}},
            "s2": {"request": {"name": "Create cart", "method": "POST", "address": "http://x/carts"}},
            "s3": {"request": {"name": "Add item", "method": "POST", "address": "http://x/carts/{{cartId}}/items"}},
        })
    }

    fn line() -> Value {
        json!({
            "nodes": [
                {"id": "start", "type": "start"},
                request("s1", json!({})),
                request("s2", json!({})),
                {"id": "s2_x1", "type": "validate", "name": "expect status == 201", "rules": [{"path": "input.status"}]},
                request("s3", json!({"variables": [{"name": "qty", "value": "2"}]})),
                {"id": "s3_x1", "type": "evaluate", "name": "expect .items | length == 1", "expression": "1"},
                {"id": "failed", "type": "display"},
            ],
            "edges": [
                edge("e1", "start", "out", "s1"), edge("e2", "s1", "success", "s2"), edge("e2b", "s1", "error", "failed"),
                edge("e3", "s2", "success", "s2_x1"), edge("e4", "s2_x1", "pass", "s3"),
                edge("e5", "s3", "success", "s3_x1"),
                {"id": "w1", "from": "s2", "port": "success", "to": "s3", "input": "var:cartId", "path": ["body", "id"]},
            ],
        })
    }

    #[test]
    fn a_chain_of_requests_and_checks_reads_back_as_steps() {
        let lines = linear(&line(), &described()).unwrap();
        assert_eq!(
            render(&lines),
            [
                "1 POST /login  Login",
                "2 POST /carts  Create cart  extract cartId=.id  expect status == 201",
                "3 POST /carts/{{cartId}}/items  Add item  vars qty=2  expect .items | length == 1",
            ]
        );
        assert_eq!(
            yaml("Checkout happy path", &lines),
            "name: Checkout happy path\nsteps:\n  - request: Login\n  - request: Create cart\n    extract: { cartId: .id }\n    expect: [ status == 201 ]\n  - request: Add item\n    vars: { qty: 2 }\n    expect: [ .items | length == 1 ]\n"
        );
    }

    #[test]
    fn anything_else_is_not_a_line() {
        let mut branching = line();
        branching["edges"].as_array_mut().unwrap().push(edge("e9", "s1", "success", "s3"));
        let mut other = line();
        other["nodes"].as_array_mut().unwrap().push(json!({"id": "d", "type": "delay"}));
        let mut lenient = line();
        lenient["nodes"][1]["failOnStatus"] = json!(false);
        let mut unnamed = line();
        unnamed["nodes"][3]["name"] = json!("Check");
        let mut backwards = line();
        backwards["edges"].as_array_mut().unwrap().push(
            json!({"id": "w2", "from": "s3", "port": "success", "to": "s2", "input": "var:x", "path": ["body", "id"]}),
        );
        let mut headers = line();
        headers["edges"].as_array_mut().unwrap().push(
            json!({"id": "w2", "from": "s2", "port": "success", "to": "s3", "input": "var:x", "path": ["headers", "id"]}),
        );
        for flow in [branching, other, lenient, unnamed, backwards, headers] {
            assert!(linear(&flow, &described()).is_none());
        }
        assert!(linear(&json!({"nodes": [{"id": "start", "type": "start"}], "edges": []}), &json!({})).is_none());
    }

    #[test]
    fn text_is_quoted_only_when_yaml_would_read_it_differently() {
        assert_eq!(scalar("Login as admin", false), "Login as admin");
        assert_eq!(scalar("Create cart: now", false), "\"Create cart: now\"");
        assert_eq!(scalar("2", false), "\"2\"");
        assert_eq!(scalar("2", true), "2");
        assert_eq!(scalar("null", false), "\"null\"");
        assert_eq!(scalar("a, b", false), "\"a, b\"");
        assert_eq!(scalar("status == 201", false), "status == 201");
    }
}
