//! Vaults: named groups of encrypted secrets that a workspace keeps next to its other entities.
//!
//! A vault's file can be committed and shared; its values open only with the vault's passphrase, and only the
//! engine ever holds the key or the plaintext. Clients get names and states: `vault.list` shows what exists
//! and whether each vault is sealed or unsealed, and nothing returns a value except `vault.secrets`,
//! which the CLI uses to launch a program with the secrets of an unsealed vault as environment variables.
//!
//! Secrets are used in requests as `{{vault.secret}}` (see `env::resolve_vars`); everything the engine sends
//! back to a client or writes to a file passes through [`Redactor`] first.

mod crypto;
mod state;
mod store;

use std::sync::Mutex;

use serde_json::{Value, json};

pub use state::{
    Redactor, redact_text, redact_value, references_in, resolve_allowed, resolve_reference, resolve_value_allowed,
};
use store::VaultFile;

use crate::{
    kernel::{CommandContext, CommandRegistry},
    protocol::{EngineError, ErrorCode, Result},
    utils::{
        id::{generate_id, now_ms},
        json::str_of,
    },
};

/// Serialises read-modify-write of vault files: the commands below run on the blocking pool.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(move || {
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        work()
    })
    .await
    .map_err(|e| EngineError::engine(format!("The vault task failed: {e}")))?
}

/// A vault as clients see it: names and state, never a value.
fn summary(ws_id: &str, vault: &VaultFile) -> Value {
    let state_name = if state::key_of(ws_id, &vault.id).is_none() { "sealed" } else { "unsealed" };
    json!({
        "id": vault.id,
        "name": vault.name,
        "state": state_name,
        "secrets": vault.secret_names(),
        "createdAt": vault.created_at,
        "path": store::rel_path(&vault.id),
        "workspaceId": ws_id,
    })
}

fn ready_key(ws_id: &str, vault: &VaultFile) -> Result<crypto::Key> {
    state::key_of(ws_id, &vault.id).ok_or_else(|| {
        EngineError::with_details(
            ErrorCode::Forbidden,
            format!("The vault \"{}\" is sealed. Enter its passphrase first.", vault.name),
            json!({ "reason": "vault_sealed", "vaultId": vault.id }),
        )
    })
}

/// The key `passphrase` gives, if it is right. Wrong ones are counted: after a few the vault asks to wait.
fn verify(ws_id: &str, vault: &VaultFile, passphrase: &str) -> Result<crypto::Key> {
    let wait = state::attempt_wait_secs(ws_id, &vault.id);
    if wait > 0 {
        return Err(EngineError::with_details(
            ErrorCode::Forbidden,
            format!("Too many wrong passphrases. Try again in {wait} s."),
            json!({ "reason": "throttled", "retryAfterSecs": wait }),
        ));
    }
    match vault.unlock(passphrase) {
        Ok(key) => {
            state::attempt_succeeded(ws_id, &vault.id);
            Ok(key)
        }
        Err(error) => {
            if error.code == ErrorCode::Forbidden {
                state::attempt_failed(ws_id, &vault.id);
            }
            Err(error)
        }
    }
}

fn check_name_free(ws_id: &str, name: &str, except_id: Option<&str>) -> Result<()> {
    let taken =
        store::list(ws_id).into_iter().any(|v| v.name.eq_ignore_ascii_case(name) && Some(v.id.as_str()) != except_id);
    if taken { Err(EngineError::conflict(format!("A vault named \"{name}\" already exists."))) } else { Ok(()) }
}

fn changed_event(ws_id: &str, id: &str) {
    crate::kernel::bus::bus().emit("vault.changed", json!({ "workspaceId": ws_id, "id": id }));
}

/// `vault.secrets` hands plaintext to its caller, so only the CLI session that will launch the program gets it.
fn require_cli_session(ctx: &CommandContext) -> Result<()> {
    let Some(session) = &ctx.session else {
        return Err(EngineError::bad_request("vault.secrets needs a client session."));
    };
    if session.client_name.as_deref() != Some("cli") {
        return Err(EngineError::new(
            ErrorCode::Forbidden,
            "Secret values are given only to the command line, which launches programs with them.",
        ));
    }
    Ok(())
}

pub use state::MIN_SECRET_LEN;

/// The name and the secrets of an unsealed vault, for the engine's own use: an import that moves values into
/// it reuses a secret that already holds a value. Never handed to a client.
pub(crate) async fn read_unsealed(ws_id: &str, id: &str) -> Result<(String, Vec<(String, String)>)> {
    let (ws_id, id) = (ws_id.to_string(), id.to_string());
    blocking(move || {
        let vault = store::get(&ws_id, &id)?;
        let key = ready_key(&ws_id, &vault)?;
        let secrets = vault.read_all(&key).into_iter().map(|(name, value)| (name, value.to_string())).collect();
        Ok((vault.name, secrets))
    })
    .await
}

/// Seal `secrets` into an unsealed vault, as `vault.setSecret` does for one.
pub(crate) async fn add_secrets(ws_id: &str, id: &str, secrets: Vec<(String, String)>) -> Result<()> {
    if secrets.is_empty() {
        return Ok(());
    }
    let (ws_id, id) = (ws_id.to_string(), id.to_string());
    blocking(move || {
        let mut vault = store::get(&ws_id, &id)?;
        let key = ready_key(&ws_id, &vault)?;
        for (name, value) in &secrets {
            store::check_secret_name(name)?;
            vault.set_secret(&key, name, value)?;
        }
        store::save(&ws_id, &vault)?;
        changed_event(&ws_id, &id);
        Ok(())
    })
    .await
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register("vault.list", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let vaults = store::list(&ws_id);
        Ok(Value::Array(vaults.iter().map(|vault| summary(&ws_id, vault)).collect()))
    });

    registry.register("vault.create", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let name = str_of(&params, "name").to_string();
        let passphrase = str_of(&params, "passphrase").to_string();
        blocking(move || {
            store::check_vault_name(&name)?;
            store::check_passphrase(&passphrase)?;
            check_name_free(&ws_id, &name, None)?;
            let (vault, key) = VaultFile::create(generate_id(), &name, &passphrase, now_ms())?;
            store::save(&ws_id, &vault)?;
            state::set_key(&ws_id, &vault.id, key);
            changed_event(&ws_id, &vault.id);
            Ok(summary(&ws_id, &vault))
        })
        .await
    });

    registry.register("vault.rename", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let (id, name) = (str_of(&params, "id").to_string(), str_of(&params, "name").to_string());
        blocking(move || {
            store::check_vault_name(&name)?;
            check_name_free(&ws_id, &name, Some(&id))?;
            let mut vault = store::get(&ws_id, &id)?;
            vault.name = name;
            store::save(&ws_id, &vault)?;
            changed_event(&ws_id, &id);
            Ok(summary(&ws_id, &vault))
        })
        .await
    });

    registry.register("vault.delete", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let id = str_of(&params, "id").to_string();
        blocking(move || {
            store::get(&ws_id, &id)?;
            state::forget(&ws_id, &id);
            store::delete(&ws_id, &id)?;
            changed_event(&ws_id, &id);
            Ok(json!({ "ok": true }))
        })
        .await
    });

    registry.register("vault.unseal", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let id = str_of(&params, "id").to_string();
        let passphrase = str_of(&params, "passphrase").to_string();
        blocking(move || {
            let vault = store::get(&ws_id, &id)?;
            if state::key_of(&ws_id, &id).is_none() {
                state::set_key(&ws_id, &id, verify(&ws_id, &vault, &passphrase)?);
            }
            Ok(summary(&ws_id, &vault))
        })
        .await
    });

    registry.register("vault.seal", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let id = str_of(&params, "id").to_string();
        blocking(move || {
            let vault = store::get(&ws_id, &id)?;
            state::seal(&ws_id, &id);
            Ok(summary(&ws_id, &vault))
        })
        .await
    });

    registry.register("vault.changePassphrase", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let id = str_of(&params, "id").to_string();
        let (old, new) = (str_of(&params, "passphrase").to_string(), str_of(&params, "newPassphrase").to_string());
        blocking(move || {
            store::check_passphrase(&new)?;
            let mut vault = store::get(&ws_id, &id)?;
            let old_key = verify(&ws_id, &vault, &old)?;
            let new_key = vault.rekey(&old_key, &new)?;
            store::save(&ws_id, &vault)?;
            state::set_key(&ws_id, &id, new_key);
            Ok(summary(&ws_id, &vault))
        })
        .await
    });

    registry.register("vault.setSecret", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let (id, name) = (str_of(&params, "id").to_string(), str_of(&params, "name").to_string());
        let value = str_of(&params, "value").to_string();
        blocking(move || {
            store::check_secret_name(&name)?;
            if value.chars().count() < state::MIN_SECRET_LEN {
                return Err(EngineError::bad_request(format!(
                    "A secret must be at least {} characters, so that it can be kept out of logs.",
                    state::MIN_SECRET_LEN
                )));
            }
            let mut vault = store::get(&ws_id, &id)?;
            let key = ready_key(&ws_id, &vault)?;
            vault.set_secret(&key, &name, &value)?;
            store::save(&ws_id, &vault)?;
            changed_event(&ws_id, &id);
            Ok(summary(&ws_id, &vault))
        })
        .await
    });

    registry.register("vault.deleteSecret", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let (id, name) = (str_of(&params, "id").to_string(), str_of(&params, "name").to_string());
        blocking(move || {
            let mut vault = store::get(&ws_id, &id)?;
            let before = vault.secrets.len();
            vault.secrets.retain(|s| s.name != name);
            if vault.secrets.len() == before {
                return Err(EngineError::not_found(format!("No secret \"{name}\" in this vault.")));
            }
            store::save(&ws_id, &vault)?;
            changed_event(&ws_id, &id);
            Ok(summary(&ws_id, &vault))
        })
        .await
    });

    // The values of an unsealed vault. A sealed one is refused: it is unsealed first (`vault.unseal`), which is
    // the only place a passphrase is asked for.
    registry.register("vault.secrets", |params, ctx| async move {
        require_cli_session(&ctx)?;
        let ws_id = str_of(&params, "workspaceId").to_string();
        let id = str_of(&params, "id").to_string();
        blocking(move || {
            let vault = store::get(&ws_id, &id)?;
            let key = ready_key(&ws_id, &vault)?;
            let secrets: serde_json::Map<String, Value> =
                vault.read_all(&key).into_iter().map(|(name, value)| (name, json!(value.as_str()))).collect();
            Ok(json!({ "id": vault.id, "name": vault.name, "secrets": secrets }))
        })
        .await
    });
}

#[cfg(test)]
mod tests;
