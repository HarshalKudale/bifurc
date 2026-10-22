//! The entity kinds the four `entity.*` commands serve, and what each one needs: where it sits in a config
//! snapshot, whether it has folders or an on/off state, how it is validated and how it is listed.

use regress::Regex;
use serde_json::{Map, Value, json};

use crate::{
    protocol::{EngineError, ErrorCode, Result},
    utils::json::{array_mut, bool_of, is_truthy, str_of},
};

/// Rejects an entity before it is written; the second argument is the kind's other entities in the workspace.
pub type Validator = fn(entity: &Value, siblings: &[Value]) -> Result<()>;

pub struct CrudKind {
    /// The store's directory name (`rules`, not `proxyRules`).
    pub kind: &'static str,
    /// The entity list's key in a config snapshot.
    pub config_key: &'static str,
    /// The folder list's key in a config snapshot; `None` for a kind without folders. Every request type is
    /// in the one request tree, so they all name `requestFolders`.
    pub folder_config_key: Option<&'static str>,
    /// One directory, no folders, no names index.
    pub is_flat: bool,
    /// Whether entities can be switched on and off (`enabled.json`).
    pub has_enabled_state: bool,
    /// Reject an entity before it is written. `siblings` are the kind's other entities in the workspace.
    pub validate: Option<Validator>,
    /// Switch off, in the snapshot, the entities a newly enabled one would shadow.
    pub on_add_conflict: Option<fn(cfg: &mut Value, target: &Value)>,
    /// The entity's names-index entry.
    pub name_entry: Option<fn(entity: &Value) -> Value>,
    pub gate_kind: Option<&'static str>,
}

/// Kinds stored as plain files with no per-kind behavior.
pub const SIMPLE_KINDS: [&str; 3] = ["graphqlSchemas", "wsdls", "protoFiles"];

fn required(entity: &Value, field: &str, message: &str) -> Result<()> {
    if str_of(entity, field).trim().is_empty() {
        Err(EngineError::engine(message).keyed("validation.required", json!({ "field": field })))
    } else {
        Ok(())
    }
}

fn field(entity: &Value, key: &str) -> Value {
    entity.get(key).cloned().unwrap_or(Value::Null)
}

fn entry(pairs: &[(&str, Value)]) -> Value {
    let mut out = Map::new();
    for (key, value) in pairs {
        // A names entry holds what `JSON.stringify` would keep: absent fields are left out.
        if !value.is_null() {
            out.insert((*key).to_string(), value.clone());
        }
    }
    Value::Object(out)
}

/// Disable every other enabled entity in `cfg[config_key]` whose signature equals the target's.
fn disable_shadowed(cfg: &mut Value, config_key: &str, target: &Value, signature: fn(&Value) -> String) {
    if !bool_of(target, "enabled", false) {
        return;
    }
    let wanted = signature(target);
    let target_id = str_of(target, "id").to_string();
    for other in array_mut(cfg, config_key) {
        if str_of(other, "id") != target_id && bool_of(other, "enabled", false) && signature(other) == wanted {
            other["enabled"] = json!(false);
        }
    }
}

fn rule_signature(rule: &Value) -> String {
    format!("{}|{}", if bool_of(rule, "useRegex", false) { "re" } else { "exact" }, str_of(rule, "pattern"))
}

fn mock_signature(mock: &Value) -> String {
    format!("{}|{}|{}", str_of(mock, "method").to_uppercase(), str_of(mock, "urlPattern"), str_of(mock, "capturedBody"))
}

fn mapping_error(field: &str, key: &str, message: String, params: Value) -> EngineError {
    EngineError::with_details(ErrorCode::BadRequest, message, json!({ "field": field }))
        .keyed(&format!("mapping.{key}"), params)
}

fn is_dns_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    !bytes.is_empty()
        && bytes.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        && bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
}

fn is_dialable_target(target: &str) -> bool {
    let lower = target.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return url::Url::parse(target).is_ok_and(|u| u.host_str().is_some_and(|h| !h.is_empty()));
    }
    let Ok(pattern) = Regex::with_flags(r"^([a-z0-9.-]+):(\d{1,5})$", "i") else { return false };
    pattern
        .find(target)
        .and_then(|m| m.group(2))
        .and_then(|range| target[range].parse::<u32>().ok())
        .is_some_and(|port| (1..=65535).contains(&port))
}

/// A mapping needs a unique `<label>.localhost` domain and a target that can be dialed.
pub fn validate_mapping(mapping: &Value, siblings: &[Value]) -> Result<()> {
    let raw_domain = str_of(mapping, "domain");
    let domain = raw_domain.trim().to_lowercase();
    if domain.is_empty() || domain == ".localhost" {
        return Err(mapping_error("domain", "domain_required", "A domain is required.".into(), json!({})));
    }
    let Some(labels) = domain.strip_suffix(".localhost") else {
        return Err(mapping_error(
            "domain",
            "domain_not_localhost",
            format!("\"{raw_domain}\" is not a .localhost domain."),
            json!({ "domain": raw_domain }),
        ));
    };
    if !labels.split('.').all(is_dns_label) {
        return Err(mapping_error(
            "domain",
            "domain_invalid",
            format!("\"{raw_domain}\" is not a valid domain: use letters, digits and hyphens."),
            json!({ "domain": raw_domain }),
        ));
    }
    if siblings.iter().any(|other| str_of(other, "domain").trim().to_lowercase() == domain) {
        return Err(mapping_error(
            "domain",
            "domain_taken",
            format!("{domain} is already mapped."),
            json!({ "domain": domain }),
        ));
    }
    let target = str_of(mapping, "target").trim();
    if target.is_empty() {
        return Err(mapping_error("target", "target_required", "A target is required.".into(), json!({})));
    }
    if !is_dialable_target(target) {
        return Err(mapping_error(
            "target",
            "target_invalid",
            format!("\"{}\" is not a target: use host:port or an http(s):// URL.", str_of(mapping, "target")),
            json!({ "target": str_of(mapping, "target") }),
        ));
    }
    Ok(())
}

pub static CRUD_KINDS: [CrudKind; 14] = [
    CrudKind {
        kind: "flows",
        config_key: "flows",
        folder_config_key: Some("flowFolders"),
        is_flat: false,
        has_enabled_state: false,
        validate: Some(|flow, _| crate::flows::check_shape(flow)),
        on_add_conflict: None,
        name_entry: Some(|f| entry(&[("name", field(f, "name"))])),
        gate_kind: None,
    },
    CrudKind {
        kind: "mappings",
        config_key: "mappings",
        folder_config_key: None,
        is_flat: true,
        has_enabled_state: true,
        validate: Some(validate_mapping),
        on_add_conflict: None,
        name_entry: Some(|m| {
            let label = m.get("label").filter(|v| is_truthy(v)).cloned().unwrap_or_else(|| field(m, "domain"));
            entry(&[("name", label), ("url", field(m, "domain"))])
        }),
        gate_kind: None,
    },
    CrudKind {
        kind: "rules",
        config_key: "proxyRules",
        folder_config_key: Some("ruleFolders"),
        is_flat: false,
        has_enabled_state: true,
        validate: Some(|rule, _| {
            if bool_of(rule, "useRegex", false) {
                crate::utils::regex::validate("pattern", str_of(rule, "pattern"))
            } else {
                Ok(())
            }
        }),
        on_add_conflict: Some(|cfg, target| disable_shadowed(cfg, "proxyRules", target, rule_signature)),
        name_entry: Some(|r| entry(&[("name", field(r, "name")), ("url", field(r, "pattern"))])),
        gate_kind: None,
    },
    CrudKind {
        kind: "mocks",
        config_key: "mocks",
        folder_config_key: Some("mockFolders"),
        is_flat: false,
        has_enabled_state: true,
        validate: Some(|mock, _| {
            required(mock, "urlPattern", "urlPattern is required for mocks")?;
            required(mock, "method", "method is required for mocks")?;
            if bool_of(mock, "useRegex", false) {
                crate::utils::regex::validate("urlPattern", str_of(mock, "urlPattern"))?;
            }
            Ok(())
        }),
        on_add_conflict: Some(|cfg, target| disable_shadowed(cfg, "mocks", target, mock_signature)),
        name_entry: Some(|m| {
            entry(&[("name", field(m, "name")), ("method", field(m, "method")), ("url", field(m, "urlPattern"))])
        }),
        gate_kind: None,
    },
    CrudKind {
        kind: "requests",
        config_key: "requests",
        folder_config_key: Some("requestFolders"),
        is_flat: false,
        has_enabled_state: false,
        validate: Some(|request, _| {
            required(request, "url", "url is required for requests")?;
            required(request, "method", "method is required for requests")
        }),
        on_add_conflict: None,
        name_entry: Some(|r| {
            entry(&[("name", field(r, "name")), ("method", field(r, "method")), ("url", field(r, "url"))])
        }),
        gate_kind: None,
    },
    CrudKind {
        kind: "sockets",
        config_key: "wsConnections",
        folder_config_key: Some("requestFolders"),
        is_flat: false,
        has_enabled_state: false,
        validate: None,
        on_add_conflict: None,
        name_entry: Some(|c| entry(&[("name", field(c, "name")), ("url", field(c, "url"))])),
        gate_kind: None,
    },
    CrudKind {
        kind: "environments",
        config_key: "environments",
        folder_config_key: None,
        is_flat: true,
        has_enabled_state: false,
        validate: None,
        on_add_conflict: None,
        name_entry: Some(|e| entry(&[("name", field(e, "name"))])),
        gate_kind: Some("environment"),
    },
    CrudKind {
        kind: "graphqlRequests",
        config_key: "graphqlRequests",
        folder_config_key: Some("requestFolders"),
        is_flat: false,
        has_enabled_state: false,
        validate: None,
        on_add_conflict: None,
        name_entry: Some(|r| entry(&[("name", field(r, "name")), ("endpointUrl", field(r, "endpointUrl"))])),
        gate_kind: None,
    },
    CrudKind {
        kind: "soapRequests",
        config_key: "soapRequests",
        folder_config_key: Some("requestFolders"),
        is_flat: false,
        has_enabled_state: false,
        validate: None,
        on_add_conflict: None,
        name_entry: Some(|r| {
            entry(&[
                ("name", field(r, "name")),
                ("endpointUrl", field(r, "endpointUrl")),
                ("soapAction", field(r, "soapAction")),
            ])
        }),
        gate_kind: None,
    },
    CrudKind {
        kind: "grpcRequests",
        config_key: "grpcRequests",
        folder_config_key: Some("requestFolders"),
        is_flat: false,
        has_enabled_state: false,
        validate: Some(|request, _| {
            required(request, "serviceName", "serviceName is required for gRPC requests")?;
            required(request, "methodName", "methodName is required for gRPC requests")
        }),
        on_add_conflict: None,
        name_entry: Some(|r| {
            entry(&[
                ("name", field(r, "name")),
                ("serverAddress", field(r, "serverAddress")),
                ("serviceName", field(r, "serviceName")),
                ("methodName", field(r, "methodName")),
            ])
        }),
        gate_kind: None,
    },
    CrudKind {
        kind: "sseRequests",
        config_key: "sseRequests",
        folder_config_key: Some("requestFolders"),
        is_flat: false,
        has_enabled_state: false,
        validate: Some(|request, _| required(request, "url", "url is required for SSE requests")),
        on_add_conflict: None,
        name_entry: Some(|r| {
            entry(&[("name", field(r, "name")), ("method", field(r, "method")), ("url", field(r, "url"))])
        }),
        gate_kind: None,
    },
    CrudKind {
        kind: "mcpRequests",
        config_key: "mcpRequests",
        folder_config_key: Some("requestFolders"),
        is_flat: false,
        has_enabled_state: false,
        validate: Some(|request, _| {
            required(
                request,
                "url",
                "url is required for MCP requests: the command that starts the server, or its address",
            )
        }),
        on_add_conflict: None,
        name_entry: Some(|r| {
            entry(&[("name", field(r, "name")), ("transport", field(r, "transport")), ("url", field(r, "url"))])
        }),
        gate_kind: None,
    },
    CrudKind {
        kind: "webhookRequests",
        config_key: "webhookRequests",
        folder_config_key: Some("requestFolders"),
        is_flat: false,
        has_enabled_state: false,
        validate: None,
        on_add_conflict: None,
        name_entry: Some(|r| entry(&[("name", field(r, "name")), ("url", field(r, "url"))])),
        gate_kind: None,
    },
    CrudKind {
        kind: "socketioRequests",
        config_key: "socketioRequests",
        folder_config_key: Some("requestFolders"),
        is_flat: false,
        has_enabled_state: false,
        validate: Some(|request, _| required(request, "url", "url is required for Socket.IO requests")),
        on_add_conflict: None,
        name_entry: Some(|r| {
            entry(&[("name", field(r, "name")), ("url", field(r, "url")), ("namespace", field(r, "namespace"))])
        }),
        gate_kind: None,
    },
];

/// The kind named `kind` (the store's name; see `kernel::kinds::to_engine_kind`).
pub fn crud_kind(kind: &str) -> Option<&'static CrudKind> {
    CRUD_KINDS.iter().find(|k| k.kind == kind)
}

/// The store's kind for a folder kind: `folder.*` name folders by `request` (the request tree, whatever the
/// types of the requests in it), `mock` and `rule`.
pub fn folder_store_kind(folder_kind: &str) -> Option<&'static str> {
    match folder_kind {
        "mock" => Some("mocks"),
        "rule" => Some("rules"),
        "request" => Some("requests"),
        "flow" => Some("flows"),
        _ => None,
    }
}

/// Whether every entity of `kind` lives in a folder: the requests and mocks, never loose at the top level.
pub fn requires_folder(kind: &CrudKind) -> bool {
    kind.folder_config_key.is_some() && kind.kind != "rules"
}

pub fn is_simple_kind(kind: &str) -> bool {
    SIMPLE_KINDS.contains(&kind)
}

/// The names-index entry of an entity of `kind`, if the kind keeps one.
pub fn kind_name_entry(kind: &str, entity: &Value) -> Option<Value> {
    if !entity.is_object() {
        return None;
    }
    crud_kind(kind)?.name_entry.map(|resolve| resolve(entity))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mappings_are_validated() {
        let ok = json!({ "domain": "App.localhost", "target": "localhost:3000" });
        assert!(validate_mapping(&ok, &[]).is_ok());
        assert!(validate_mapping(&ok, &[json!({ "domain": "app.localhost" })]).is_err());
        assert!(validate_mapping(&json!({ "domain": "app.example", "target": "a:1" }), &[]).is_err());
        assert!(validate_mapping(&json!({ "domain": "-a.localhost", "target": "a:1" }), &[]).is_err());
        assert!(validate_mapping(&json!({ "domain": "a.localhost", "target": "nope" }), &[]).is_err());
        assert!(validate_mapping(&json!({ "domain": "a.localhost", "target": "https://example.com/x" }), &[]).is_ok());
        assert!(validate_mapping(&json!({ "domain": "a.localhost", "target": "a:70000" }), &[]).is_err());
    }

    #[test]
    fn an_invalid_regex_is_rejected_on_save() {
        let validate = |kind: &str, entity: Value| (crud_kind(kind).unwrap().validate.unwrap())(&entity, &[]);
        let bad_mock = json!({ "method": "GET", "urlPattern": "(", "useRegex": true });
        let message = validate("mocks", bad_mock).unwrap_err().to_string();
        assert!(message.contains("urlPattern \"(\""), "{message}");
        assert!(validate("mocks", json!({ "method": "GET", "urlPattern": "(", "useRegex": false })).is_ok());
        assert!(validate("mocks", json!({ "method": "GET", "urlPattern": r"^/a/\d+$", "useRegex": true })).is_ok());
        assert!(validate("rules", json!({ "pattern": "(", "useRegex": true })).is_err());
        assert!(validate("rules", json!({ "pattern": "(", "useRegex": false })).is_ok());
        assert!(validate("rules", json!({ "pattern": r"^http://a\.test/", "useRegex": true })).is_ok());
    }

    #[test]
    fn a_new_enabled_mock_disables_the_one_it_shadows() {
        let mut cfg = json!({ "mocks": [
            { "id": "old", "enabled": true, "method": "get", "urlPattern": "/a" },
            { "id": "other", "enabled": true, "method": "GET", "urlPattern": "/b" },
        ] });
        let target = json!({ "id": "new", "enabled": true, "method": "GET", "urlPattern": "/a" });
        (crud_kind("mocks").unwrap().on_add_conflict.unwrap())(&mut cfg, &target);
        assert_eq!(cfg["mocks"][0]["enabled"], false);
        assert_eq!(cfg["mocks"][1]["enabled"], true);
    }
}
