//! What the Mocks screen learns beyond the mock itself: the scenario group a mock is in, how many captured
//! exchanges each mock answered, and the mock servers that are running.
//!
//! These follow the CLI's `mocks scenario`, `mocks status` and `mocks serve --status`, so the app and the
//! CLI agree on what a group, a hit and a server are.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::model::traffic::TrafficEntry;

const SEPARATOR: &str = " · ";
/// The name of a scenario that has no suffix.
pub const DEFAULT_SCENARIO: &str = "default";

fn text<'a>(mock: &'a Value, key: &str) -> &'a str {
    mock.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The part of a mock's name after the last ` · `, or `default`.
pub fn scenario_of(name: &str) -> &str {
    name.rsplit_once(SEPARATOR).map_or(DEFAULT_SCENARIO, |(_, scenario)| scenario)
}

/// What makes two mocks the same call: method, URL pattern, regex flag and captured request body.
fn call_key(mock: &Value) -> String {
    let regex = mock.get("useRegex").and_then(Value::as_bool).unwrap_or(false);
    format!(
        "{}|{}|{regex}|{}",
        text(mock, "method").to_uppercase(),
        text(mock, "urlPattern"),
        text(mock, "capturedBody")
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sibling {
    pub id: String,
    pub scenario: String,
}

/// The mocks of the call `id` makes, `id` included: the default first, then the scenarios by age. Empty when
/// `id` is the only one (or unknown), since there is nothing to switch to.
pub fn group_of(mocks: &[Value], id: &str) -> Vec<Sibling> {
    let Some(mine) = mocks.iter().find(|m| text(m, "id") == id) else { return Vec::new() };
    let key = call_key(mine);
    let mut group: Vec<&Value> = mocks.iter().filter(|m| call_key(m) == key).collect();
    if group.len() < 2 {
        return Vec::new();
    }
    group.sort_by_key(|m| {
        (scenario_of(text(m, "name")) != DEFAULT_SCENARIO, m.get("createdAt").and_then(Value::as_i64))
    });
    group
        .into_iter()
        .map(|m| Sibling { id: text(m, "id").to_owned(), scenario: scenario_of(text(m, "name")).to_owned() })
        .collect()
}

/// Captured exchanges answered by each mock: read from `capture.list` (filtered to `via: "mock"`) and then
/// extended by live entries.
#[derive(Debug, Default)]
pub struct Hits {
    counts: HashMap<String, u64>,
    /// Entries already counted, so one that is both in a list and delivered live counts once.
    seen: HashSet<String>,
}

impl Hits {
    pub fn replace(&mut self, entries: &[TrafficEntry]) {
        self.counts.clear();
        self.seen.clear();
        self.add(entries);
    }

    pub fn add(&mut self, entries: &[TrafficEntry]) {
        for entry in entries {
            let Some(mock) = entry.mock_id.as_ref().filter(|_| entry.via == "mock") else { continue };
            if self.seen.insert(entry.id.clone()) {
                *self.counts.entry(mock.clone()).or_default() += 1;
            }
        }
    }

    pub fn of(&self, id: &str) -> u64 {
        self.counts.get(id).copied().unwrap_or_default()
    }
}

/// A mock server the engine is running.
#[derive(Debug, Clone, PartialEq)]
pub struct Server {
    pub port: u64,
    pub fallthrough: Option<String>,
    pub hits: u64,
}

impl Server {
    /// The app only starts servers on the default host.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

/// The servers of `workspace` in a `mocks.serveStatus` answer.
pub fn servers_of(status: &Value, workspace: &str) -> Vec<Server> {
    status
        .get("servers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|s| s.get("workspaceId").and_then(Value::as_str) == Some(workspace))
        .filter_map(|s| {
            Some(Server {
                port: s.get("port").and_then(Value::as_u64)?,
                fallthrough: s.get("fallthrough").and_then(Value::as_str).map(str::to_owned),
                hits: s.get("hits").and_then(Value::as_u64).unwrap_or_default(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::model::traffic::parse_list;

    #[test]
    fn mocks_of_one_call_form_a_group_with_the_default_first() {
        let mocks = vec![
            json!({"id": "b", "name": "Users · error500", "method": "get", "urlPattern": "/u", "createdAt": 2}),
            json!({"id": "a", "name": "Users", "method": "GET", "urlPattern": "/u", "createdAt": 3}),
            json!({"id": "c", "name": "Users", "method": "GET", "urlPattern": "/u", "useRegex": true}),
            json!({"id": "d", "name": "Other", "method": "GET", "urlPattern": "/o"}),
        ];
        let ids = |id| group_of(&mocks, id).into_iter().map(|s| (s.id, s.scenario)).collect::<Vec<_>>();
        assert_eq!(ids("b"), [("a".into(), "default".into()), ("b".into(), "error500".into())]);
        assert!(group_of(&mocks, "c").is_empty(), "the regex flag tells calls apart");
        assert!(group_of(&mocks, "d").is_empty() && group_of(&mocks, "nope").is_empty());
    }

    #[test]
    fn hits_count_mock_answers_once_each() {
        let entry = |id: &str, via: &str, mock: &str| json!({"id": id, "ts": 1, "method": "GET", "url": "http://x/", "via": via, "mockId": mock});
        let listed = parse_list(
            &json!({"entries": [entry("1", "mock", "a"), entry("2", "mock", "a"), entry("3", "proxy", "a")]}),
        );
        let mut hits = Hits::default();
        hits.replace(&listed);
        hits.add(&parse_list(&json!({"entries": [entry("2", "mock", "a"), entry("4", "mock", "b")]})));
        assert_eq!((hits.of("a"), hits.of("b"), hits.of("c")), (2, 1, 0));
    }

    #[test]
    fn servers_are_those_of_the_workspace() {
        let status = json!({"servers": [
            {"port": 4010, "workspaceId": "w1", "fallthrough": "http://x", "detached": true, "hits": 3},
            {"port": 4011, "workspaceId": "w2", "fallthrough": null, "detached": true, "hits": 0},
        ]});
        assert_eq!(servers_of(&status, "w1"), [Server { port: 4010, fallthrough: Some("http://x".into()), hits: 3 }]);
    }
}
