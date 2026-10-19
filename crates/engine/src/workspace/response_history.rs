//! Response history: the last few responses of each saved request, kept on this device only.
//!
//! They live in `<dataDir>/responses/<workspaceId>/<entityId>.json`, beside the workspace directories and not
//! in them, so they are never part of the workspace's git history (`history.*` is that, and is unrelated).

use std::path::PathBuf;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use super::paths::data_dir;
use crate::{
    kernel::CommandRegistry,
    utils::{
        id::now_ms,
        json::{read_json, str_of},
    },
};

pub const KEEP: usize = 5;
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// One finished send, as the sends hand it over.
pub struct Capture {
    pub status: i64,
    pub status_text: Option<String>,
    pub headers: Value,
    pub body: Vec<u8>,
    pub duration_ms: u64,
    pub error: Option<String>,
    /// The key and params the UI translates `error` by. Stored beside it only when there is one, so a history
    /// file written before the key existed reads the same and an older build ignores the new members.
    pub error_key: Option<(String, Value)>,
    pub method: String,
    pub url: String,
}

fn is_plain(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn responses_dir(ws_id: &str) -> PathBuf {
    data_dir().join("responses").join(ws_id)
}

fn file_of(ws_id: &str, id: &str) -> Option<PathBuf> {
    (is_plain(ws_id) && is_plain(id)).then(|| responses_dir(ws_id).join(format!("{id}.json")))
}

fn entry_of(capture: Capture) -> Value {
    let size = capture.body.len();
    let truncated = size > MAX_BODY_BYTES;
    let mut entry = json!({
        "at": now_ms(),
        "durationMs": capture.duration_ms,
        "status": capture.status,
        "headers": capture.headers,
        "size": size,
        "truncated": truncated,
        "request": { "method": capture.method, "url": capture.url },
    });
    match std::str::from_utf8(&capture.body) {
        Ok(text) => {
            let mut end = text.len().min(MAX_BODY_BYTES);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            entry["body"] = json!(&text[..end]);
        }
        Err(_) => {
            entry["body"] = json!(STANDARD.encode(&capture.body[..size.min(MAX_BODY_BYTES)]));
            entry["encoding"] = json!("base64");
        }
    }
    if let Some(text) = capture.status_text {
        entry["statusText"] = json!(text);
    }
    if let Some(error) = capture.error {
        entry["error"] = json!(error);
        if let Some((key, params)) = capture.error_key {
            entry["errorKey"] = json!(key);
            entry["errorParams"] = params;
        }
    }
    entry
}

pub fn entries(ws_id: &str, id: &str) -> Vec<Value> {
    let parsed = file_of(ws_id, id).and_then(|path| read_json(&path));
    parsed.and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

/// Add a response to the entity's history, newest first, dropping the oldest past [`KEEP`]. History is a
/// convenience, so a failure to write it is not the send's failure.
pub fn record(ws_id: &str, id: &str, capture: Capture) {
    let Some(path) = file_of(ws_id, id) else { return };
    let mut entry = entry_of(capture);
    crate::vault::redact_value(ws_id, &mut entry);
    let mut all = entries(ws_id, id);
    all.insert(0, entry);
    all.truncate(KEEP);
    let _ = write_atomic(&path, &Value::Array(all));
}

fn write_atomic(path: &std::path::Path, value: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec(value)?)?;
    std::fs::rename(&temp, path)
}

/// The entity a send belongs to: the saved request it names, or `entityId` for an inline send of its edits.
pub fn entity_id(params: &Value) -> Option<&str> {
    ["id", "entityId"].into_iter().find_map(|key| params.get(key)?.as_str()).filter(|id| !id.is_empty())
}

/// Record a send whose answer is `{ status, headers, body, durationMs }` with a text body (GraphQL, SOAP),
/// or whose failure is an error.
pub fn record_text(ws_id: &str, params: &Value, method: &str, url: &str, outcome: &crate::protocol::Result<Value>) {
    let Some(id) = entity_id(params) else { return };
    let (method, url) = (method.to_string(), url.to_string());
    let capture = match outcome {
        Ok(v) => Capture {
            status: v["status"].as_i64().unwrap_or(0),
            status_text: None,
            headers: v["headers"].clone(),
            body: str_of(v, "body").as_bytes().to_vec(),
            duration_ms: v["durationMs"].as_u64().unwrap_or(0),
            error: None,
            error_key: None,
            method,
            url,
        },
        Err(e) => failed(e, method, url),
    };
    record(ws_id, id, capture);
}

pub fn failed(error: &crate::protocol::EngineError, method: String, url: String) -> Capture {
    Capture {
        status: 0,
        status_text: None,
        headers: json!({}),
        body: Vec::new(),
        duration_ms: 0,
        error: Some(error.message.clone()),
        error_key: error.key().zip(error.params()).map(|(key, params)| (key.to_owned(), params.clone())),
        method,
        url,
    }
}

/// Forget an entity's history: it is gone for good.
pub fn forget(ws_id: &str, id: &str) {
    if let Some(path) = file_of(ws_id, id) {
        let _ = std::fs::remove_file(path);
    }
}

/// Forget a workspace's whole history.
pub fn forget_workspace(ws_id: &str) {
    if is_plain(ws_id) {
        let _ = std::fs::remove_dir_all(responses_dir(ws_id));
    }
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("response.history", |params, _| {
        Ok(json!({ "entries": entries(str_of(&params, "workspaceId"), str_of(&params, "id")) }))
    });
    registry.register_sync("response.clearHistory", |params, _| {
        forget(str_of(&params, "workspaceId"), str_of(&params, "id"));
        Ok(json!({ "ok": true }))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{testing::scratch_data_root, workspace::fs::ws_dir};

    fn capture(body: &[u8]) -> Capture {
        Capture {
            status: 200,
            status_text: None,
            headers: json!({}),
            body: body.to_vec(),
            duration_ms: 3,
            error: None,
            error_key: None,
            method: "GET".into(),
            url: "http://x".into(),
        }
    }

    #[test]
    fn the_five_newest_are_kept_newest_first_outside_the_workspace() {
        let root = scratch_data_root();
        for n in 0..7 {
            record(root.ws_id, "r1", capture(format!("body {n}").as_bytes()));
        }
        let kept = entries(root.ws_id, "r1");
        let bodies: Vec<_> = kept.iter().map(|e| e["body"].as_str().unwrap()).collect();
        assert_eq!(bodies, ["body 6", "body 5", "body 4", "body 3", "body 2"]);
        let file = file_of(root.ws_id, "r1").unwrap();
        assert!(file.exists() && !file.starts_with(ws_dir(root.ws_id)));
    }

    #[test]
    fn a_history_file_from_before_error_keys_loads_as_it_did() {
        let root = scratch_data_root();
        let old = json!([
            { "at": 900, "durationMs": 0, "status": 0, "headers": {}, "size": 0, "truncated": false,
              "request": { "method": "GET", "url": "http://x" }, "body": "", "error": "Nothing is listening." },
            { "at": 800, "durationMs": 5, "status": 200, "headers": { "a": "b" }, "size": 2, "truncated": false,
              "request": { "method": "GET", "url": "http://x" }, "body": "hi", "statusText": "OK" },
        ]);
        let path = file_of(root.ws_id, "old").unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(entries(root.ws_id, "old"), old.as_array().unwrap().clone());
        assert!(entries(root.ws_id, "old")[0].get("errorKey").is_none());
        // A new entry is added on top of an old file without touching the old ones.
        record(root.ws_id, "old", capture(b"new"));
        let all = entries(root.ws_id, "old");
        assert_eq!((all.len(), all[1].clone(), all[2].clone()), (3, old[0].clone(), old[1].clone()));
    }

    #[test]
    fn a_failed_send_keeps_its_error_key_and_params_beside_the_english_text() {
        let root = scratch_data_root();
        let refused =
            crate::protocol::EngineError::upstream("Nothing is listening at localhost:1. Is the server running?")
                .keyed("request.send.refused", json!({ "host": "localhost", "port": "1" }));
        record(root.ws_id, "r1", failed(&refused, "GET".into(), "http://localhost:1".into()));
        record(root.ws_id, "r1", failed(&crate::protocol::EngineError::engine("boom"), "GET".into(), "u".into()));
        let kept = entries(root.ws_id, "r1");
        assert_eq!(kept[0]["error"], "boom");
        assert!(kept[0].get("errorKey").is_none() && kept[0].get("errorParams").is_none());
        assert_eq!(kept[1]["error"], "Nothing is listening at localhost:1. Is the server running?");
        assert_eq!(kept[1]["errorKey"], "request.send.refused");
        assert_eq!(kept[1]["errorParams"], json!({ "host": "localhost", "port": "1" }));
    }

    #[test]
    fn a_big_body_is_cut_and_binary_is_base64() {
        let root = scratch_data_root();
        record(root.ws_id, "r1", capture(&vec![b'a'; MAX_BODY_BYTES + 10]));
        record(root.ws_id, "r2", capture(&[0xff, 0xfe, 0x00]));
        let big = &entries(root.ws_id, "r1")[0];
        assert_eq!((big["truncated"].clone(), big["size"].clone()), (json!(true), json!(MAX_BODY_BYTES + 10)));
        assert_eq!(big["body"].as_str().unwrap().len(), MAX_BODY_BYTES);
        let bin = &entries(root.ws_id, "r2")[0];
        assert_eq!((bin["encoding"].clone(), bin["body"].clone()), (json!("base64"), json!("//4A")));
    }

    #[test]
    fn clearing_and_forgetting_remove_the_file_and_odd_ids_are_refused() {
        let root = scratch_data_root();
        record(root.ws_id, "r1", capture(b"x"));
        record(root.ws_id, "../evil", capture(b"x"));
        assert!(entries(root.ws_id, "../evil").is_empty());
        forget(root.ws_id, "r1");
        assert!(entries(root.ws_id, "r1").is_empty());
        record(root.ws_id, "r1", capture(b"x"));
        forget_workspace(root.ws_id);
        assert!(!responses_dir(root.ws_id).exists());
    }
}
