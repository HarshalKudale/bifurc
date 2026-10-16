//! cURL to entity construction: turning a pasted cURL command into a request of the type it describes, or
//! into a mock of the call it makes.
//!
//! Both functions return the entity body, ready for `entity.create`. Which request type a command is
//! (REST, GraphQL, SOAP or WebSocket) is decided here, so every interface opens the same command the same way.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};

use crate::utils::curl::{CurlRequest, is_skipped_header, parse_curl_strict};

/// The `EntityKind` of each request type a command can turn out to be.
const REST: &str = "requests";
const GRAPHQL: &str = "graphqlRequests";
const SOAP: &str = "soapRequests";
const WEBSOCKET: &str = "wsConnections";
const SSE: &str = "sseRequests";

/// The command's headers as an entity stores them: hop-by-hop ones dropped, and the ones in `owned` too,
/// which the request type sets itself.
fn headers_without(request: &CurlRequest, owned: &[&str]) -> Map<String, Value> {
    let mut headers = Map::new();
    for (name, value) in &request.headers {
        let lower = name.to_lowercase();
        if !is_skipped_header(&lower) && !owned.contains(&lower.as_str()) {
            headers.insert(name.clone(), json!(value));
        }
    }
    headers
}

/// What a URL is called in a name: its path, or its host when it has none.
fn target_of(url: &str) -> String {
    let bare = url.split(['?', '#']).next().unwrap_or(url);
    let rest = bare.split_once("://").map_or(bare, |(_, rest)| rest);
    match rest.split_once('/') {
        Some((_, path)) if !path.is_empty() => format!("/{path}"),
        _ => rest.trim_end_matches('/').to_string(),
    }
}

/// The query, variables and operation name of a GraphQL call, when the body is one.
fn graphql_of(request: &CurlRequest) -> Option<(String, String, String)> {
    let content_type = request.header("content-type").unwrap_or("").to_lowercase();
    let looks_like_query = |text: &str| {
        let text = text.trim_start();
        ["query", "mutation", "subscription", "fragment", "{"].iter().any(|start| text.starts_with(start))
    };
    if content_type.starts_with("application/graphql") && looks_like_query(&request.body) {
        return Some((request.body.clone(), String::new(), String::new()));
    }
    let body: Value = serde_json::from_str(&request.body).ok()?;
    let query = body.get("query").and_then(Value::as_str).filter(|query| looks_like_query(query))?;
    let variables = match body.get("variables") {
        Some(variables) if variables.as_object().is_some_and(|map| !map.is_empty()) => {
            serde_json::to_string_pretty(variables).unwrap_or_default()
        }
        _ => String::new(),
    };
    let operation = body.get("operationName").and_then(Value::as_str).unwrap_or("").to_string();
    Some((query.to_string(), variables, operation))
}

fn is_soap(request: &CurlRequest) -> bool {
    let content_type = request.header("content-type").unwrap_or("").to_lowercase();
    let envelope = request.body.contains("schemas.xmlsoap.org/soap/envelope")
        || request.body.contains("www.w3.org/2003/05/soap-envelope");
    request.header("soapaction").is_some()
        || content_type.starts_with("application/soap+xml")
        || (content_type.contains("xml") && envelope)
}

fn is_websocket(request: &CurlRequest) -> bool {
    request.url.starts_with("ws://")
        || request.url.starts_with("wss://")
        || request.header("upgrade").is_some_and(|upgrade| upgrade.eq_ignore_ascii_case("websocket"))
}

fn websocket_entity(request: &CurlRequest) -> Value {
    // A handshake written for curl is an HTTP call; the connection it asks for has a WebSocket address.
    let url = match request.url.split_once("://") {
        Some(("http", rest)) => format!("ws://{rest}"),
        Some(("https", rest)) => format!("wss://{rest}"),
        _ => request.url.clone(),
    };
    let handshake = ["upgrade", "sec-websocket-key", "sec-websocket-version", "sec-websocket-extensions"];
    json!({
        "name": format!("WebSocket {}", target_of(&url)),
        "url": url,
        "headers": headers_without(request, &handshake),
    })
}

fn soap_entity(request: &CurlRequest) -> Value {
    let action = request.header("soapaction").unwrap_or("").trim().trim_matches('"');
    json!({
        "name": format!("SOAP {}", target_of(&request.url)),
        "protocol": "soap",
        "endpointUrl": request.url,
        "soapAction": action,
        "headers": headers_without(request, &["soapaction", "content-type"]),
        "body": request.body,
    })
}

fn graphql_entity(request: &CurlRequest, (query, variables, operation): (String, String, String)) -> Value {
    let name = if operation.is_empty() { format!("GraphQL {}", target_of(&request.url)) } else { operation.clone() };
    json!({
        "name": name,
        "protocol": "graphql",
        "endpointUrl": request.url,
        "operationName": operation,
        "query": query,
        "variables": variables,
        "headers": headers_without(request, &["content-type"]),
    })
}

fn rest_entity(request: &CurlRequest) -> Value {
    let mut headers = headers_without(request, &[]);
    let has_content_type = request.header("content-type").is_some();
    let mut out = Map::new();
    out.insert("name".into(), json!(format!("{} {}", request.method, target_of(&request.url))));
    out.insert("protocol".into(), json!("rest"));
    out.insert("method".into(), json!(request.method));
    out.insert("url".into(), json!(request.url));
    if !request.parts.is_empty() {
        if !has_content_type {
            headers.insert("Content-Type".into(), json!("multipart/form-data"));
        }
        let parts: Vec<Value> = request
            .parts
            .iter()
            .map(|part| {
                if part.file {
                    // The file is on the machine the command was written for; the field is kept, empty.
                    json!({
                        "name": part.name, "kind": "file", "value": "",
                        "fileName": part.file_name, "contentType": part.content_type,
                    })
                } else {
                    json!({ "name": part.name, "kind": "text", "value": part.value })
                }
            })
            .collect();
        out.insert("headers".into(), Value::Object(headers));
        out.insert("body".into(), json!(""));
        out.insert("bodyParts".into(), Value::Array(parts));
    } else {
        // curl sends data as a form unless a header says otherwise.
        if !request.body.is_empty() && !has_content_type {
            headers.insert("Content-Type".into(), json!("application/x-www-form-urlencoded"));
        }
        out.insert("headers".into(), Value::Object(headers));
        out.insert("body".into(), json!(request.body));
    }
    Value::Object(out)
}

/// A cURL command as the request it describes: `{ kind, request }`, where `kind` is the `EntityKind` the
/// request is created as. The error says why the text is not a usable cURL command.
pub fn curl_to_request(curl: &str) -> Result<Value, &'static str> {
    let (kind, entity) = request_entity(&parse_curl_strict(curl)?);
    Ok(json!({ "kind": kind, "request": entity }))
}

/// An HTTP call as the request of the type it describes, with the `EntityKind` it is created as. The
/// importers build their requests through this, so a call is the same type wherever it came from.
pub fn request_entity(request: &CurlRequest) -> (&'static str, Value) {
    if is_websocket(request) {
        (WEBSOCKET, websocket_entity(request))
    } else if is_soap(request) {
        (SOAP, soap_entity(request))
    } else if let Some(graphql) = graphql_of(request) {
        (GRAPHQL, graphql_entity(request, graphql))
    } else if request.header("accept").is_some_and(|accept| accept.to_lowercase().contains("text/event-stream")) {
        let mut entity = rest_entity(request);
        if let Some(fields) = entity.as_object_mut() {
            fields.shift_remove("protocol");
        }
        (SSE, entity)
    } else {
        (REST, rest_entity(request))
    }
}

/// A cURL command as a mock of the call it makes. The command says nothing about the answer, so the mock
/// starts with an empty `200`; the call's headers and body are kept as the captured request.
pub fn curl_to_mock(curl: &str) -> Result<Value, &'static str> {
    let request = parse_curl_strict(curl)?;
    Ok(json!({
        "name": format!("{} {}", request.method, target_of(&request.url)),
        "method": request.method,
        "urlPattern": request.url,
        "useRegex": false,
        "capturedHeaders": headers_without(&request, &[]),
        "capturedBody": STANDARD.encode(&request.body),
        "responseStatus": 200,
        "responseStatusMocked": true,
        "responseHeaders": {},
        "mockedResponseHeaders": [],
        "responseBody": "",
        "responseBodyMocked": true,
        "responseDelayMocked": true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(curl: &str) -> (String, Value) {
        let out = curl_to_request(curl).unwrap();
        (out["kind"].as_str().unwrap().to_string(), out["request"].clone())
    }

    #[test]
    fn a_plain_call_is_a_rest_request() {
        let (kind, request) = built(
            "curl -X PUT https://api.test/users/7?full=1 -H 'Host: api.test' -H 'X-A: 1' -d '{\"a\":1}' -H 'Content-Type: application/json'",
        );
        assert_eq!(kind, "requests");
        assert_eq!(
            request,
            json!({
                "name": "PUT /users/7", "protocol": "rest", "method": "PUT", "url": "https://api.test/users/7?full=1",
                "headers": { "X-A": "1", "Content-Type": "application/json" }, "body": "{\"a\":1}",
            })
        );
    }

    #[test]
    fn data_without_a_type_is_a_form_and_a_bare_host_is_named_by_it() {
        let (_, request) = built("curl api.test -d a=1 -d b=2");
        assert_eq!(request["name"], "POST api.test");
        assert_eq!(request["url"], "http://api.test");
        assert_eq!(request["headers"], json!({ "Content-Type": "application/x-www-form-urlencoded" }));
        assert_eq!(request["body"], "a=1&b=2");
    }

    #[test]
    fn form_fields_are_multipart_parts() {
        let (kind, request) = built("curl https://api.test/up -F note=hi -F 'file=@/tmp/a.png;type=image/png'");
        assert_eq!(kind, "requests");
        assert_eq!(request["method"], "POST");
        assert_eq!(request["headers"], json!({ "Content-Type": "multipart/form-data" }));
        assert_eq!(request["body"], "");
        assert_eq!(
            request["bodyParts"],
            json!([
                { "name": "note", "kind": "text", "value": "hi" },
                { "name": "file", "kind": "file", "value": "", "fileName": "a.png", "contentType": "image/png" },
            ])
        );
    }

    #[test]
    fn a_json_body_with_a_query_is_graphql() {
        let body = r#"{"operationName":"Users","query":"query Users($n: Int) { users(first: $n) { id } }","variables":{"n":2}}"#;
        let (kind, request) = built(&format!(
            "curl https://api.test/graphql -H 'content-type: application/json' -H 'Authorization: Bearer t' --data-raw '{body}'"
        ));
        assert_eq!(kind, "graphqlRequests");
        assert_eq!(request["name"], "Users");
        assert_eq!(request["protocol"], "graphql");
        assert_eq!(request["endpointUrl"], "https://api.test/graphql");
        assert_eq!(request["query"], "query Users($n: Int) { users(first: $n) { id } }");
        assert_eq!(request["variables"], "{\n  \"n\": 2\n}");
        assert_eq!(request["headers"], json!({ "Authorization": "Bearer t" }));

        // A JSON body that merely has a field called `query` is not GraphQL.
        let (kind, _) =
            built(r#"curl https://api.test/search -H 'Content-Type: application/json' -d '{"query":"shoes"}'"#);
        assert_eq!(kind, "requests");
        let (kind, request) = built("curl https://api.test/graphql --json '{\"query\":\"{ me { id } }\"}'");
        assert_eq!(kind, "graphqlRequests");
        assert_eq!(request["name"], "GraphQL /graphql");
        assert_eq!(request["variables"], "");
    }

    #[test]
    fn an_envelope_or_a_soap_action_is_soap() {
        let envelope =
            "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\"><soap:Body/></soap:Envelope>";
        let (kind, request) = built(&format!(
            "curl https://svc.test/ws -H 'Content-Type: text/xml; charset=utf-8' -H 'SOAPAction: \"urn:Get\"' -H 'X-A: 1' -d '{envelope}'"
        ));
        assert_eq!(kind, "soapRequests");
        assert_eq!(
            request,
            json!({
                "name": "SOAP /ws", "protocol": "soap", "endpointUrl": "https://svc.test/ws", "soapAction": "urn:Get",
                "headers": { "X-A": "1" }, "body": envelope,
            })
        );
        let (kind, _) = built(&format!("curl https://svc.test/ws -H 'Content-Type: text/xml' -d '{envelope}'"));
        assert_eq!(kind, "soapRequests");
        let (kind, _) = built("curl https://svc.test/ws -H 'Content-Type: text/xml' -d '<a/>'");
        assert_eq!(kind, "requests");
    }

    #[test]
    fn a_socket_address_or_an_upgrade_is_a_websocket() {
        let (kind, request) = built("curl wss://live.test/feed -H 'Authorization: Bearer t'");
        assert_eq!(kind, "wsConnections");
        assert_eq!(
            request,
            json!({ "name": "WebSocket /feed", "url": "wss://live.test/feed", "headers": { "Authorization": "Bearer t" } })
        );
        let (kind, request) = built(
            "curl -i -N https://live.test/feed -H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Sec-WebSocket-Key: k' -H 'Sec-WebSocket-Version: 13' -H 'Origin: https://live.test'",
        );
        assert_eq!(kind, "wsConnections");
        assert_eq!(request["url"], "wss://live.test/feed");
        assert_eq!(request["headers"], json!({ "Origin": "https://live.test" }));
    }

    #[test]
    fn a_mock_answers_the_call_the_command_makes() {
        let mock =
            curl_to_mock("curl -X POST https://api.test/orders?x=1 -H 'Host: api.test' -H 'X-A: 1' -d '{\"a\":1}'")
                .unwrap();
        assert_eq!(mock["name"], "POST /orders");
        assert_eq!(mock["method"], "POST");
        assert_eq!(mock["urlPattern"], "https://api.test/orders?x=1");
        assert_eq!(mock["capturedHeaders"], json!({ "X-A": "1" }));
        assert_eq!(mock["capturedBody"], json!(STANDARD.encode("{\"a\":1}")));
        assert_eq!(mock["responseStatus"], 200);
        let keys: Vec<&str> = mock.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "name",
                "method",
                "urlPattern",
                "useRegex",
                "capturedHeaders",
                "capturedBody",
                "responseStatus",
                "responseStatusMocked",
                "responseHeaders",
                "mockedResponseHeaders",
                "responseBody",
                "responseBodyMocked",
                "responseDelayMocked"
            ]
        );
    }

    #[test]
    fn text_that_is_not_a_command_is_refused() {
        assert!(curl_to_request("GET https://api.test/").is_err());
        assert!(curl_to_mock("curl -H 'A: 1'").is_err());
    }
}
