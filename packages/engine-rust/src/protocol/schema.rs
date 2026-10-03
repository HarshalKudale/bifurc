//! Payload validation against the protocol's command schemas.
//!
//! `schemas/commands.json` is generated from `@bifurc/protocol`'s zod schemas
//! (`scripts/gen-engine-rust-schemas.mjs`). This validator covers the JSON Schema subset that generator
//! emits and reproduces what a zod `parse` hands the handler: a strict object rejects unknown keys, a plain
//! object drops them, and a union yields the first member that accepts the value.

use std::{collections::HashMap, sync::OnceLock};

use serde_json::{Map, Value};

const SCHEMAS_JSON: &str = include_str!("../../schemas/commands.json");

fn schemas() -> &'static HashMap<String, Value> {
    static SCHEMAS: OnceLock<HashMap<String, Value>> = OnceLock::new();
    SCHEMAS.get_or_init(|| {
        let parsed: Value = serde_json::from_str(SCHEMAS_JSON).expect("schemas/commands.json is valid JSON");
        parsed["commands"]
            .as_object()
            .expect("schemas/commands.json has a commands object")
            .iter()
            .map(|(name, schema)| (name.clone(), schema.clone()))
            .collect()
    })
}

/// Whether `action` is a command the protocol declares.
pub fn is_known_command(action: &str) -> bool {
    schemas().contains_key(action)
}

/// Every command the protocol declares.
pub fn command_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = schemas().keys().map(String::as_str).collect();
    names.sort_unstable();
    names
}

/// Validate `payload` for `action`, returning the payload as the handler should see it.
pub fn validate(action: &str, payload: &Value) -> Result<Value, String> {
    let schema = schemas().get(action).ok_or_else(|| format!("no schema for command \"{action}\""))?;
    check(schema, payload, "")
}

fn at(path: &str) -> String {
    if path.is_empty() { "payload".to_string() } else { path.to_string() }
}

fn type_matches(expected: &str, value: &Value) -> bool {
    match expected {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_f64().is_some_and(|n| n.fract() == 0.0),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn check(schema: &Value, value: &Value, path: &str) -> Result<Value, String> {
    let Some(schema) = schema.as_object() else {
        return Ok(value.clone());
    };

    if let Some(members) = schema.get("anyOf").and_then(Value::as_array) {
        let mut first_error = None;
        for member in members {
            match check(member, value, path) {
                Ok(out) => return Ok(out),
                Err(err) => {
                    first_error.get_or_insert(err);
                }
            }
        }
        return Err(first_error.unwrap_or_else(|| format!("{}: matches no allowed shape", at(path))));
    }

    match schema.get("type") {
        Some(Value::String(expected)) => {
            if !type_matches(expected, value) {
                return Err(format!("{}: expected {expected}, received {}", at(path), type_name(value)));
            }
        }
        Some(Value::Array(allowed)) => {
            if !allowed.iter().filter_map(Value::as_str).any(|t| type_matches(t, value)) {
                let names: Vec<&str> = allowed.iter().filter_map(Value::as_str).collect();
                return Err(format!("{}: expected {}, received {}", at(path), names.join(" or "), type_name(value)));
            }
        }
        _ => {}
    }

    if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
        if !allowed.contains(value) {
            let names: Vec<String> = allowed.iter().map(Value::to_string).collect();
            return Err(format!("{}: expected one of {}", at(path), names.join(" | ")));
        }
    }
    if let Some(expected) = schema.get("const") {
        if expected != value {
            return Err(format!("{}: expected {expected}", at(path)));
        }
    }

    match value {
        Value::String(text) => {
            if let Some(min) = schema.get("minLength").and_then(Value::as_u64) {
                if (text.chars().count() as u64) < min {
                    return Err(format!("{}: expected at least {min} character(s)", at(path)));
                }
            }
            Ok(value.clone())
        }
        Value::Number(number) => {
            let n = number.as_f64().unwrap_or(f64::NAN);
            if let Some(min) = schema.get("minimum").and_then(Value::as_f64) {
                if n < min {
                    return Err(format!("{}: expected a number >= {min}", at(path)));
                }
            }
            if let Some(max) = schema.get("maximum").and_then(Value::as_f64) {
                if n > max {
                    return Err(format!("{}: expected a number <= {max}", at(path)));
                }
            }
            if let Some(min) = schema.get("exclusiveMinimum").and_then(Value::as_f64) {
                if n <= min {
                    return Err(format!("{}: expected a number > {min}", at(path)));
                }
            }
            if let Some(max) = schema.get("exclusiveMaximum").and_then(Value::as_f64) {
                if n >= max {
                    return Err(format!("{}: expected a number < {max}", at(path)));
                }
            }
            Ok(value.clone())
        }
        Value::Array(items) => {
            if let Some(min) = schema.get("minItems").and_then(Value::as_u64) {
                if (items.len() as u64) < min {
                    return Err(format!("{}: expected at least {min} item(s)", at(path)));
                }
            }
            match schema.get("items") {
                Some(item_schema) => {
                    let mut out = Vec::with_capacity(items.len());
                    for (index, item) in items.iter().enumerate() {
                        out.push(check(item_schema, item, &format!("{path}[{index}]"))?);
                    }
                    Ok(Value::Array(out))
                }
                None => Ok(value.clone()),
            }
        }
        Value::Object(fields) => check_object(schema, fields, path),
        _ => Ok(value.clone()),
    }
}

fn check_object(schema: &Map<String, Value>, fields: &Map<String, Value>, path: &str) -> Result<Value, String> {
    let properties = schema.get("properties").and_then(Value::as_object);
    let additional = schema.get("additionalProperties");
    if properties.is_none() && additional.is_none() {
        return Ok(Value::Object(fields.clone()));
    }

    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for name in required.iter().filter_map(Value::as_str) {
            if !fields.contains_key(name) {
                return Err(format!("{}: required", at(&join(path, name))));
            }
        }
    }

    let mut out = Map::new();
    // A zod parse hands back the declared keys in the schema's order, whatever order the caller sent them in.
    for (key, property) in properties.into_iter().flatten() {
        if let Some(value) = fields.get(key) {
            out.insert(key.clone(), check(property, value, &join(path, key))?);
        }
    }
    for (key, value) in fields {
        let child = join(path, key);
        if properties.is_some_and(|p| p.contains_key(key)) {
            continue;
        }
        match additional {
            Some(Value::Bool(false)) => return Err(format!("{}: unrecognized key \"{key}\"", at(path))),
            Some(Value::Bool(true)) => {
                out.insert(key.clone(), value.clone());
            }
            Some(extra) => {
                out.insert(key.clone(), check(extra, value, &child)?);
            }
            // A plain zod object: unknown keys are dropped, not an error.
            None => {}
        }
    }
    Ok(Value::Object(out))
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() { key.to_string() } else { format!("{path}.{key}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strict_objects_reject_unknown_keys() {
        assert!(validate("config.get", &json!({})).is_ok());
        assert!(validate("config.get", &json!({ "extra": 1 })).is_err());
    }

    #[test]
    fn declared_keys_come_back_in_schema_order() {
        let report = json!({ "folderId": "f", "totalRequests": 1, "completedAt": 3, "startedAt": 2, "folderName": "n" });
        let out = validate("runner.saveReport", &json!({ "report": report, "workspaceId": "w" })).unwrap();
        let keys = |value: &Value| value.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        assert_eq!(keys(&out), ["workspaceId", "report"]);
        assert_eq!(keys(&out["report"]), ["folderId", "folderName", "startedAt", "completedAt", "totalRequests"]);
    }

    #[test]
    fn plain_objects_drop_unknown_keys() {
        let out = validate("entity.create", &json!({ "kind": "mocks", "entity": { "a": 1 }, "extra": true })).unwrap();
        assert_eq!(out, json!({ "kind": "mocks", "entity": { "a": 1 } }));
    }

    #[test]
    fn unions_take_the_first_member_that_accepts() {
        assert!(validate("request.send", &json!({ "workspaceId": "w", "id": "r" })).is_ok());
        assert!(validate("request.send", &json!({ "workspaceId": "w" })).is_err());
    }

    #[test]
    fn nullable_and_ranged_fields() {
        assert!(validate("settings.update", &json!({ "tlsCaCertPath": null, "port": 8081 })).is_ok());
        assert!(validate("settings.update", &json!({ "port": 0 })).is_err());
        assert!(validate("settings.update", &json!({ "port": 80.5 })).is_err());
    }

    #[test]
    fn every_command_has_a_schema() {
        assert!(command_names().len() > 100);
        assert!(is_known_command("engine.shutdown"));
        assert!(!is_known_command("hello"));
    }
}
