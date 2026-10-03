//! Shared constants and JavaScript-flavoured helpers for the application modules. Application definitions are
//! `serde_json::Value`s so a file keeps every field it had.

use serde_json::Value;

use crate::utils::json::is_truthy;

pub const DEFAULT_DEBUG_PORT_NODE: i64 = 9229;
pub const DEFAULT_DEBUG_PORT_JAVA: i64 = 5005;
pub const DEFAULT_DEBUG_PORT_SPRING_BOOT: i64 = 5005;
pub const DEFAULT_DEBUG_PORT_PYTHON: i64 = 5678;
pub const DEFAULT_DEBUG_PORT_GO: i64 = 2345;

/// The most log chunks kept per application.
pub const MAX_LOG_LINES: usize = 5000;

/// `String(value)` for the scalar values an application definition can hold.
pub fn js_str(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        Value::String(s) => s.clone(),
        Value::Array(items) => js_join(items, ","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

/// `array.join(sep)`: `null` elements join as empty strings.
pub fn js_join(items: &[Value], sep: &str) -> String {
    items.iter().map(|v| if v.is_null() { String::new() } else { js_str(v) }).collect::<Vec<_>>().join(sep)
}

/// `array.filter(Boolean).join(sep)`.
pub fn join_truthy(items: &[Value], sep: &str) -> String {
    items.iter().filter(|v| is_truthy(v)).map(js_str).collect::<Vec<_>>().join(sep)
}
