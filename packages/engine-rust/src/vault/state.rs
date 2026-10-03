//! What the engine knows about vaults only while it runs: which vaults it holds a key for, and who has
//! each one open. Nothing here is written to disk, and a restart forgets all of it.
//!
//! A vault is **sealed** (no key in memory: its secrets cannot be read), **ready** (the key is held; its
//! secrets resolve in `{{vault.secret}}` and are never sent to a client) or **open** (ready, and handed to
//! a CLI session as environment variables for a program it launched). That lease belongs to the session and
//! ends when the session does.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use serde_json::json;
use zeroize::Zeroizing;

use super::{crypto::Key, store};
use crate::{kernel::bus::bus, utils::id::generate_id};

#[derive(Debug, Clone)]
pub struct Lease {
    pub id: String,
    pub workspace_id: String,
    pub vault_id: String,
    /// The CLI session that holds it.
    pub session_id: String,
}

#[derive(Default)]
struct State {
    keys: HashMap<(String, String), Key>,
    leases: Vec<Lease>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Mutex::default);

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn changed(ws_id: &str, vault_id: &str) {
    bus().emit("vault.changed", json!({ "workspaceId": ws_id, "id": vault_id }));
}

pub fn key_of(ws_id: &str, vault_id: &str) -> Option<Key> {
    state().keys.get(&(ws_id.to_string(), vault_id.to_string())).cloned()
}

pub fn set_key(ws_id: &str, vault_id: &str, key: Key) {
    state().keys.insert((ws_id.to_string(), vault_id.to_string()), key);
    changed(ws_id, vault_id);
}

pub fn leases_of(ws_id: &str, vault_id: &str) -> Vec<Lease> {
    state().leases.iter().filter(|l| l.workspace_id == ws_id && l.vault_id == vault_id).cloned().collect()
}

pub fn add_lease(ws_id: &str, vault_id: &str, session_id: String) -> Lease {
    let lease = Lease {
        id: generate_id(),
        workspace_id: ws_id.to_string(),
        vault_id: vault_id.to_string(),
        session_id,
    };
    state().leases.push(lease.clone());
    changed(ws_id, vault_id);
    lease
}

/// Tell the holders of `leases` that theirs is over: a CLI session stops the program it launched.
fn revoked(leases: &[Lease]) {
    for lease in leases {
        bus().emit("vault.revoked", json!({ "leaseId": lease.id }));
        changed(&lease.workspace_id, &lease.vault_id);
    }
}

/// End one lease. Returns whether it existed.
pub fn release(lease_id: &str) -> bool {
    let removed: Vec<Lease> = {
        let mut state = state();
        let (gone, kept): (Vec<Lease>, Vec<Lease>) = state.leases.drain(..).partition(|l| l.id == lease_id);
        state.leases = kept;
        gone
    };
    revoked(&removed);
    !removed.is_empty()
}

/// End every lease of a vault, whoever holds it.
pub fn close(ws_id: &str, vault_id: &str) -> usize {
    let removed: Vec<Lease> = {
        let mut state = state();
        let (gone, kept): (Vec<Lease>, Vec<Lease>) = state.leases.drain(..).partition(|l| l.workspace_id == ws_id && l.vault_id == vault_id);
        state.leases = kept;
        gone
    };
    revoked(&removed);
    removed.len()
}

/// Forget a vault's key (and close it). Its secrets can be read again only after the passphrase is given.
pub fn seal(ws_id: &str, vault_id: &str) {
    close(ws_id, vault_id);
    state().keys.remove(&(ws_id.to_string(), vault_id.to_string()));
    changed(ws_id, vault_id);
}

/// A vault was deleted: nothing of it may stay in memory.
pub fn forget(ws_id: &str, vault_id: &str) {
    seal(ws_id, vault_id);
}

/// A client session ended: whatever it had open closes with it.
pub fn session_ended(session_id: &str) {
    let removed: Vec<Lease> = {
        let mut state = state();
        let (gone, kept): (Vec<Lease>, Vec<Lease>) = state.leases.drain(..).partition(|l| l.session_id == session_id);
        state.leases = kept;
        gone
    };
    revoked(&removed);
}

/// `{{vault.secret}}`: the value, when the vault exists and is ready.
pub fn resolve_reference(ws_id: &str, vault_name: &str, secret_name: &str) -> Option<Zeroizing<String>> {
    if ws_id.is_empty() {
        return None;
    }
    let vault = store::find_by_name(ws_id, vault_name)?;
    let key = key_of(ws_id, &vault.id)?;
    vault.read_secret(&key, secret_name)
}

/// `{{vault.secret}}` references in `text`, as written, without repeats. `{{random.*}}` is not one.
pub fn references_in(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let inner = &after[..end];
        if let Some((vault, secret)) = inner.split_once('.') {
            let word = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if word(vault) && word(secret) && !vault.eq_ignore_ascii_case("random") {
                let reference = format!("{{{{{inner}}}}}");
                if !found.contains(&reference) {
                    found.push(reference);
                }
            }
        }
        rest = &after[end + 2..];
    }
    found
}

/// Replaces the vault references of `text` that are in `allowed` with their values. A reference the author of
/// a template did not write (one that arrived in a client's request, say) is left as it is, so nothing that
/// passes through the engine can ask it for a secret.
pub fn resolve_allowed(ws_id: &str, text: &str, allowed: &[String]) -> String {
    let mut out = text.to_string();
    for reference in references_in(text).into_iter().filter(|r| allowed.contains(r)) {
        let inner = &reference[2..reference.len() - 2];
        let Some((vault, secret)) = inner.split_once('.') else { continue };
        if let Some(value) = resolve_reference(ws_id, vault, secret) {
            out = out.replace(&reference, &value);
        }
    }
    out
}

/// [`resolve_allowed`] over every string of a JSON value.
pub fn resolve_value_allowed(ws_id: &str, value: &mut serde_json::Value, allowed: &[String]) {
    use serde_json::Value;
    match value {
        Value::String(text) if text.contains("{{") => *text = resolve_allowed(ws_id, text, allowed),
        Value::Array(items) => items.iter_mut().for_each(|item| resolve_value_allowed(ws_id, item, allowed)),
        Value::Object(map) => map.values_mut().for_each(|item| resolve_value_allowed(ws_id, item, allowed)),
        _ => {}
    }
}

/// Masks, in place, whatever of a workspace's ready vaults' secrets `value` holds. A no-op when none is ready.
pub fn redact_value(ws_id: &str, value: &mut serde_json::Value) {
    if let Some(redactor) = Redactor::for_workspace(ws_id) {
        redactor.value(value);
    }
}

/// [`redact_value`] for one string.
pub fn redact_text(ws_id: &str, text: String) -> String {
    match Redactor::for_workspace(ws_id) {
        Some(redactor) => redactor.text(&text),
        None => text,
    }
}

/// How many wrong passphrases in a row a vault takes before it makes the next attempt wait.
const FREE_ATTEMPTS: u32 = 3;
const MAX_WAIT_SECS: u64 = 60;

#[derive(Default)]
struct Failures {
    count: u32,
    last: Option<std::time::Instant>,
}

static FAILURES: LazyLock<Mutex<HashMap<(String, String), Failures>>> = LazyLock::new(Mutex::default);

/// Seconds a caller must still wait before trying a passphrase on this vault; 0 when it may try now. After
/// three wrong ones the wait doubles with each, up to a minute, and a right one resets it. The derivation is
/// slow already; this makes guessing at a running engine slower still.
pub fn attempt_wait_secs(ws_id: &str, vault_id: &str) -> u64 {
    let failures = FAILURES.lock().unwrap_or_else(|p| p.into_inner());
    let Some(entry) = failures.get(&(ws_id.to_string(), vault_id.to_string())) else { return 0 };
    if entry.count < FREE_ATTEMPTS {
        return 0;
    }
    let wait = (1u64 << (entry.count - FREE_ATTEMPTS).min(6)).min(MAX_WAIT_SECS);
    let waited = entry.last.map(|t| t.elapsed().as_secs()).unwrap_or(wait);
    wait.saturating_sub(waited)
}

pub fn attempt_failed(ws_id: &str, vault_id: &str) {
    let mut failures = FAILURES.lock().unwrap_or_else(|p| p.into_inner());
    let entry = failures.entry((ws_id.to_string(), vault_id.to_string())).or_default();
    entry.count += 1;
    entry.last = Some(std::time::Instant::now());
}

pub fn attempt_succeeded(ws_id: &str, vault_id: &str) {
    FAILURES.lock().unwrap_or_else(|p| p.into_inner()).remove(&(ws_id.to_string(), vault_id.to_string()));
}

/// Secrets shorter than this are refused (`vault.setSecret`): masking a short value would blank out every
/// response that happens to contain it, so it could not be kept out of logs.
pub const MIN_SECRET_LEN: usize = 6;

/// Replaces the values of a workspace's ready vaults with the `{{vault.secret}}` that names them, in
/// anything that is about to leave the engine for a client or a file.
pub struct Redactor {
    /// Longest first, so a secret that contains another is masked whole.
    secrets: Vec<(Zeroizing<String>, String)>,
}

impl Redactor {
    /// `None` when no vault of the workspace is ready, which is the usual case and costs nothing.
    pub fn for_workspace(ws_id: &str) -> Option<Self> {
        // Nothing is ready: answer from memory, without reading a file.
        if !state().keys.keys().any(|(ws, _)| ws == ws_id) {
            return None;
        }
        let ready: Vec<(store::VaultFile, Key)> = store::list(ws_id)
            .into_iter()
            .filter_map(|vault| key_of(ws_id, &vault.id).map(|key| (vault, key)))
            .collect();
        let mut secrets = Vec::new();
        for (vault, key) in &ready {
            for (name, value) in vault.read_all(key) {
                if value.len() >= MIN_SECRET_LEN {
                    let reference = format!("{{{{{}.{}}}}}", vault.name, name);
                    for variant in encodings(&value) {
                        secrets.push((variant, reference.clone()));
                    }
                }
            }
        }
        if secrets.is_empty() {
            return None;
        }
        secrets.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        Some(Self { secrets })
    }

    /// Masks the references' values and the forms they take in transit.
    pub fn text(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (value, reference) in &self.secrets {
            if out.contains(value.as_str()) {
                out = out.replace(value.as_str(), reference);
            }
        }
        out
    }

    /// Every string in `value`, including object keys' values and array items.
    pub fn value(&self, value: &mut serde_json::Value) {
        use serde_json::Value;
        match value {
            Value::String(text) => {
                let masked = self.text(text);
                if masked != *text {
                    *text = masked;
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| self.value(item)),
            Value::Object(map) => map.values_mut().for_each(|item| self.value(item)),
            _ => {}
        }
    }

    /// A base64 body: decoded, masked and encoded again when it is text that holds a secret.
    pub fn base64_body(&self, value: &mut serde_json::Value) {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let Some(encoded) = value.as_str() else { return };
        let Ok(bytes) = STANDARD.decode(encoded) else { return };
        let Ok(text) = String::from_utf8(bytes) else { return };
        let masked = self.text(&text);
        if masked != text {
            *value = serde_json::Value::String(STANDARD.encode(masked));
        }
    }
}

/// The value as it appears in a log: itself, base64, percent-encoded and JSON-escaped.
fn encodings(value: &str) -> Vec<Zeroizing<String>> {
    use base64::{Engine as _, engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD}};
    let mut forms = vec![
        value.to_string(),
        STANDARD.encode(value),
        URL_SAFE_NO_PAD.encode(value),
        percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC).to_string(),
        serde_json::to_string(value).map(|s| s[1..s.len() - 1].to_string()).unwrap_or_default(),
    ];
    forms.retain(|form| form.len() >= MIN_SECRET_LEN);
    forms.sort();
    forms.dedup();
    forms.into_iter().map(Zeroizing::new).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_are_found_once_and_random_is_not_one() {
        let found = references_in("a {{v.A}} b {{v.A}} {{random.int}} {{plain}} {{w.B_2}} {{bad name.x}}");
        assert_eq!(found, vec!["{{v.A}}".to_string(), "{{w.B_2}}".to_string()]);
    }

    #[test]
    fn wrong_passphrases_slow_down_after_the_free_attempts_and_a_right_one_resets() {
        let (ws, vault) = ("ws-throttle", "v-throttle");
        for _ in 0..FREE_ATTEMPTS {
            assert_eq!(attempt_wait_secs(ws, vault), 0);
            attempt_failed(ws, vault);
        }
        assert!(attempt_wait_secs(ws, vault) >= 1);
        attempt_succeeded(ws, vault);
        assert_eq!(attempt_wait_secs(ws, vault), 0);
    }

    #[test]
    fn redaction_also_masks_encoded_forms() {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let value = "p@ss word/123";
        let mut secrets: Vec<_> = encodings(value).into_iter().map(|v| (v, "{{v.P}}".to_string())).collect();
        secrets.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        let redactor = Redactor { secrets };
        let text = format!("a={} b={} c={}", value, STANDARD.encode(value), percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC));
        assert_eq!(redactor.text(&text), "a={{v.P}} b={{v.P}} c={{v.P}}");
    }

    #[test]
    fn redaction_masks_longest_secrets_first_and_skips_short_ones() {
        let redactor = Redactor {
            secrets: vec![
                (Zeroizing::new("abcdef-long".into()), "{{v.LONG}}".into()),
                (Zeroizing::new("abcdef".into()), "{{v.SHORT}}".into()),
            ],
        };
        assert_eq!(redactor.text("k=abcdef-long&j=abcdef"), "k={{v.LONG}}&j={{v.SHORT}}");
        let mut value = serde_json::json!({ "h": { "a": ["x abcdef y"] }, "n": 1 });
        redactor.value(&mut value);
        assert_eq!(value, serde_json::json!({ "h": { "a": ["x {{v.SHORT}} y"] }, "n": 1 }));
    }

    #[test]
    fn closing_a_vault_revokes_session_leases_and_sealing_forgets_the_key() {
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let id = bus().on("vault.revoked", move |payload| sink.lock().unwrap().push(payload["leaseId"].clone()));
        let (ws, vault) = ("ws-state-test", "vault-state-test");
        let (_, key) = store::VaultFile::create(vault.into(), "k", "long enough pass", 0).unwrap();
        set_key(ws, vault, key);
        let by_cli = add_lease(ws, vault, "s1".into());
        let other = add_lease(ws, vault, "s2".into());
        assert_eq!(leases_of(ws, vault).len(), 2);
        session_ended("s1");
        assert_eq!(leases_of(ws, vault).len(), 1);
        seal(ws, vault);
        assert!(leases_of(ws, vault).is_empty());
        assert!(key_of(ws, vault).is_none());
        bus().off(id);
        let seen = seen.lock().unwrap();
        assert!(seen.contains(&serde_json::json!(by_cli.id)));
        assert!(seen.contains(&serde_json::json!(other.id)));
    }
}
