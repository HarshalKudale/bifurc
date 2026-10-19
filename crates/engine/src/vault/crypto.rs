//! The vault's cryptography: a passphrase becomes a key with Argon2id, and each secret is sealed on its own
//! with AES-256-GCM. The associated data names the vault and the secret, so a sealed value pasted under another
//! name (or into another vault) fails to open instead of silently answering for the wrong secret.

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::protocol::{EngineError, Result};

const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

/// A vault's key. Held only in engine memory and wiped when dropped.
#[derive(Clone)]
pub struct Key(Zeroizing<[u8; KEY_LEN]>);

/// How a passphrase becomes a key. Stored in the vault file, so the cost can be raised later without
/// breaking the vaults already written.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Kdf {
    pub alg: String,
    /// Memory in KiB.
    pub m: u32,
    /// Passes.
    pub t: u32,
    pub p: u32,
    pub salt: String,
}

impl Kdf {
    /// The parameters new vaults get, with a fresh salt.
    pub fn new() -> Self {
        // Tests create many vaults; the cost matters to an attacker, not to them.
        let (m, t) = if cfg!(test) { (256, 1) } else { (64 * 1024, 3) };
        Self { alg: "argon2id".into(), m, t, p: 1, salt: STANDARD.encode(rand::random::<[u8; SALT_LEN]>()) }
    }

    pub fn derive(&self, passphrase: &str) -> Result<Key> {
        if self.alg != "argon2id" {
            return Err(EngineError::unsupported(format!(
                "This vault uses the unknown key derivation \"{}\".",
                self.alg
            ))
            .keyed("vault.unknown_kdf", serde_json::json!({ "alg": self.alg })));
        }
        let salt = STANDARD.decode(&self.salt).map_err(|_| {
            EngineError::engine("The vault's salt is not valid.").keyed("vault.salt_invalid", serde_json::json!({}))
        })?;
        let params = Params::new(self.m, self.t, self.p, Some(KEY_LEN)).map_err(|e| {
            EngineError::engine(format!("The vault's key derivation settings are invalid: {e}"))
                .keyed("vault.kdf_settings_invalid", serde_json::json!({ "detail": e.to_string() }))
        })?;
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(passphrase.as_bytes(), &salt, key.as_mut())
            .map_err(|e| {
                EngineError::engine(format!("Could not derive the vault key: {e}"))
                    .keyed("vault.derive_failed", serde_json::json!({ "detail": e.to_string() }))
            })?;
        Ok(Key(key))
    }
}

/// One sealed value: a random nonce and the ciphertext with its authentication tag, both base64.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sealed {
    pub nonce: String,
    pub ct: String,
}

pub fn seal(key: &Key, aad: &str, plaintext: &[u8]) -> Result<Sealed> {
    let cipher = Aes256Gcm::new_from_slice(key.0.as_ref()).expect("a 32 byte key");
    let nonce: [u8; NONCE_LEN] = rand::random();
    let ct =
        cipher.encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad: aad.as_bytes() }).map_err(|_| {
            EngineError::engine("Could not encrypt the secret.").keyed("vault.encrypt_failed", serde_json::json!({}))
        })?;
    Ok(Sealed { nonce: STANDARD.encode(nonce), ct: STANDARD.encode(ct) })
}

/// The plaintext, or `None` when the key is wrong or the value was changed or moved.
pub fn open(key: &Key, aad: &str, sealed: &Sealed) -> Option<Zeroizing<Vec<u8>>> {
    let nonce = STANDARD.decode(&sealed.nonce).ok().filter(|n| n.len() == NONCE_LEN)?;
    let ct = STANDARD.decode(&sealed.ct).ok()?;
    let cipher = Aes256Gcm::new_from_slice(key.0.as_ref()).expect("a 32 byte key");
    cipher.decrypt(Nonce::from_slice(&nonce), Payload { msg: &ct, aad: aad.as_bytes() }).ok().map(Zeroizing::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_opens_only_with_its_key_and_its_name() {
        let kdf = Kdf::new();
        let key = kdf.derive("correct horse").unwrap();
        let sealed = seal(&key, "v/a", b"s3cret").unwrap();
        assert_eq!(open(&key, "v/a", &sealed).unwrap().as_slice(), b"s3cret");
        assert!(open(&key, "v/b", &sealed).is_none(), "moved under another name");
        assert!(open(&kdf.derive("wrong horse").unwrap(), "v/a", &sealed).is_none(), "wrong passphrase");
        assert!(!sealed.ct.contains("s3cret"));
    }

    #[test]
    fn the_same_value_seals_differently_each_time() {
        let key = Kdf::new().derive("pass phrase").unwrap();
        assert_ne!(seal(&key, "a", b"x").unwrap(), seal(&key, "a", b"x").unwrap());
    }

    #[test]
    fn the_same_passphrase_and_salt_give_the_same_key() {
        let kdf = Kdf::new();
        let (a, b) = (kdf.derive("p").unwrap(), kdf.derive("p").unwrap());
        assert_eq!(a.0.as_slice(), b.0.as_slice());
        assert_ne!(a.0.as_slice(), Kdf::new().derive("p").unwrap().0.as_slice());
    }
}
