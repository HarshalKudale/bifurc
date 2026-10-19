//! Sample webhook events (`webhook.fixtures`) and `webhook.trigger`: build one with fresh ids and times, sign it
//! with a secret the engine reads from the unsealed vault itself, and send it through `request.replay`.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ring::{
    hmac,
    rand::{SecureRandom, SystemRandom},
};
use serde_json::{Value, json};

use super::RUNNING;
use crate::{
    kernel::{CommandContext, CommandRegistry},
    protocol::{EngineError, Result},
    utils::json::{opt_str, str_of},
};

const FIXTURES: [(&str, &str); 2] = [
    ("stripe.checkout.session.completed", "A paid Stripe Checkout session (checkout.session.completed event)"),
    ("github.push", "A GitHub push event with one commit"),
];

const ALNUM: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const HEX: &str = "0123456789abcdef";

fn unix_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

/// `len` random characters of `alphabet`.
fn random_chars(alphabet: &str, len: usize) -> String {
    let chars: Vec<char> = alphabet.chars().collect();
    let mut bytes = vec![0u8; len];
    // If the system RNG fails the ids repeat one character: still valid fake data.
    let _ = SystemRandom::new().fill(&mut bytes);
    bytes.iter().map(|b| chars[usize::from(*b) % chars.len()]).collect()
}

/// `2026-10-18T09:30:00Z` for unix seconds.
fn format_time(secs: u64) -> String {
    let iso = crate::importexport::support::iso_from_ms(secs as i64 * 1000).unwrap_or_default();
    iso.replace(".000Z", "Z")
}

/// The event of a fixture, with fresh ids and times.
fn fixture(name: &str, now: u64) -> Value {
    if name == "github.push" {
        let (before, after) = (random_chars(HEX, 40), random_chars(HEX, 40));
        let commit = json!({ "id": after, "message": "Update README", "timestamp": format_time(now),
            "url": format!("https://github.com/octo/demo/commit/{after}"),
            "author": { "name": "Octo Cat", "email": "octocat@example.com", "username": "octocat" },
            "added": [], "removed": [], "modified": ["README.md"] });
        return json!({ "ref": "refs/heads/main", "before": before, "after": after,
            "repository": { "id": 100_000_001, "name": "demo", "full_name": "octo/demo", "private": false,
                "owner": { "login": "octo", "id": 1_000_001 }, "html_url": "https://github.com/octo/demo",
                "default_branch": "main" },
            "pusher": { "name": "octocat", "email": "octocat@example.com" },
            "sender": { "login": "octocat", "id": 583_231 },
            "created": false, "deleted": false, "forced": false,
            "compare": format!("https://github.com/octo/demo/compare/{}...{}", &before[..12], &after[..12]),
            "commits": [commit.clone()], "head_commit": commit });
    }
    json!({ "id": format!("evt_{}", random_chars(ALNUM, 24)), "object": "event", "api_version": "2024-06-20",
        "created": now, "type": "checkout.session.completed", "livemode": false, "pending_webhooks": 1,
        "request": { "id": null, "idempotency_key": null },
        "data": { "object": { "id": format!("cs_test_{}", random_chars(ALNUM, 40)), "object": "checkout.session",
            "amount_subtotal": 2000, "amount_total": 2000, "currency": "usd",
            "customer": format!("cus_{}", random_chars(ALNUM, 14)),
            "customer_details": { "email": "jenny@example.com", "name": "Jenny Rosen" },
            "mode": "payment", "payment_status": "paid", "status": "complete", "metadata": {},
            "payment_intent": format!("pi_{}", random_chars(ALNUM, 24)), "client_reference_id": null,
            "created": now, "success_url": "https://example.com/success" } } })
}

/// The fixture `wanted` names: exactly, or by the start of its name in any case (`stripe`).
fn resolve(wanted: &str) -> Result<&'static str> {
    let lower = wanted.to_lowercase();
    let hits: Vec<&str> = FIXTURES.iter().map(|(name, _)| *name).filter(|name| name.starts_with(&lower)).collect();
    match (FIXTURES.iter().find(|(name, _)| *name == wanted), hits.as_slice()) {
        (Some((name, _)), _) | (None, [name]) => Ok(name),
        _ => {
            let names: Vec<&str> = FIXTURES.iter().map(|(name, _)| *name).collect();
            Err(EngineError::bad_request(format!("no fixture \"{wanted}\"; fixtures: {}", names.join(", "))))
        }
    }
}

fn hmac_hex(key: &[u8], data: &[u8]) -> String {
    let tag = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data);
    tag.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

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
    let name = resolve(str_of(&params, "fixture"))?;
    let (vault, secret_name) = (opt_str(&params, "vault"), opt_str(&params, "secret"));
    if vault.is_none() && secret_name.is_some() {
        return Err(EngineError::bad_request("secret needs vault to read the secret from"));
    }
    let now = unix_secs();
    let mut payload = fixture(name, now);
    for edit in params["patch"].as_array().into_iter().flatten() {
        let path = edit["path"].as_array().ok_or_else(|| EngineError::bad_request("a patch needs a path array"))?;
        assign(&mut payload, path, edit["value"].clone())?;
    }
    let stripe = name.starts_with("stripe.");
    let key_name = secret_name.unwrap_or(if stripe { "STRIPE_WEBHOOK_SECRET" } else { "GITHUB_WEBHOOK_SECRET" });
    let secret = match vault {
        Some(vault) => {
            let secrets = crate::vault::secrets_by_name(workspace_id, vault).await?;
            let found = secrets.into_iter().find(|(key, _)| key == key_name).map(|(_, value)| value);
            Some(found.ok_or_else(|| {
                EngineError::not_found(format!(
                    "no secret \"{key_name}\" in vault \"{vault}\" (pick another with --secret)"
                ))
            })?)
        }
        None => None,
    };
    let url = target(workspace_id, opt_str(&params, "to"), opt_str(&params, "name"))?;

    // These exact bytes are signed and sent.
    let body = serde_json::to_vec(&payload).map_err(|err| EngineError::bad_request(err.to_string()))?;
    let mut headers = serde_json::Map::new();
    let mut put = |name: &str, value: String| headers.insert(name.to_string(), json!(value));
    if stripe {
        put("content-type", "application/json; charset=utf-8".into());
        put("user-agent", "Stripe/1.0 (+https://stripe.com/docs/webhooks)".into());
        if let Some(secret) = &secret {
            let mut signed = format!("{now}.").into_bytes();
            signed.extend(&body);
            put("stripe-signature", format!("t={now},v1={}", hmac_hex(secret.as_bytes(), &signed)));
        }
    } else {
        let id = [8, 4, 4, 4, 12].map(|n| random_chars(HEX, n)).join("-");
        put("content-type", "application/json".into());
        put("user-agent", "GitHub-Hookshot/bifurc".into());
        put("x-github-event", "push".into());
        put("x-github-delivery", id);
        put("x-github-hook-id", random_chars("0123456789", 9));
        if let Some(secret) = &secret {
            put("x-hub-signature-256", format!("sha256={}", hmac_hex(secret.as_bytes(), &body)));
        }
    }
    let sent = json!({ "method": "POST", "url": url, "headers": headers, "body": STANDARD.encode(&body) });
    ctx.registry.invoke("request.replay", sent, ctx.clone()).await
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("webhook.fixtures", |_, _| {
        let rows: Vec<Value> =
            FIXTURES.iter().map(|(name, about)| json!({ "name": name, "description": about })).collect();
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

    #[test]
    fn hmac_matches_rfc_4231() {
        assert_eq!(
            hmac_hex(b"Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(
            hmac_hex(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First"),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn times_read_like_the_cli_always_printed_them() {
        assert_eq!(format_time(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_time(1_760_779_800), "2025-10-18T09:30:00Z");
    }

    #[tokio::test]
    async fn fixtures_lists_both() {
        let listed = call(&registry(), "webhook.fixtures", json!({})).await.unwrap();
        let rows = listed["fixtures"].as_array().unwrap();
        let names: Vec<&str> = rows.iter().map(|f| f["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["stripe.checkout.session.completed", "github.push"]);
        assert!(rows[1]["description"].as_str().unwrap().contains("push"));
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
        let params = json!({ "workspaceId": "ws1", "fixture": "stripe", "vault": "hooks",
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
            let mut params = json!({ "workspaceId": "ws1", "fixture": "stripe", "to": "http://127.0.0.1:1/" });
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
