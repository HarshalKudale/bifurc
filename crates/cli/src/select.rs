//! A small jq subset for agents: `--select`, `--expect`, `--extract` and `--patch` paths.
//!
//! Grammar: `.`, `.a.b`, `."a key"`, `.["a key"]`, `.a[0]`, `.a[-1]`, `.a[]`, then optional `| length`, `| keys`,
//! `| first`, `| last` or another path. A missing key or index is `null`; indexing a scalar is an error.

use std::cmp::Ordering;

use serde_json::{Map, Number, Value};

/// One step of a path.
#[derive(Debug, Clone, PartialEq)]
enum Step {
    Key(String),
    Index(i64),
    Iterate,
}

/// The functions a pipe may call.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Func {
    Length,
    Keys,
    First,
    Last,
}

/// One pipe stage: a path applied to every value, or a function.
#[derive(Debug, Clone, PartialEq)]
enum Stage {
    Path(Vec<Step>),
    Func(Func),
}

/// Characters that end an unquoted key.
fn ends_key(c: char) -> bool {
    c.is_whitespace() || ".[]|=!<>\"".contains(c)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn new(text: &str) -> Self {
        Parser { chars: text.chars().collect(), pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    /// The text from `start` to the cursor.
    fn since(&self, start: usize) -> String {
        self.chars[start..self.pos].iter().collect()
    }

    /// Fails unless only whitespace is left.
    fn finish(&mut self) -> Result<(), String> {
        self.skip_ws();
        match self.peek() {
            None => Ok(()),
            Some(c) => Err(format!("unexpected '{c}' at position {} in path", self.pos + 1)),
        }
    }

    /// A pipeline: a path, then `| function` or `| path` stages.
    fn pipeline(&mut self) -> Result<Vec<Stage>, String> {
        let mut stages = vec![Stage::Path(self.path()?)];
        loop {
            self.skip_ws();
            if self.peek() != Some('|') {
                return Ok(stages);
            }
            self.pos += 1;
            self.skip_ws();
            if matches!(self.peek(), Some('.' | '[')) {
                stages.push(Stage::Path(self.path()?));
                continue;
            }
            let start = self.pos;
            while self.peek().is_some_and(char::is_alphabetic) {
                self.pos += 1;
            }
            let name = self.since(start);
            stages.push(Stage::Func(match name.as_str() {
                "length" => Func::Length,
                "keys" => Func::Keys,
                "first" => Func::First,
                "last" => Func::Last,
                "" => return Err("expected a function or path after '|'".to_string()),
                _ => return Err(format!("unknown function '{name}' (use length, keys, first, last)")),
            }));
        }
    }

    /// Steps of one path; the leading `.` is optional so `items[0]` works too.
    fn path(&mut self) -> Result<Vec<Step>, String> {
        let mut steps = Vec::new();
        if self.peek() == Some('.') {
            self.pos += 1;
        }
        if let Some(key) = self.key()? {
            steps.push(Step::Key(key));
        }
        loop {
            match self.peek() {
                Some('.') => {
                    self.pos += 1;
                    match self.key()? {
                        Some(key) => steps.push(Step::Key(key)),
                        None if self.peek() == Some('[') => {}
                        None => return Err("expected a key after '.'".to_string()),
                    }
                }
                Some('[') => steps.push(self.bracket()?),
                _ => return Ok(steps),
            }
        }
    }

    /// A quoted or bare key at the cursor, if there is one.
    fn key(&mut self) -> Result<Option<String>, String> {
        if self.peek() == Some('"') {
            return self.quoted().map(Some);
        }
        let start = self.pos;
        while self.peek().is_some_and(|c| !ends_key(c)) {
            self.pos += 1;
        }
        Ok((self.pos > start).then(|| self.since(start)))
    }

    /// A JSON string literal at the cursor, escapes included.
    fn quoted(&mut self) -> Result<String, String> {
        let start = self.pos;
        self.pos += 1;
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                '\\' => self.pos += 1,
                '"' => {
                    let literal = self.since(start);
                    return serde_json::from_str(&literal).map_err(|_| format!("bad string {literal}"));
                }
                _ => {}
            }
        }
        Err("unterminated string in path".to_string())
    }

    /// `[]`, `[3]`, `[-1]` or `["key"]`.
    fn bracket(&mut self) -> Result<Step, String> {
        self.pos += 1;
        self.skip_ws();
        let step = match self.peek() {
            Some(']') => Step::Iterate,
            Some('"') => Step::Key(self.quoted()?),
            _ => {
                let start = self.pos;
                if self.peek() == Some('-') {
                    self.pos += 1;
                }
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.pos += 1;
                }
                let digits = self.since(start);
                Step::Index(digits.parse().map_err(|_| "expected an index, \"key\" or ] after '['".to_string())?)
            }
        };
        self.skip_ws();
        if self.peek() != Some(']') {
            return Err("expected ']'".to_string());
        }
        self.pos += 1;
        Ok(step)
    }
}

fn parse_pipeline(path: &str) -> Result<Vec<Stage>, String> {
    let mut parser = Parser::new(path);
    let stages = parser.pipeline()?;
    parser.finish()?;
    Ok(stages)
}

/// Kind name for error messages.
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

/// An array position from a possibly negative index; `None` when out of range.
fn position(index: i64, len: usize) -> Option<usize> {
    let at = if index < 0 { len as i64 + index } else { index };
    (0..len as i64).contains(&at).then_some(at as usize)
}

fn apply_step(value: &Value, step: &Step, out: &mut Vec<Value>) -> Result<(), String> {
    match (step, value) {
        (Step::Key(key), Value::Object(map)) => out.push(map.get(key).cloned().unwrap_or(Value::Null)),
        (Step::Index(i), Value::Array(items)) => {
            out.push(position(*i, items.len()).map_or(Value::Null, |at| items[at].clone()))
        }
        (Step::Key(_) | Step::Index(_), Value::Null) => out.push(Value::Null),
        (Step::Key(key), other) => return Err(format!("cannot index {} with \"{key}\"", kind(other))),
        (Step::Index(i), other) => return Err(format!("cannot index {} with {i}", kind(other))),
        (Step::Iterate, Value::Array(items)) => out.extend(items.iter().cloned()),
        (Step::Iterate, Value::Object(map)) => out.extend(map.values().cloned()),
        (Step::Iterate, other) => return Err(format!("cannot iterate over {}", kind(other))),
    }
    Ok(())
}

fn apply_func(value: &Value, func: Func) -> Result<Value, String> {
    let count = |n: usize| Value::from(n as u64);
    match (func, value) {
        (Func::Length, Value::Null) => Ok(count(0)),
        (Func::Length, Value::Array(items)) => Ok(count(items.len())),
        (Func::Length, Value::Object(map)) => Ok(count(map.len())),
        (Func::Length, Value::String(text)) => Ok(count(text.chars().count())),
        (Func::Length, other) => Err(format!("{} has no length", kind(other))),
        (Func::Keys, Value::Object(map)) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            Ok(Value::Array(keys.into_iter().map(|k| Value::String(k.clone())).collect()))
        }
        (Func::Keys, Value::Array(items)) => Ok(Value::Array((0..items.len()).map(count).collect())),
        (Func::Keys, other) => Err(format!("{} has no keys", kind(other))),
        (Func::First, Value::Array(items)) => Ok(items.first().cloned().unwrap_or(Value::Null)),
        (Func::Last, Value::Array(items)) => Ok(items.last().cloned().unwrap_or(Value::Null)),
        (Func::First | Func::Last, other) => Err(format!("cannot take first or last of {}", kind(other))),
    }
}

fn run(value: &Value, stages: &[Stage]) -> Result<Vec<Value>, String> {
    let mut current = vec![value.clone()];
    for stage in stages {
        let mut next = Vec::new();
        for item in &current {
            match stage {
                Stage::Path(steps) => {
                    let mut layer = vec![item.clone()];
                    for step in steps {
                        let mut out = Vec::new();
                        for v in &layer {
                            apply_step(v, step, &mut out)?;
                        }
                        layer = out;
                    }
                    next.extend(layer);
                }
                Stage::Func(func) => next.push(apply_func(item, *func)?),
            }
        }
        current = next;
    }
    Ok(current)
}

/// Every output of `path` applied to `value`: one value, or several when the path iterates with `[]`.
pub fn select(value: &Value, path: &str) -> Result<Vec<Value>, String> {
    run(value, &parse_pipeline(path)?)
}

/// What `--select` prints: one value per line, strings raw, everything else compact JSON.
pub fn render(values: &[Value]) -> String {
    values
        .iter()
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A comparison of an `--expect`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
}

impl Op {
    fn symbol(self) -> &'static str {
        match self {
            Op::Eq => "==",
            Op::Ne => "!=",
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::Contains => "contains",
        }
    }

    fn holds(self, found: &Value, wanted: &Value) -> bool {
        match self {
            Op::Eq => loose_eq(found, wanted),
            Op::Ne => !loose_eq(found, wanted),
            Op::Lt => order(found, wanted) == Some(Ordering::Less),
            Op::Le => matches!(order(found, wanted), Some(Ordering::Less | Ordering::Equal)),
            Op::Gt => order(found, wanted) == Some(Ordering::Greater),
            Op::Ge => matches!(order(found, wanted), Some(Ordering::Greater | Ordering::Equal)),
            Op::Contains => match (found, wanted) {
                (Value::String(text), Value::String(part)) => text.contains(part.as_str()),
                (Value::Array(items), _) => items.iter().any(|item| loose_eq(item, wanted)),
                _ => false,
            },
        }
    }
}

/// Equality where numbers compare by value (201 == 201.0), also inside arrays and objects.
fn loose_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| loose_eq(p, q)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, p)| y.get(k).is_some_and(|q| loose_eq(p, q)))
        }
        _ => a == b,
    }
}

fn order(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64()?.partial_cmp(&y.as_f64()?),
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

/// The left side of an expectation.
#[derive(Debug, Clone, PartialEq)]
enum Subject {
    Status,
    Path(Vec<Stage>),
}

/// A parsed `--expect '<lhs> <op> <rhs>'`.
#[derive(Debug, Clone, PartialEq)]
pub struct Expectation {
    lhs: String,
    subject: Subject,
    op: Op,
    rhs: String,
    wanted: Value,
}

/// A JSON literal, or else the text itself as a string.
fn literal(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
}

/// JSON of at most ~80 characters for an error line.
fn short(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() <= 80 {
        return text;
    }
    let cut: String = text.chars().take(79).collect();
    format!("{cut}…")
}

/// Parses `<path|status> <op> <value>`.
pub fn parse(text: &str) -> Result<Expectation, String> {
    let text = text.trim();
    let usage =
        || format!("expectation '{text}' should look like '<path> <op> <value>' (ops: == != < <= > >= contains)");
    let (subject, lhs, mut parser) = match text.strip_prefix("status") {
        Some(rest) if rest.starts_with(char::is_whitespace) => {
            (Subject::Status, "status".to_string(), Parser::new(rest))
        }
        _ => {
            let mut parser = Parser::new(text);
            let stages = parser.pipeline()?;
            let lhs = parser.since(0).trim_end().to_string();
            (Subject::Path(stages), lhs, parser)
        }
    };
    parser.skip_ws();
    let rest: String = parser.chars[parser.pos..].iter().collect();
    let (op, after) = [
        ("==", Op::Eq),
        ("!=", Op::Ne),
        ("<=", Op::Le),
        (">=", Op::Ge),
        ("<", Op::Lt),
        (">", Op::Gt),
        ("contains", Op::Contains),
    ]
    .into_iter()
    .find_map(|(symbol, op)| rest.strip_prefix(symbol).map(|after| (op, after)))
    .ok_or_else(usage)?;
    let rhs = after.trim();
    if rhs.is_empty() {
        return Err(usage());
    }
    Ok(Expectation { lhs, subject, op, rhs: rhs.to_string(), wanted: literal(rhs) })
}

impl Expectation {
    /// Passes when the comparison holds for every value the left side yields (and it yields some).
    pub fn check(&self, body: &Value, status: u16) -> Result<(), String> {
        let head = format!("expect {} {} {}", self.lhs, self.op.symbol(), self.rhs);
        let found = match &self.subject {
            Subject::Status => vec![Value::Number(Number::from(status))],
            Subject::Path(stages) => run(body, stages).map_err(|e| format!("{head}, {e}"))?,
        };
        if found.is_empty() {
            return Err(format!("{head}, got no value"));
        }
        match found.iter().find(|v| !self.op.holds(v, &self.wanted)) {
            Some(bad) => Err(format!("{head}, got {}", short(bad))),
            None => Ok(()),
        }
    }
}

/// Splits `name=<path>` (for `--extract`); `what` names the option in the error.
pub fn parse_pair(text: &str, what: &str) -> Result<(String, String), String> {
    match text.split_once('=') {
        Some((name, path)) if !name.trim().is_empty() && !path.trim().is_empty() => {
            Ok((name.trim().to_string(), path.trim().to_string()))
        }
        _ => Err(format!("{what} expects name=<path>, got '{text}'")),
    }
}

/// Applies `<path>=<value>` to `value`, creating missing object keys on the way.
pub fn patch(value: &mut Value, assignment: &str) -> Result<(), String> {
    let (steps, new) = parse_patch(assignment)?;
    assign(value, &steps, new)
}

/// `<path>=<value>` as the engine takes it: the path's keys (text) and positions (numbers), and the value.
pub fn patch_edit(assignment: &str) -> Result<(Vec<Value>, Value), String> {
    let (steps, new) = parse_patch(assignment)?;
    let path = steps
        .into_iter()
        .filter_map(|step| match step {
            Step::Key(key) => Some(Value::from(key)),
            Step::Index(i) => Some(Value::from(i)),
            Step::Iterate => None,
        })
        .collect();
    Ok((path, new))
}

fn parse_patch(assignment: &str) -> Result<(Vec<Step>, Value), String> {
    let mut parser = Parser::new(assignment);
    let steps = parser.path()?;
    parser.skip_ws();
    if parser.peek() == Some('|') {
        return Err("a patch path cannot use '|'".to_string());
    }
    if parser.peek() != Some('=') {
        return Err(format!("patch '{assignment}' should look like '<path>=<value>'"));
    }
    if steps.contains(&Step::Iterate) {
        return Err("a patch path cannot use '[]'".to_string());
    }
    parser.pos += 1;
    let rest: String = parser.chars[parser.pos..].iter().collect();
    Ok((steps, literal(rest.trim())))
}

fn assign(current: &mut Value, steps: &[Step], new: Value) -> Result<(), String> {
    let Some((step, rest)) = steps.split_first() else {
        *current = new;
        return Ok(());
    };
    let found = kind(current);
    match step {
        Step::Key(key) => {
            if current.is_null() {
                *current = Value::Object(Map::new());
            }
            let map = current.as_object_mut().ok_or_else(|| format!("cannot index {found} with \"{key}\""))?;
            assign(map.entry(key.clone()).or_insert(Value::Null), rest, new)
        }
        Step::Index(i) => {
            let items = current.as_array_mut().ok_or_else(|| format!("cannot index {found} with {i}"))?;
            let at = position(*i, items.len()).ok_or_else(|| format!("index {i} is out of range"))?;
            assign(&mut items[at], rest, new)
        }
        Step::Iterate => Err("a patch path cannot use '[]'".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sel(value: &Value, path: &str) -> Vec<Value> {
        select(value, path).unwrap()
    }

    fn doc() -> Value {
        json!({"items": [{"id": 1, "name": "a b"}, {"id": 2}, {"id": 3}], "user": {"name": "Ann", "key with space": 7}, "s": "héllo"})
    }

    #[test]
    fn dot_is_the_value_itself() {
        assert_eq!(sel(&doc(), "."), vec![doc()]);
    }

    #[test]
    fn keys_nest() {
        assert_eq!(sel(&doc(), ".user.name"), vec![json!("Ann")]);
    }

    #[test]
    fn bare_leading_key_works_without_dot() {
        assert_eq!(sel(&doc(), "user.name"), vec![json!("Ann")]);
    }

    #[test]
    fn quoted_keys_work_in_both_forms() {
        assert_eq!(sel(&doc(), r#".user."key with space""#), vec![json!(7)]);
        assert_eq!(sel(&doc(), r#".user["key with space"]"#), vec![json!(7)]);
        assert_eq!(sel(&doc(), r#"."user".name"#), vec![json!("Ann")]);
    }

    #[test]
    fn indexes_count_from_either_end() {
        assert_eq!(sel(&doc(), ".items[0].id"), vec![json!(1)]);
        assert_eq!(sel(&doc(), ".items[-1].id"), vec![json!(3)]);
        assert_eq!(sel(&json!([5, 6]), ".[0]"), vec![json!(5)]);
        assert_eq!(sel(&json!([5, 6]), "[1]"), vec![json!(6)]);
    }

    #[test]
    fn iteration_yields_many_values() {
        assert_eq!(sel(&doc(), ".items[].id"), vec![json!(1), json!(2), json!(3)]);
        assert_eq!(sel(&json!({"a": 1, "b": 2}), ".[]"), vec![json!(1), json!(2)]);
    }

    #[test]
    fn missing_key_or_index_is_null() {
        assert_eq!(sel(&doc(), ".nope"), vec![Value::Null]);
        assert_eq!(sel(&doc(), ".nope.deeper[2]"), vec![Value::Null]);
        assert_eq!(sel(&doc(), ".items[9]"), vec![Value::Null]);
        assert_eq!(sel(&doc(), ".items[-9]"), vec![Value::Null]);
    }

    #[test]
    fn indexing_a_scalar_is_an_error() {
        assert_eq!(select(&doc(), ".s.x").unwrap_err(), "cannot index string with \"x\"");
        assert_eq!(select(&doc(), ".s[0]").unwrap_err(), "cannot index string with 0");
        assert_eq!(select(&doc(), ".items.id").unwrap_err(), "cannot index array with \"id\"");
        assert_eq!(select(&doc(), ".s[]").unwrap_err(), "cannot iterate over string");
    }

    #[test]
    fn length_counts_each_kind() {
        assert_eq!(sel(&doc(), ".items | length"), vec![json!(3)]);
        assert_eq!(sel(&doc(), ".user | length"), vec![json!(2)]);
        assert_eq!(sel(&doc(), ".s | length"), vec![json!(5)]);
        assert_eq!(sel(&doc(), ".nope | length"), vec![json!(0)]);
        assert_eq!(select(&json!(3), ". | length").unwrap_err(), "number has no length");
    }

    #[test]
    fn keys_are_sorted_or_indices() {
        assert_eq!(sel(&json!({"b": 1, "a": 2}), ". | keys"), vec![json!(["a", "b"])]);
        assert_eq!(sel(&json!(["x", "y"]), ". | keys"), vec![json!([0, 1])]);
        assert_eq!(select(&json!(1), ". | keys").unwrap_err(), "number has no keys");
    }

    #[test]
    fn first_and_last_take_array_ends() {
        assert_eq!(sel(&doc(), ".items | first | .id"), vec![json!(1)]);
        assert_eq!(sel(&doc(), ".items | last | .id"), vec![json!(3)]);
        assert_eq!(sel(&json!([]), ". | first"), vec![Value::Null]);
        assert!(select(&json!({}), ". | last").is_err());
    }

    #[test]
    fn function_applies_to_each_iterated_value() {
        assert_eq!(sel(&json!([[1], [1, 2]]), ".[] | length"), vec![json!(1), json!(2)]);
    }

    #[test]
    fn bad_paths_give_short_errors() {
        assert_eq!(
            select(&doc(), ".items | sum").unwrap_err(),
            "unknown function 'sum' (use length, keys, first, last)"
        );
        assert_eq!(select(&doc(), ".items |").unwrap_err(), "expected a function or path after '|'");
        assert_eq!(select(&doc(), ".items[").unwrap_err(), "expected an index, \"key\" or ] after '['");
        assert_eq!(select(&doc(), ".items[0").unwrap_err(), "expected ']'");
        assert_eq!(select(&doc(), ".a..b").unwrap_err(), "expected a key after '.'");
        assert_eq!(select(&doc(), ".\"a").unwrap_err(), "unterminated string in path");
        assert!(select(&doc(), ".a b").unwrap_err().starts_with("unexpected 'b'"));
    }

    #[test]
    fn render_prints_strings_raw_and_the_rest_compact() {
        let values = [json!("a b"), json!(2), json!({"k": [1, null]}), json!(null)];
        assert_eq!(render(&values), "a b\n2\n{\"k\":[1,null]}\nnull");
        assert_eq!(render(&[]), "");
    }

    fn check(text: &str, body: &Value, status: u16) -> Result<(), String> {
        parse(text).unwrap().check(body, status)
    }

    #[test]
    fn status_expectation_compares_the_code() {
        assert!(check("status == 201", &json!({}), 201).is_ok());
        assert_eq!(check("status == 201", &json!({}), 422).unwrap_err(), "expect status == 201, got 422");
        assert!(check("status >= 200", &json!({}), 204).is_ok());
        assert!(check("status < 300", &json!({}), 404).is_err());
    }

    #[test]
    fn every_operator_passes_and_fails() {
        let body = json!({"n": 5, "s": "hello", "a": [1, "x"]});
        for (pass, fail) in [
            (".n == 5", ".n == 6"),
            (".n != 6", ".n != 5"),
            (".n < 6", ".n < 5"),
            (".n <= 5", ".n <= 4"),
            (".n > 4", ".n > 5"),
            (".n >= 5", ".n >= 6"),
            (".s contains ell", ".s contains xyz"),
            (".a contains 1", ".a contains 2"),
            (".a contains x", ".a contains y"),
            (".s < \"z\"", ".s > \"z\""),
        ] {
            assert!(check(pass, &body, 200).is_ok(), "{pass}");
            assert!(check(fail, &body, 200).is_err(), "{fail}");
        }
    }

    #[test]
    fn operators_containing_equals_do_not_confuse_the_split() {
        let body = json!({"n": 5});
        assert!(check(".n != 6", &body, 200).is_ok());
        assert!(check(".n<=5", &body, 200).is_ok());
        assert!(check(".n>=5", &body, 200).is_ok());
        assert!(check(".n==5", &body, 200).is_ok());
    }

    #[test]
    fn numbers_compare_by_value() {
        assert!(check(".n == 201.0", &json!({"n": 201}), 200).is_ok());
        assert!(check(".n == 201", &json!({"n": 201.0}), 200).is_ok());
        assert!(check(".a == [1, 2]", &json!({"a": [1.0, 2]}), 200).is_ok());
    }

    #[test]
    fn rhs_is_json_or_a_bare_string() {
        let body = json!({"a": null, "b": true, "c": "x y", "d": "201a"});
        assert!(check(".a == null", &body, 200).is_ok());
        assert!(check(".b == true", &body, 200).is_ok());
        assert!(check(".c == \"x y\"", &body, 200).is_ok());
        assert!(check(".c == x y", &body, 200).is_ok());
        assert!(check(".d == 201a", &body, 200).is_ok());
    }

    #[test]
    fn equals_inside_quotes_stays_in_its_side() {
        let body = json!({"a=b": "c=d"});
        assert!(check(r#".["a=b"] == "c=d""#, &body, 200).is_ok());
        assert!(check(r#".["a=b"] == c=d"#, &body, 200).is_ok());
    }

    #[test]
    fn pipe_expectation_checks_the_function_result() {
        let body = json!({"items": [1]});
        assert!(check(".items | length == 1", &body, 200).is_ok());
        assert_eq!(check(".items | length == 2", &body, 200).unwrap_err(), "expect .items | length == 2, got 1");
    }

    #[test]
    fn expectation_must_hold_for_every_value() {
        let body = json!({"items": [{"id": 1}, {"id": 2}, {"id": null}]});
        assert!(check(".items[].id != 9", &body, 200).is_ok());
        assert_eq!(check(".items[].id != null", &body, 200).unwrap_err(), "expect .items[].id != null, got null");
    }

    #[test]
    fn empty_result_fails() {
        let err = check(".items[].id == 1", &json!({"items": []}), 200).unwrap_err();
        assert_eq!(err, "expect .items[].id == 1, got no value");
    }

    #[test]
    fn failure_text_names_expected_and_found() {
        assert_eq!(check(".id != null", &json!({"id": null}), 200).unwrap_err(), "expect .id != null, got null");
        assert_eq!(
            check(".name == Bob", &json!({"name": "Ann"}), 200).unwrap_err(),
            "expect .name == Bob, got \"Ann\""
        );
    }

    #[test]
    fn long_found_values_are_truncated() {
        let body = json!({"t": "x".repeat(200)});
        let err = check(".t == y", &body, 200).unwrap_err();
        assert!(err.ends_with('…'));
        assert!(err.chars().count() < 120, "{err}");
    }

    #[test]
    fn ordering_across_kinds_fails() {
        assert!(check(".s < 3", &json!({"s": "a"}), 200).is_err());
        assert!(check(".n contains 1", &json!({"n": 1}), 200).is_err());
    }

    #[test]
    fn path_error_inside_check_is_reported_with_the_expectation() {
        let err = check(".s.x == 1", &json!({"s": "a"}), 200).unwrap_err();
        assert_eq!(err, "expect .s.x == 1, cannot index string with \"x\"");
    }

    #[test]
    fn malformed_expectations_are_refused() {
        assert!(parse("status").is_err());
        assert!(parse(".id").is_err());
        assert!(parse(".id ==").is_err());
        assert!(parse(".id = 3").is_err());
        assert!(parse(".id ~ 3").is_err());
        assert!(parse(".a | nope == 3").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn path_named_status_prefix_is_still_a_path() {
        assert!(check(".statusCode == 1", &json!({"statusCode": 1}), 500).is_ok());
        assert!(check("statusCode == 1", &json!({"statusCode": 1}), 500).is_ok());
    }

    #[test]
    fn pair_splits_on_the_first_equals() {
        assert_eq!(parse_pair("id=.a=b", "--extract").unwrap(), ("id".to_string(), ".a=b".to_string()));
        assert_eq!(parse_pair("no-equals", "--extract").unwrap_err(), "--extract expects name=<path>, got 'no-equals'");
        assert!(parse_pair("=.a", "--extract").is_err());
        assert!(parse_pair("a=", "--extract").is_err());
    }

    fn patched(mut value: Value, assignment: &str) -> Value {
        patch(&mut value, assignment).unwrap();
        value
    }

    #[test]
    fn patch_sets_existing_and_literal_values() {
        assert_eq!(patched(json!({"qty": 1}), ".qty=2"), json!({"qty": 2}));
        assert_eq!(patched(json!({"a": 1}), ".a=true"), json!({"a": true}));
        assert_eq!(patched(json!({"a": 1}), ".a=null"), json!({"a": null}));
        assert_eq!(patched(json!({}), ".a=[1,2]"), json!({"a": [1, 2]}));
    }

    #[test]
    fn patch_bare_text_is_a_string() {
        assert_eq!(patched(json!({}), ".user.name=Ann"), json!({"user": {"name": "Ann"}}));
        assert_eq!(patched(json!({}), ".t=hello world"), json!({"t": "hello world"}));
    }

    #[test]
    fn patch_creates_missing_keys_and_replaces_null() {
        assert_eq!(patched(json!({}), ".a.b.c=1"), json!({"a": {"b": {"c": 1}}}));
        assert_eq!(patched(json!({"a": null}), ".a.b=1"), json!({"a": {"b": 1}}));
    }

    #[test]
    fn patch_reaches_into_arrays() {
        assert_eq!(patched(json!({"items": [{}, {}]}), ".items[0].done=true"), json!({"items": [{"done": true}, {}]}));
        assert_eq!(patched(json!({"items": [1, 2]}), ".items[-1]=9"), json!({"items": [1, 9]}));
    }

    #[test]
    fn patch_equals_inside_quoted_key_or_value_is_kept() {
        assert_eq!(patched(json!({}), r#".["a=b"]=c=d"#), json!({"a=b": "c=d"}));
        assert_eq!(patched(json!({}), r#".k="x=y""#), json!({"k": "x=y"}));
    }

    #[test]
    fn patch_dot_replaces_the_whole_value() {
        assert_eq!(patched(json!({"a": 1}), ".=5"), json!(5));
    }

    #[test]
    fn patch_refuses_iteration_and_pipes() {
        let mut v = json!({"items": [1]});
        assert_eq!(patch(&mut v, ".items[]=1").unwrap_err(), "a patch path cannot use '[]'");
        assert_eq!(patch(&mut v, ".items | first=1").unwrap_err(), "a patch path cannot use '|'");
        assert_eq!(v, json!({"items": [1]}));
    }

    #[test]
    fn patch_errors_leave_clear_messages() {
        let mut v = json!({"s": "a", "items": [1]});
        assert_eq!(patch(&mut v, ".s.x=1").unwrap_err(), "cannot index string with \"x\"");
        assert_eq!(patch(&mut v, ".items[5]=1").unwrap_err(), "index 5 is out of range");
        assert_eq!(patch(&mut v, ".items.x=1").unwrap_err(), "cannot index array with \"x\"");
        assert_eq!(patch(&mut v, ".s").unwrap_err(), "patch '.s' should look like '<path>=<value>'");
    }
}
