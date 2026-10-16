//! References to what a run holds: `{{input.body.items[0].id}}` is what the node before produced,
//! `{{nodes.login.headers.location}}` what an earlier node did, `{{flow.user}}` an input of the run.
//!
//! A reference is resolved once, against data: what it yields is never read for further references, so a
//! response cannot name a secret or a variable for the flow to fill in.

use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{Arc, LazyLock},
};

use regress::Regex;
use serde_json::Value;

/// What a node's references are resolved against.
pub struct Scope<'a> {
    pub input: &'a Value,
    pub nodes: &'a HashMap<String, Arc<Value>>,
    /// The run's inputs, an object of texts.
    pub flow: &'a Value,
}

enum Segment<'a> {
    Key(&'a str),
    Index(usize),
}

/// `body.items[0].id` as its steps. `None` when the path is not written the way a path is.
fn segments(path: &str) -> Option<Vec<Segment<'_>>> {
    let mut out = Vec::new();
    for part in path.split('.') {
        let (key, mut rest) = part.split_at(part.find('[').unwrap_or(part.len()));
        if key.is_empty() && rest.is_empty() {
            return None;
        }
        if !key.is_empty() {
            out.push(Segment::Key(key));
        }
        while !rest.is_empty() {
            let end = rest.find(']')?;
            out.push(Segment::Index(rest[1..end].parse().ok()?));
            rest = &rest[end + 1..];
            if !rest.is_empty() && !rest.starts_with('[') {
                return None;
            }
        }
    }
    Some(out)
}

/// One step into a value. Text that holds JSON (a response body, an event's data) is read as what it holds, and
/// a name that matches no key exactly matches one whatever its case, as a header's does.
fn step<'a>(value: Cow<'a, Value>, segment: &Segment) -> Option<Cow<'a, Value>> {
    if let Value::String(text) = value.as_ref() {
        let parsed: Value = serde_json::from_str(text).ok()?;
        return step(Cow::Owned(parsed), segment).map(|found| Cow::Owned(found.into_owned()));
    }
    let index_of = |items: &[Value]| {
        match segment {
            Segment::Index(index) => Some(*index),
            Segment::Key(key) => key.parse().ok(),
        }
        .filter(|index| *index < items.len())
    };
    let key_of = |map: &serde_json::Map<String, Value>| match segment {
        Segment::Key(key) if map.contains_key(*key) => Some(key.to_string()),
        Segment::Key(key) => map.keys().find(|name| name.eq_ignore_ascii_case(key)).cloned(),
        Segment::Index(_) => None,
    };
    match value {
        Cow::Borrowed(Value::Object(map)) => map.get(&key_of(map)?).map(Cow::Borrowed),
        Cow::Borrowed(Value::Array(items)) => items.get(index_of(items)?).map(Cow::Borrowed),
        Cow::Owned(Value::Object(mut map)) => map.remove(&key_of(&map)?).map(Cow::Owned),
        Cow::Owned(Value::Array(mut items)) => index_of(&items).map(|index| Cow::Owned(items.swap_remove(index))),
        _ => None,
    }
}

/// What `path` (`input.status`, `nodes.login.body.token`, `flow.user`) names, if the run holds it.
pub fn lookup(path: &str, scope: &Scope) -> Option<Value> {
    let mut steps = segments(path.trim())?.into_iter();
    let root = match steps.next()? {
        Segment::Key("input") => Cow::Borrowed(scope.input),
        Segment::Key("flow") => Cow::Borrowed(scope.flow),
        Segment::Key("nodes") => match steps.next()? {
            Segment::Key(id) => Cow::Borrowed(scope.nodes.get(id)?.as_ref()),
            Segment::Index(_) => return None,
        },
        _ => return None,
    };
    steps.try_fold(root, |value, segment| step(value, &segment)).map(Cow::into_owned)
}

/// The part of `value` a wire takes: `path` is the keys and indexes that lead to it, and an empty one is all
/// of it. Text that holds JSON is read on the way down, as [`lookup`] does.
pub fn pick(value: &Value, path: &[Value]) -> Option<Value> {
    let mut at = Cow::Borrowed(value);
    for part in path {
        let segment = match part {
            Value::String(key) => Segment::Key(key),
            Value::Number(index) => Segment::Index(index.as_u64()? as usize),
            _ => return None,
        };
        at = step(at, &segment)?;
    }
    Some(at.into_owned())
}

fn pattern() -> &'static Regex {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\{\{\s*((?:input|nodes|flow)(?:\.[\w-]+|\[\d+\])*)\s*\}\}").expect("a valid pattern")
    });
    &PATTERN
}

/// The paths `template` refers to.
pub fn references(template: &str) -> Vec<String> {
    pattern().find_iter(template).filter_map(|found| found.group(1)).map(|range| template[range].to_string()).collect()
}

/// A value as the text it is sent as: a string as it is, anything else as JSON.
pub fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// `template` with each reference replaced by what it names. A reference the run does not hold is an error
/// that names it: sending an empty value in its place would hide the mistake.
pub fn render(template: &str, scope: &Scope) -> Result<String, String> {
    let mut out = String::with_capacity(template.len());
    let mut last = 0;
    for found in pattern().find_iter(template) {
        let path = found.group(1).map(|range| &template[range]).unwrap_or_default();
        let value = lookup(path, scope).ok_or_else(|| format!("{{{{{path}}}}} has no value in this run."))?;
        out.push_str(&template[last..found.start()]);
        out.push_str(&text_of(&value));
        last = found.end();
    }
    out.push_str(&template[last..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn scope_with<'a>(input: &'a Value, nodes: &'a HashMap<String, Arc<Value>>, flow: &'a Value) -> Scope<'a> {
        Scope { input, nodes, flow }
    }

    #[test]
    fn paths_reach_into_objects_lists_and_text_that_holds_json() {
        let input = json!({
            "status": 200,
            "headers": { "Content-Type": "application/json" },
            "body": r#"{"items":[{"id":"a"},{"id":"b"}],"ok":true}"#,
        });
        let nodes = HashMap::from([("login".to_string(), Arc::new(json!({ "body": { "token": "t-1" } })))]);
        let flow = json!({ "user": "42" });
        let scope = scope_with(&input, &nodes, &flow);

        assert_eq!(lookup("input.status", &scope), Some(json!(200)));
        assert_eq!(lookup("input.body.items[1].id", &scope), Some(json!("b")));
        assert_eq!(lookup("input.body.items.0.id", &scope), Some(json!("a")));
        assert_eq!(lookup("input.headers.content-type", &scope), Some(json!("application/json")));
        assert_eq!(lookup("nodes.login.body.token", &scope), Some(json!("t-1")));
        assert_eq!(lookup("flow.user", &scope), Some(json!("42")));
        assert_eq!(lookup("input", &scope), Some(input.clone()));
        assert_eq!(pick(&input, &[json!("body"), json!("items"), json!(1), json!("id")]), Some(json!("b")));
        assert_eq!(pick(&input, &[]), Some(input.clone()));
        assert_eq!(pick(&input, &[json!("body"), json!("items"), json!(2)]), None);
        assert_eq!(pick(&json!({ "a.b": 1 }), &[json!("a.b")]), Some(json!(1)), "a key is taken whole");
        for missing in
            ["input.body.items[2]", "input.nope", "nodes.other.body", "env.host", "input..x", "input.body[x]"]
        {
            assert_eq!(lookup(missing, &scope), None, "{missing}");
        }
    }

    #[test]
    fn a_template_is_filled_once_and_a_missing_reference_is_named() {
        let input = json!({ "token": "{{vault.key}} {{flow.user}}", "n": 3, "list": [1, 2] });
        let nodes = HashMap::new();
        let flow = json!({ "user": "42" });
        let scope = scope_with(&input, &nodes, &flow);

        assert_eq!(render("Bearer {{ input.token }}", &scope).unwrap(), "Bearer {{vault.key}} {{flow.user}}");
        assert_eq!(render("{{input.n}}/{{input.list}}/{{host}}", &scope).unwrap(), "3/[1,2]/{{host}}");
        assert_eq!(render("plain", &scope).unwrap(), "plain");
        let error = render("{{nodes.login.body}}", &scope).unwrap_err();
        assert!(error.contains("{{nodes.login.body}}"), "{error}");
        assert_eq!(references("{{input.a}} {{host}} {{nodes.x.b[0]}}"), ["input.a", "nodes.x.b[0]"]);
    }
}
