//! Sample webhook events (`webhook.fixtures`) and `webhook.trigger`: build one with fresh ids and times, sign it
//! with a secret the engine reads from the unsealed vault itself, and send it through `request.replay`.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use super::{
    RUNNING,
    request::{secret, secret_hint, unix_secs},
    signing, templates,
};
use crate::{
    kernel::{CommandContext, CommandRegistry},
    protocol::{EngineError, Result},
    utils::json::{opt_str, str_of},
};

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Set `new` at `path` (keys and array positions, negative from the end), creating missing object keys.
fn assign(current: &mut Value, path: &[Value], new: Value) -> Result<()> {
    let Some((step, rest)) = path.split_first() else {
        *current = new;
        return Ok(());
    };
    let found = kind(current);
    match (step.as_str(), step.as_i64()) {
        (Some(key), _) => {
            if current.is_null() {
                *current = json!({});
            }
            let map = current
                .as_object_mut()
                .ok_or_else(|| EngineError::bad_request(format!("cannot index {found} with \"{key}\"")))?;
            assign(map.entry(key).or_insert(Value::Null), rest, new)
        }
        (None, Some(i)) => {
            let items = current
                .as_array_mut()
                .ok_or_else(|| EngineError::bad_request(format!("cannot index {found} with {i}")))?;
            let at = if i < 0 { items.len() as i64 + i } else { i };
            let at = (0..items.len() as i64)
                .contains(&at)
                .then_some(at as usize)
                .ok_or_else(|| EngineError::bad_request(format!("index {i} is out of range")))?;
            assign(&mut items[at], rest, new)
        }
        _ => Err(EngineError::bad_request("a patch path is made of keys (text) and positions (integers)")),
    }
}

/// Where a trigger goes: `to`, else the listener `name`, else the one listener running in the workspace.
fn target(workspace_id: &str, to: Option<&str>, name: Option<&str>) -> Result<String> {
    if let Some(to) = to {
        return Ok(to.to_string());
    }
    let running = RUNNING.lock();
    let here: Vec<_> = running.iter().filter(|l| l.workspace_id == workspace_id).collect();
    let chosen = if let Some(name) = name {
        here.iter()
            .find(|l| l.name.as_deref() == Some(name))
            .ok_or_else(|| EngineError::not_found(format!("no webhook listener \"{name}\" is running")))?
    } else {
        match here.as_slice() {
            [] => {
                return Err(EngineError::bad_request(
                    "no webhook listener running; use --to <url> or bifurc webhooks listen",
                ));
            }
            [one] => one,
            _ => return Err(EngineError::bad_request("several listeners running; pick one with --name")),
        }
    };
    Ok(format!("http://127.0.0.1:{}/", chosen.port))
}

/// `webhook.trigger`: a fixture, patched, signed when `vault` is given, sent through `request.replay` so the
/// workspace policy still guards it. `patch` is a list of `{ path: [key | position, …], value }`.
async fn trigger(params: Value, ctx: CommandContext) -> Result<Value> {
    let workspace_id = str_of(&params, "workspaceId");
    let template = templates::find(str_of(&params, "fixture")).map_err(EngineError::bad_request)?;
    let now = unix_secs();
    let mut payload = template.payload(now);
    for edit in params["patch"].as_array().into_iter().flatten() {
        let path = edit["path"].as_array().ok_or_else(|| EngineError::bad_request("a patch needs a path array"))?;
        assign(&mut payload, path, edit["value"].clone())?;
    }
    let hint = secret_hint(template.provider());
    let signing_secret =
        secret(workspace_id, opt_str(&params, "vault"), opt_str(&params, "secret"), Some(&hint)).await?;
    let url = target(workspace_id, opt_str(&params, "to"), opt_str(&params, "name"))?;

    // These exact bytes are signed and sent.
    let body = serde_json::to_vec(&payload).map_err(|err| EngineError::bad_request(err.to_string()))?;
    let mut headers = template.headers(now);
    if let Some(secret) = &signing_secret {
        let name = match template.signature_header.as_str() {
            "" => signing::DEFAULT_SIGNATURE_HEADER,
            name => name,
        };
        headers.push((name.to_string(), signing::signature(name, &body, secret, now)));
    }
    let headers: serde_json::Map<String, Value> =
        headers.into_iter().map(|(name, value)| (name, json!(value))).collect();
    let sent = json!({ "method": "POST", "url": url, "headers": headers, "body": STANDARD.encode(&body) });
    ctx.registry.invoke("request.replay", sent, ctx.clone()).await
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("webhook.fixtures", |_, _| {
        let rows: Vec<Value> = templates::all()
            .iter()
            .map(|t| json!({ "name": t.name, "description": t.about, "signatureHeader": t.signature_header }))
            .collect();
        Ok(json!({ "fixtures": rows }))
    });
    registry.register("webhook.trigger", trigger);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;
    use crate::{protocol::ErrorCode, testing::scratch_data_root};

    fn registry() -> Arc<CommandRegistry> {
        let mut registry = CommandRegistry::new();
        super::super::register(&mut registry);
        crate::rest::commands::register(&mut registry);
        crate::vault::register(&mut registry);
        Arc::new(registry)
    }

    async fn call(registry: &Arc<CommandRegistry>, command: &str, params: Value) -> Result<Value> {
        let ctx = CommandContext { registry: registry.clone(), session: None };
        registry.invoke(command, params, ctx).await
    }

    /// A one-shot HTTP server: the head (lowercased) and body of the request it receives.
    async fn catcher() -> (u16, tokio::sync::oneshot::Receiver<(String, Vec<u8>)>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                got.extend_from_slice(&buf[..n]);
                if n == 0 {
                    break;
                }
                let Some(end) = got.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
                let head = String::from_utf8_lossy(&got[..end]).to_lowercase();
                let want: usize = head
                    .split("content-length: ")
                    .nth(1)
                    .and_then(|v| v.split("\r\n").next()?.parse().ok())
                    .unwrap_or(0);
                if got.len() >= end + 4 + want {
                    break;
                }
            }
            let _ = socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok").await;
            let end = got.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
            let _ = tx.send((String::from_utf8_lossy(&got[..end]).to_lowercase(), got[end + 4..].to_vec()));
        });
        (port, rx)
    }

    fn header<'h>(head: &'h str, name: &str) -> &'h str {
        head.split("\r\n").find_map(|line| line.strip_prefix(&format!("{name}: "))).unwrap()
    }

    fn hmac_hex(key: &[u8], data: &[u8]) -> String {
        signing::hmac_hex(key, data)
    }

    #[tokio::test]
    async fn fixtures_lists_every_template() {
        let listed = call(&registry(), "webhook.fixtures", json!({})).await.unwrap();
        let rows = listed["fixtures"].as_array().unwrap();
        let names: Vec<&str> = rows.iter().map(|f| f["name"].as_str().unwrap()).collect();
        assert_eq!(names, templates::all().iter().map(|t| t.name.as_str()).collect::<Vec<_>>());
        let push = rows.iter().find(|f| f["name"] == "github.push").unwrap();
        assert!(push["description"].as_str().unwrap().contains("push"));
        assert_eq!(push["signatureHeader"], "X-Hub-Signature-256");
    }

    /// A workspace with an unsealed vault `hooks` holding both signing secrets.
    async fn vaulted(registry: &Arc<CommandRegistry>) -> (crate::testing::ScratchRoot, Value) {
        let root = scratch_data_root();
        let made = call(
            registry,
            "vault.create",
            json!({"workspaceId": "ws1", "name": "hooks", "passphrase": "correct horse battery"}),
        )
        .await
        .unwrap();
        for (name, value) in [("STRIPE_WEBHOOK_SECRET", "whsec_abc"), ("GITHUB_WEBHOOK_SECRET", "gh-s3cret")] {
            let secret = json!({"workspaceId": "ws1", "id": made["id"], "name": name, "value": value});
            call(registry, "vault.setSecret", secret).await.unwrap();
        }
        (root, made["id"].clone())
    }

    #[tokio::test]
    async fn trigger_signs_stripe_and_github_with_the_vault_secret_and_applies_patches() {
        let registry = registry();
        let (_root, _) = vaulted(&registry).await;

        let (port, got) = catcher().await;
        let params = json!({ "workspaceId": "ws1", "fixture": "stripe.checkout", "vault": "hooks",
            "to": format!("http://127.0.0.1:{port}/in"),
            "patch": [{ "path": ["data", "object", "amount_total"], "value": 999 }] });
        let sent = call(&registry, "webhook.trigger", params).await.unwrap();
        assert_eq!(sent["status"], 200);
        let (head, body) = got.await.unwrap();
        assert!(head.starts_with("post /in "), "{head}");
        assert_eq!(header(&head, "content-type"), "application/json; charset=utf-8");
        let event: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(event["type"], "checkout.session.completed");
        assert_eq!(event["data"]["object"]["amount_total"], 999);
        assert!(event["id"].as_str().unwrap().starts_with("evt_"));
        let (t, v1) = header(&head, "stripe-signature").strip_prefix("t=").unwrap().split_once(",v1=").unwrap();
        let mut signed = format!("{t}.").into_bytes();
        signed.extend(&body);
        assert_eq!(v1, hmac_hex(b"whsec_abc", &signed));

        let (port, got) = catcher().await;
        let to = format!("http://127.0.0.1:{port}/");
        let params = json!({ "workspaceId": "ws1", "fixture": "github.push", "vault": "hooks", "to": to });
        call(&registry, "webhook.trigger", params).await.unwrap();
        let (head, body) = got.await.unwrap();
        assert_eq!(header(&head, "x-github-event"), "push");
        assert_eq!(header(&head, "x-hub-signature-256"), format!("sha256={}", hmac_hex(b"gh-s3cret", &body)));
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["ref"], "refs/heads/main");

        // Without a vault nothing is signed.
        let (port, got) = catcher().await;
        let to = format!("http://127.0.0.1:{port}/");
        call(&registry, "webhook.trigger", json!({ "workspaceId": "ws1", "fixture": "github.push", "to": to }))
            .await
            .unwrap();
        assert!(!got.await.unwrap().0.contains("x-hub-signature-256"));
    }

    #[tokio::test]
    async fn trigger_refuses_what_it_cannot_sign_or_find() {
        let registry = registry();
        let (_root, id) = vaulted(&registry).await;
        let refused = |params: Value| {
            let registry = registry.clone();
            async move { call(&registry, "webhook.trigger", params).await.unwrap_err() }
        };
        let with = |extra: Value| {
            let mut params = json!({ "workspaceId": "ws1", "fixture": "stripe.checkout", "to": "http://127.0.0.1:1/" });
            params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            params
        };
        assert_eq!(refused(with(json!({ "fixture": "paypal" }))).await.code, ErrorCode::BadRequest);
        assert_eq!(refused(with(json!({ "secret": "K" }))).await.code, ErrorCode::BadRequest);
        let missing = refused(with(json!({ "vault": "hooks", "secret": "NOPE" }))).await;
        assert_eq!(missing.code, ErrorCode::NotFound);
        assert!(missing.message.contains("NOPE") && missing.message.contains("hooks"), "{}", missing.message);
        assert_eq!(refused(with(json!({ "vault": "nope" }))).await.code, ErrorCode::NotFound);
        let bad = json!({ "patch": [{ "path": ["id", "x"], "value": 1 }] });
        assert_eq!(refused(with(bad)).await.code, ErrorCode::BadRequest);

        // No listener, none by that name: the CLI's texts.
        let none = json!({ "workspaceId": "ws-without-listeners", "fixture": "github.push" });
        let err = refused(none.clone()).await;
        assert_eq!(err.code, ErrorCode::BadRequest);
        assert!(err.message.starts_with("no webhook listener running; use --to <url>"), "{}", err.message);
        let mut named = none;
        named["name"] = json!("zz");
        assert_eq!(refused(named).await.code, ErrorCode::NotFound);

        // Sealed, the engine has no secret to sign with.
        call(&registry, "vault.seal", json!({"workspaceId": "ws1", "id": id})).await.unwrap();
        let sealed = refused(with(json!({ "vault": "hooks" }))).await;
        assert_eq!(sealed.code, ErrorCode::Forbidden);
        assert!(sealed.message.contains("sealed"), "{}", sealed.message);
    }

    #[tokio::test]
    async fn trigger_without_to_goes_to_the_named_listener_of_the_workspace() {
        let registry = registry();
        let started =
            call(&registry, "webhook.listen", json!({ "workspaceId": "ws1", "name": "trig-one", "detach": true }))
                .await
                .unwrap();
        // Other tests' listeners may run too: the name picks ours, and it answers 200 itself.
        let params = json!({ "workspaceId": "ws1", "fixture": "github.push", "name": "trig-one" });
        let sent = call(&registry, "webhook.trigger", params).await;
        call(&registry, "webhook.stop", json!({ "id": started["id"] })).await.unwrap();
        assert_eq!(sent.unwrap()["status"], 200);
    }
}
