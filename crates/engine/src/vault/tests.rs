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

    let created =
        engine.call("vault.create", json!({ "workspaceId": ws, "name": "api_keys", "passphrase": PASS })).await;
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["state"], "unsealed");
    engine
        .call("vault.setSecret", json!({ "workspaceId": ws, "id": id, "name": "STRIPE", "value": "sk_live_123456" }))
        .await;

    let listed = engine.call("vault.list", json!({ "workspaceId": ws })).await;
    assert_eq!(listed[0]["secrets"], json!(["STRIPE"]));
    assert!(!listed.to_string().contains("sk_live_123456"));
    // On this device only, outside the workspace's repository, and sealed there.
    assert_eq!(listed[0]["provider"], "local");
    assert_eq!(listed[0]["path"], Value::Null);
    assert!(!crate::workspace::fs::entity_dir(ws, "vaults").join(format!("{id}.json")).exists());
    let on_disk = std::fs::read_to_string(device_dir(ws).join(format!("{id}.json"))).unwrap();
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

    let wrong =
        engine.try_as(None, "vault.unseal", json!({ "workspaceId": ws, "id": id, "passphrase": "not the one" })).await;
    assert_eq!(wrong.unwrap_err().code, ErrorCode::Forbidden);
    let unsealed = engine.call("vault.unseal", json!({ "workspaceId": ws, "id": id, "passphrase": PASS })).await;
    assert_eq!(unsealed["state"], "unsealed");

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
async fn only_the_cli_reads_the_secrets_and_only_of_an_unsealed_vault() {
    let root = scratch_data_root();
    let (ws, engine) = (root.ws_id, Engine::new());
    let created = engine.call("vault.create", json!({ "workspaceId": ws, "name": "keys", "passphrase": PASS })).await;
    let id = created["id"].as_str().unwrap().to_string();
    engine
        .call("vault.setSecret", json!({ "workspaceId": ws, "id": id, "name": "TOKEN", "value": "t0ken-value" }))
        .await;
    engine.call("vault.seal", json!({ "workspaceId": ws, "id": id })).await;

    let secrets = json!({ "workspaceId": ws, "id": id });
    let sealed = engine.try_as(session("cli1", "cli"), "vault.secrets", secrets.clone()).await.unwrap_err();
    assert_eq!(sealed.code, ErrorCode::Forbidden);
    assert_eq!(sealed.details.unwrap()["reason"], "vault_sealed");

    engine.call("vault.unseal", json!({ "workspaceId": ws, "id": id, "passphrase": PASS })).await;
    let ui = engine.try_as(session("ui", "desktop-ui"), "vault.secrets", secrets.clone()).await;
    assert_eq!(ui.unwrap_err().code, ErrorCode::Forbidden);

    // Once unsealed it is read without a passphrase, by any CLI session, until it is sealed again.
    for cli in ["cli1", "cli2"] {
        let out = engine.try_as(session(cli, "cli"), "vault.secrets", secrets.clone()).await.unwrap();
        assert_eq!(out["secrets"], json!({ "TOKEN": "t0ken-value" }));
        assert_eq!(out["name"], "keys");
    }
    let listed = engine.call("vault.list", json!({ "workspaceId": ws })).await;
    assert_eq!(listed[0]["state"], "unsealed");
    assert!(listed[0].get("openedBy").is_none());

    engine.call("vault.seal", json!({ "workspaceId": ws, "id": id })).await;
    assert!(engine.try_as(session("cli1", "cli"), "vault.secrets", secrets).await.is_err());
}

#[tokio::test]
async fn wrong_passphrases_are_throttled_and_short_secrets_refused() {
    let root = scratch_data_root();
    let (ws, engine) = (root.ws_id, Engine::new());
    let created = engine.call("vault.create", json!({ "workspaceId": ws, "name": "keys", "passphrase": PASS })).await;
    let id = created["id"].as_str().unwrap().to_string();
    let short = engine
        .try_as(None, "vault.setSecret", json!({ "workspaceId": ws, "id": id, "name": "PIN", "value": "1234" }))
        .await;
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
        .call(
            "vault.changePassphrase",
            json!({ "workspaceId": ws, "id": id, "passphrase": PASS, "newPassphrase": "a new passphrase" }),
        )
        .await;
    engine.call("vault.seal", json!({ "workspaceId": ws, "id": id })).await;
    assert!(
        engine.try_as(None, "vault.unseal", json!({ "workspaceId": ws, "id": id, "passphrase": PASS })).await.is_err()
    );
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
    engine
        .call("vault.setSecret", json!({ "workspaceId": ws, "id": id, "name": "TOKEN", "value": "t0ken-value-123" }))
        .await;

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

fn device_dir(ws: &str) -> std::path::PathBuf {
    crate::workspace::paths::data_dir().join("vaults").join(ws)
}

#[tokio::test]
async fn a_vault_kept_in_the_repository_moves_to_this_device_and_says_so() {
    let root = scratch_data_root();
    let (ws, engine) = (root.ws_id, Engine::new());
    // What an older version wrote: a sealed vault in the workspace's repository.
    let (mut old, key) = store::VaultFile::create("old1".into(), "legacy", PASS, 0).unwrap();
    old.set_secret(&key, "TOKEN", "t0ken-value").unwrap();
    let repo_file = crate::workspace::fs::entity_dir(ws, "vaults").join("old1.json");
    std::fs::create_dir_all(repo_file.parent().unwrap()).unwrap();
    std::fs::write(&repo_file, serde_json::to_string(&old).unwrap()).unwrap();

    let listed = engine.call("vault.list", json!({ "workspaceId": ws })).await;
    assert!(!repo_file.exists(), "no longer in the repository");
    assert!(device_dir(ws).join("old1.json").exists());
    assert_eq!(listed[0]["provider"], "local");
    assert!(listed[0]["movedAt"].is_i64(), "the user is told");

    engine.call("vault.unseal", json!({ "workspaceId": ws, "id": "old1", "passphrase": PASS })).await;
    let env = load_merged_environment(ws);
    assert_eq!(resolve_vars("{{legacy.TOKEN}}", env.as_ref()), "t0ken-value");
    let seen = engine.call("vault.acknowledge", json!({ "workspaceId": ws, "id": "old1" })).await;
    assert_eq!(seen["movedAt"], Value::Null);

    // Brought back by a pull from an older version: this device's copy stays, the file goes again.
    std::fs::write(&repo_file, serde_json::to_string(&old).unwrap()).unwrap();
    engine.call("vault.list", json!({ "workspaceId": ws })).await;
    assert!(!repo_file.exists());
    let kept: Value =
        serde_json::from_str(&std::fs::read_to_string(device_dir(ws).join("old1.json")).unwrap()).unwrap();
    assert!(kept.get("movedAt").is_none(), "already acknowledged here");
}

/// A linked vault on the fake `op`, pulling the item `item`.
async fn linked(engine: &Engine, ws: &str, item: &str) -> String {
    let created = engine
        .call(
            "vault.create",
            json!({
                "workspaceId": ws, "name": "team", "provider": "1password",
                "settings": { "item": item, "account": "acme" }, "credential": "ops_token_for_this_device",
            }),
        )
        .await;
    created["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_linked_vault_pulls_every_secret_of_its_source_and_edits_none() {
    let root = scratch_data_root();
    let (ws, engine) = (root.ws_id, Engine::new());
    providers::tests::fake_op(root.dir.path());
    let id = linked(&engine, ws, "Stripe").await;
    let listed = engine.call("vault.list", json!({ "workspaceId": ws })).await;
    assert_eq!((listed[0]["state"].as_str(), listed[0]["approved"].as_bool()), (Some("sealed"), Some(true)));
    assert_eq!(listed[0]["secrets"], json!([]), "nothing pulled yet");
    assert_eq!(listed[0]["path"], format!("vaults/{id}.json"));

    // The synced file names the source, never a value or the token.
    let synced =
        std::fs::read_to_string(crate::workspace::fs::entity_dir(ws, "vaults").join(format!("{id}.json"))).unwrap();
    assert!(synced.contains("\"item\": \"Stripe\""));
    assert!(!synced.contains("ops_token_for_this_device") && !synced.contains("value-for"));

    let tested = engine.call("vault.test", json!({ "workspaceId": ws, "id": id })).await;
    assert_eq!(tested, json!({ "names": ["api_key"] }));

    let unsealed = engine.call("vault.unseal", json!({ "workspaceId": ws, "id": id })).await;
    assert_eq!(unsealed["secrets"], json!(["api_key"]));
    let env = load_merged_environment(ws);
    assert_eq!(resolve_vars("{{team.api_key}}", env.as_ref()), "value-for-Stripe");
    assert!(!engine.call("vault.list", json!({ "workspaceId": ws })).await.to_string().contains("value-for"));

    // Sealed again, it still shows what it held when this device last pulled it.
    engine.call("vault.seal", json!({ "workspaceId": ws, "id": id })).await;
    let listed = engine.call("vault.list", json!({ "workspaceId": ws })).await;
    assert_eq!(listed[0]["secrets"], json!(["api_key"]));

    // Its secrets are changed in the provider, not here.
    let set = json!({ "workspaceId": ws, "id": id, "name": "X", "value": "abcdefgh" });
    assert!(engine.try_as(None, "vault.setSecret", set).await.is_err());
    let remove = json!({ "workspaceId": ws, "id": id, "name": "api_key" });
    assert!(engine.try_as(None, "vault.deleteSecret", remove).await.is_err());

    // A source it cannot read unseals nothing.
    let missing = linked_named(&engine, ws, "other", "missing-item").await;
    let failed = engine.try_as(None, "vault.unseal", json!({ "workspaceId": ws, "id": missing })).await.unwrap_err();
    assert!(failed.message.contains("no such item"), "{}", failed.message);
}

async fn linked_named(engine: &Engine, ws: &str, name: &str, item: &str) -> String {
    let created = engine
        .call(
            "vault.create",
            json!({ "workspaceId": ws, "name": name, "provider": "1password", "settings": { "item": item } }),
        )
        .await;
    created["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_synced_change_of_source_needs_approval_on_this_device() {
    let root = scratch_data_root();
    let (ws, engine) = (root.ws_id, Engine::new());
    providers::tests::fake_op(root.dir.path());
    let id = linked(&engine, ws, "Stripe").await;

    // A teammate points the vault at another item, and the change arrives by a pull.
    let file = crate::workspace::fs::entity_dir(ws, "vaults").join(format!("{id}.json"));
    let changed = std::fs::read_to_string(&file).unwrap().replace("\"Stripe\"", "\"Bank\"");
    std::fs::write(&file, changed).unwrap();

    let listed = engine.call("vault.list", json!({ "workspaceId": ws })).await;
    assert_eq!(listed[0]["approved"], false);
    let refused = engine.try_as(None, "vault.unseal", json!({ "workspaceId": ws, "id": id })).await.unwrap_err();
    assert_eq!(refused.details.unwrap()["reason"], "vault_not_approved");

    let approved = engine.call("vault.configure", json!({ "workspaceId": ws, "id": id })).await;
    assert_eq!(approved["approved"], true);
    assert_eq!(approved["settings"]["item"], "Bank");
    engine.call("vault.unseal", json!({ "workspaceId": ws, "id": id })).await;
    let env = load_merged_environment(ws);
    assert_eq!(resolve_vars("{{team.api_key}}", env.as_ref()), "value-for-Bank");
}

#[tokio::test]
async fn a_linked_vault_checks_its_provider_and_settings() {
    let root = scratch_data_root();
    let (ws, engine) = (root.ws_id, Engine::new());
    for (payload, why) in [
        (json!({ "provider": "nope" }), "unknown provider"),
        (json!({ "provider": "azure" }), "Azure needs its key vault's name"),
        (json!({ "provider": "1password", "settings": { "item": "--help" } }), "read as an option"),
        (json!({ "provider": "bitwarden" }), "Bitwarden needs a token"),
    ] {
        let mut payload = payload;
        payload["workspaceId"] = json!(ws);
        payload["name"] = json!("v");
        assert!(engine.try_as(None, "vault.create", payload).await.is_err(), "{why}");
    }
    let providers = engine.call("vault.providers", json!({})).await;
    assert_eq!(providers[0]["id"], "local");
    assert!(providers.as_array().unwrap().iter().any(|p| p["id"] == "aws" && p["installed"].is_boolean()));
}

#[tokio::test]
async fn only_a_person_decides_where_a_linked_vault_pulls_from() {
    let root = scratch_data_root();
    let (ws, engine) = (root.ws_id, Engine::new());
    providers::tests::fake_op(root.dir.path());
    let at_terminal = |id: &str| {
        Some(SessionIdentity {
            session_id: id.into(),
            client_name: Some("cli".into()),
            client_info: Some(json!({ "terminal": "yes" })),
            ..Default::default()
        })
    };
    let create =
        json!({ "workspaceId": ws, "name": "team", "provider": "1password", "settings": { "item": "Stripe" } });

    // An agent: the command line through a pipe, or MCP (which connects the same way).
    let refused = engine.try_as(session("agent", "cli"), "vault.create", create.clone()).await.unwrap_err();
    assert_eq!(refused.details.unwrap()["reason"], "vault_needs_person");
    let created = engine.try_as(at_terminal("person"), "vault.create", create).await.unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    let vault = json!({ "workspaceId": ws, "id": id });
    for action in ["vault.unseal", "vault.configure", "vault.test"] {
        assert!(engine.try_as(session("agent", "cli"), action, vault.clone()).await.is_err(), "{action}");
    }
    engine.try_as(session("app", "desktop-ui"), "vault.unseal", vault.clone()).await.unwrap();
    // Once a person unsealed it, its secrets are used like a local vault's.
    let read = engine.try_as(session("agent", "cli"), "vault.secrets", vault).await.unwrap();
    assert_eq!(read["secrets"]["api_key"], "value-for-Stripe");
    // A local vault keeps its own gate, the passphrase.
    let local = json!({ "workspaceId": ws, "name": "mine", "passphrase": PASS });
    assert!(engine.try_as(session("agent", "cli"), "vault.create", local).await.is_ok());
}
