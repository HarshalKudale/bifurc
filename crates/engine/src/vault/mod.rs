//! Vaults: named groups of secrets that a workspace's requests use as `{{vault.secret}}`.
//!
//! No vault puts a value in the workspace's repository (see `store`). A **local** vault keeps its values
//! sealed with a passphrase on this device only, and is the only kind whose secrets are added, changed or
//! removed here. A **linked** vault pulls them from a secret manager (1Password, AWS Secrets Manager, Azure Key
//! Vault, …): it names a source there, and unsealing reads every secret the source holds. Secrets are changed
//! in the manager; the source is synced, so a teammate with access to the same manager uses the same vault.
//! Only the engine
//! ever holds a value: clients get names and states, and nothing returns a value except `vault.secrets`,
//! which the CLI uses to launch a program with the secrets of an unsealed vault as environment variables.
//!
//! Secrets are used in requests as `{{vault.secret}}` (see `env::resolve_vars`); everything the engine sends
//! back to a client or writes to a file passes through [`Redactor`] first.

mod crypto;
mod providers;
mod state;
mod store;

use std::sync::Mutex;

use serde_json::{Value, json};
use zeroize::Zeroizing;

pub use state::{
    Redactor, redact_text, redact_value, references_in, resolve_allowed, resolve_reference, resolve_value_allowed,
};
use store::{LinkedVault, Vault, VaultFile};

use crate::{
    kernel::{CommandContext, CommandRegistry},
    protocol::{EngineError, ErrorCode, Result},
    utils::{
        id::{generate_id, now_ms},
        json::{array_of, opt_str, str_of},
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
    .map_err(|e| {
        EngineError::engine(format!("The vault task failed: {e}"))
            .keyed("vault.task_failed", json!({ "detail": e.to_string() }))
    })?
}

/// Moves the vaults an older version kept in the workspace's repository to this device, and commits their
/// removal (pushing it, with a remote) so they stop being synced. Their old copies stay in the repository's
/// history; the vault says so until the user has seen it.
pub fn move_off_repository(ws_id: &str) {
    let moved = store::migrate(ws_id);
    if moved.is_empty() {
        return;
    }
    for (path, _) in &moved {
        changed_event(ws_id, path.trim_start_matches("vaults/").trim_end_matches(".json"));
    }
    if !crate::workspace::git_store::is_repo(ws_id) {
        return;
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
    let ws_id = ws_id.to_string();
    let names: Vec<String> = moved.iter().map(|(_, name)| name.clone()).collect();
    let paths: Vec<String> = moved.into_iter().map(|(path, _)| path).collect();
    runtime.spawn(async move {
        let message = format!("Vault {} moved to this device; it is no longer synced", names.join(", "));
        // A workspace this account may not push to keeps the deletion as a change of its own.
        let _ = crate::gitops::sync_changes(&ws_id, &paths, Some(message)).await;
        crate::runtime::entity_status::emit_entity_status(&ws_id);
    });
}

/// Whether this device has approved what the linked vault's file now says.
fn approved(ws_id: &str, vault: &LinkedVault) -> bool {
    store::link(ws_id, &vault.id).approved.as_deref() == Some(vault.fingerprint().as_str())
}

/// A vault as clients see it: names, references and state, never a value.
fn summary(ws_id: &str, vault: &Vault) -> Value {
    let state_name = if state::is_unsealed(ws_id, vault.id()) { "unsealed" } else { "sealed" };
    let mut out = json!({
        "id": vault.id(),
        "name": vault.name(),
        "provider": vault.provider(),
        "state": state_name,
        "secrets": vault.secret_names(),
        "createdAt": vault.created_at(),
        "workspaceId": ws_id,
    });
    match vault {
        Vault::Local(local) => {
            out["path"] = Value::Null;
            out["approved"] = json!(true);
            out["movedAt"] = json!(local.moved_at);
        }
        Vault::Linked(linked) => {
            let link = store::link(ws_id, &linked.id);
            out["path"] = json!(store::rel_path(&linked.id));
            out["approved"] = json!(link.approved.as_deref() == Some(linked.fingerprint().as_str()));
            out["settings"] = Value::Object(linked.settings.clone());
            out["hasCredential"] = json!(link.credential.is_some());
            out["providerLabel"] = json!(providers::get(&linked.provider).map(|p| p.label));
        }
    }
    out
}

fn sealed_error(name: &str, id: &str) -> EngineError {
    EngineError::with_details(
        ErrorCode::Forbidden,
        format!("The vault \"{name}\" is sealed. Unseal it first."),
        json!({ "reason": "vault_sealed", "vaultId": id }),
    )
    .keyed("vault.sealed", json!({ "name": name }))
}

fn ready_values(ws_id: &str, vault: &Vault) -> Result<state::Values> {
    state::values_of(ws_id, vault.id()).ok_or_else(|| sealed_error(vault.name(), vault.id()))
}

fn ready_key(ws_id: &str, vault: &VaultFile) -> Result<crypto::Key> {
    state::key_of(ws_id, &vault.id).ok_or_else(|| sealed_error(&vault.name, &vault.id))
}

/// The key `passphrase` gives, if it is right. Wrong ones are counted: after a few the vault asks to wait.
fn verify(ws_id: &str, vault: &VaultFile, passphrase: &str) -> Result<crypto::Key> {
    let wait = state::attempt_wait_secs(ws_id, &vault.id);
    if wait > 0 {
        return Err(EngineError::with_details(
            ErrorCode::Forbidden,
            format!("Too many wrong passphrases. Try again in {wait} s."),
            json!({ "reason": "throttled", "retryAfterSecs": wait }),
        )
        .keyed("vault.throttled", json!({ "seconds": wait })));
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
        store::list(ws_id).into_iter().any(|v| v.name().eq_ignore_ascii_case(name) && Some(v.id()) != except_id);
    if taken {
        Err(EngineError::conflict(format!("A vault named \"{name}\" already exists."))
            .keyed("vault.name_taken", json!({ "name": name })))
    } else {
        Ok(())
    }
}

fn changed_event(ws_id: &str, id: &str) {
    crate::kernel::bus::bus().emit("vault.changed", json!({ "workspaceId": ws_id, "id": id }));
}

/// `vault.secrets` hands plaintext to its caller, so only the CLI session that will launch the program gets it.
fn require_cli_session(ctx: &CommandContext) -> Result<()> {
    let Some(session) = &ctx.session else {
        return Err(
            EngineError::bad_request("vault.secrets needs a client session.").keyed("vault.needs_session", json!({}))
        );
    };
    if session.client_name.as_deref() != Some("cli") {
        return Err(EngineError::new(
            ErrorCode::Forbidden,
            "Secret values are given only to the command line, which launches programs with them.",
        )
        .keyed("vault.cli_only", json!({})));
    }
    Ok(())
}

/// Deciding where a linked vault pulls from (making one, approving it) and pulling (unsealing one, trying a
/// source) is left to a person: the desktop app, or the command line at a terminal. An agent works through the
/// command line's pipes or MCP; were it allowed, it could point a vault at any secret the user's secret
/// manager login can read, or approve a source a sync redirected. A local
/// vault needs its passphrase for that, which is the same line drawn another way. Calls made inside the
/// engine have no session and are its own.
fn require_person(ctx: &CommandContext) -> Result<()> {
    let Some(session) = &ctx.session else { return Ok(()) };
    let at_terminal =
        session.client_info.as_ref().and_then(|info| info.get("terminal")).and_then(Value::as_str) == Some("yes");
    match session.client_name.as_deref() {
        Some("desktop-ui") => Ok(()),
        Some("cli") if at_terminal => Ok(()),
        _ => Err(EngineError::with_details(
            ErrorCode::Forbidden,
            "A person decides where a linked vault reads its secrets from: do this in Bifurc, or in a terminal.",
            json!({ "reason": "vault_needs_person" }),
        )
        .keyed("vault.needs_person", json!({}))),
    }
}

fn check_secret_value(value: &str) -> Result<()> {
    if value.chars().count() < state::MIN_SECRET_LEN {
        return Err(EngineError::bad_request(format!(
            "A secret must be at least {} characters, so that it can be kept out of logs.",
            state::MIN_SECRET_LEN
        ))
        .keyed("vault.secret_short", json!({ "min": state::MIN_SECRET_LEN })));
    }
    Ok(())
}

fn provider_of(id: &str) -> Result<&'static providers::Provider> {
    providers::check_provider(id)?;
    providers::get(id).ok_or_else(|| {
        EngineError::bad_request("A local vault has no provider.").keyed("vault.unknown_provider", json!({ "id": id }))
    })
}

fn pull_failed(provider: &providers::Provider, reason: &str) -> EngineError {
    EngineError::engine(format!("Couldn't pull the secrets from {}. {reason}", provider.label))
        .keyed("vault.pull_failed", json!({ "provider": provider.label, "detail": reason }))
}

fn not_approved(vault: &LinkedVault) -> EngineError {
    EngineError::with_details(
        ErrorCode::Forbidden,
        format!(
            "\"{}\" changed where its secrets are pulled from since this device last approved it. Review it first.",
            vault.name
        ),
        json!({ "reason": "vault_not_approved", "vaultId": vault.id }),
    )
    .keyed("vault.not_approved", json!({ "name": vault.name }))
}

/// Pulls a linked vault: every secret its source holds, or none. The names are kept on this device, so the
/// vault shows what it holds while it is sealed.
async fn pull_linked(ws_id: &str, vault: &LinkedVault) -> Result<state::Values> {
    if !approved(ws_id, vault) {
        return Err(not_approved(vault));
    }
    let provider = provider_of(&vault.provider)?;
    let credential = Zeroizing::new(store::link(ws_id, &vault.id).credential().unwrap_or_default());
    let values = providers::pull(provider, &vault.settings, Some(&credential))
        .await
        .map_err(|reason| pull_failed(provider, &reason))?;
    let names: Vec<String> = values.iter().map(|(name, _)| name.clone()).collect();
    let (ws, id) = (ws_id.to_string(), vault.id.clone());
    blocking(move || {
        let mut link = store::link(&ws, &id);
        if link.names != names {
            link.names = names;
            store::save_link(&ws, &id, &link)?;
        }
        Ok(())
    })
    .await?;
    Ok(values)
}

pub use state::MIN_SECRET_LEN;
#[cfg(test)]
pub use state::forget_all;

/// The name and the secrets of an unsealed vault, for the engine's own use: an import that moves values into
/// it reuses a secret that already holds a value. Never handed to a client.
pub(crate) async fn read_unsealed(ws_id: &str, id: &str) -> Result<(String, Vec<(String, String)>)> {
    let (ws_id, id) = (ws_id.to_string(), id.to_string());
    blocking(move || {
        let vault = store::get(&ws_id, &id)?;
        let values = ready_values(&ws_id, &vault)?;
        Ok((vault.name().to_string(), values.into_iter().map(|(name, value)| (name, value.to_string())).collect()))
    })
    .await
}

/// The secrets of the unsealed vault named `name`, for the engine's own use (`jwt.mint` signs with one).
/// A missing vault is NOT_FOUND, a sealed one the usual "sealed" error. Never handed to a client.
pub(crate) async fn secrets_by_name(ws_id: &str, name: &str) -> Result<Vec<(String, String)>> {
    let (ws_id, name) = (ws_id.to_string(), name.to_string());
    blocking(move || {
        let vault = store::find_by_name(&ws_id, &name)
            .ok_or_else(|| EngineError::not_found(format!("No vault named \"{name}\".")))?;
        Ok(ready_values(&ws_id, &vault)?.into_iter().map(|(name, value)| (name, value.to_string())).collect())
    })
    .await
}

/// [`secrets_by_name`] for variable resolution, which is synchronous; the error is the reason text for a warning.
pub(crate) fn secrets_for_variable(ws_id: &str, name: &str) -> std::result::Result<Vec<(String, String)>, String> {
    let vault = store::find_by_name(ws_id, name).ok_or_else(|| format!("no vault named \"{name}\""))?;
    let values = state::values_of(ws_id, vault.id())
        .ok_or_else(|| format!("vault \"{}\" is sealed (bifurc vault unseal {})", vault.name(), vault.name()))?;
    Ok(values.into_iter().map(|(name, value)| (name, value.to_string())).collect())
}

/// The names of the workspace's vaults that hold a secret called `secret`: `(unsealed, sealed)`.
pub(crate) fn vaults_holding(ws_id: &str, secret: &str) -> (Vec<String>, Vec<String>) {
    let (mut open, mut sealed) = (Vec::new(), Vec::new());
    for vault in store::list(ws_id).into_iter().filter(|v| v.secret_names().contains(&secret)) {
        let name = vault.name().to_string();
        if state::is_unsealed(ws_id, vault.id()) { open.push(name) } else { sealed.push(name) }
    }
    (open, sealed)
}

/// Seal `secrets` into an unsealed local vault, as `vault.setSecret` does for one. A linked vault's values are
/// its provider's, so it is refused.
pub(crate) async fn add_secrets(ws_id: &str, id: &str, secrets: Vec<(String, String)>) -> Result<()> {
    if secrets.is_empty() {
        return Ok(());
    }
    let (ws_id, id) = (ws_id.to_string(), id.to_string());
    blocking(move || {
        let mut vault = store::get_local(&ws_id, &id)?;
        let key = ready_key(&ws_id, &vault)?;
        for (name, value) in &secrets {
            store::check_secret_name(name)?;
            vault.set_secret(&key, name, value)?;
        }
        store::save(&ws_id, &vault)?;
        state::update(&ws_id, &id, &vault.name, |values| *values = vault.read_all(&key));
        changed_event(&ws_id, &id);
        Ok(())
    })
    .await
}

/// A secret given for a local vault at creation: `[{ name, value }]`.
fn local_secrets_from(params: &Value) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for item in array_of(params, "secrets") {
        let (name, value) = (str_of(item, "name").trim(), str_of(item, "value"));
        store::check_secret_name(name)?;
        check_secret_value(value)?;
        out.push((name.to_string(), value.to_string()));
    }
    Ok(out)
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("vault.providers", |_, _| Ok(providers::catalog()));

    registry.register("vault.list", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        blocking(move || {
            move_off_repository(&ws_id);
            Ok(Value::Array(store::list(&ws_id).iter().map(|vault| summary(&ws_id, vault)).collect()))
        })
        .await
    });

    // `provider` is `local` (the default) with a `passphrase`, or a secret manager with the `settings` that name
    // the source to pull from and an optional `credential` (its token, kept on this device). A linked vault is
    // created approved on this device, and sealed: its secrets are pulled when it is first unsealed.
    registry.register("vault.create", |params, ctx| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let name = str_of(&params, "name").trim().to_string();
        let provider = opt_str(&params, "provider").filter(|p| !p.is_empty()).unwrap_or(providers::LOCAL).to_string();
        providers::check_provider(&provider)?;
        if provider == providers::LOCAL {
            let passphrase = str_of(&params, "passphrase").to_string();
            let secrets = local_secrets_from(&params)?;
            return blocking(move || {
                store::check_vault_name(&name)?;
                store::check_passphrase(&passphrase)?;
                check_name_free(&ws_id, &name, None)?;
                let (mut vault, key) = VaultFile::create(generate_id(), &name, &passphrase, now_ms())?;
                for (secret, value) in &secrets {
                    vault.set_secret(&key, secret, value)?;
                }
                store::save(&ws_id, &vault)?;
                state::unseal(&ws_id, &vault.id, &vault.name, Some(key.clone()), vault.read_all(&key));
                changed_event(&ws_id, &vault.id);
                Ok(summary(&ws_id, &Vault::Local(vault)))
            })
            .await;
        }
        require_person(&ctx)?;
        let manager = provider_of(&provider)?;
        let settings = providers::clean_settings(manager, params.get("settings").unwrap_or(&Value::Null))?;
        let credential = Zeroizing::new(str_of(&params, "credential").to_string());
        if manager.credential_required && credential.trim().is_empty() {
            return Err(EngineError::bad_request(format!("{} needs a token.", manager.label))
                .keyed("vault.credential_missing", json!({ "provider": manager.label })));
        }
        blocking(move || {
            store::check_vault_name(&name)?;
            check_name_free(&ws_id, &name, None)?;
            let vault = LinkedVault {
                id: generate_id(),
                name,
                version: 1,
                created_at: now_ms(),
                provider,
                settings,
                names: Vec::new(),
            };
            let mut link = store::Link::default();
            link.set_credential(Some(&credential))?;
            link.approved = Some(vault.fingerprint());
            store::save_link(&ws_id, &vault.id, &link)?;
            store::save_linked(&ws_id, &vault)?;
            changed_event(&ws_id, &vault.id);
            Ok(summary(&ws_id, &Vault::Linked(vault)))
        })
        .await
    });

    // Pulls a source without keeping anything: `{ provider, settings, credential? }`, or `{ workspaceId, id }`
    // for a saved linked vault (with its token from this device; what the call gives wins over what is saved,
    // so a vault being set up is tried as it was edited). Answers `{ names }`; a value never leaves the engine.
    registry.register("vault.test", |params, ctx| async move {
        require_person(&ctx)?;
        let ws_id = str_of(&params, "workspaceId").to_string();
        let saved = match opt_str(&params, "id") {
            Some(id) => Some(
                blocking({
                    let (ws_id, id) = (ws_id.clone(), id.to_string());
                    move || Ok((store::get_linked(&ws_id, &id)?, store::link(&ws_id, &id)))
                })
                .await?,
            ),
            None => None,
        };
        let provider = match (&saved, opt_str(&params, "provider")) {
            (_, Some(provider)) => provider.to_string(),
            (Some((vault, _)), None) => vault.provider.clone(),
            (None, None) => String::new(),
        };
        let manager = provider_of(&provider)?;
        let settings = match (&saved, params.get("settings")) {
            (Some((vault, _)), None) => vault.settings.clone(),
            (_, given) => providers::clean_settings(manager, given.unwrap_or(&Value::Null))?,
        };
        let credential = Zeroizing::new(match (&saved, opt_str(&params, "credential")) {
            (_, Some(given)) if !given.is_empty() => given.to_string(),
            (Some((_, link)), _) => link.credential().unwrap_or_default(),
            _ => String::new(),
        });
        if providers::find_program(manager.program).is_none() {
            return Err(EngineError::bad_request(format!(
                "{} is not installed: the \"{}\" program was not found.",
                manager.label, manager.program
            ))
            .keyed("vault.program_missing", json!({ "provider": manager.label, "program": manager.program })));
        }
        let pulled = providers::pull(manager, &settings, Some(&credential))
            .await
            .map_err(|reason| pull_failed(manager, &reason))?;
        Ok(json!({ "names": pulled.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>() }))
    });

    registry.register("vault.rename", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let (id, name) = (str_of(&params, "id").to_string(), str_of(&params, "name").to_string());
        blocking(move || {
            store::check_vault_name(&name)?;
            check_name_free(&ws_id, &name, Some(&id))?;
            let vault = match store::get(&ws_id, &id)? {
                Vault::Local(mut vault) => {
                    vault.name = name.clone();
                    store::save(&ws_id, &vault)?;
                    Vault::Local(vault)
                }
                Vault::Linked(mut vault) => {
                    vault.name = name.clone();
                    store::save_linked(&ws_id, &vault)?;
                    Vault::Linked(vault)
                }
            };
            state::update(&ws_id, &id, &name, |_| {});
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

    // A local vault takes its `passphrase`; a linked one reads its values from its provider.
    registry.register("vault.unseal", |params, ctx| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let id = str_of(&params, "id").to_string();
        let passphrase = str_of(&params, "passphrase").to_string();
        let vault = blocking({
            let (ws_id, id) = (ws_id.clone(), id.clone());
            move || {
                move_off_repository(&ws_id);
                store::get(&ws_id, &id)
            }
        })
        .await?;
        if state::is_unsealed(&ws_id, &id) {
            return Ok(summary(&ws_id, &vault));
        }
        match vault {
            Vault::Local(local) => {
                blocking(move || {
                    let key = verify(&ws_id, &local, &passphrase)?;
                    state::unseal(&ws_id, &id, &local.name, Some(key.clone()), local.read_all(&key));
                    Ok(summary(&ws_id, &Vault::Local(local)))
                })
                .await
            }
            Vault::Linked(mut linked) => {
                require_person(&ctx)?;
                let values = pull_linked(&ws_id, &linked).await?;
                linked.names = values.iter().map(|(name, _)| name.clone()).collect();
                state::unseal(&ws_id, &id, &linked.name, None, values);
                Ok(summary(&ws_id, &Vault::Linked(linked)))
            }
        }
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
            let mut vault = store::get_local(&ws_id, &id)?;
            let old_key = verify(&ws_id, &vault, &old)?;
            let new_key = vault.rekey(&old_key, &new)?;
            store::save(&ws_id, &vault)?;
            state::unseal(&ws_id, &id, &vault.name, Some(new_key.clone()), vault.read_all(&new_key));
            Ok(summary(&ws_id, &Vault::Local(vault)))
        })
        .await
    });

    registry.register("vault.setSecret", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let (id, name) = (str_of(&params, "id").to_string(), str_of(&params, "name").to_string());
        let value = str_of(&params, "value").to_string();
        blocking(move || {
            store::check_secret_name(&name)?;
            check_secret_value(&value)?;
            let mut vault = store::get_local(&ws_id, &id)?;
            let key = ready_key(&ws_id, &vault)?;
            vault.set_secret(&key, &name, &value)?;
            store::save(&ws_id, &vault)?;
            state::update(&ws_id, &id, &vault.name, |values| *values = vault.read_all(&key));
            changed_event(&ws_id, &id);
            Ok(summary(&ws_id, &Vault::Local(vault)))
        })
        .await
    });

    // Sets up a linked vault on this device: its `settings` (shared with the vault) and `credential` (this
    // device's only; an empty one removes it, none keeps it), then approves what the vault's file says. This
    // is how a vault a teammate linked, or one whose file a sync changed, is accepted here.
    registry.register("vault.configure", |params, ctx| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let id = str_of(&params, "id").to_string();
        let credential = params.get("credential").and_then(Value::as_str).map(|c| Zeroizing::new(c.to_string()));
        let settings = params.get("settings").cloned();
        require_person(&ctx)?;
        blocking(move || {
            let mut vault = store::get_linked(&ws_id, &id)?;
            let manager = provider_of(&vault.provider)?;
            let mut link = store::link(&ws_id, &id);
            if let Some(settings) = settings {
                vault.settings = providers::clean_settings(manager, &settings)?;
                store::save_linked(&ws_id, &vault)?;
            }
            if let Some(credential) = &credential {
                link.set_credential(Some(credential))?;
            }
            if manager.credential_required && link.credential.is_none() {
                return Err(EngineError::bad_request(format!("{} needs a token.", manager.label))
                    .keyed("vault.credential_missing", json!({ "provider": manager.label })));
            }
            link.approved = Some(vault.fingerprint());
            store::save_link(&ws_id, &id, &link)?;
            // Values read under what was approved before are not kept past a change of it.
            state::seal(&ws_id, &id);
            Ok(summary(&ws_id, &Vault::Linked(vault)))
        })
        .await
    });

    registry.register("vault.deleteSecret", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let (id, name) = (str_of(&params, "id").to_string(), str_of(&params, "name").to_string());
        blocking(move || {
            // A linked vault's secrets are its provider's: they are removed there, and go at the next pull.
            let mut vault = store::get_local(&ws_id, &id)?;
            let before = vault.secrets.len();
            vault.secrets.retain(|s| s.name != name);
            if vault.secrets.len() == before {
                return Err(EngineError::not_found(format!("No secret \"{name}\" in this vault."))
                    .keyed("vault.secret_not_found", json!({ "name": name })));
            }
            store::save(&ws_id, &vault)?;
            state::update(&ws_id, &id, &vault.name, |values| values.retain(|(n, _)| *n != name));
            changed_event(&ws_id, &id);
            Ok(summary(&ws_id, &Vault::Local(vault)))
        })
        .await
    });

    // The user has seen that a local vault was moved out of the workspace's repository.
    registry.register("vault.acknowledge", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let id = str_of(&params, "id").to_string();
        blocking(move || {
            let mut vault = store::get_local(&ws_id, &id)?;
            if vault.moved_at.take().is_some() {
                store::save(&ws_id, &vault)?;
                changed_event(&ws_id, &id);
            }
            Ok(summary(&ws_id, &Vault::Local(vault)))
        })
        .await
    });

    // The values of an unsealed vault. A sealed one is refused: it is unsealed first (`vault.unseal`).
    registry.register("vault.secrets", |params, ctx| async move {
        require_cli_session(&ctx)?;
        let ws_id = str_of(&params, "workspaceId").to_string();
        let id = str_of(&params, "id").to_string();
        blocking(move || {
            let vault = store::get(&ws_id, &id)?;
            let secrets: serde_json::Map<String, Value> =
                ready_values(&ws_id, &vault)?.into_iter().map(|(name, value)| (name, json!(value.as_str()))).collect();
            Ok(json!({ "id": vault.id(), "name": vault.name(), "secrets": secrets }))
        })
        .await
    });
}

#[cfg(test)]
mod tests;
