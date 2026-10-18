//! JWT signing: [`mint`] is a pure function (key, claims, clock in; compact token out), so the `jwt.mint`
//! command and later the `{{$jwt …}}` dynamic variable share it. The key is a vault secret and never logged.
//!
//! Algorithms: HS256/384/512 (the key is the secret text), RS256 (RSA PEM, PKCS#8 or PKCS#1), ES256 (P-256
//! PKCS#8 PEM). Without an explicit algorithm the key decides.

use std::{str::FromStr, time::Duration};

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use ring::{
    hmac,
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, RSA_PKCS1_SHA256, RsaKeyPair},
};
use serde_json::{Map, Value, json};

use crate::{
    kernel::CommandRegistry,
    protocol::{EngineError, Result},
    utils::json::{opt_str, str_of},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alg {
    HS256,
    HS384,
    HS512,
    RS256,
    ES256,
}

impl Alg {
    pub fn as_str(self) -> &'static str {
        match self {
            Alg::HS256 => "HS256",
            Alg::HS384 => "HS384",
            Alg::HS512 => "HS512",
            Alg::RS256 => "RS256",
            Alg::ES256 => "ES256",
        }
    }
}

impl FromStr for Alg {
    type Err = EngineError;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_uppercase().as_str() {
            "HS256" => Ok(Alg::HS256),
            "HS384" => Ok(Alg::HS384),
            "HS512" => Ok(Alg::HS512),
            "RS256" => Ok(Alg::RS256),
            "ES256" => Ok(Alg::ES256),
            _ => Err(EngineError::bad_request(format!("Unsupported alg \"{s}\" (HS256, HS384, HS512, RS256, ES256)."))),
        }
    }
}

// shortcut: only HS256/384/512, RS256 and ES256 (P-256) with unencrypted PEM keys; add PS256, ES384, EdDSA, SEC1 or encrypted keys when someone needs them.
enum Key {
    Secret(Vec<u8>),
    Rsa(RsaKeyPair),
    Ec(EcdsaKeyPair),
}

/// The DER between the BEGIN/END lines of a PEM, and its label.
fn pem_der(pem: &str) -> Result<(String, Vec<u8>)> {
    let bad =
        || EngineError::bad_request("The key looks like a PEM but is not valid base64 between its BEGIN/END lines.");
    let mut lines = pem.lines().map(str::trim).filter(|l| !l.is_empty());
    let label =
        lines.next().and_then(|l| l.strip_prefix("-----BEGIN ")?.strip_suffix("-----")).ok_or_else(bad)?.to_string();
    let body: String = lines.take_while(|l| !l.starts_with("-----END")).collect();
    Ok((label, STANDARD.decode(body).map_err(|_| bad())?))
}

fn parse_key(key: &str, rng: &SystemRandom) -> Result<Key> {
    if !key.trim_start().starts_with("-----BEGIN ") {
        return Ok(Key::Secret(key.as_bytes().to_vec()));
    }
    let (label, der) = pem_der(key)?;
    match label.as_str() {
        "RSA PRIVATE KEY" => RsaKeyPair::from_der(&der).map(Key::Rsa).map_err(|e| {
            EngineError::bad_request(format!("The RSA key is not usable: {e} (RS256 needs 2048 bits or more)."))
        }),
        "PRIVATE KEY" => RsaKeyPair::from_pkcs8(&der)
            .map(Key::Rsa)
            .or_else(|_| EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, rng).map(Key::Ec))
            .map_err(|_| {
                EngineError::bad_request("The PKCS#8 key is neither an RSA key (2048 bits or more) nor a P-256 EC key.")
            }),
        "EC PRIVATE KEY" => Err(EngineError::bad_request(
            "SEC1 \"BEGIN EC PRIVATE KEY\" keys are not supported; convert it: openssl pkcs8 -topk8 -nocrypt -in key.pem -out key-pkcs8.pem",
        )),
        other => Err(EngineError::bad_request(format!(
            "Unsupported key type \"{other}\" (use an unencrypted PKCS#8 or RSA PEM)."
        ))),
    }
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// A signed token with what went into it.
pub struct Minted {
    pub token: String,
    pub alg: Alg,
    pub header: Map<String, Value>,
    pub claims: Map<String, Value>,
}

/// Sign a compact JWT. `claims` gets `iat` = `now` (unix seconds) and, with `exp`, `exp` = `now + exp`; an `iat`
/// or `exp` already in `claims` wins. `header_extra` adds header members (never `alg`).
pub fn mint(
    key: &str,
    alg: Option<Alg>,
    header_extra: &Map<String, Value>,
    claims: &Map<String, Value>,
    exp: Option<Duration>,
    now: u64,
) -> Result<Minted> {
    let rng = SystemRandom::new();
    let key = parse_key(key, &rng)?;
    let alg = match (&key, alg) {
        (Key::Secret(_), None) => Alg::HS256,
        (Key::Rsa(_), None) => Alg::RS256,
        (Key::Ec(_), None) => Alg::ES256,
        (Key::Secret(_), Some(a @ (Alg::HS256 | Alg::HS384 | Alg::HS512)))
        | (Key::Rsa(_), Some(a @ Alg::RS256))
        | (Key::Ec(_), Some(a @ Alg::ES256)) => a,
        (Key::Secret(_), Some(a)) => {
            return Err(EngineError::bad_request(format!(
                "{} needs a PEM private key, but the secret is plain text.",
                a.as_str()
            )));
        }
        (Key::Rsa(_), Some(a)) => {
            return Err(EngineError::bad_request(format!(
                "The key is an RSA key; {} does not fit (use RS256).",
                a.as_str()
            )));
        }
        (Key::Ec(_), Some(a)) => {
            return Err(EngineError::bad_request(format!(
                "The key is a P-256 EC key; {} does not fit (use ES256).",
                a.as_str()
            )));
        }
    };

    let mut header = Map::new();
    header.insert("alg".into(), json!(alg.as_str()));
    header.insert("typ".into(), json!("JWT"));
    header.extend(header_extra.iter().filter(|(k, _)| *k != "alg").map(|(k, v)| (k.clone(), v.clone())));
    let mut claims = claims.clone();
    claims.entry("iat").or_insert(json!(now));
    if let Some(exp) = exp {
        claims.entry("exp").or_insert(json!(now + exp.as_secs()));
    }

    let signing_input = format!(
        "{}.{}",
        b64(Value::Object(header.clone()).to_string().as_bytes()),
        b64(Value::Object(claims.clone()).to_string().as_bytes())
    );
    let signature = match (&key, alg) {
        (Key::Secret(secret), Alg::HS256) => {
            hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, secret), signing_input.as_bytes()).as_ref().to_vec()
        }
        (Key::Secret(secret), Alg::HS384) => {
            hmac::sign(&hmac::Key::new(hmac::HMAC_SHA384, secret), signing_input.as_bytes()).as_ref().to_vec()
        }
        (Key::Secret(secret), _) => {
            hmac::sign(&hmac::Key::new(hmac::HMAC_SHA512, secret), signing_input.as_bytes()).as_ref().to_vec()
        }
        (Key::Rsa(pair), _) => {
            let mut signature = vec![0; pair.public().modulus_len()];
            pair.sign(&RSA_PKCS1_SHA256, &rng, signing_input.as_bytes(), &mut signature)
                .map_err(|_| EngineError::bad_request("RSA signing failed."))?;
            signature
        }
        (Key::Ec(pair), _) => pair
            .sign(&rng, signing_input.as_bytes())
            .map_err(|_| EngineError::bad_request("ECDSA signing failed."))?
            .as_ref()
            .to_vec(),
    };
    Ok(Minted { token: format!("{signing_input}.{}", b64(&signature)), alg, header, claims })
}

/// `5m`, `1h`, `30s`, `2d`, or a bare number of seconds.
pub fn parse_duration(raw: &str) -> Result<Duration> {
    let raw = raw.trim();
    let (number, unit) = match raw.chars().last() {
        Some('s') => (&raw[..raw.len() - 1], 1),
        Some('m') => (&raw[..raw.len() - 1], 60),
        Some('h') => (&raw[..raw.len() - 1], 3600),
        Some('d') => (&raw[..raw.len() - 1], 86400),
        _ => (raw, 1),
    };
    number
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(unit))
        .map(Duration::from_secs)
        .ok_or_else(|| EngineError::bad_request(format!("exp is 30s, 5m, 1h, 2d or none, not \"{raw}\".")))
}

/// The secret that signs: the named one, else the vault's only one, else `JWT_KEY`.
fn choose<'s>(secrets: &'s [(String, String)], vault: &str, wanted: Option<&str>) -> Result<&'s str> {
    let names = || secrets.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ");
    let found = match wanted {
        Some(name) => secrets.iter().find(|(n, _)| n == name).ok_or_else(|| {
            EngineError::not_found(format!("No secret \"{name}\" in vault \"{vault}\" (it has: {}).", names()))
        })?,
        None => match secrets {
            [only] => only,
            _ => secrets.iter().find(|(n, _)| n == "JWT_KEY").ok_or_else(|| {
                EngineError::bad_request(format!(
                    "Vault \"{vault}\" has {} secrets; pick one with secret (it has: {}).",
                    secrets.len(),
                    names()
                ))
            })?,
        },
    };
    Ok(&found.1)
}

fn object_param(params: &Value, key: &str) -> Result<Map<String, Value>> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map.clone()),
        Some(_) => Err(EngineError::bad_request(format!("{key} must be a JSON object."))),
    }
}

pub fn register(registry: &mut CommandRegistry) {
    // Default lifetime 15 minutes; `exp: "none"` leaves the claim out.
    registry.register("jwt.mint", |params, _| async move {
        let vault = str_of(&params, "vault");
        let secrets = crate::vault::secrets_by_name(str_of(&params, "workspaceId"), vault).await?;
        let exp = match opt_str(&params, "exp") {
            Some("none") => None,
            Some(raw) => Some(parse_duration(raw)?),
            None => Some(Duration::from_secs(15 * 60)),
        };
        let alg = opt_str(&params, "alg").map(str::parse).transpose()?;
        let (header, claims) = (object_param(&params, "header")?, object_param(&params, "claims")?);
        let key = choose(&secrets, vault, opt_str(&params, "secret"))?;
        let now = (crate::utils::id::now_ms() / 1000) as u64;
        let minted = mint(key, alg, &header, &claims, exp, now)?;
        Ok(json!({ "token": minted.token, "alg": minted.alg.as_str(), "header": minted.header, "claims": minted.claims }))
    });
}

#[cfg(test)]
mod tests;
