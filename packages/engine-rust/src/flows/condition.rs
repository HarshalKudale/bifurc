//! What a condition node decides: the first of its branches whose rules hold.

use regress::Regex;
use serde_json::Value;

use super::{
    model::{Branch, Match, Op, Rule},
    reference::{Scope, lookup, render, text_of},
};

fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
        Value::Bool(_) | Value::Number(_) => false,
    }
}

/// Both sides as numbers when both are numbers, so that `10` is greater than `9`.
fn numbers(left: &str, right: &str) -> Option<(f64, f64)> {
    Some((left.trim().parse().ok()?, right.trim().parse().ok()?))
}

/// Whether one rule holds. A path the run does not hold makes every comparison false, and only `notExists`
/// and `isEmpty` true: a missing value is not an error here, it is what a condition is asked about.
pub fn holds(rule: &Rule, scope: &Scope) -> bool {
    let found = lookup(&rule.path, scope);
    match rule.op {
        Op::IsType => {
            let name = match &found {
                None => return false,
                Some(Value::Null) => "null",
                Some(Value::Bool(_)) => "boolean",
                Some(Value::Number(_)) => "number",
                Some(Value::String(_)) => "string",
                Some(Value::Array(_)) => "array",
                Some(Value::Object(_)) => "object",
            };
            return name == rule.value.trim();
        }
        Op::Exists => return found.is_some(),
        Op::NotExists => return found.is_none(),
        Op::IsEmpty => return found.as_ref().is_none_or(is_empty),
        Op::NotEmpty => return found.as_ref().is_some_and(|value| !is_empty(value)),
        _ => {}
    }
    let (Some(found), Ok(wanted)) = (found, render(&rule.value, scope)) else { return false };
    let actual = text_of(&found);
    let ordered = |test: fn(std::cmp::Ordering) -> bool| match numbers(&actual, &wanted) {
        Some((left, right)) => left.partial_cmp(&right).is_some_and(test),
        None => test(actual.as_str().cmp(wanted.as_str())),
    };
    match rule.op {
        Op::Equals => numbers(&actual, &wanted).map_or(actual == wanted, |(left, right)| left == right),
        Op::NotEquals => numbers(&actual, &wanted).map_or(actual != wanted, |(left, right)| left != right),
        Op::Contains => actual.contains(&wanted),
        Op::NotContains => !actual.contains(&wanted),
        Op::StartsWith => actual.starts_with(&wanted),
        Op::EndsWith => actual.ends_with(&wanted),
        Op::Matches => Regex::new(&wanted).is_ok_and(|pattern| pattern.find(&actual).is_some()),
        Op::GreaterThan => ordered(|order| order.is_gt()),
        Op::GreaterOrEqual => ordered(|order| order.is_ge()),
        Op::LessThan => ordered(|order| order.is_lt()),
        Op::LessOrEqual => ordered(|order| order.is_le()),
        Op::Exists | Op::NotExists | Op::IsEmpty | Op::NotEmpty | Op::IsType => unreachable!("answered above"),
    }
}

/// Whether a branch is taken. A branch without rules never is: an empty condition must not swallow the flow.
pub fn matches(branch: &Branch, scope: &Scope) -> bool {
    if branch.rules.is_empty() {
        return false;
    }
    match branch.matching {
        Match::All => branch.rules.iter().all(|rule| holds(rule, scope)),
        Match::Any => branch.rules.iter().any(|rule| holds(rule, scope)),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;

    fn decide(input: Value, matching: Match, rules: &[(&str, Op, &str)]) -> bool {
        let rules = rules
            .iter()
            .map(|(path, op, value)| Rule { path: path.to_string(), op: *op, value: value.to_string() })
            .collect();
        let branch = Branch { id: "b".into(), name: String::new(), matching, rules };
        let (nodes, flow) = (HashMap::new(), json!({ "limit": "5" }));
        matches(&branch, &Scope { input: &input, nodes: &nodes, flow: &flow })
    }

    #[test]
    fn rules_compare_text_and_numbers() {
        let input =
            json!({ "status": 200, "body": r#"{"name":"Ada","count":10,"tags":[]}"#, "headers": { "X-Id": "abc-1" } });
        let one = |path, op, value| decide(input.clone(), Match::All, &[(path, op, value)]);
        assert!(one("input.status", Op::Equals, "200"));
        assert!(one("input.status", Op::Equals, "200.0"));
        assert!(one("input.status", Op::NotEquals, "404"));
        assert!(one("input.body.name", Op::Contains, "d"));
        assert!(one("input.body.name", Op::NotContains, "z"));
        assert!(one("input.headers.x-id", Op::StartsWith, "abc"));
        assert!(one("input.headers.x-id", Op::EndsWith, "-1"));
        assert!(one("input.headers.x-id", Op::Matches, r"^[a-z]+-\d$"));
        assert!(!one("input.headers.x-id", Op::Matches, "("));
        assert!(one("input.body.count", Op::GreaterThan, "9"), "10 > 9 as numbers, not as text");
        assert!(one("input.body.count", Op::GreaterOrEqual, "10"));
        assert!(one("input.body.count", Op::LessThan, "{{flow.limit}}0"));
        assert!(one("input.body.count", Op::LessOrEqual, "10"));
        assert!(one("input.body.name", Op::GreaterThan, "Aba"));
        assert!(one("input.body.tags", Op::IsEmpty, ""));
        assert!(one("input.body.name", Op::NotEmpty, ""));
        assert!(one("input.body.name", Op::Exists, ""));
        assert!(one("input.body.tags", Op::IsType, "array") && one("input.status", Op::IsType, "number"));
        assert!(!one("input.body.name", Op::IsType, "number") && !one("input.gone", Op::IsType, "null"));
    }

    #[test]
    fn a_missing_value_only_satisfies_what_asks_for_one() {
        let one = |op, value| decide(json!({ "status": 200 }), Match::All, &[("input.body.id", op, value)]);
        assert!(one(Op::NotExists, ""));
        assert!(one(Op::IsEmpty, ""));
        for op in [Op::Exists, Op::NotEmpty, Op::Equals, Op::NotEquals, Op::NotContains, Op::LessThan] {
            assert!(!one(op, ""), "{op:?}");
        }
        assert!(!decide(json!({ "a": 1 }), Match::All, &[("input.a", Op::Equals, "{{input.gone}}")]));
    }

    #[test]
    fn all_needs_every_rule_any_needs_one_and_no_rules_never_match() {
        let rules = [("input.a", Op::Equals, "1"), ("input.b", Op::Equals, "nope")];
        assert!(!decide(json!({ "a": 1, "b": 2 }), Match::All, &rules));
        assert!(decide(json!({ "a": 1, "b": 2 }), Match::Any, &rules));
        assert!(!decide(json!({ "a": 1 }), Match::All, &[]));
        assert!(!decide(json!({ "a": 1 }), Match::Any, &[]));
    }
}
