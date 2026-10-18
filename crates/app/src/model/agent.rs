//! What agents using the CLI or MCP server have started that the app's user should see: fault rules,
//! cassette recording and replay, and approval requests waiting on a person.

use serde_json::Value;

use crate::i18n::{t, t_with};

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn count(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq)]
pub struct FaultRule {
    pub id: String,
    /// What it hits and what it does to it, on one line.
    pub description: String,
}

impl FaultRule {
    /// The rules in a `fault.list` answer or `event.faults.changed` payload.
    pub fn list_from_value(value: &Value) -> Vec<Self> {
        let rules = value.get("faults").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        rules
            .iter()
            .map(|rule| {
                let mut effects = Vec::new();
                if let Some(ms) = rule.get("latencyMs").and_then(Value::as_u64) {
                    effects.push(t_with("model.agent.fault_latency", &[("ms", &ms)]));
                }
                if let Some(status) = rule.get("status").and_then(Value::as_u64) {
                    effects.push(status.to_string());
                }
                for (key, label) in [("drop", "model.agent.fault_drop"), ("reset", "model.agent.fault_reset")] {
                    if rule.get(key).and_then(Value::as_bool) == Some(true) {
                        effects.push(t(label).to_owned());
                    }
                }
                let rate = rule.get("rate").and_then(Value::as_f64).unwrap_or(100.0);
                Self {
                    id: text(rule, "id"),
                    description: t_with(
                        "model.agent.fault",
                        &[
                            ("target", &text(rule, "target")),
                            ("effects", &effects.join(t("model.agent.fault_separator"))),
                            ("rate", &rate),
                        ],
                    ),
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Recording {
    pub name: String,
    pub interactions: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Replay {
    pub name: String,
    pub unmatched: u64,
}

/// `cassette.recordStatus` and `cassette.replayStatus` answer `{recording}` and `{replaying}`; the event
/// carries both as `{record, replay}`.
pub fn recording_from_value(status: &Value) -> Option<Recording> {
    let recording = status.get("recording").filter(|r| r.is_object())?;
    Some(Recording { name: text(recording, "name"), interactions: count(recording, "interactions") })
}

pub fn replay_from_value(status: &Value) -> Option<Replay> {
    let replaying = status.get("replaying").filter(|r| r.is_object())?;
    Some(Replay { name: text(replaying, "name"), unmatched: count(replaying, "unmatchedCount") })
}

/// A call that a `require_human` policy holds until a person decides.
#[derive(Debug, Clone, PartialEq)]
pub struct Approval {
    pub id: String,
    /// Who asked, such as `session:agent-a`.
    pub by: String,
    pub method: String,
    pub url: String,
    pub policy: String,
    /// Milliseconds since the epoch.
    pub created_at: i64,
    pub expires_at: i64,
}

impl Approval {
    pub fn from_value(value: &Value) -> Self {
        let time = |key| value.get(key).and_then(Value::as_i64).unwrap_or_default();
        Self {
            id: text(value, "id"),
            by: text(value, "by"),
            method: text(value, "method"),
            url: text(value, "url"),
            policy: text(value, "policy"),
            created_at: time("createdAt"),
            expires_at: time("expiresAt"),
        }
    }

    /// The items in an `approvals.list` answer.
    pub fn list_from_value(value: &Value) -> Vec<Self> {
        let items = value.get("pending").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        items.iter().map(Self::from_value).collect()
    }

    /// How long it has waited, and how long until it lapses.
    pub fn timing(&self, now_ms: i64) -> String {
        t_with(
            "model.agent.approval_timing",
            &[("waited", &minutes(now_ms - self.created_at)), ("left", &minutes(self.expires_at - now_ms))],
        )
    }
}

fn minutes(ms: i64) -> String {
    let seconds = ms.max(0) / 1000;
    if seconds < 60 {
        t_with("model.agent.seconds", &[("count", &seconds)])
    } else {
        t_with("model.agent.minutes", &[("count", &(seconds / 60))])
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_fault_rule_reads_as_a_line() {
        let rules = FaultRule::list_from_value(&json!({"faults": [
            {"id": "f1", "target": "api.example.com/pay", "status": 503, "rate": 30},
            {"id": "f2", "target": "*.test", "latencyMs": 800, "drop": true, "rate": 100},
        ]}));
        assert_eq!(rules[0].description, "api.example.com/pay · 503 at 30%");
        assert_eq!(rules[1].description, "*.test · +800 ms, dropped at 100%");
    }

    #[test]
    fn idle_cassette_statuses_are_none() {
        assert_eq!(recording_from_value(&json!({"recording": null})), None);
        assert_eq!(replay_from_value(&json!({"ok": true})), None);
        let replay = replay_from_value(&json!({"replaying": {"name": "c", "unmatchedCount": 2}}));
        assert_eq!(replay, Some(Replay { name: "c".into(), unmatched: 2 }));
    }
}
