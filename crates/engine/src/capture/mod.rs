//! The engine's captured-traffic history (`capture.*`) and the capture to mock/request builders.

pub mod entities;
mod import;
pub mod store;

use serde_json::{Value, json};

use crate::{
    kernel::{CommandRegistry, ListenerId, bus},
    transport::event_pump::to_log_entry_event,
    utils::{
        cache_policy::{is_cache_disabled, set_cache_disabled},
        json::{array_of, bool_of, opt_str, str_of, truthy},
    },
};

pub use entities::{captured_to_har, captured_to_mock, captured_to_request};
pub use store::{
    MAX_CAPTURE_ENTRIES, clear_capture, get_capture, is_capture_paused, list_capture, record_capture, remove_capture,
    resolve_capture_entry, set_capture_paused,
};

pub fn register(registry: &mut CommandRegistry) {
    import::register(registry);

    registry.register_sync("capture.list", |params, _| {
        let ws_id = str_of(&params, "workspaceId");
        let (entries, matched) = filter_entries(list_capture(ws_id), &params);
        let entries: Vec<Value> =
            if truthy(&params, "summary") { entries.iter().map(to_log_entry_event).collect() } else { entries };
        Ok(json!({
            "entries": entries, "matched": matched, "paused": is_capture_paused(ws_id),
            "disableCache": is_cache_disabled()
        }))
    });

    registry.register_sync("capture.setDisableCache", |params, _| {
        set_cache_disabled(bool_of(&params, "disableCache", true));
        Ok(json!({ "disableCache": is_cache_disabled() }))
    });

    registry.register_sync("capture.setPaused", |params, _| {
        let ws_id = str_of(&params, "workspaceId");
        set_capture_paused(ws_id, bool_of(&params, "paused", false));
        Ok(json!({ "paused": is_capture_paused(ws_id) }))
    });

    registry.register_sync("capture.get", |params, _| {
        let entry = get_capture(str_of(&params, "workspaceId"), str_of(&params, "id"));
        Ok(json!({ "entry": entry }))
    });

    registry.register_sync("capture.clear", |params, _| {
        clear_capture(str_of(&params, "workspaceId"));
        Ok(json!({ "ok": true }))
    });

    registry.register_sync("capture.remove", |params, _| {
        let ids: Vec<String> =
            array_of(&params, "ids").iter().filter_map(|id| id.as_str().map(str::to_string)).collect();
        Ok(json!({ "ok": true, "removed": remove_capture(str_of(&params, "workspaceId"), &ids) }))
    });

    registry.register_sync("capture.buildMock", |params, _| {
        Ok(json!({ "mock": captured_to_mock(&resolve_capture_entry(&params)?) }))
    });
}

/// Whether `text` matches `pattern`, whose `*` stands for any run of characters. Case-insensitive.
fn glob(pattern: &str, text: &str) -> bool {
    let (pattern, text) = (pattern.to_lowercase(), text.to_lowercase());
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return text == pattern;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    let Some(mut rest) = text.strip_prefix(first) else { return false };
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// Whether `status` passes a filter of comma-separated terms: `422`, `4xx`, and `!` before either to exclude.
/// An entry passes when some wanted term matches (or there are none) and no excluded term does.
fn status_matches(filter: &str, status: Option<u64>) -> bool {
    let hit = |term: &str| match (term.strip_suffix("xx"), status) {
        (Some(class), Some(status)) => class.parse::<u64>().is_ok_and(|c| status / 100 == c),
        (None, Some(status)) => term.parse::<u64>().is_ok_and(|t| t == status),
        (_, None) => false,
    };
    let terms: Vec<&str> = filter.split(',').map(str::trim).filter(|t| !t.is_empty()).collect();
    let (excluded, wanted): (Vec<&str>, Vec<&str>) = terms.iter().partition(|t| t.starts_with('!'));
    (wanted.is_empty() || wanted.iter().any(|t| hit(t))) && !excluded.iter().any(|t| hit(&t[1..]))
}

/// The entries `capture.list` was asked for (newest first, as given) and how many matched before `last` cut them.
/// Filters: `status`, `method`, `host` and `path` (glob), `since` (ms), `via` and `mockId` (one or several, comma-separated), `grep` (the URL or a decoded body).
fn filter_entries(entries: Vec<Value>, params: &Value) -> (Vec<Value>, usize) {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let text_of = |entry: &Value, key: &str| {
        let bytes = STANDARD.decode(str_of(entry, key)).unwrap_or_default();
        String::from_utf8_lossy(&bytes).to_lowercase()
    };
    let grep = opt_str(params, "grep").map(str::to_lowercase);
    let mut kept: Vec<Value> = entries
        .into_iter()
        .filter(|entry| {
            let url = str_of(entry, "url");
            let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
            let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
            let path = format!("/{}", path.split(['?', '#']).next().unwrap_or_default());
            let host = authority.rsplit('@').next().unwrap_or_default();
            let host_ok = |pattern: &str| glob(pattern, host) || glob(pattern, host.split(':').next().unwrap_or(host));
            opt_str(params, "status").is_none_or(|f| status_matches(f, entry.get("status").and_then(Value::as_u64)))
                && opt_str(params, "method").is_none_or(|m| str_of(entry, "method").eq_ignore_ascii_case(m))
                && opt_str(params, "host").is_none_or(host_ok)
                && opt_str(params, "path").is_none_or(|p| glob(p, &path))
                && opt_str(params, "via").is_none_or(|v| v.split(',').any(|via| via == str_of(entry, "via")))
                && opt_str(params, "mockId").is_none_or(|ids| {
                    let answered = str_of(entry, "target").strip_prefix("mock:").unwrap_or("");
                    ids.split(',').any(|id| id == answered)
                })
                && params.get("since").and_then(Value::as_i64).is_none_or(|s| entry["ts"].as_i64().unwrap_or(0) >= s)
                && grep.as_deref().is_none_or(|g| {
                    url.to_lowercase().contains(g)
                        || text_of(entry, "reqBody").contains(g)
                        || text_of(entry, "resBody").contains(g)
                })
        })
        .collect();
    let matched = kept.len();
    if let Some(last) = params.get("last").and_then(Value::as_u64) {
        kept.truncate(last as usize);
    }
    (kept, matched)
}

/// Record every exchange the proxies report, whether or not a client is listening.
pub fn start_capture_recording() -> ListenerId {
    store::remember_runtime();
    bus().on("log.entry", record_capture)
}

/// Write pending capture history to disk now.
pub fn flush_capture() {
    store::flush_capture();
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{kernel::CommandContext, testing::scratch_data_root};

    async fn call(registry: &Arc<CommandRegistry>, action: &str, payload: Value) -> crate::protocol::Result<Value> {
        registry.invoke(action, payload, CommandContext::internal(registry.clone())).await
    }

    #[test]
    fn list_filters_by_status_method_host_path_since_via_and_text_then_cuts_to_last() {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let entry = |id: &str, ts: i64, method: &str, url: &str, status: Value, via: &str, req: &str, res: &str| {
            json!({ "id": id, "ts": ts, "method": method, "url": url, "status": status, "via": via,
                    "reqBody": STANDARD.encode(req), "resBody": STANDARD.encode(res) })
        };
        let all = vec![
            entry("e", 50, "GET", "http://api.localhost:8080/cart?x=1", json!(500), "proxy", "", "boom"),
            entry("d", 40, "POST", "http://api.localhost/orders", json!(422), "mock", "{\"Qty\":0}", ""),
            entry("c", 30, "GET", "http://auth.localhost/login", json!(200), "proxy", "", "ok"),
            entry("b", 20, "GET", "http://api.localhost/orders/7", json!(404), "mapping", "", ""),
            entry("a", 10, "GET", "http://down.localhost/", Value::Null, "error", "", ""),
        ];
        let ids = |params: Value| -> Vec<String> {
            filter_entries(all.clone(), &params).0.iter().map(|e| str_of(e, "id").to_string()).collect()
        };
        assert_eq!(ids(json!({})), ["e", "d", "c", "b", "a"]);
        assert_eq!(ids(json!({ "status": "422" })), ["d"]);
        assert_eq!(ids(json!({ "status": "4xx,5xx" })), ["e", "d", "b"]);
        assert_eq!(ids(json!({ "status": "!2xx" })), ["e", "d", "b", "a"]);
        assert_eq!(ids(json!({ "method": "post" })), ["d"]);
        assert_eq!(ids(json!({ "host": "api.localhost" })), ["e", "d", "b"]);
        assert_eq!(ids(json!({ "host": "*.localhost", "path": "/orders*" })), ["d", "b"]);
        assert_eq!(ids(json!({ "path": "/cart" })), ["e"]);
        assert_eq!(ids(json!({ "since": 30 })), ["e", "d", "c"]);
        assert_eq!(ids(json!({ "via": "mock" })), ["d"]);
        assert_eq!(ids(json!({ "via": "proxy,rfc6761" })), ["e", "c"]);
        assert_eq!(ids(json!({ "grep": "QTY" })), ["d"]);
        assert_eq!(ids(json!({ "grep": "boom" })), ["e"]);
        assert_eq!(ids(json!({ "grep": "login" })), ["c"]);
        // `last` counts after the other filters, and `matched` is what it cut from.
        let (kept, matched) = filter_entries(all.clone(), &json!({ "status": "!2xx", "last": 2 }));
        assert_eq!((kept.len(), matched), (2, 4));
        assert_eq!(str_of(&kept[0], "id"), "e");
    }

    #[test]
    fn list_filters_by_the_mock_that_answered_and_the_summary_names_it() {
        let entry = |id: &str, target: Value| json!({ "id": id, "via": "mock", "target": target });
        let all = vec![entry("c", json!("mock:m2")), entry("b", json!("proxy-target")), entry("a", json!("mock:m1"))];
        let ids = |params: Value| -> Vec<String> {
            filter_entries(all.clone(), &params).0.iter().map(|e| str_of(e, "id").to_string()).collect()
        };
        assert_eq!(ids(json!({ "mockId": "m1" })), ["a"]);
        assert_eq!(ids(json!({ "mockId": "m1,m2" })), ["c", "a"]);
        assert_eq!(to_log_entry_event(&all[0])["mockId"], "m2");
        assert!(to_log_entry_event(&all[1]).get("mockId").is_none());
    }

    #[tokio::test]
    async fn commands_read_and_change_the_history() {
        let _root = scratch_data_root();
        store::reset_capture();
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        let registry = Arc::new(registry);
        let listener = start_capture_recording();

        for id in ["a", "b"] {
            bus().emit(
                "log.entry",
                json!({ "id": id, "ts": 5, "method": "GET", "url": "http://h/", "status": 200, "workspaceId": "ws1",
                         "resHeaders": { "Content-Type": "text/plain" }, "resBody": "aGk=", "reqBody": "" }),
            );
        }
        bus().off(listener);

        let full = call(&registry, "capture.list", json!({ "workspaceId": "ws1" })).await.unwrap();
        assert_eq!(full["entries"].as_array().unwrap().len(), 2);
        assert_eq!(full["entries"][0]["id"], "b");
        assert_eq!(full["paused"], false);
        assert!(full["entries"][0].get("resBody").is_some());

        let summary = call(&registry, "capture.list", json!({ "workspaceId": "ws1", "summary": true })).await.unwrap();
        assert_eq!(summary["entries"][0]["contentType"], "text/plain");
        assert!(summary["entries"][0].get("resBody").is_none());

        let got = call(&registry, "capture.get", json!({ "workspaceId": "ws1", "id": "a" })).await.unwrap();
        assert_eq!(got["entry"]["id"], "a");
        let none = call(&registry, "capture.get", json!({ "workspaceId": "ws1", "id": "q" })).await.unwrap();
        assert_eq!(none, json!({ "entry": null }));

        let mock = call(&registry, "capture.buildMock", json!({ "workspaceId": "ws1", "id": "a" })).await.unwrap();
        assert_eq!(mock["mock"]["responseBody"], "hi");
        let inline =
            call(&registry, "capture.buildMock", json!({ "entry": { "method": "GET", "url": "u" } })).await.unwrap();
        assert_eq!(inline["mock"]["urlPattern"], "u");
        let missing =
            call(&registry, "capture.buildMock", json!({ "workspaceId": "ws1", "id": "zz" })).await.unwrap_err();
        assert_eq!(missing.code, crate::protocol::ErrorCode::NotFound);

        let paused =
            call(&registry, "capture.setPaused", json!({ "workspaceId": "ws1", "paused": true })).await.unwrap();
        assert_eq!(paused, json!({ "paused": true }));
        call(&registry, "capture.setPaused", json!({ "workspaceId": "ws1", "paused": false })).await.unwrap();

        let removed =
            call(&registry, "capture.remove", json!({ "workspaceId": "ws1", "ids": ["a", "nope"] })).await.unwrap();
        assert_eq!(removed, json!({ "ok": true, "removed": 1 }));
        let cleared = call(&registry, "capture.clear", json!({ "workspaceId": "ws1" })).await.unwrap();
        assert_eq!(cleared, json!({ "ok": true }));
        assert!(list_capture("ws1").is_empty());

        let cache = call(&registry, "capture.setDisableCache", json!({ "disableCache": false })).await.unwrap();
        assert_eq!(cache, json!({ "disableCache": false }));
        call(&registry, "capture.setDisableCache", json!({ "disableCache": true })).await.unwrap();
        store::reset_capture();
    }
}
