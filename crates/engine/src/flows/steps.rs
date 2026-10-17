//! `flow.fromSteps`: a linear list of steps (YAML or JSON) compiled into the flow graph the canvas draws.
//!
//! The graph is the ordinary one: a start node, then each step's request node and one validate (or, where
//! a check needs `| length`, evaluate) node for each of its `expect`s, left to right. What a step `extract`s
//! is wired to every later request node as the variable of that name, so `{{cartId}}` resolves wherever a
//! later request uses it. Nothing is saved but the requests of `inline` steps; the caller saves the flow.

use serde_json::{Map, Value, json};

use crate::{
    importexport::support::parse_any,
    kernel::{CommandContext, CommandRegistry, kinds::to_engine_kind},
    protocol::Result,
    utils::json::{array_of, opt_str, str_of},
    workspace::{
        config::load_config,
        fs::{read_all_entities, sanitize_dir_name},
    },
};

/// How far apart the nodes are, left to right.
const NODE_STEP: f64 = 360.0;

/// The kinds of saved request a flow can send, by their protocol names.
const SENDABLE: [&str; 4] = ["requests", "graphqlRequests", "soapRequests", "grpcRequests"];

const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

/// What is wrong with the step file: `usage` for a mistake in how it is written, `notFound` for a request it
/// names that is not there.
struct Problem {
    step: usize,
    kind: &'static str,
    message: String,
}

fn usage(step: usize, message: impl Into<String>) -> Problem {
    Problem { step, kind: "usage", message: message.into() }
}

#[derive(Clone, Copy, PartialEq)]
enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
}

#[derive(Clone, Copy, PartialEq)]
enum Pipe {
    Length,
    Keys,
}

/// One `expect`: `<status | .path> [| length | keys] <op> <value>`.
struct Expect {
    text: String,
    /// `None` is the status; else the keys and indexes into the body.
    path: Option<Vec<Value>>,
    pipe: Option<Pipe>,
    cmp: Cmp,
    wanted: Value,
}

fn tail(rest: &str) -> Option<&str> {
    match rest.strip_prefix('.') {
        Some("") => None,
        Some(after) => Some(after),
        None if rest.is_empty() || rest.starts_with('[') => Some(rest),
        None => None,
    }
}

/// `.items[0].name` as its keys and indexes; `.` is the whole body. Nothing fancier is a path here.
fn simple_path(text: &str) -> Option<Vec<Value>> {
    let mut rest = text.strip_prefix('.')?;
    let mut out = Vec::new();
    while !rest.is_empty() {
        if let Some(inside) = rest.strip_prefix('[') {
            let (digits, after) = inside.split_once(']')?;
            out.push(json!(digits.parse::<u64>().ok()?));
            rest = tail(after)?;
        } else {
            let end = rest.find(['.', '[']).unwrap_or(rest.len());
            let key = &rest[..end];
            if key.is_empty() || !key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
                return None;
            }
            out.push(json!(key));
            rest = tail(&rest[end..])?;
        }
    }
    Some(out)
}

fn parse_expect(text: &str) -> std::result::Result<Expect, String> {
    let text = text.trim();
    let bad = || format!("expectation '{text}' should look like '<path> <op> <value>' (ops: == != < <= > >= contains)");
    let ops = [("==", Cmp::Eq), ("!=", Cmp::Ne), ("<=", Cmp::Le), (">=", Cmp::Ge), ("<", Cmp::Lt), (">", Cmp::Gt)];
    let (at, symbol, cmp) = (0..text.len())
        .filter(|at| text.is_char_boundary(*at))
        .find_map(|at| {
            let rest = &text[at..];
            ops.iter()
                .find(|(symbol, _)| rest.starts_with(symbol))
                .map(|(symbol, cmp)| (at, symbol.len(), *cmp))
                .or_else(|| rest.starts_with(" contains ").then_some((at, " contains ".len(), Cmp::Contains)))
        })
        .ok_or_else(bad)?;
    let (lhs, rhs) = (text[..at].trim(), text[at + symbol..].trim());
    if lhs.is_empty() || rhs.is_empty() {
        return Err(bad());
    }
    let mut parts = lhs.split('|').map(str::trim);
    let subject = parts.next().unwrap_or_default();
    let pipe = match parts.next() {
        None => None,
        Some("length") => Some(Pipe::Length),
        Some("keys") => Some(Pipe::Keys),
        Some(_) => return Err(format!("expectation '{text}': only `| length` and `| keys` can follow a path")),
    };
    if parts.next().is_some() {
        return Err(format!("expectation '{text}': only one `| length` or `| keys` can follow a path"));
    }
    let path = match subject {
        "status" if pipe.is_none() => None,
        "status" => return Err(format!("expectation '{text}': the status takes no `|`")),
        _ => Some(simple_path(subject).ok_or_else(|| {
            format!("expectation '{text}': '{subject}' should be `status` or a path like .items[0].id")
        })?),
    };
    let wanted = serde_json::from_str(rhs).unwrap_or_else(|_| json!(rhs));
    if wanted.is_null() && !matches!(cmp, Cmp::Eq | Cmp::Ne) {
        return Err(format!("expectation '{text}': null is only compared with == or !="));
    }
    Ok(Expect { text: text.to_string(), path, pipe, cmp, wanted })
}

impl Expect {
    fn name(&self) -> String {
        format!("expect {}", self.text)
    }

    /// The check as a validate rule, when a rule can say it: no `| length`, no `| keys`.
    fn rule(&self) -> Option<Value> {
        if self.pipe.is_some() {
            return None;
        }
        let mut path = "input.status".to_string();
        if let Some(keys) = &self.path {
            path = "input.body".to_string();
            for key in keys {
                match key {
                    Value::Number(index) => path.push_str(&format!("[{index}]")),
                    other => path.push_str(&format!(".{}", other.as_str().unwrap_or_default())),
                }
            }
        }
        let (op, value) = match (self.cmp, &self.wanted) {
            (Cmp::Eq, Value::Null) => ("notExists", String::new()),
            (Cmp::Ne, Value::Null) => ("exists", String::new()),
            (cmp, wanted) => {
                let op = match cmp {
                    Cmp::Eq => "equals",
                    Cmp::Ne => "notEquals",
                    Cmp::Lt => "lessThan",
                    Cmp::Le => "lessOrEqual",
                    Cmp::Gt => "greaterThan",
                    Cmp::Ge => "greaterOrEqual",
                    Cmp::Contains => "contains",
                };
                (op, wanted.as_str().map_or_else(|| wanted.to_string(), str::to_string))
            }
        };
        Some(json!({ "path": path, "op": op, "value": value }))
    }

    /// The check as a JavaScript expression over `input` that throws, naming the expectation and what it got,
    /// when it does not hold, and hands `input` on when it does.
    fn expression(&self) -> String {
        let keys = json!(self.path.clone().unwrap_or_default());
        let got = match self.pipe {
            Some(Pipe::Length) => "v.length",
            Some(Pipe::Keys) => "Object.keys(v)",
            None => "v",
        };
        let holds = match self.cmp {
            Cmp::Eq => "same(got, want)",
            Cmp::Ne => "!same(got, want)",
            Cmp::Lt => "got < want",
            Cmp::Le => "got <= want",
            Cmp::Gt => "got > want",
            Cmp::Ge => "got >= want",
            Cmp::Contains => {
                "(Array.isArray(got) ? got.some(function (x) { return same(x, want); }) : \
                 (typeof got === 'string' && typeof want === 'string' && got.indexOf(want) >= 0))"
            }
        };
        format!(
            "(function () {{ var b = input.body; try {{ b = JSON.parse(b); }} catch (e) {{}} \
             var v = b, p = {keys}; for (var i = 0; v != null && i < p.length; i++) {{ v = v[p[i]]; }} \
             var got = v == null ? undefined : {got}, want = {want}; \
             var same = function (x, y) {{ return JSON.stringify(x) === JSON.stringify(y); }}; \
             if (!({holds})) throw new Error({text} + ', got ' + JSON.stringify(got)); return input; }})()",
            want = self.wanted,
            text = json!(self.name()),
        )
    }
}

/// An `inline` step's request.
struct Inline {
    headers: Map<String, Value>,
    body: String,
}

struct Step {
    /// The saved request's name, or the name the inline one is saved as.
    name: String,
    kind: &'static str,
    id: String,
    method: String,
    address: String,
    inline: Option<Inline>,
    vars: Vec<(String, String)>,
    extract: Vec<(String, String, Vec<Value>)>,
    expects: Vec<Expect>,
}

fn is_name(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The text of a scalar value of the file.
fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
        _ => None,
    }
}

/// A saved request and its kind.
type Row = (&'static str, Value);

/// The saved request `wanted` names: its id, an exact name, a name whatever the case, a unique name prefix.
fn resolve<'a>(rows: &'a [Row], wanted: &str, n: usize) -> std::result::Result<&'a Row, Problem> {
    let lower = wanted.to_lowercase();
    let matches = |at: usize, row: &Row| {
        let name = str_of(&row.1, "name");
        match at {
            0 => str_of(&row.1, "id") == wanted,
            1 => name == wanted,
            2 => name.to_lowercase() == lower,
            _ => name.to_lowercase().starts_with(&lower),
        }
    };
    for at in 0..4 {
        let hits: Vec<&Row> = rows.iter().filter(|row| matches(at, row)).collect();
        match hits.as_slice() {
            [] => continue,
            [one] => return Ok(one),
            many => {
                let names: Vec<String> = many.iter().map(|row| format!("\"{}\"", str_of(&row.1, "name"))).collect();
                let how = if at == 3 { "start with" } else { "are named" };
                return Err(usage(
                    n,
                    format!(
                        "step {n}: {} requests {how} \"{wanted}\": {} (use the id or more of the name)",
                        many.len(),
                        names.join(", ")
                    ),
                ));
            }
        }
    }
    Err(Problem { step: n, kind: "notFound", message: format!("step {n}: no saved request \"{wanted}\"") })
}

/// The method and address a saved request shows in a step line.
fn summary(kind: &str, entity: &Value) -> (String, String) {
    let address = ["url", "endpointUrl", "serverAddress"].iter().map(|key| str_of(entity, key)).find(|t| !t.is_empty());
    let method = match (kind, opt_str(entity, "method")) {
        (_, Some(method)) => method.to_string(),
        ("requests", None) => "GET".to_string(),
        ("grpcRequests", None) => String::new(),
        _ => "POST".to_string(),
    };
    (method, address.unwrap_or_default().to_string())
}

/// The path (and query) of `url`, without its scheme and host or a leading `{{variable}}`.
fn path_of(url: &str) -> String {
    let rest = match url.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or("/", |at| &rest[at..]),
        None => match url.strip_prefix("{{").and_then(|rest| rest.split_once("}}")) {
            Some((_, rest)) if !rest.is_empty() => rest,
            _ => url,
        },
    };
    if rest.is_empty() { "/".to_string() } else { rest.to_string() }
}

fn read_step(n: usize, item: &Value, flow_name: &str, rows: &[Row], problems: &mut Vec<Problem>) -> Option<Step> {
    let before = problems.len();
    let Some(map) = item.as_object() else {
        problems.push(usage(n, format!("step {n}: a step is a mapping like `request: <name>` or `inline: GET <url>`")));
        return None;
    };
    for key in map
        .keys()
        .filter(|key| !["request", "inline", "vars", "extract", "expect", "headers", "body"].contains(&key.as_str()))
    {
        problems.push(usage(n, format!("step {n}: unknown key `{key}`")));
    }
    if problems.len() > before {
        return None;
    }
    let mut step = Step {
        name: String::new(),
        kind: "requests",
        id: String::new(),
        method: String::new(),
        address: String::new(),
        inline: None,
        vars: Vec::new(),
        extract: Vec::new(),
        expects: Vec::new(),
    };
    match (map.get("request").and_then(Value::as_str), map.get("inline").and_then(Value::as_str)) {
        (Some(wanted), None) => {
            if map.contains_key("headers") || map.contains_key("body") {
                problems.push(usage(n, format!("step {n}: `headers` and `body` belong to an `inline` step")));
            }
            match resolve(rows, wanted.trim(), n) {
                Ok((kind, entity)) => {
                    step.name = str_of(entity, "name").to_string();
                    step.kind = kind;
                    step.id = str_of(entity, "id").to_string();
                    (step.method, step.address) = summary(kind, entity);
                }
                Err(problem) => problems.push(problem),
            }
        }
        (None, Some(line)) => {
            let (method, url) = line.trim().split_once(char::is_whitespace).unwrap_or((line.trim(), ""));
            let method = method.to_uppercase();
            if !METHODS.contains(&method.as_str()) || url.trim().is_empty() {
                problems.push(usage(n, format!("step {n}: `inline` should look like `POST {{{{baseUrl}}}}/orders`")));
            }
            let mut headers = Map::new();
            for (name, value) in map.get("headers").and_then(Value::as_object).into_iter().flatten() {
                match scalar(value) {
                    Some(text) => headers.insert(name.clone(), json!(text)),
                    None => {
                        problems.push(usage(n, format!("step {n}: header `{name}` should be text")));
                        None
                    }
                };
            }
            let body = match map.get("body") {
                None | Some(Value::Null) => String::new(),
                Some(Value::String(text)) => text.clone(),
                Some(other) => {
                    if !headers.keys().any(|name| name.eq_ignore_ascii_case("content-type")) {
                        headers.insert("Content-Type".into(), json!("application/json"));
                    }
                    other.to_string()
                }
            };
            step.name = format!("{flow_name} · {n}");
            step.method = method;
            step.address = url.trim().to_string();
            step.inline = Some(Inline { headers, body });
        }
        _ => {
            problems.push(usage(n, format!("step {n}: give exactly one of `request: <name>` or `inline: METHOD URL`")))
        }
    }
    for (name, value) in map.get("vars").and_then(Value::as_object).into_iter().flatten() {
        match (is_name(name), scalar(value)) {
            (true, Some(text)) => step.vars.push((name.clone(), text)),
            _ => problems
                .push(usage(n, format!("step {n}: `vars` takes `name: text` with letters, digits and _ in the name"))),
        }
    }
    for (name, value) in map.get("extract").and_then(Value::as_object).into_iter().flatten() {
        let path = value.as_str().and_then(|text| simple_path(text.trim()).map(|keys| (text.trim(), keys)));
        match (is_name(name), path) {
            (true, Some((text, keys))) => step.extract.push((name.clone(), text.to_string(), keys)),
            _ => problems.push(usage(n, format!("step {n}: `extract` takes `name: .path` like `cartId: .id`"))),
        }
    }
    let expects = match map.get("expect") {
        None => Vec::new(),
        Some(Value::Array(items)) => items.iter().map(|item| item.as_str().unwrap_or_default()).collect(),
        Some(Value::String(text)) => vec![text.as_str()],
        Some(_) => {
            problems.push(usage(n, format!("step {n}: `expect` is a list of texts like `status == 201`")));
            Vec::new()
        }
    };
    for text in expects {
        match parse_expect(text) {
            Ok(expect) => step.expects.push(expect),
            Err(message) => problems.push(usage(n, format!("step {n}: {message}"))),
        }
    }
    (problems.len() == before).then_some(step)
}

/// The one-line description of step `n`: `2 POST /carts  Create cart  extract cartId=.id`.
fn describe(n: usize, step: &Step) -> String {
    let target = [step.method.as_str(), path_of(&step.address).as_str()].join(" ");
    let mut line = format!("{n} {}  {}", target.trim(), step.name);
    let pairs = |pairs: Vec<String>| pairs.join(", ");
    if !step.vars.is_empty() {
        line += &format!("  vars {}", pairs(step.vars.iter().map(|(k, v)| format!("{k}={v}")).collect()));
    }
    if !step.extract.is_empty() {
        line +=
            &format!("  extract {}", pairs(step.extract.iter().map(|(k, text, _)| format!("{k}={text}")).collect()));
    }
    if !step.expects.is_empty() {
        line += &format!("  expect {}", pairs(step.expects.iter().map(|e| e.text.clone()).collect()));
    }
    line
}

/// The id of the request folder named after the flow, made when there is none.
async fn folder_of(ctx: &CommandContext, ws_id: &str, name: &str) -> Result<String> {
    let wanted = sanitize_dir_name(name);
    let config = load_config();
    let found = array_of(&config, "requestFolders").iter().find(|folder| {
        str_of(folder, "workspaceId") == ws_id
            && folder.get("parentId").is_none_or(Value::is_null)
            && str_of(folder, "name") == wanted
            && folder.get("deleted").and_then(Value::as_bool) != Some(true)
    });
    if let Some(folder) = found {
        return Ok(str_of(folder, "id").to_string());
    }
    let made = json!({ "kind": "request", "name": name, "parentId": null, "workspaceId": ws_id });
    Ok(str_of(&ctx.registry.invoke("folder.add", made, ctx.clone()).await?, "id").to_string())
}

/// Save the requests of the `inline` steps, reusing (and updating) the ones of the same name in the flow's
/// folder from an earlier compile.
async fn save_inline(ctx: &CommandContext, ws_id: &str, flow_name: &str, steps: &mut [Step]) -> Result<()> {
    if steps.iter().all(|step| step.inline.is_none()) {
        return Ok(());
    }
    let folder = folder_of(ctx, ws_id, flow_name).await?;
    let existing = read_all_entities(ws_id, to_engine_kind("requests"));
    for step in steps.iter_mut() {
        let Some(inline) = &step.inline else { continue };
        let fields = json!({
            "name": step.name, "method": step.method, "url": step.address, "headers": inline.headers,
            "body": inline.body,
        });
        let same = existing.iter().find(|row| str_of(row, "folderId") == folder && str_of(row, "name") == step.name);
        step.id = match same {
            Some(row) => {
                let mut entity = row.clone();
                entity
                    .as_object_mut()
                    .expect("an entity is an object")
                    .extend(fields.as_object().cloned().unwrap_or_default());
                let payload = json!({ "kind": "requests", "workspaceId": ws_id, "entity": entity });
                ctx.registry.invoke("entity.update", payload, ctx.clone()).await?;
                str_of(row, "id").to_string()
            }
            None => {
                let mut entity = fields;
                entity["folderId"] = json!(folder);
                let payload = json!({ "kind": "requests", "workspaceId": ws_id, "entity": entity });
                str_of(&ctx.registry.invoke("entity.create", payload, ctx.clone()).await?, "id").to_string()
            }
        };
    }
    Ok(())
}

/// The flow graph for `steps`, left to right.
fn build(name: &str, steps: &[Step], folder_id: Option<&str>) -> Value {
    let mut nodes = vec![json!({ "id": "start", "type": "start", "name": "", "x": 60.0, "y": 120.0 })];
    let mut edges: Vec<Value> = Vec::new();
    let mut edge = |from: &str, port: &str, to: &str, extra: Value| {
        let mut one = json!({ "id": format!("e{}", edges.len() + 1), "from": from, "port": port, "to": to });
        one.as_object_mut().expect("an object").extend(extra.as_object().cloned().unwrap_or_default());
        edges.push(one);
    };
    let x = |nodes: &Vec<Value>| 60.0 + NODE_STEP * nodes.len() as f64;
    let mut tail = ("start".to_string(), "out");
    // The latest node and path each extracted name comes from.
    let mut known: Vec<(String, String, Vec<Value>)> = Vec::new();
    for (at, step) in steps.iter().enumerate() {
        let id = format!("s{}", at + 1);
        let variables: Vec<Value> = step.vars.iter().map(|(k, v)| json!({ "name": k, "value": v })).collect();
        nodes.push(json!({
            "id": id, "type": "request", "name": step.name, "x": x(&nodes), "y": 120.0,
            "requestKind": step.kind, "requestId": step.id, "variables": variables,
        }));
        edge(&tail.0, tail.1, &id, json!({}));
        for (variable, from, keys) in &known {
            let path: Vec<Value> = std::iter::once(json!("body")).chain(keys.iter().cloned()).collect();
            edge(from, "success", &id, json!({ "input": format!("var:{variable}"), "path": path }));
        }
        tail = (id.clone(), "success");
        for (variable, _, keys) in &step.extract {
            known.retain(|(name, ..)| name != variable);
            known.push((variable.clone(), id.clone(), keys.clone()));
        }
        for (index, expect) in step.expects.iter().enumerate() {
            let check = format!("{id}_x{}", index + 1);
            let mut node = json!({ "id": check, "name": expect.name(), "x": x(&nodes), "y": 120.0 });
            let port = match expect.rule() {
                Some(rule) => {
                    node["type"] = json!("validate");
                    node["rules"] = json!([rule]);
                    "pass"
                }
                None => {
                    node["type"] = json!("evaluate");
                    node["expression"] = json!(expect.expression());
                    "out"
                }
            };
            nodes.push(node);
            edge(&tail.0, tail.1, &check, json!({}));
            tail = (check, port);
        }
    }
    // A validate node needs a node after it, whatever it passes to.
    if tail.1 == "pass" {
        nodes.push(json!({ "id": "done", "type": "display", "name": "Done", "x": x(&nodes), "y": 120.0 }));
        edge(&tail.0, tail.1, "done", json!({}));
    }
    let mut flow = json!({ "name": name, "inputs": [], "nodes": nodes, "edges": edges, "settings": {} });
    if let Some(folder) = folder_id {
        flow["folderId"] = json!(folder);
    }
    flow
}

async fn from_steps(ctx: &CommandContext, params: &Value) -> Result<Value> {
    let ws_id = str_of(params, "workspaceId");
    let refused = |problems: Vec<Problem>| {
        let problems: Vec<Value> =
            problems.iter().map(|p| json!({ "step": p.step, "kind": p.kind, "message": p.message })).collect();
        Ok(json!({ "flow": null, "steps": [], "problems": problems }))
    };
    let doc = match parse_any(str_of(params, "text")) {
        Ok(doc) => doc,
        Err(error) => return refused(vec![usage(0, format!("the step file cannot be read: {error}"))]),
    };
    let name = opt_str(params, "name")
        .or_else(|| opt_str(&doc, "name"))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    let items = if doc.is_array() { doc.as_array() } else { doc.get("steps").and_then(Value::as_array) };
    let mut problems = Vec::new();
    if name.is_none() {
        problems.push(usage(0, "name the flow: a `name:` in the file, or --name"));
    }
    let items = items.filter(|items| !items.is_empty());
    if items.is_none() {
        problems.push(usage(0, "the file has no `steps:` list"));
    }
    let (Some(name), Some(items)) = (name, items) else { return refused(problems) };
    let rows: Vec<Row> = if items.iter().any(|item| item.get("request").is_some()) {
        SENDABLE
            .iter()
            .flat_map(|kind| read_all_entities(ws_id, to_engine_kind(kind)).into_iter().map(move |row| (*kind, row)))
            .collect()
    } else {
        Vec::new()
    };
    let mut steps: Vec<Step> = Vec::new();
    for (at, item) in items.iter().enumerate() {
        steps.extend(read_step(at + 1, item, &name, &rows, &mut problems));
    }
    if !problems.is_empty() {
        return refused(problems);
    }
    save_inline(ctx, ws_id, &name, &mut steps).await?;
    let lines: Vec<String> = steps.iter().enumerate().map(|(at, step)| describe(at + 1, step)).collect();
    Ok(json!({ "flow": build(&name, &steps, opt_str(params, "folderId")), "steps": lines, "problems": [] }))
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register("flow.fromSteps", |params, ctx| async move { from_steps(&ctx, &params).await });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expectations_are_read_and_become_rules_or_expressions() {
        let rule = |text: &str| parse_expect(text).unwrap().rule();
        assert_eq!(rule("status == 201"), Some(json!({ "path": "input.status", "op": "equals", "value": "201" })));
        assert_eq!(
            rule(".items[0].name contains \"ab\""),
            Some(json!({ "path": "input.body.items[0].name", "op": "contains", "value": "ab" }))
        );
        assert_eq!(rule(".id != null").unwrap()["op"], "exists");
        assert_eq!(rule(".id==null").unwrap()["op"], "notExists");
        assert_eq!(rule("status >= 200").unwrap()["op"], "greaterOrEqual");
        let counted = parse_expect(".items | length == 1").unwrap();
        assert!(counted.rule().is_none() && counted.expression().contains("v.length"));
        for bad in [
            "status",
            "status ==",
            ".a | foo == 1",
            "status | length == 1",
            "items == 1",
            ".a < null",
            ".a..b == 1",
            ".a[x] == 1",
        ] {
            assert!(parse_expect(bad).is_err(), "{bad}");
        }
        assert_eq!(simple_path("."), Some(vec![]));
        assert_eq!(simple_path(".a-b[2].c"), Some(vec![json!("a-b"), json!(2), json!("c")]));
    }

    #[test]
    fn step_lines_show_method_path_name_and_what_the_step_does() {
        let steps = [Step {
            name: "Create cart".into(),
            kind: "requests",
            id: "r".into(),
            method: "POST".into(),
            address: "{{baseUrl}}/carts".into(),
            inline: None,
            vars: vec![("qty".into(), "2".into())],
            extract: vec![("cartId".into(), ".id".into(), vec![json!("id")])],
            expects: vec![parse_expect("status == 201").unwrap()],
        }];
        assert_eq!(
            describe(2, &steps[0]),
            "2 POST /carts  Create cart  vars qty=2  extract cartId=.id  expect status == 201"
        );
    }
}
