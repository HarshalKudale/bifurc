//! Typed read models of the entities a proxy reads on every request: mocks, proxy rules, mappings and the
//! merged environment, composed once per config snapshot.
//!
//! These are views, not the stored form. The files keep their JSON (`serde_json::Value`, field order and
//! unknown fields included) and are still written from it; a view is made from a stored entity with
//! `from_value`, which reads each field exactly as the `utils::json` helpers always did (`flag` and
//! `truthy` for switches, `str_of` for text, a missing or oddly typed value falling back to the same default),
//! so every shape an older version wrote reads the same. A view is never written back, so nothing it does
//! not model can be lost.

use std::collections::HashSet;

use serde_json::Value;

use crate::utils::json::{array_of, is_truthy, opt_str, str_of, string_map, truthy};

/// Whether `value[key]` is on: absent or null means `default`, anything else is JavaScript truthiness.
fn flag(value: &Value, key: &str, default: bool) -> bool {
    match value.get(key) {
        None | Some(Value::Null) => default,
        Some(v) => is_truthy(v),
    }
}

fn text(value: &Value, key: &str) -> String {
    str_of(value, key).to_string()
}

/// A mock: what request it answers and the response it gives.
#[derive(Debug, Clone, PartialEq)]
pub struct Mock {
    pub id: String,
    /// Absent or null: off.
    pub enabled: bool,
    pub method: String,
    pub url_pattern: String,
    pub use_regex: bool,
    /// The whole response is the mock's (nothing is fetched from the real server).
    pub standalone: bool,
    /// 200 when absent, not a number, or outside `u16`.
    pub response_status: u16,
    /// Milliseconds; zero when absent or not positive.
    pub response_delay_ms: u64,
    pub response_body: String,
    pub base64: bool,
    /// In file order; values that are not text are skipped.
    pub response_headers: Vec<(String, String)>,
    /// Lowercased names of the headers the mock replaces on a partial mock.
    pub mocked_response_headers: HashSet<String>,
    pub response_status_mocked: bool,
    pub response_body_mocked: bool,
    pub response_delay_mocked: bool,
    pub streaming_mode: Option<String>,
    /// 100 when absent or not a number; zero when not positive.
    pub streaming_chunk_delay_ms: u64,
    pub streaming_chunk_separator: Option<String>,
}

impl Mock {
    pub fn from_value(v: &Value) -> Self {
        let number = |key: &str| v.get(key).and_then(Value::as_f64);
        Mock {
            id: text(v, "id"),
            enabled: flag(v, "enabled", false),
            method: text(v, "method"),
            url_pattern: text(v, "urlPattern"),
            use_regex: flag(v, "useRegex", false),
            standalone: flag(v, "standalone", false),
            response_status: v
                .get("responseStatus")
                .and_then(|n| n.as_u64().or_else(|| n.as_f64().map(|f| f as u64)))
                .and_then(|n| u16::try_from(n).ok())
                .unwrap_or(200),
            response_delay_ms: match number("responseDelay") {
                Some(ms) if ms > 0.0 => ms as u64,
                _ => 0,
            },
            response_body: text(v, "responseBody"),
            base64: opt_str(v, "responseBodyEncoding") == Some("base64"),
            response_headers: string_map(v, "responseHeaders"),
            mocked_response_headers: array_of(v, "mockedResponseHeaders")
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_lowercase)
                .collect(),
            response_status_mocked: flag(v, "responseStatusMocked", true),
            response_body_mocked: flag(v, "responseBodyMocked", true),
            response_delay_mocked: flag(v, "responseDelayMocked", true),
            streaming_mode: opt_str(v, "streamingMode").map(str::to_string),
            streaming_chunk_delay_ms: match number("streamingChunkDelay") {
                Some(ms) if ms > 0.0 => ms as u64,
                Some(_) => 0,
                None => 100,
            },
            streaming_chunk_separator: opt_str(v, "streamingChunkSeparator").map(str::to_string),
        }
    }
}

/// A proxy rule: the URLs it takes and where it sends them.
#[derive(Debug, Clone, PartialEq)]
pub struct ProxyRule {
    pub id: String,
    pub pattern: String,
    /// JavaScript truthiness: `1` and `"true"` count, `0`, `""` and `null` do not.
    pub use_regex: bool,
    /// `external` sends to `target_external`; anything else to the mapping `target_mapping_id`.
    pub target_type: String,
    pub target_external: String,
    pub target_mapping_id: String,
    pub request_script: String,
    pub response_script: String,
}

impl ProxyRule {
    pub fn from_value(v: &Value) -> Self {
        ProxyRule {
            id: text(v, "id"),
            pattern: text(v, "pattern"),
            use_regex: truthy(v, "useRegex"),
            target_type: text(v, "targetType"),
            target_external: text(v, "targetExternal"),
            target_mapping_id: text(v, "targetMappingId"),
            request_script: text(v, "requestScript"),
            response_script: text(v, "responseScript"),
        }
    }
}

/// A mapping: a domain served by the edge and the `host:port` behind it.
#[derive(Debug, Clone, PartialEq)]
pub struct Mapping {
    pub id: String,
    /// Lowercased, as the edge compares it with the request's host.
    pub domain: String,
    /// `None` when the mapping holds none, or one that is not text.
    pub target: Option<String>,
    /// JavaScript truthiness.
    pub enabled: bool,
}

impl Mapping {
    pub fn from_value(v: &Value) -> Self {
        Mapping {
            id: text(v, "id"),
            domain: str_of(v, "domain").to_lowercase(),
            target: opt_str(v, "target").map(str::to_string),
            enabled: truthy(v, "enabled"),
        }
    }
}

/// An environment as `{{variables}}` are resolved against it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Environment {
    /// Whose vault `{{vault.secret}}` reads; empty never resolves one.
    pub workspace_id: String,
    pub variables: Vec<EnvVariable>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnvVariable {
    pub key: String,
    /// `None` when the stored value is not text.
    pub value: Option<String>,
}

impl Environment {
    pub fn from_value(v: &Value) -> Self {
        Environment {
            workspace_id: text(v, "workspaceId"),
            variables: array_of(v, "variables")
                .iter()
                .map(|var| EnvVariable { key: text(var, "key"), value: opt_str(var, "value").map(str::to_string) })
                .collect(),
        }
    }

    /// The first variable named `key` decides, as `env::env_value` does.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.variables.iter().find(|v| v.key == key).and_then(|v| v.value.as_deref())
    }
}

/// What a proxy routes a request with, composed from a dispatch config.
#[derive(Debug, Clone, Default)]
pub struct Routing {
    pub workspace_id: String,
    pub mocks: Vec<Mock>,
    pub rules: Vec<ProxyRule>,
    pub mappings: Vec<Mapping>,
    pub env: Option<Environment>,
}

impl Routing {
    pub fn from_cfg(cfg: &Value) -> Self {
        Routing {
            workspace_id: text(cfg, "activeWorkspaceId"),
            mocks: array_of(cfg, "mocks").iter().map(Mock::from_value).collect(),
            rules: array_of(cfg, "proxyRules").iter().map(ProxyRule::from_value).collect(),
            mappings: array_of(cfg, "mappings").iter().map(Mapping::from_value).collect(),
            env: super::dispatch::active_env(cfg).as_ref().map(Environment::from_value),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_bare_mock_reads_as_the_helpers_always_did() {
        let m = Mock::from_value(&json!({ "id": "m" }));
        assert!(!m.enabled && !m.use_regex && !m.standalone && !m.base64);
        assert!(m.response_status_mocked && m.response_body_mocked && m.response_delay_mocked);
        assert_eq!((m.response_status, m.response_delay_ms, m.streaming_chunk_delay_ms), (200, 0, 100));
        assert!(m.response_headers.is_empty() && m.streaming_mode.is_none());
    }

    #[test]
    fn legacy_loose_mock_values_read_as_before() {
        let m = Mock::from_value(&json!({
            "enabled": "true", "useRegex": 1, "standalone": 0, "responseStatusMocked": null,
            "responseBodyMocked": "", "responseDelayMocked": [], "responseStatus": 404.0,
            "responseDelay": "50", "streamingChunkDelay": -1,
            "responseHeaders": { "a": "1", "b": 2 }, "mockedResponseHeaders": ["X-One", 3],
            "streamingMode": 4, "responseBodyEncoding": "base64",
        }));
        assert!(m.enabled && m.use_regex && !m.standalone && m.base64);
        assert!(m.response_status_mocked && !m.response_body_mocked && m.response_delay_mocked);
        assert_eq!((m.response_status, m.response_delay_ms, m.streaming_chunk_delay_ms), (404, 0, 0));
        assert_eq!(m.response_headers, [("a".to_string(), "1".to_string())]);
        assert!(m.mocked_response_headers.contains("x-one") && m.mocked_response_headers.len() == 1);
        assert_eq!(m.streaming_mode, None);
        assert_eq!(Mock::from_value(&json!({ "responseStatus": 70000 })).response_status, 200);
        assert_eq!(Mock::from_value(&json!({ "responseStatus": "x" })).response_status, 200);
    }

    #[test]
    fn legacy_rule_and_mapping_values_read_as_before() {
        let rule = ProxyRule::from_value(&json!({ "id": "r", "pattern": "p", "useRegex": "yes", "unknown": 1 }));
        assert!(rule.use_regex && rule.target_type.is_empty() && rule.request_script.is_empty());
        assert!(!ProxyRule::from_value(&json!({ "useRegex": 0 })).use_regex);
        assert!(!ProxyRule::from_value(&json!({ "useRegex": null })).use_regex);
        let mapping = Mapping::from_value(&json!({ "id": "m", "domain": "API.Localhost", "target": 5, "enabled": 1 }));
        assert_eq!((mapping.domain.as_str(), mapping.target, mapping.enabled), ("api.localhost", None, true));
        assert!(!Mapping::from_value(&json!({})).enabled);
    }

    #[test]
    fn routing_reads_an_old_dispatch_config_and_ignores_fields_it_does_not_know() {
        let cfg = json!({
            "activeWorkspaceId": "w", "activeEnvironmentId": "e", "future": { "a": 1 },
            "environments": [
                { "id": "__global__", "variables": [{ "key": "h", "value": "g" }] },
                { "id": "e", "variables": [{ "key": "h", "value": "e" }] },
            ],
            "mocks": [{ "id": "m", "enabled": true, "later": 1 }],
            "proxyRules": [{ "id": "r", "pattern": "p", "useRegex": true }],
            "mappings": [{ "id": "a", "domain": "A.localhost", "target": "x:1", "enabled": true }],
        });
        let routing = Routing::from_cfg(&cfg);
        assert_eq!(routing.workspace_id, "w");
        assert_eq!((routing.mocks.len(), routing.rules.len(), routing.mappings.len()), (1, 1, 1));
        assert_eq!(routing.env.unwrap().get("h"), Some("e"));
        let empty = Routing::from_cfg(&json!({}));
        assert!(empty.mocks.is_empty() && empty.env.is_none() && empty.workspace_id.is_empty());
    }

    #[test]
    fn the_first_variable_of_a_name_decides() {
        let env = Environment::from_value(&json!({ "workspaceId": "w", "variables": [
            { "key": "a", "value": 1 }, { "key": "a", "value": "late" }, { "key": "b", "value": "x" }, {},
        ] }));
        assert_eq!((env.get("a"), env.get("b"), env.get("c"), env.workspace_id.as_str()), (None, Some("x"), None, "w"));
    }
}
