use std::sync::Arc;

use serde_json::{Value, json};

use super::*;
use crate::{
    env::{load_merged_environment, resolve_vars},
    kernel::SessionIdentity,
    testing::scratch_data_root,
};

struct Engine(Arc<CommandRegistry>);

impl Engine {
    fn new() -> Self {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        Self(Arc::new(registry))
    }

    async fn try_as(&self, session: Option<SessionIdentity>, action: &str, payload: Value) -> Result<Value> {
        let ctx = CommandContext { registry: self.0.clone(), session };
        self.0.invoke(action, payload, ctx).await
    }

    async fn call(&self, action: &str, payload: Value) -> Value {
        self.try_as(None, action, payload).await.unwrap_or_else(|e| panic!("{action} failed: {e:?}"))
    }
}

fn session(id: &str, client: &str) -> Option<SessionIdentity> {
    Some(SessionIdentity { session_id: id.into(), client_name: Some(client.into()), ..Default::default() })
}

const PASS: &str = "correct horse battery";

#[tokio::test]
async fn a_vault_resolves_in_text_but_its_values_never_reach_a_client() {
    let root = scratch_data_root();
    let (ws, engine) = (root.ws_id, Engine::new());

    let created = engine.call("vault.create", json!({ "workspaceId": ws, "name": "api_keys", "passphrase": PASS })).await;
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["state"], "ready");
    engine
        .call("vault.setSecret", json!({ "workspaceId": ws, "id": id, "name": "STRIPE", "value": "sk_live_123456" }))
        .await;

    let listed = engine.call("vault.list", json!({ "workspaceId": ws })).await;
    assert_eq!(listed[0]["secrets"], json!(["STRIPE"]));
    assert!(!listed.to_string().contains("sk_live_123456"));
    let on_disk = std::fs::read_to_string(crate::workspace::fs::entity_dir(ws, "vaults").join(format!("{id}.json"))).unwrap();
    assert!(!on_disk.contains("sk_live_123456"));

    let env = load_merged_environment(ws);
    assert_eq!(resolve_vars("Bearer {{api_keys.STRIPE}}", env.as_ref()), "Bearer sk_live_123456");
    assert_eq!(resolve_vars("{{api_keys.MISSING}} {{nope.X}}", env.as_ref()), "{{api_keys.MISSING}} {{nope.X}}");
    // The proxies' environment names no workspace, so mocks and rules never serve a secret.
    let proxy_env = json!({ "variables": [] });
    assert_eq!(resolve_vars("{{api_keys.STRIPE}}", Some(&proxy_env)), "{{api_keys.STRIPE}}");

    let redactor = Redactor::for_workspace(ws).expect("a ready vault");
    assert_eq!(redactor.text("token sk_live_123456!"), "token {{api_keys.STRIPE}}!");

    engine.call("vault.seal", json!({ "workspaceId": ws, "id": id })).await;
    assert_eq!(resolve_vars("{{api_keys.STRIPE}}", env.as_ref()), "{{api_keys.STRIPE}}");
    assert!(Redactor::for_workspace(ws).is_none());
}

#[tokio::test]
async fn unsealing_needs_the_passphrase_and_names_are_checked() {
    let root = scratch_data_root();
    let (ws, engine) = (root.ws_id, Engine::new());
    let created = engine.call("vault.create", json!({ "workspaceId": ws, "name": "keys", "passphrase": PASS })).await;
    let id = created["id"].as_str().unwrap().to_string();
    engine.call("vault.seal", json!({ "workspaceId": ws, "id": id })).await;

    let wrong = engine.try_as(None, "vault.unseal", json!({ "workspaceId": ws, "id": id, "passphrase": "not the one" })).await;
    assert_eq!(wrong.unwrap_err().code, ErrorCode::Forbidden);
    let unsealed = engine.call("vault.unseal", json!({ "workspaceId": ws, "id": id, "passphrase": PASS })).await;
    assert_eq!(unsealed["state"], "ready");

    for (action, payload) in [
        ("vault.create", json!({ "workspaceId": ws, "name": "has space", "passphrase": PASS })),
        ("vault.create", json!({ "workspaceId": ws, "name": "random", "passphrase": PASS })),
        ("vault.create", json!({ "workspaceId": ws, "name": "KEYS", "passphrase": PASS })),
        ("vault.create", json!({ "workspaceId": ws, "name": "short", "passphrase": "abc" })),
        ("vault.setSecret", json!({ "workspaceId": ws, "id": id, "name": "bad-name", "value": "v" })),
    ] {
        assert!(engine.try_as(None, action, payload.clone()).await.is_err(), "{action} {payload}");
    }
}

#[tokio::test]
async fn only_the_cli_checks_a_vault_out_and_the_lease_ends_with_its_session() {
    let root = scratch_data_root();
    let (ws, engine) = (root.ws_id, Engine::new());
    let created = engine.call("vault.create", json!({ "workspaceId": ws, "name": "keys", "passphrase": PASS })).await;
    let id = created["id"].as_str().unwrap().to_string();
    engine.call("vault.setSecret", json!({ "workspaceId": ws, "id": id, "name": "TOKEN", "value": "t0ken-value" })).await;
    engine.call("vault.seal", json!({ "workspaceId": ws, "id": id })).await;

    let checkout = |passphrase: &str| json!({ "workspaceId": ws, "id": id, "passphrase": passphrase });
    let ui = engine.try_as(session("ui", "desktop-ui"), "vault.checkout", checkout(PASS)).await;
    assert_eq!(ui.unwrap_err().code, ErrorCode::Forbidden);
    let wrong = engine.try_as(session("cli1", "cli"), "vault.checkout", checkout("not the passphrase")).await;
    assert_eq!(wrong.unwrap_err().code, ErrorCode::Forbidden);
    assert!(engine.call("vault.list", json!({ "workspaceId": ws })).await[0]["openedBy"] == 0);

    let out = engine.try_as(session("cli1", "cli"), "vault.checkout", checkout(PASS)).await.unwrap();
    assert_eq!(out["secrets"], json!({ "TOKEN": "t0ken-value" }));
    let listed = engine.call("vault.list", json!({ "workspaceId": ws })).await;
    assert_eq!(listed[0]["state"], "open");

    session_ended("cli1");
    let listed = engine.call("vault.list", json!({ "workspaceId": ws })).await;
    assert_eq!(listed[0]["state"], "ready");

    let out = engine.try_as(session("cli2", "cli"), "vault.checkout", checkout(PASS)).await.unwrap();
    engine.call("vault.release", json!({ "leaseId": out["leaseId"] })).await;
    assert_eq!(engine.call("vault.list", json!({ "workspaceId": ws })).await[0]["state"], "ready");
}

#[tokio::test]
async fn wrong_passphrases_are_throttled_and_short_secrets_refused() {
    let root = scratch_data_root();
    let (ws, engine) = (root.ws_id, Engine::new());
    let created = engine.call("vault.create", json!({ "workspaceId": ws, "name": "keys", "passphrase": PASS })).await;
    let id = created["id"].as_str().unwrap().to_string();
    let short = engine.try_as(None, "vault.setSecret", json!({ "workspaceId": ws, "id": id, "name": "PIN", "value": "1234" })).await;
    assert_eq!(short.unwrap_err().code, ErrorCode::BadRequest);
    engine.call("vault.seal", json!({ "workspaceId": ws, "id": id })).await;
    let try_pass = |p: &str| json!({ "workspaceId": ws, "id": id, "passphrase": p });
    for _ in 0..3 {
        assert!(engine.try_as(None, "vault.unseal", try_pass("not the passphrase")).await.is_err());
    }
    let throttled = engine.try_as(None, "vault.unseal", try_pass(PASS)).await.unwrap_err();
    assert_eq!(throttled.details.unwrap()["reason"], "throttled", "even the right passphrase waits");
}

#[tokio::test]
async fn changing_the_passphrase_keeps_the_secrets() {
    let root = scratch_data_root();
    let (ws, engine) = (root.ws_id, Engine::new());
    let created = engine.call("vault.create", json!({ "workspaceId": ws, "name": "keys", "passphrase": PASS })).await;
    let id = created["id"].as_str().unwrap().to_string();
    engine.call("vault.setSecret", json!({ "workspaceId": ws, "id": id, "name": "A", "value": "value-of-a" })).await;
    engine
        .call("vault.changePassphrase", json!({ "workspaceId": ws, "id": id, "passphrase": PASS, "newPassphrase": "a new passphrase" }))
        .await;
    engine.call("vault.seal", json!({ "workspaceId": ws, "id": id })).await;
    assert!(engine.try_as(None, "vault.unseal", json!({ "workspaceId": ws, "id": id, "passphrase": PASS })).await.is_err());
    engine.call("vault.unseal", json!({ "workspaceId": ws, "id": id, "passphrase": "a new passphrase" })).await;
    let env = load_merged_environment(ws);
    assert_eq!(resolve_vars("{{keys.A}}", env.as_ref()), "value-of-a");
}

#[tokio::test]
async fn a_request_is_sent_with_the_secret_but_echoed_and_returned_without_it() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    use crate::rest::pipeline::{PipelineScripts, run_request_pipeline};

    let root = scratch_data_root();
    let (ws, engine) = (root.ws_id, Engine::new());
    let created = engine.call("vault.create", json!({ "workspaceId": ws, "name": "keys", "passphrase": PASS })).await;
    let id = created["id"].as_str().unwrap().to_string();
    engine.call("vault.setSecret", json!({ "workspaceId": ws, "id": id, "name": "TOKEN", "value": "t0ken-value-123" })).await;

    let request = json!({ "method": "GET", "url": "http://h/{{keys.TOKEN}}", "headers": { "authorization": "Bearer {{keys.TOKEN}}" }, "body": "" });
    let sent = std::sync::Arc::new(parking_lot::Mutex::new(Value::Null));
    let seen = sent.clone();
    let scripts = PipelineScripts { pre: None, post: None, test: Some("console.log('token is ' + 'x');".into()) };
    let out = run_request_pipeline(ws, &request, &scripts, move |wire| async move {
        *seen.lock() = wire;
        // The server echoes the secret back, in the body and encoded in a header.
        Ok(json!({ "status": 200, "headers": { "x-echo": STANDARD.encode("t0ken-value-123") }, "body": STANDARD.encode("you sent t0ken-value-123") }))
    })
    .await
    .unwrap();

    let wire = sent.lock().clone();
    assert_eq!(wire["url"], "http://h/t0ken-value-123");
    assert_eq!(wire["headers"]["authorization"], "Bearer t0ken-value-123");
    assert_eq!(out["request"]["url"], "http://h/{{keys.TOKEN}}");
    assert_eq!(out["request"]["headers"]["authorization"], "Bearer {{keys.TOKEN}}");
    assert!(!out.to_string().contains("t0ken-value-123"));
    let body = STANDARD.decode(out["response"]["body"].as_str().unwrap()).unwrap();
    assert_eq!(String::from_utf8(body).unwrap(), "you sent {{keys.TOKEN}}");
    assert_eq!(out["response"]["headers"]["x-echo"], "{{keys.TOKEN}}");
}
