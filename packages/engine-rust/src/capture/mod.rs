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
        json::{array_of, bool_of, str_of, truthy},
    },
};

pub use entities::{captured_to_mock, captured_to_request};
pub use store::{
    MAX_CAPTURE_ENTRIES, clear_capture, get_capture, is_capture_paused, list_capture, record_capture, remove_capture,
    resolve_capture_entry, set_capture_paused,
};

pub fn register(registry: &mut CommandRegistry) {
    import::register(registry);

    registry.register_sync("capture.list", |params, _| {
        let ws_id = str_of(&params, "workspaceId");
        let entries = list_capture(ws_id);
        let entries: Vec<Value> =
            if truthy(&params, "summary") { entries.iter().map(to_log_entry_event).collect() } else { entries };
        Ok(json!({ "entries": entries, "paused": is_capture_paused(ws_id), "disableCache": is_cache_disabled() }))
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
