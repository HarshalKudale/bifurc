//! `request.replay`, `request.parseCurl`, `curl.buildRequest`, `curl.buildMock` and `capture.buildRequest`.

use serde_json::{Value, json};

use crate::{
    capture::{captured_to_request, resolve_capture_entry},
    kernel::{CommandRegistry, telemetry::Headers},
    protocol::EngineError,
    rest::from_curl::{curl_to_mock, curl_to_request},
    utils::{
        curl::parse_curl,
        json::{str_of, string_map},
        replay::replay_request,
    },
};

pub fn register(registry: &mut CommandRegistry) {
    // The result carries no `durationMs` although the protocol declares it; clients time the call themselves.
    registry.register("request.replay", |params: Value, _| async move {
        let headers: Headers = string_map(&params, "headers").into_iter().collect();
        let response =
            replay_request(str_of(&params, "method"), str_of(&params, "url"), &headers, str_of(&params, "body"))
                .await
                .map_err(EngineError::engine)?;
        Ok(json!({ "status": response.status, "headers": response.headers, "body": response.body }))
    });

    // The parse is returned unfiltered; dropping hop-by-hop headers is the caller's decision.
    registry.register_sync("request.parseCurl", |params, _| Ok(parse_curl(str_of(&params, "curl"))));

    // A pasted command as the request it describes, of the type it turns out to be, or as a mock of its
    // call. Text that is not a usable cURL command is refused with the reason.
    registry.register_sync("curl.buildRequest", |params, _| {
        curl_to_request(str_of(&params, "curl")).map_err(EngineError::bad_request)
    });
    registry.register_sync("curl.buildMock", |params, _| {
        Ok(json!({ "mock": curl_to_mock(str_of(&params, "curl")).map_err(EngineError::bad_request)? }))
    });

    registry.register_sync("capture.buildRequest", |params, _| {
        Ok(json!({ "request": captured_to_request(&resolve_capture_entry(&params)?) }))
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::{io::AsyncWriteExt, net::TcpListener};

    use super::*;
    use crate::{kernel::CommandContext, protocol::ErrorCode};

    fn registry() -> Arc<CommandRegistry> {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        Arc::new(registry)
    }

    async fn call(registry: &Arc<CommandRegistry>, action: &str, payload: Value) -> crate::protocol::Result<Value> {
        registry.invoke(action, payload, CommandContext::internal(registry.clone())).await
    }

    #[tokio::test]
    async fn parse_curl_command() {
        let out = call(&registry(), "request.parseCurl", json!({ "curl": "curl -H 'A: b' http://h/" })).await.unwrap();
        assert_eq!(out, json!({ "method": "GET", "url": "http://h/", "headers": { "a": "b" }, "body": "" }));
    }

    #[tokio::test]
    async fn build_from_a_curl_command() {
        let registry = registry();
        let curl = "curl https://h.test/a -H 'A: b'";
        let out = call(&registry, "curl.buildRequest", json!({ "curl": curl })).await.unwrap();
        assert_eq!(out["kind"], "requests");
        assert_eq!(out["request"]["url"], "https://h.test/a");
        assert_eq!(out["request"]["headers"], json!({ "A": "b" }));
        let out = call(&registry, "curl.buildMock", json!({ "curl": curl })).await.unwrap();
        assert_eq!(out["mock"]["urlPattern"], "https://h.test/a");
        for action in ["curl.buildRequest", "curl.buildMock"] {
            let refused = call(&registry, action, json!({ "curl": "ls -la" })).await.unwrap_err();
            assert_eq!(refused.code, ErrorCode::BadRequest, "{action}");
        }
    }

    #[tokio::test]
    async fn build_request_from_an_inline_entry() {
        let entry =
            json!({ "method": "GET", "url": "http://h/", "reqHeaders": { "Host": "h", "A": "1" }, "reqBody": "" });
        let out = call(&registry(), "capture.buildRequest", json!({ "entry": entry })).await.unwrap();
        assert_eq!(
            out,
            json!({ "request": { "name": "", "method": "GET", "url": "http://h/", "headers": { "A": "1" }, "body": "" } })
        );
    }

    #[tokio::test]
    async fn replay_returns_the_response_and_failures_are_engine_errors() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
            socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nx-a: 1\r\n\r\nok").await.unwrap();
        });
        let registry = registry();
        let payload = json!({ "method": "GET", "url": format!("http://127.0.0.1:{port}/"), "headers": {}, "body": "" });
        let out = call(&registry, "request.replay", payload).await.unwrap();
        assert_eq!(out, json!({ "status": 200, "headers": { "content-length": "2", "x-a": "1" }, "body": "b2s=" }));

        let bad = json!({ "method": "GET", "url": "nope", "headers": {}, "body": "" });
        let err = call(&registry, "request.replay", bad).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::EngineError);
        assert_eq!(err.message, "Invalid URL");
    }
}
