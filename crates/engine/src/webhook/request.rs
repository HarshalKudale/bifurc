//! The `webhook` request type: a saved payload that `webhook.send` signs with a vault secret and POSTs to a URL.
//! `webhook.template` hands the editor a predefined payload, with the headers its sender uses.

use std::time::Instant;

use serde_json::{Value, json};

use super::{signing, templates};
use crate::{
    env::{load_merged_environment, resolve_headers, resolve_vars, send_scoped, with_overrides},
    kernel::CommandRegistry,
    protocol::{EngineError, ErrorCode, Result},
    rest::direct_http::{exchange_with, header_pairs},
    utils::{
        http::{RequestSettings, send_failure},
        json::{opt_str, str_of},
    },
    workspace::{config::load_entity, response_history},
};

const KIND: &str = "webhookRequests";

fn failure(message: String) -> EngineError {
    send_failure(ErrorCode::EngineError, "webhook.failed", message)
}

pub(super) fn unix_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

/// The secret a webhook is signed with, from the unsealed vault `vault`: the one called `name`, else the only one
/// the vault holds, else the one called `hint`. No vault, no signature.
pub(super) async fn secret(
    workspace_id: &str,
    vault: Option<&str>,
    name: Option<&str>,
    hint: Option<&str>,
) -> Result<Option<String>> {
    let Some(vault) = vault else {
        return match name {
            Some(_) => Err(EngineError::bad_request("secret needs vault to read the secret from")),
            None => Ok(None),
        };
    };
    let mut secrets = crate::vault::secrets_by_name(workspace_id, vault).await?;
    let position = match (name, secrets.as_slice(), hint) {
        (Some(name), _, _) => secrets.iter().position(|(key, _)| key == name),
        (None, [_], _) => Some(0),
        (None, _, Some(hint)) => secrets.iter().position(|(key, _)| key == hint),
        (None, _, None) => None,
    };
    match (position, name) {
        (Some(at), _) => Ok(Some(secrets.swap_remove(at).1)),
        (None, Some(name)) => Err(EngineError::not_found(format!(
            "no secret \"{name}\" in vault \"{vault}\" (pick another with --secret)"
        ))),
        (None, None) if secrets.is_empty() => {
            Err(EngineError::not_found(format!("vault \"{vault}\" holds no secrets")))
        }
        (None, None) => {
            let names: Vec<&str> = secrets.iter().map(|(key, _)| key.as_str()).collect();
            Err(EngineError::bad_request(format!(
                "vault \"{vault}\" holds several secrets; name one: {}",
                names.join(", ")
            )))
        }
    }
}

/// The default secret name of a sender: `STRIPE_WEBHOOK_SECRET` for `stripe`.
pub(super) fn secret_hint(provider: &str) -> String {
    format!("{}_WEBHOOK_SECRET", provider.to_uppercase())
}

/// A text field of the request that is filled in: an editor saves an empty one as "".
fn filled<'a>(request: &'a Value, key: &str) -> Option<&'a str> {
    opt_str(request, key).map(str::trim).filter(|text| !text.is_empty())
}

/// POST the saved or inline webhook: `{{variables}}` resolved, the body signed exactly as sent.
async fn send(params: Value, _: crate::kernel::CommandContext) -> Result<Value> {
    send_scoped(&params.clone(), async move {
        let ws_id = str_of(&params, "workspaceId");
        let stored = match opt_str(&params, "id") {
            Some(id) => load_entity(ws_id, KIND, id).ok_or_else(|| {
                EngineError::not_found(format!("No saved {KIND} entity \"{id}\" in workspace \"{ws_id}\"."))
                    .keyed("request.saved_not_found", json!({ "kind": KIND, "id": id, "workspace": ws_id }))
            })?,
            None => params.get("request").cloned().unwrap_or(Value::Null),
        };
        let stored = with_overrides(stored, &params);
        let env = load_merged_environment(ws_id);
        let env = env.as_ref();
        let url = resolve_vars(str_of(&stored, "url"), env);
        let body = resolve_vars(str_of(&stored, "payload"), env);
        crate::workspace::policy::guard("POST", &url).await?;
        let signing_secret = secret(ws_id, filled(&stored, "vault"), filled(&stored, "secretName"), None).await?;
        let mut headers = header_pairs(&resolve_headers(stored.get("headers").unwrap_or(&Value::Null), env));
        if !headers.iter().any(|(name, _)| name.eq_ignore_ascii_case("content-type")) {
            headers.push(("Content-Type".into(), "application/json".into()));
        }
        if let Some(secret) = &signing_secret {
            let name = filled(&stored, "signatureHeader").unwrap_or(signing::DEFAULT_SIGNATURE_HEADER);
            headers.retain(|(own, _)| !own.eq_ignore_ascii_case(name));
            headers.push((name.to_string(), signing::signature(name, body.as_bytes(), secret, unix_secs())));
        }
        let settings = RequestSettings::resolve(stored.get("settings"));
        let start = Instant::now();
        let outcome = crate::cookies::scoped(ws_id, async {
            exchange_with("POST", &url, &headers, Some(&body), &settings).await.map_err(failure)
        })
        .await
        .map(|response| {
            json!({
                "status": response.status,
                "headers": response.headers,
                "body": response.body,
                "durationMs": start.elapsed().as_millis() as u64,
                "sentHeaders": headers.iter().cloned().collect::<std::collections::BTreeMap<_, _>>(),
            })
        });
        response_history::record_text(ws_id, &params, "POST", &url, &outcome);
        let mut result = outcome?;
        crate::vault::redact_value(ws_id, &mut result);
        Ok(result)
    })
    .await
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register("webhook.send", send);

    registry.register_sync("webhook.template", |params, _| {
        let template = templates::find(str_of(&params, "name")).map_err(EngineError::bad_request)?;
        let now = unix_secs();
        let payload = template.payload(now);
        let text = serde_json::to_string_pretty(&payload).map_err(|err| EngineError::bad_request(err.to_string()))?;
        let headers: serde_json::Map<String, Value> =
            template.headers(now).into_iter().map(|(name, value)| (name, json!(value))).collect();
        Ok(json!({
            "name": template.name,
            "about": template.about,
            "headers": headers,
            "signatureHeader": template.signature_header,
            "payload": text,
        }))
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;
    use crate::{
        kernel::CommandContext,
        testing::{ScratchRoot, scratch_data_root},
        workspace::fs::write_entity,
    };

    fn registry() -> Arc<CommandRegistry> {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
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

    /// A workspace with an unsealed vault `hooks` holding `secrets`.
    async fn vaulted(registry: &Arc<CommandRegistry>, secrets: &[(&str, &str)]) -> (ScratchRoot, Value) {
        let root = scratch_data_root();
        let made = call(
            registry,
            "vault.create",
            json!({"workspaceId": root.ws_id, "name": "hooks", "passphrase": "correct horse battery"}),
        )
        .await
        .unwrap();
        for (name, value) in secrets {
            let secret = json!({"workspaceId": root.ws_id, "id": made["id"], "name": name, "value": value});
            call(registry, "vault.setSecret", secret).await.unwrap();
        }
        (root, made["id"].clone())
    }

    #[tokio::test]
    async fn a_saved_webhook_is_signed_by_its_header_name_over_its_exact_body() {
        let registry = registry();
        let (root, _) = vaulted(&registry, &[("ONLY_SECRET", "whsec_abc")]).await;
        let (port, got) = catcher().await;
        let payload = "{\n  \"id\": \"evt_{{n}}\",   \"type\": \"x\"\n}";
        write_entity(
            root.ws_id,
            KIND,
            "w1",
            &json!({ "id": "w1", "name": "w", "url": format!("http://127.0.0.1:{port}/in"), "payload": payload,
                "headers": { "X-Extra": "1" }, "vault": "hooks", "signatureHeader": "Stripe-Signature" }),
            None,
        )
        .unwrap();
        let sent = call(
            &registry,
            "webhook.send",
            json!({ "workspaceId": root.ws_id, "id": "w1", "variables": { "n": "42" } }),
        )
        .await
        .unwrap();
        assert_eq!(sent["status"], 200);
        let (head, body) = got.await.unwrap();
        assert!(head.starts_with("post /in "), "{head}");
        assert_eq!(body, payload.replace("{{n}}", "42").into_bytes());
        assert_eq!(header(&head, "x-extra"), "1");
        assert_eq!(header(&head, "content-type"), "application/json");
        let (t, v1) = header(&head, "stripe-signature").strip_prefix("t=").unwrap().split_once(",v1=").unwrap();
        let mut signed = format!("{t}.").into_bytes();
        signed.extend(&body);
        assert_eq!(v1, signing::hmac_hex(b"whsec_abc", &signed));
        assert_eq!(sent["sentHeaders"]["Stripe-Signature"], header(&head, "stripe-signature"));
    }

    #[tokio::test]
    async fn another_header_gets_the_prefixed_hex_and_no_vault_means_no_signature() {
        let registry = registry();
        let (root, _) = vaulted(&registry, &[("A_SECRET", "first-secret"), ("MY_SECRET", "mine-secret")]).await;
        let (port, got) = catcher().await;
        let request = json!({ "url": format!("http://127.0.0.1:{port}/"), "payload": "{}", "vault": "hooks",
            "secretName": "MY_SECRET", "signatureHeader": "X-Hub-Signature-256", "headers": { "Content-Type": "text/plain" } });
        call(&registry, "webhook.send", json!({ "workspaceId": root.ws_id, "request": request })).await.unwrap();
        let (head, body) = got.await.unwrap();
        assert_eq!(
            header(&head, "x-hub-signature-256"),
            format!("sha256={}", signing::hmac_hex(b"mine-secret", &body))
        );
        assert_eq!(header(&head, "content-type"), "text/plain");

        let (port, got) = catcher().await;
        let plain = json!({ "url": format!("http://127.0.0.1:{port}/"), "payload": "{}" });
        call(&registry, "webhook.send", json!({ "workspaceId": root.ws_id, "request": plain })).await.unwrap();
        let (head, _) = got.await.unwrap();
        assert!(!head.contains("signature"));
    }

    #[tokio::test]
    async fn a_vault_without_a_secret_name_gives_its_only_secret_and_otherwise_asks() {
        let registry = registry();
        let (root, id) = vaulted(&registry, &[("A_SECRET", "first-secret"), ("B_SECRET", "second-secret")]).await;
        let ws = root.ws_id;
        let asked = secret(ws, Some("hooks"), None, None).await.unwrap_err();
        assert_eq!(asked.code, ErrorCode::BadRequest);
        assert!(asked.message.contains("A_SECRET") && asked.message.contains("B_SECRET"), "{}", asked.message);
        assert_eq!(secret(ws, Some("hooks"), None, Some("B_SECRET")).await.unwrap().as_deref(), Some("second-secret"));
        assert_eq!(
            secret(ws, Some("hooks"), Some("A_SECRET"), Some("B_SECRET")).await.unwrap().as_deref(),
            Some("first-secret")
        );
        assert_eq!(secret(ws, None, None, None).await.unwrap(), None);
        assert_eq!(secret(ws, None, Some("A_SECRET"), None).await.unwrap_err().code, ErrorCode::BadRequest);
        assert_eq!(secret(ws, Some("hooks"), Some("NOPE"), None).await.unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(secret(ws, Some("nope"), None, None).await.unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(secret_hint("stripe"), "STRIPE_WEBHOOK_SECRET");

        call(&registry, "vault.deleteSecret", json!({"workspaceId": ws, "id": id, "name": "B_SECRET"})).await.unwrap();
        assert_eq!(secret(ws, Some("hooks"), None, None).await.unwrap().as_deref(), Some("first-secret"));
        call(&registry, "vault.seal", json!({"workspaceId": ws, "id": id})).await.unwrap();
        assert_eq!(secret(ws, Some("hooks"), None, None).await.unwrap_err().code, ErrorCode::Forbidden);
    }

    #[tokio::test]
    async fn a_missing_saved_webhook_is_not_found() {
        let registry = registry();
        let root = scratch_data_root();
        let none = call(&registry, "webhook.send", json!({ "workspaceId": root.ws_id, "id": "nope" })).await;
        assert_eq!(none.unwrap_err().code, ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn a_template_hands_over_its_payload_headers_and_signature_header() {
        let registry = registry();
        let got = call(&registry, "webhook.template", json!({ "name": "github.ping" })).await.unwrap();
        assert_eq!(got["signatureHeader"], "X-Hub-Signature-256");
        assert_eq!(got["headers"]["X-GitHub-Event"], "ping");
        let payload: Value = serde_json::from_str(got["payload"].as_str().unwrap()).unwrap();
        assert!(payload["zen"].is_string());
        let bare = call(&registry, "webhook.template", json!({ "name": "stripe" })).await.unwrap();
        assert_eq!(bare["name"], "stripe.checkout.session.completed");
        let ambiguous = call(&registry, "webhook.template", json!({ "name": "github.p" })).await.unwrap_err();
        assert_eq!(ambiguous.code, ErrorCode::BadRequest);
        let unsigned = call(&registry, "webhook.template", json!({ "name": "discord" })).await.unwrap();
        assert_eq!(unsigned["signatureHeader"], "");
    }
}
