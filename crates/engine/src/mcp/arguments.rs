//! What a call asks of an MCP server, and how what a user typed becomes its arguments.
//!
//! Nothing here does I/O: the SDK carries the messages, the registry decides what to send.

use serde_json::{Map, Value};

/// What a server declares and a client lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Catalog {
    Tools,
    Prompts,
    Resources,
    ResourceTemplates,
}

impl Catalog {
    pub const ALL: [Catalog; 4] = [Catalog::Tools, Catalog::Prompts, Catalog::Resources, Catalog::ResourceTemplates];

    /// The key of the list, in the server's answer and in a connection's snapshot.
    pub fn key(self) -> &'static str {
        match self {
            Catalog::Tools => "tools",
            Catalog::Prompts => "prompts",
            Catalog::Resources => "resources",
            Catalog::ResourceTemplates => "resourceTemplates",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    /// The lists a `list_changed` notification says to read again.
    pub fn changed_by(method: &str) -> &'static [Catalog] {
        match method {
            "notifications/tools/list_changed" => &[Catalog::Tools],
            "notifications/prompts/list_changed" => &[Catalog::Prompts],
            "notifications/resources/list_changed" => &[Catalog::Resources, Catalog::ResourceTemplates],
            _ => &[],
        }
    }
}

/// What a call asks the server for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    Tool,
    Prompt,
    Resource,
}

impl CallKind {
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "tool" => Some(CallKind::Tool),
            "prompt" => Some(CallKind::Prompt),
            "resource" => Some(CallKind::Resource),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            CallKind::Tool => "tool",
            CallKind::Prompt => "prompt",
            CallKind::Resource => "resource",
        }
    }
}

/// The schemas a property is one of (`anyOf` / `oneOf`), or the property itself when it is not a choice of
/// schemas.
fn alternatives(property: &Value) -> Vec<&Value> {
    ["anyOf", "oneOf"]
        .iter()
        .find_map(|key| property.get(*key)?.as_array())
        .map(|schemas| schemas.iter().collect())
        .unwrap_or_else(|| vec![property])
}

/// The type a schema names: its `type`, or the first one that is not `null` when it lists several.
fn named_type(schema: &Value) -> Option<&str> {
    match schema.get("type")? {
        Value::String(name) => Some(name.as_str()),
        Value::Array(names) => names.iter().filter_map(Value::as_str).find(|name| *name != "null"),
        _ => None,
    }
}

/// The type a JSON Schema gives a property: its own, or the one its alternatives agree on besides `null`
/// (which is how an optional argument is often declared). Alternatives of different types give it none: what
/// is typed is then read as JSON when it is JSON.
fn schema_type(property: Option<&Value>) -> Option<&str> {
    let property = property?;
    if let Some(name) = named_type(property) {
        return Some(name);
    }
    let alternatives = alternatives(property);
    let mut named = alternatives.iter().map(|schema| named_type(schema)).filter(|name| *name != Some("null"));
    let first = named.next()??;
    named.all(|name| name == Some(first)).then_some(first)
}

/// The values a property is limited to: its `enum` or `const`, or those of its alternatives. Empty when it
/// takes any value of its type.
fn choices(property: Option<&Value>) -> Vec<&Value> {
    let Some(property) = property else { return Vec::new() };
    let mut allowed = Vec::new();
    for schema in alternatives(property) {
        if let Some(values) = schema.get("enum").and_then(Value::as_array) {
            allowed.extend(values.iter().filter(|value| !value.is_null()));
        } else if let Some(only) = schema.get("const") {
            allowed.extend(Some(only).filter(|value| !value.is_null()));
        } else if schema.get("type").and_then(Value::as_str) != Some("null") {
            // One alternative takes any value, so the property is not limited to the others.
            return Vec::new();
        }
    }
    allowed
}

/// A value as it is typed, and as a choice is shown: text without its quotes, anything else as JSON.
fn as_typed(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The arguments of a tool call from what was typed for each: text is resolved with `resolve` and becomes the
/// type the tool's input schema declares for that argument (a number, a boolean, JSON for an object or an
/// array); a value that is not text is sent as it is. An argument left empty is left out unless the schema
/// requires it and it is text.
pub fn tool_arguments(
    schema: Option<&Value>,
    typed: Option<&Map<String, Value>>,
    resolve: &dyn Fn(&str) -> String,
) -> Result<Map<String, Value>, String> {
    let properties = schema.and_then(|schema| schema.get("properties"));
    let required = |name: &str| {
        schema
            .and_then(|schema| schema.get("required")?.as_array())
            .is_some_and(|names| names.iter().any(|n| n.as_str() == Some(name)))
    };
    let mut out = Map::new();
    for (name, value) in typed.into_iter().flatten() {
        let Value::String(text) = value else {
            out.insert(name.clone(), value.clone());
            continue;
        };
        let text = resolve(text);
        let property = properties.and_then(|properties| properties.get(name));
        // An argument limited to certain values is sent as the one that was named, exactly as declared.
        let allowed = choices(property);
        if !allowed.is_empty() {
            let wanted = text.trim();
            if wanted.is_empty() {
                continue;
            }
            match allowed.iter().find(|value| as_typed(value) == wanted) {
                Some(value) => out.insert(name.clone(), (*value).clone()),
                None => {
                    let listed: Vec<String> = allowed.iter().map(|value| as_typed(value)).collect();
                    return Err(format!("Argument \"{name}\" must be one of {}, not \"{wanted}\".", listed.join(", ")));
                }
            };
            continue;
        }
        let kind = schema_type(property);
        if kind == Some("string") {
            if !text.is_empty() || required(name) {
                out.insert(name.clone(), Value::String(text));
            }
            continue;
        }
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let wrong = |what: &str| format!("Argument \"{name}\" must be {what}, not \"{trimmed}\".");
        let parsed = serde_json::from_str::<Value>(trimmed).ok();
        let value = match kind {
            Some("integer") => match parsed.filter(|v| v.is_i64() || v.is_u64()) {
                Some(number) => number,
                None => return Err(wrong("a whole number")),
            },
            Some("number") => match parsed.filter(Value::is_number) {
                Some(number) => number,
                None => return Err(wrong("a number")),
            },
            Some("boolean") => match trimmed.to_ascii_lowercase().as_str() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => return Err(wrong("true or false")),
            },
            Some("object") => match parsed.filter(Value::is_object) {
                Some(object) => object,
                None => return Err(wrong("a JSON object")),
            },
            Some("array") => match parsed.filter(Value::is_array) {
                Some(array) => array,
                None => return Err(wrong("a JSON array")),
            },
            // No declared type: JSON when it reads as JSON, the text otherwise.
            _ => parsed.unwrap_or(Value::String(text.clone())),
        };
        out.insert(name.clone(), value);
    }
    Ok(out)
}

/// The arguments of a prompt, which are all text: each is resolved with `resolve`, and one left empty is left
/// out unless the prompt requires it.
pub fn prompt_arguments(
    prompt: Option<&Value>,
    typed: Option<&Map<String, Value>>,
    resolve: &dyn Fn(&str) -> String,
) -> Map<String, Value> {
    let required = |name: &str| {
        prompt.and_then(|prompt| prompt.get("arguments")?.as_array()).is_some_and(|arguments| {
            arguments.iter().any(|a| a.get("name").and_then(Value::as_str) == Some(name) && a["required"] == true)
        })
    };
    let mut out = Map::new();
    for (name, value) in typed.into_iter().flatten() {
        let text = match value {
            Value::String(text) => resolve(text),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        if !text.is_empty() || required(name) {
            out.insert(name.clone(), Value::String(text));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn same(text: &str) -> String {
        text.to_string()
    }

    fn typed(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn text_becomes_the_type_the_schema_declares() {
        let schema = json!({
            "type": "object",
            "properties": {
                "text": { "type": "string" },
                "count": { "type": "integer" },
                "ratio": { "type": "number" },
                "loud": { "type": "boolean" },
                "extra": { "type": "object" },
                "tags": { "type": "array" },
                "maybe": { "type": ["null", "integer"] },
                "any": {},
            },
            "required": ["text"],
        });
        let arguments = tool_arguments(
            Some(&schema),
            Some(&typed(json!({
                "text": "123", "count": " 4 ", "ratio": "0.5", "loud": "True", "extra": "{\"a\": 1}",
                "tags": "[\"x\"]", "maybe": "7", "any": "plain", "unknown": "{\"b\": 2}", "given": 9,
            }))),
            &same,
        )
        .unwrap();
        assert_eq!(
            Value::Object(arguments),
            json!({
                "text": "123", "count": 4, "ratio": 0.5, "loud": true, "extra": { "a": 1 }, "tags": ["x"],
                "maybe": 7, "any": "plain", "unknown": { "b": 2 }, "given": 9,
            })
        );
    }

    #[test]
    fn an_argument_left_empty_is_left_out_unless_it_is_required_text() {
        let schema = json!({
            "properties": { "text": { "type": "string" }, "note": { "type": "string" }, "count": { "type": "integer" } },
            "required": ["text", "count"],
        });
        let arguments =
            tool_arguments(Some(&schema), Some(&typed(json!({ "text": "", "note": "", "count": " " }))), &same);
        assert_eq!(Value::Object(arguments.unwrap()), json!({ "text": "" }));
        assert_eq!(tool_arguments(None, None, &same).unwrap(), Map::new());
    }

    #[test]
    fn text_that_is_not_of_the_declared_type_is_refused_by_name() {
        let schema = json!({ "properties": {
            "count": { "type": "integer" }, "ratio": { "type": "number" }, "loud": { "type": "boolean" },
            "extra": { "type": "object" }, "tags": { "type": "array" },
        } });
        for (name, text, wanted) in [
            ("count", "1.5", "a whole number"),
            ("count", "many", "a whole number"),
            ("ratio", "half", "a number"),
            ("loud", "yes", "true or false"),
            ("extra", "[1]", "a JSON object"),
            ("tags", "{}", "a JSON array"),
        ] {
            let error = tool_arguments(Some(&schema), Some(&typed(json!({ name: text }))), &same).unwrap_err();
            assert_eq!(error, format!("Argument \"{name}\" must be {wanted}, not \"{text}\"."));
        }
    }

    #[test]
    fn an_argument_limited_to_certain_values_is_sent_as_the_one_named() {
        let schema = json!({ "properties": {
            "interval": { "type": "string", "enum": ["day", "week", "month"] },
            "level": { "enum": [1, 2, "3"] },
            "mode": { "anyOf": [{ "const": "fast" }, { "const": "safe" }, { "type": "null" }] },
            "size": { "oneOf": [{ "type": "integer", "enum": [10, 20] }, { "type": "null" }] },
            "open": { "anyOf": [{ "const": "auto" }, { "type": "integer" }] },
        } });
        let arguments = tool_arguments(
            Some(&schema),
            Some(&typed(json!({ "interval": " week ", "level": "3", "mode": "safe", "size": "20", "open": "7" }))),
            &same,
        )
        .unwrap();
        // The declared value is sent, of the type it was declared with: `"3"` stays text, `20` is a number.
        assert_eq!(
            Value::Object(arguments),
            json!({ "interval": "week", "level": "3", "mode": "safe", "size": 20, "open": 7 })
        );
        let level = tool_arguments(Some(&schema), Some(&typed(json!({ "level": "2" }))), &same).unwrap();
        assert_eq!(level["level"], json!(2));

        let error = tool_arguments(Some(&schema), Some(&typed(json!({ "interval": "year" }))), &same).unwrap_err();
        assert_eq!(error, "Argument \"interval\" must be one of day, week, month, not \"year\".");
        // Nothing chosen is nothing sent.
        assert!(tool_arguments(Some(&schema), Some(&typed(json!({ "interval": " " }))), &same).unwrap().is_empty());
    }

    #[test]
    fn the_type_of_an_optional_argument_is_the_one_beside_null() {
        let schema = json!({ "properties": {
            "count": { "anyOf": [{ "type": "integer" }, { "type": "null" }] },
            "loud": { "oneOf": [{ "type": "null" }, { "type": "boolean" }] },
            "text": { "anyOf": [{ "type": "string" }, { "type": "null" }] },
        } });
        let arguments =
            tool_arguments(Some(&schema), Some(&typed(json!({ "count": "4", "loud": "false", "text": "12" }))), &same);
        assert_eq!(Value::Object(arguments.unwrap()), json!({ "count": 4, "loud": false, "text": "12" }));
        let error = tool_arguments(Some(&schema), Some(&typed(json!({ "count": "x" }))), &same).unwrap_err();
        assert_eq!(error, "Argument \"count\" must be a whole number, not \"x\".");
    }

    #[test]
    fn what_was_typed_is_resolved_before_it_is_read() {
        let schema = json!({ "properties": { "count": { "type": "integer" }, "text": { "type": "string" } } });
        let resolve = |text: &str| text.replace("{{N}}", "12");
        let arguments =
            tool_arguments(Some(&schema), Some(&typed(json!({ "count": "{{N}}", "text": "n={{N}}" }))), &resolve);
        assert_eq!(Value::Object(arguments.unwrap()), json!({ "count": 12, "text": "n=12" }));
    }

    #[test]
    fn the_arguments_of_a_prompt_are_text() {
        let prompt = json!({ "name": "greet", "arguments": [{ "name": "who", "required": true }, { "name": "tone" }] });
        let arguments = prompt_arguments(Some(&prompt), Some(&typed(json!({ "who": "", "tone": "", "n": 3 }))), &same);
        assert_eq!(Value::Object(arguments), json!({ "who": "", "n": "3" }));
    }

    #[test]
    fn a_changed_list_names_what_to_read_again() {
        assert_eq!(Catalog::changed_by("notifications/tools/list_changed"), [Catalog::Tools]);
        assert_eq!(
            Catalog::changed_by("notifications/resources/list_changed"),
            [Catalog::Resources, Catalog::ResourceTemplates]
        );
        assert!(Catalog::changed_by("notifications/message").is_empty());
    }
}
