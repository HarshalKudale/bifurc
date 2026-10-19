//! What an MCP server answered, as a person reads it. The log keeps the answer as it came (`data`); this is
//! the text in it (`text`), so every client shows the same thing without knowing the shapes of the protocol.

use serde_json::Value;

/// The text of one piece of content: its `text`, the text of the resource it embeds, or what it is when it
/// is not text (an image, audio, a link).
fn piece(item: &Value) -> String {
    if let Some(text) = item.get("text").and_then(Value::as_str) {
        return text.to_string();
    }
    if let Some(resource) = item.get("resource").filter(|resource| resource.is_object()) {
        return piece(resource);
    }
    let kind = item.get("type").and_then(Value::as_str);
    let mime = item.get("mimeType").and_then(Value::as_str);
    let uri = item.get("uri").and_then(Value::as_str);
    match (kind, mime, uri) {
        (Some("resource_link"), _, Some(uri)) => format!("[link: {uri}]"),
        (Some(kind), Some(mime), _) => format!("[{kind}: {mime}]"),
        (None, Some(mime), Some(uri)) => format!("[{uri}: {mime}]"),
        _ => item.to_string(),
    }
}

/// The text of a result: a tool's content, a prompt's messages (each after its role) or a resource's
/// contents, one piece after another. A result with none of them is shown whole, as indented JSON.
pub fn of_result(result: &Value) -> String {
    let items = |key: &str| result.get(key).and_then(Value::as_array).filter(|items| !items.is_empty());
    let whole = || serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string());
    if let Some(content) = items("content") {
        return content.iter().map(piece).collect::<Vec<_>>().join("\n");
    }
    if let Some(messages) = items("messages") {
        let said = |message: &Value| {
            let role = message.get("role").and_then(Value::as_str).unwrap_or("message");
            format!("{role}: {}", message.get("content").map(piece).unwrap_or_default())
        };
        return messages.iter().map(said).collect::<Vec<_>>().join("\n");
    }
    if let Some(contents) = items("contents") {
        return contents.iter().map(piece).collect::<Vec<_>>().join("\n");
    }
    match result.get("structuredContent") {
        Some(structured) => serde_json::to_string_pretty(structured).unwrap_or_else(|_| whole()),
        None => whole(),
    }
}

/// The text of an error: its message, or the whole of it when it has none.
pub fn of_error(error: &Value) -> String {
    match error.get("message").and_then(Value::as_str).filter(|message| !message.is_empty()) {
        Some(message) => message.to_string(),
        None => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_text_of_a_tools_content_is_read_piece_by_piece() {
        let result = json!({ "content": [
            { "type": "text", "text": "one" },
            { "type": "image", "data": "AA==", "mimeType": "image/png" },
            { "type": "resource", "resource": { "uri": "mem://a", "text": "embedded" } },
            { "type": "resource_link", "uri": "mem://b", "name": "b" },
        ] });
        assert_eq!(of_result(&result), "one\n[image: image/png]\nembedded\n[link: mem://b]");
    }

    #[test]
    fn a_prompt_is_its_messages_and_a_resource_its_contents() {
        let prompt = json!({ "messages": [
            { "role": "user", "content": { "type": "text", "text": "Hello" } },
            { "role": "assistant", "content": { "type": "text", "text": "Hi" } },
        ] });
        assert_eq!(of_result(&prompt), "user: Hello\nassistant: Hi");
        let resource = json!({ "contents": [
            { "uri": "mem://a", "text": "alpha" },
            { "uri": "mem://b", "blob": "AA==", "mimeType": "application/pdf" },
        ] });
        assert_eq!(of_result(&resource), "alpha\n[mem://b: application/pdf]");
    }

    #[test]
    fn a_result_without_text_is_shown_whole() {
        assert_eq!(of_result(&json!({ "content": [], "structuredContent": { "n": 1 } })), "{\n  \"n\": 1\n}");
        assert_eq!(of_result(&json!({ "ok": true })), "{\n  \"ok\": true\n}");
    }

    #[test]
    fn an_error_is_its_message() {
        assert_eq!(of_error(&json!({ "code": -32000, "message": "boom" })), "boom");
        assert_eq!(of_error(&json!({ "code": 7 })), "{\"code\":7}");
    }
}
