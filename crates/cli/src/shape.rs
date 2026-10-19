//! Comparing two responses: status, content type, header names and the structure of a JSON body, with values on
//! (`capture diff`) or off (shape only, what `contract check` compares). Pure functions, no I/O.
//!
//! Paths print in the syntax `select.rs` parses: `.a.b[0]`, `.items[]` (shape mode: all elements merged),
//! `.["a b"]` for a key that needs quoting.

use serde_json::{Value, json};

use crate::response::{short_type, size};

/// The most lines [`capped`] keeps.
pub const CAP: usize = 50;

/// Headers that change on every call and say nothing about the API.
const VOLATILE: [&str; 13] = [
    "date",
    "content-length",
    "etag",
    "last-modified",
    "age",
    "expires",
    "set-cookie",
    "x-request-id",
    "server-timing",
    "connection",
    "keep-alive",
    "transfer-encoding",
    "vary",
];

/// One response as the comparison sees it.
#[derive(Clone)]
pub struct Side {
    pub status: u16,
    /// The short type of the body (json, html, text, binary, ...).
    pub content_type: String,
    /// Lower-case header names, sorted, without `content-type` and the volatile ones.
    pub headers: Vec<String>,
    /// The body when it is JSON.
    pub body: Option<Value>,
    pub raw: Vec<u8>,
}

impl Side {
    pub fn new(status: u16, content_type: &str, headers: &[(String, String)], raw: Vec<u8>) -> Self {
        let content_type = short_type(content_type, &raw);
        let body = if content_type == "json" { serde_json::from_slice(&raw).ok() } else { None };
        let mut names: Vec<String> = headers
            .iter()
            .map(|(name, _)| name.to_ascii_lowercase())
            .filter(|name| name != "content-type" && !VOLATILE.contains(&name.as_str()))
            .collect();
        names.sort();
        names.dedup();
        Self { status, content_type, headers: names, body, raw }
    }
}

/// One difference. `kind` is `+` (only on the right), `-` (only on the left) or `~` (changed).
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub kind: char,
    pub path: String,
    pub left: Option<String>,
    pub right: Option<String>,
    pub text: String,
}

impl Line {
    fn changed(path: &str, left: String, right: String) -> Self {
        Self {
            kind: '~',
            path: path.into(),
            text: format!("~ {path} {left} → {right}"),
            left: Some(left),
            right: Some(right),
        }
    }

    /// `+`/`-` of a path, with the type it has when it is a body key.
    fn only(kind: char, path: &str, ty: Option<String>) -> Self {
        let text = match &ty {
            Some(ty) => format!("{kind} {path} ({ty})"),
            None => format!("{kind} {path}"),
        };
        let (left, right) = if kind == '+' { (None, ty) } else { (ty, None) };
        Self { kind, path: path.into(), left, right, text }
    }

    pub fn to_json(&self) -> Value {
        let mut out = json!({ "kind": self.kind.to_string(), "path": self.path, "text": self.text });
        if let Some(left) = &self.left {
            out["left"] = json!(left);
        }
        if let Some(right) = &self.right {
            out["right"] = json!(right);
        }
        out
    }
}

/// The first [`CAP`] lines and how many were left out.
pub fn capped(mut lines: Vec<Line>) -> (Vec<Line>, usize) {
    let more = lines.len().saturating_sub(CAP);
    lines.truncate(CAP);
    (lines, more)
}

/// What `text` would be after the cap: the lines, then `… N more differences`.
pub fn render(lines: &[Line], more: usize) -> String {
    let mut out: Vec<String> = lines.iter().map(|line| line.text.clone()).collect();
    if more > 0 {
        out.push(format!("… {more} more differences"));
    }
    out.join("\n")
}

type Map = serde_json::Map<String, Value>;

fn objects<'a>(side: &[&'a Value]) -> Vec<&'a Map> {
    side.iter().filter_map(|v| v.as_object()).collect()
}

fn of<'a>(maps: &[&'a Map], key: &str) -> Vec<&'a Value> {
    maps.iter().filter_map(|m| m.get(key)).collect()
}

fn arrays<'a>(side: &[&'a Value]) -> Vec<&'a Vec<Value>> {
    side.iter().filter_map(|v| v.as_array()).collect()
}

fn all<'a>(side: &[&'a Vec<Value>]) -> Vec<&'a Value> {
    side.iter().flat_map(|a| a.iter()).collect()
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// The types of a set of values, `a|b` when they differ among themselves.
fn types(values: &[&Value]) -> String {
    let mut names: Vec<&str> = values.iter().map(|v| kind(v)).collect();
    names.sort_unstable();
    names.dedup();
    names.join("|")
}

fn key_path(path: &str, key: &str) -> String {
    let bare = !key.is_empty() && !key.chars().any(|c| c.is_whitespace() || ".[]|=!<>\"".contains(c));
    if bare { format!("{path}.{key}") } else { format!("{path}.[{}]", Value::String(key.into())) }
}

/// A scalar as JSON, a long string cut at 40 characters.
fn scalar(value: &Value) -> String {
    match value {
        Value::String(text) if text.chars().count() > 40 => {
            let cut = Value::String(text.chars().take(40).collect()).to_string();
            format!("{}…\"", &cut[..cut.len() - 1])
        }
        other => other.to_string(),
    }
}

/// Compare the values found at one path on each side. Shape mode holds every instance (all elements of an array
/// merged); values mode holds exactly one on each side.
fn walk(left: &[&Value], right: &[&Value], path: &str, values: bool, out: &mut Vec<Line>) {
    // An empty array matches any element shape.
    if left.is_empty() || right.is_empty() {
        return;
    }
    let shown = if path.is_empty() { "." } else { path };
    let (tl, tr) = (types(left), types(right));
    if tl != tr {
        return out.push(Line::changed(shown, tl, tr));
    }
    let (lo, ro) = (objects(left), objects(right));
    if !lo.is_empty() {
        let mut keys: Vec<&String> = Vec::new();
        for key in lo.iter().flat_map(|m| m.keys()) {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        for key in &keys {
            let (l, r) = (of(&lo, key), of(&ro, key));
            let child = key_path(path, key);
            if r.is_empty() {
                out.push(Line::only('-', &child, Some(types(&l))));
            } else {
                walk(&l, &r, &child, values, out);
            }
        }
        for key in ro.iter().flat_map(|m| m.keys()).collect::<Vec<_>>() {
            if !keys.contains(&key) {
                keys.push(key);
                out.push(Line::only('+', &key_path(path, key), Some(types(&of(&ro, key)))));
            }
        }
    }
    let (la, ra) = (arrays(left), arrays(right));
    if !la.is_empty() {
        if values {
            let (l, r) = (la[0], ra[0]);
            if l.len() != r.len() {
                let (from, to) = (l.len().to_string(), r.len().to_string());
                out.push(Line {
                    kind: '~',
                    path: shown.into(),
                    text: format!("~ {shown} length {from} → {to}"),
                    left: Some(from),
                    right: Some(to),
                });
            }
            for (i, (a, b)) in l.iter().zip(r).enumerate() {
                walk(&[a], &[b], &format!("{path}[{i}]"), values, out);
            }
        } else {
            walk(&all(&la), &all(&ra), &format!("{path}[]"), values, out);
        }
    }
    if values && lo.is_empty() && la.is_empty() && left[0] != right[0] {
        out.push(Line::changed(shown, scalar(left[0]), scalar(right[0])));
    }
}

/// Every difference between two responses, in order: status, content type, headers, then the body.
pub fn diff(left: &Side, right: &Side, values: bool) -> Vec<Line> {
    let mut out = Vec::new();
    if left.status != right.status {
        out.push(Line::changed("status", left.status.to_string(), right.status.to_string()));
    }
    if left.content_type != right.content_type {
        let show = |t: &str| if t.is_empty() { "none".to_string() } else { t.to_string() };
        out.push(Line::changed("content-type", show(&left.content_type), show(&right.content_type)));
    }
    for name in left.headers.iter().filter(|h| !right.headers.contains(h)) {
        out.push(Line::only('-', &format!("header {name}"), None));
    }
    for name in right.headers.iter().filter(|h| !left.headers.contains(h)) {
        out.push(Line::only('+', &format!("header {name}"), None));
    }
    match (&left.body, &right.body) {
        (Some(l), Some(r)) => walk(&[l], &[r], "", values, &mut out),
        (None, None) if values && left.raw != right.raw => out.push(Line {
            kind: '~',
            path: "body".into(),
            left: Some(size(left.raw.len())),
            right: Some(size(right.raw.len())),
            text: format!("~ body (text differs, {} → {})", size(left.raw.len()), size(right.raw.len())),
        }),
        _ => {}
    }
    out
}

/// The two merged into one when they can be one shape: objects take the union of their keys, equal scalars stay,
/// arrays pool their elements. `None` when a key has two different types (`null` and a number, say).
fn join(a: &Value, b: &Value) -> Option<Value> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let mut out = x.clone();
            for (key, value) in y {
                let joined = match out.get(key) {
                    Some(have) => join(have, value)?,
                    None => value.clone(),
                };
                out.insert(key.clone(), joined);
            }
            Some(Value::Object(out))
        }
        (Value::Array(x), Value::Array(y)) => Some(Value::Array(pooled(x.iter().chain(y).cloned()))),
        (x, y) if kind(x) == kind(y) && !x.is_object() && !x.is_array() => Some(x.clone()),
        _ => None,
    }
}

fn pooled(items: impl Iterator<Item = Value>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for item in items {
        match out.iter_mut().find_map(|have| join(have, &item).map(|joined| (have, joined))) {
            Some((have, joined)) => *have = joined,
            None => out.push(item),
        }
    }
    out
}

/// The shape of a JSON value with every real value gone: strings `""`, numbers `0`, booleans `false`, `null`
/// kept, objects keep their keys, an array holds one merged element (more only when elements of the same key
/// have different types). Compared in shape mode it equals the value it came from.
pub fn skeleton(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::Bool(_) => json!(false),
        Value::Number(_) => json!(0),
        Value::String(_) => json!(""),
        Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), skeleton(v))).collect()),
        Value::Array(items) => Value::Array(pooled(items.iter().map(skeleton))),
    }
}

/// `mock` rewritten to the shape of `real`: a key both have with the same type keeps the mock's value (objects
/// recurse; arrays merge each mock element against the first real one, and a mock `[]` becomes the real array);
/// every other key takes the real value, and keys only the mock has are dropped.
pub fn merge(mock: &Value, real: &Value) -> Value {
    match (mock, real) {
        (Value::Object(m), Value::Object(r)) => Value::Object(
            r.iter()
                .map(|(key, value)| (key.clone(), m.get(key).map_or_else(|| value.clone(), |have| merge(have, value))))
                .collect(),
        ),
        (Value::Array(m), Value::Array(r)) if m.is_empty() => Value::Array(r.clone()),
        (Value::Array(m), Value::Array(r)) => match r.first() {
            Some(first) => Value::Array(m.iter().map(|item| merge(item, first)).collect()),
            None => mock.clone(),
        },
        (have, _) if kind(have) == kind(real) => have.clone(),
        _ => real.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(status: u16, body: Value) -> Side {
        Side::new(status, "application/json", &[], body.to_string().into_bytes())
    }

    fn texts(left: Value, right: Value, values: bool) -> Vec<String> {
        diff(&side(200, left), &side(200, right), values).into_iter().map(|l| l.text).collect()
    }

    #[test]
    fn added_removed_and_retyped_keys() {
        let lines =
            texts(json!({"total": "5", "a": 1, "same": true}), json!({"a": "x", "same": true, "discount": 2}), false);
        assert_eq!(lines, ["- .total (string)", "~ .a number → string", "+ .discount (number)"]);
    }

    #[test]
    fn values_only_show_with_values_on() {
        let (l, r) = (json!({"status": "paid", "n": 1}), json!({"status": "pending", "n": 1}));
        assert_eq!(texts(l.clone(), r.clone(), true), [r#"~ .status "paid" → "pending""#]);
        assert!(texts(l, r, false).is_empty());
    }

    #[test]
    fn long_strings_are_cut() {
        let long = "x".repeat(60);
        let lines = texts(json!({"a": long}), json!({"a": "y"}), true);
        assert_eq!(lines[0], format!("~ .a \"{}…\" → \"y\"", "x".repeat(40)));
    }

    #[test]
    fn nested_paths_and_odd_keys() {
        let lines = texts(
            json!({"a": {"b": {"c": 1}}, "a-b": 1, "x y": 1}),
            json!({"a": {"b": {"c": "s"}}, "a-b": 1, "x y": null}),
            false,
        );
        assert_eq!(lines, ["~ .a.b.c number → string", r#"~ .["x y"] number → null"#]);
    }

    #[test]
    fn null_against_a_type_is_a_change() {
        assert_eq!(texts(json!({"a": null}), json!({"a": "s"}), false), ["~ .a null → string"]);
    }

    #[test]
    fn shape_mode_merges_array_elements() {
        let l = json!({"items": [{"id": 1}, {"id": 2, "tag": "a"}]});
        let r = json!({"items": [{"id": "1", "tag": "b"}]});
        assert_eq!(texts(l, r, false), ["~ .items[].id number → string"]);
        // Keys present in some elements only are a union, and an empty array matches any shape.
        assert!(
            texts(json!({"items": [{"id": 1}, {"x": 1}]}), json!({"items": [{"id": 2, "x": 3}]}), false).is_empty()
        );
        assert!(texts(json!({"items": []}), json!({"items": [{"id": 1}]}), false).is_empty());
        assert_eq!(texts(json!([{"a": 1}]), json!([{"b": 1}]), false), ["- [].a (number)", "+ [].b (number)"]);
    }

    #[test]
    fn values_mode_compares_by_index_and_length() {
        let l = json!({"items": [{"id": 1}, {"id": 2}, {"id": 3}]});
        let r = json!({"items": [{"id": 1}, {"id": 5}]});
        assert_eq!(texts(l, r, true), ["~ .items length 3 → 2", "~ .items[1].id 2 → 5"]);
    }

    #[test]
    fn status_content_type_and_header_names() {
        let h =
            |names: &[&str]| -> Vec<(String, String)> { names.iter().map(|n| (n.to_string(), "v".into())).collect() };
        let a = Side::new(200, "application/json", &h(&["X-Bar", "Date", "ETag", "Content-Length"]), b"{}".to_vec());
        let b = Side::new(201, "text/html", &h(&["x-foo", "date", "Set-Cookie", "x-bar"]), b"<p>".to_vec());
        let lines: Vec<String> = diff(&a, &b, false).into_iter().map(|l| l.text).collect();
        assert_eq!(lines, ["~ status 200 → 201", "~ content-type json → html", "+ header x-foo"]);
        let c = Side::new(200, "application/json", &h(&["X-Gone"]), b"{}".to_vec());
        assert_eq!(
            diff(&a, &c, false).into_iter().map(|l| l.text).collect::<Vec<_>>(),
            ["- header x-bar", "+ header x-gone"]
        );
    }

    #[test]
    fn text_bodies_differ_only_with_values_on() {
        let a = Side::new(200, "text/plain", &[], vec![b'a'; 1200]);
        let b = Side::new(200, "text/plain", &[], vec![b'b'; 1400]);
        assert!(diff(&a, &b, false).is_empty());
        assert_eq!(diff(&a, &b, true)[0].text, "~ body (text differs, 1.2KB → 1.4KB)");
        assert!(diff(&a, &a, true).is_empty());
    }

    #[test]
    fn order_is_status_then_headers_then_body_with_right_only_keys_last() {
        let a = Side::new(200, "application/json", &[], br#"{"b":1,"a":1}"#.to_vec());
        let b = Side::new(500, "application/json", &[("x-new".into(), "1".into())], br#"{"z":1,"a":"s"}"#.to_vec());
        let lines: Vec<String> = diff(&a, &b, false).into_iter().map(|l| l.text).collect();
        assert_eq!(
            lines,
            ["~ status 200 → 500", "+ header x-new", "- .b (number)", "~ .a number → string", "+ .z (number)"]
        );
    }

    #[test]
    fn the_cap_keeps_fifty_lines() {
        let many: serde_json::Map<String, Value> = (0..62).map(|i| (format!("k{i:02}"), json!(1))).collect();
        let lines = diff(&side(200, json!({})), &side(200, Value::Object(many)), false);
        let (kept, more) = capped(lines);
        assert_eq!((kept.len(), more), (50, 12));
        assert!(render(&kept, more).ends_with("… 12 more differences"));
    }

    #[test]
    fn json_form_of_a_line() {
        let line = &diff(&side(200, json!({"a": 1})), &side(200, json!({"a": "x"})), false)[0];
        assert_eq!(
            line.to_json(),
            json!({"kind": "~", "path": ".a", "left": "number", "right": "string", "text": "~ .a number → string"})
        );
    }

    #[test]
    fn a_skeleton_keeps_types_and_no_values() {
        let real = json!({"id": 7, "name": "secret", "ok": true, "n": null, "tags": ["a", "b"], "items": [{"id": 1}, {"x": "t", "id": 2}], "none": []});
        assert_eq!(
            skeleton(&real),
            json!({"id": 0, "name": "", "ok": false, "n": null, "tags": [""], "items": [{"id": 0, "x": ""}], "none": []})
        );
        assert!(!skeleton(&real).to_string().contains("secret"));
    }

    #[test]
    fn a_skeleton_compares_like_the_real_body() {
        for real in [
            json!({"items": [{"id": 1}, {"x": "t"}, {"a": null}, {"a": 3}], "m": [[1], ["s"]], "z": []}),
            json!([{"a": [1, "x"]}, {"a": []}]),
            json!({"deep": {"list": [{"k": [{"v": null}, {"v": true}]}]}}),
            json!("text"),
        ] {
            let (a, b) = (side(200, real.clone()), side(200, skeleton(&real)));
            assert!(diff(&a, &b, false).is_empty(), "{real}");
            assert!(diff(&b, &a, false).is_empty(), "{real}");
        }
    }

    #[test]
    fn merging_keeps_mock_values_where_the_shape_stays() {
        let mock = json!({"id": 5, "name": "Ann", "gone": 1, "n": 1, "o": {"a": "keep"}, "items": [{"id": 9, "old": 1}], "empty": []});
        let real = json!({"id": 1, "name": "Bob", "n": "str", "o": {"a": "x", "b": true}, "items": [{"id": 2, "new": "v"}], "empty": [{"q": 1}], "added": null});
        assert_eq!(
            merge(&mock, &real),
            json!({"id": 5, "name": "Ann", "n": "str", "o": {"a": "keep", "b": true}, "items": [{"id": 9, "new": "v"}], "empty": [{"q": 1}], "added": null})
        );
        assert_eq!(merge(&json!([1, 2]), &json!(["a"])), json!(["a", "a"]));
        assert_eq!(merge(&json!({"a": 1}), &json!([1])), json!([1]));
        assert_eq!(merge(&json!({"a": [1]}), &json!({"a": []})), json!({"a": [1]}));
    }
}
