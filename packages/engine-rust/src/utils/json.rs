//! Helpers for the store's JSON. Entities and settings are kept as `serde_json::Value` rather than typed
//! structs so a file round-trips with every field it had, in the order it had them.

use std::path::Path;

use serde_json::{Map, Value};

/// `value[key]` as a string, or `""`.
pub fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// `value[key]` as a string, if it is one.
pub fn opt_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// `value[key]` as a boolean; `default` when absent or not a boolean.
pub fn bool_of(value: &Value, key: &str, default: bool) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(default)
}

/// JavaScript truthiness of `value[key]`.
pub fn truthy(value: &Value, key: &str) -> bool {
    is_truthy(value.get(key).unwrap_or(&Value::Null))
}

/// JavaScript truthiness.
pub fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

pub fn i64_of(value: &Value, key: &str, default: i64) -> i64 {
    value.get(key).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))).unwrap_or(default)
}

/// `value[key]` as an array, or an empty slice.
pub fn array_of<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

/// `value[key]` as a mutable array, created (or replaced, if it is not an array) when needed.
pub fn array_mut<'a>(value: &'a mut Value, key: &str) -> &'a mut Vec<Value> {
    let object = object_mut(value);
    let slot = object.entry(key.to_string()).or_insert_with(|| Value::Array(Vec::new()));
    if !slot.is_array() {
        *slot = Value::Array(Vec::new());
    }
    slot.as_array_mut().expect("just made an array")
}

/// `value` as a mutable object; a non-object is replaced by an empty one.
pub fn object_mut(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value.as_object_mut().expect("just made an object")
}

/// A `Record<string, string>` field as pairs, in order. Non-string values are skipped.
pub fn string_map(value: &Value, key: &str) -> Vec<(String, String)> {
    value
        .get(key)
        .and_then(Value::as_object)
        .map(|object| object.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect())
        .unwrap_or_default()
}

/// `{ ...base, ...patch }`: `patch`'s keys win, new keys are appended.
pub fn merged(base: &Value, patch: &Value) -> Value {
    let mut out = base.as_object().cloned().unwrap_or_default();
    if let Some(patch) = patch.as_object() {
        for (key, value) in patch {
            out.insert(key.clone(), value.clone());
        }
    }
    Value::Object(out)
}

/// Parse a JSON file; `None` when it is missing or invalid.
pub fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Write `value` the way the store always has: two-space indented, creating the parent directory.
pub fn write_json_pretty(path: &Path, value: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(value)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merged_keeps_base_order_and_appends_new_keys() {
        let out = merged(&json!({ "a": 1, "b": 2 }), &json!({ "c": 3, "a": 9 }));
        assert_eq!(serde_json::to_string(&out).unwrap(), r#"{"a":9,"b":2,"c":3}"#);
    }

    #[test]
    fn truthiness_follows_javascript() {
        let value = json!({ "zero": 0, "empty": "", "text": "x", "list": [] });
        assert!(!truthy(&value, "zero"));
        assert!(!truthy(&value, "empty"));
        assert!(!truthy(&value, "missing"));
        assert!(truthy(&value, "text"));
        assert!(truthy(&value, "list"));
    }
}
