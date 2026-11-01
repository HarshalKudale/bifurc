//! What the engine knows about vaults only while it runs: which vaults are unsealed, and their values. Nothing
//! here is written to disk, and a restart forgets all of it.
//!
//! A vault is **sealed** (nothing in memory: its secrets cannot be read) or **unsealed** (its values are
//! held until the vault is sealed or the engine stops; they resolve in `{{vault.secret}}` and the command line
//! can launch a program with them as environment variables). A local vault's values are opened with its key,
//! which is held too so that secrets can be added; a linked vault's are read from its provider once, when it
//! is unsealed.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use serde_json::json;
use zeroize::Zeroizing;

use super::crypto::Key;
use crate::kernel::bus::bus;

pub type Values = Vec<(String, Zeroizing<String>)>;

struct Open {
    name: String,
    /// A local vault's key; `None` for a linked one.
    key: Option<Key>,
    values: Values,
}

#[derive(Default)]
struct State {
    open: HashMap<(String, String), Open>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Mutex::default);

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn changed(ws_id: &str, vault_id: &str) {
    bus().emit("vault.changed", json!({ "workspaceId": ws_id, "id": vault_id }));
}

fn slot(ws_id: &str, vault_id: &str) -> (String, String) {
    (ws_id.to_string(), vault_id.to_string())
}

pub fn is_unsealed(ws_id: &str, vault_id: &str) -> bool {
    state().open.contains_key(&slot(ws_id, vault_id))
}

/// A local vault's key, while it is unsealed.
pub fn key_of(ws_id: &str, vault_id: &str) -> Option<Key> {
    state().open.get(&slot(ws_id, vault_id)).and_then(|open| open.key.clone())
}

/// The values of an unsealed vault.
pub fn values_of(ws_id: &str, vault_id: &str) -> Option<Values> {
    state().open.get(&slot(ws_id, vault_id)).map(|open| open.values.clone())
}

/// Unseals a vault with these values (and, for a local one, its key).
pub fn unseal(ws_id: &str, vault_id: &str, name: &str, key: Option<Key>, values: Values) {
    state().open.insert(slot(ws_id, vault_id), Open { name: name.to_string(), key, values });
    changed(ws_id, vault_id);
}

/// Replaces the values of an unsealed vault, after one of them changed; a sealed vault stays sealed.
pub fn update(ws_id: &str, vault_id: &str, name: &str, values: impl FnOnce(&mut Values)) {
    if let Some(open) = state().open.get_mut(&slot(ws_id, vault_id)) {
        open.name = name.to_string();
        values(&mut open.values);
    }
}

/// Forget a vault's values. Its secrets can be read again only after it is unsealed again.
pub fn seal(ws_id: &str, vault_id: &str) {
    state().open.remove(&slot(ws_id, vault_id));
    changed(ws_id, vault_id);
}

/// Every test's data root starts with nothing unsealed.
#[cfg(test)]
pub fn forget_all() {
    state().open.clear();
}

/// A vault was deleted: nothing of it may stay in memory.
pub fn forget(ws_id: &str, vault_id: &str) {
    seal(ws_id, vault_id);
}

/// `{{vault.secret}}`: the value, when the vault exists and is ready.
pub fn resolve_reference(ws_id: &str, vault_name: &str, secret_name: &str) -> Option<Zeroizing<String>> {
    if ws_id.is_empty() {
        return None;
    }
    // By the vault's file, not the name it had when unsealed: a sync may have renamed or removed it since.
    let vault = super::store::find_by_name(ws_id, vault_name)?;
    let state = state();
    let open = state.open.get(&slot(ws_id, vault.id()))?;
    open.values.iter().find(|(name, _)| name == secret_name).map(|(_, value)| value.clone())
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
    // `{{$jwt …}}` too, as its text reads once a JSON string is decoded (`\"` is `"`).
    for (range, _) in crate::jwt::placeholders(text) {
        let reference = text[range].replace("\\\"", "\"").replace("\\\\", "\\");
        if !found.contains(&reference) {
            found.push(reference);
        }
    }
    found
}

/// Replaces the vault references of `text` that are in `allowed` with their values. A reference the author of
/// a template did not write (one that arrived in a client's request, say) is left as it is, so nothing that
/// passes through the engine can ask it for a secret.
pub fn resolve_allowed(ws_id: &str, text: &str, allowed: &[String], warnings: &mut Vec<String>) -> String {
    let mut out = crate::jwt::expand(ws_id, text, Some(allowed), warnings);
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
pub fn resolve_value_allowed(
    ws_id: &str,
    value: &mut serde_json::Value,
    allowed: &[String],
    warnings: &mut Vec<String>,
) {
    use serde_json::Value;
    match value {
        Value::String(text) if text.contains("{{") => *text = resolve_allowed(ws_id, text, allowed, warnings),
        Value::Array(items) => items.iter_mut().for_each(|item| resolve_value_allowed(ws_id, item, allowed, warnings)),
        Value::Object(map) => map.values_mut().for_each(|item| resolve_value_allowed(ws_id, item, allowed, warnings)),
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
        let mut secrets = Vec::new();
        for (_, open) in state().open.iter().filter(|((ws, _), _)| ws == ws_id) {
            for (name, value) in &open.values {
                if value.len() >= MIN_SECRET_LEN {
                    let reference = format!("{{{{{}.{}}}}}", open.name, name);
                    for variant in encodings(value) {
                        secrets.push((variant, reference.clone()));
                    }
                }
            }
        }
        if secrets.is_empty() {
            return None;
        }
        secrets.sort_by_key(|b| std::cmp::Reverse(b.0.len()));
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
    use base64::{
        Engine as _,
        engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    };
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
        secrets.sort_by_key(|b| std::cmp::Reverse(b.0.len()));
        let redactor = Redactor { secrets };
        let text = format!(
            "a={} b={} c={}",
            value,
            STANDARD.encode(value),
            percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC)
        );
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
    fn values_are_held_until_the_vault_is_sealed() {
        // Holds the store's turn: another test's scratch root clears what is unsealed.
        let _root = crate::testing::scratch_data_root();
        let (ws, vault) = ("ws-state-test", "vault-state-test");
        let (_, key) = super::super::store::VaultFile::create(vault.into(), "k", "long enough pass", 0).unwrap();
        unseal(ws, vault, "keys", Some(key), vec![("A".into(), Zeroizing::new("value-a".into()))]);
        assert!(key_of(ws, vault).is_some());
        update(ws, vault, "keys", |values| values.push(("B".into(), Zeroizing::new("value-b".into()))));
        assert_eq!(values_of(ws, vault).unwrap().len(), 2);
        seal(ws, vault);
        assert!(key_of(ws, vault).is_none());
        assert!(values_of(ws, vault).is_none());
        update(ws, vault, "keys", |values| values.clear());
        assert!(!is_unsealed(ws, vault), "an update does not unseal");
    }
}
