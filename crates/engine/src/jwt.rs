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

// ── `{{$jwt …}}` ────────────────────────────────────────────────────────────────────────────────────────

// shortcut: no variables inside a placeholder and no `}}` inside a quoted value (the first `}}` ends it); nest or
// track quotes when someone needs `{{$jwt sub="{{user}}"}}`.
/// Every `{{$jwt …}}` in `text`: its byte range and the argument text between `$jwt` and `}}`.
pub fn placeholders(text: &str) -> Vec<(std::ops::Range<usize>, &str)> {
    let (mut found, mut from) = (Vec::new(), 0);
    while let Some(at) = text[from..].find("{{$jwt") {
        let (start, args) = (from + at, from + at + "{{$jwt".len());
        let rest = &text[args..];
        match rest.find("}}") {
            Some(end) if rest.starts_with("}}") || rest.starts_with(char::is_whitespace) => {
                found.push((start..args + end + 2, &rest[..end]));
                from = args + end + 2;
            }
            _ => from = args,
        }
    }
    found
}

/// `key=value` pairs; a double-quoted value may hold spaces and `\"`, and is always a string.
fn parse_args(text: &str) -> std::result::Result<Vec<(String, String, bool)>, String> {
    let mut pairs = Vec::new();
    let mut chars = text.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        if chars.peek().is_none() {
            return Ok(pairs);
        }
        let mut key = String::new();
        while let Some(c) = chars.next_if(|c| !c.is_whitespace() && *c != '=') {
            key.push(c);
        }
        if key.is_empty() || key.contains('"') || chars.next_if_eq(&'=').is_none() {
            return Err(format!("expected key=value, got \"{key}\""));
        }
        let mut value = String::new();
        let quoted = chars.next_if_eq(&'"').is_some();
        if quoted {
            loop {
                match chars.next() {
                    Some('\\') if matches!(chars.peek(), Some('"' | '\\')) => value.extend(chars.next()),
                    Some('"') => break,
                    Some(c) => value.push(c),
                    None => return Err(format!("the quote after {key}= is not closed")),
                }
            }
            if chars.peek().is_some_and(|c| !c.is_whitespace()) {
                return Err(format!("text right after the closing quote of {key}"));
            }
        } else {
            while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                value.push(c);
            }
        }
        pairs.push((key, value, quoted));
    }
}

/// An unquoted claim value: JSON when it is a number, true/false/null, an array or an object; else text.
fn claim_value(raw: &str, quoted: bool) -> Value {
    let jsonish = raw.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '-' | '[' | '{'))
        || matches!(raw, "true" | "false" | "null");
    match serde_json::from_str::<Value>(raw) {
        Ok(value) if !quoted && jsonish => value,
        _ => json!(raw),
    }
}

/// Mint the token a `{{$jwt …}}` placeholder (`args`: what follows `$jwt`) asks for. The error is the reason as
/// a sentence for a warning; it never holds the key or a token.
pub fn resolve_placeholder(ws_id: &str, args: &str) -> std::result::Result<String, String> {
    let (mut exp, mut alg, mut vault, mut secret) = (None, None, None, None);
    let (mut header, mut claims) = (Map::new(), Map::new());
    for (key, value, quoted) in parse_args(args)? {
        match key.as_str() {
            "exp" => exp = Some(value),
            "alg" => alg = Some(value),
            "vault" => vault = Some(value),
            "secret" => secret = Some(value),
            "kid" => drop(header.insert(key, json!(value))),
            _ => drop(claims.insert(key, claim_value(&value, quoted))),
        }
    }
    let exp = match exp.as_deref() {
        Some("none") => None,
        Some(raw) => Some(parse_duration(raw).map_err(|e| e.message)?),
        None => Some(Duration::from_secs(15 * 60)),
    };
    let alg = alg.map(|a| a.parse::<Alg>()).transpose().map_err(|e| e.message)?;
    let vault = match vault {
        Some(name) => name,
        None => {
            let (mut open, sealed) = crate::vault::vaults_holding(ws_id, "JWT_KEY");
            match (open.len(), sealed.is_empty()) {
                (1, _) => open.remove(0),
                (0, true) => return Err("no vault holds a JWT_KEY secret (name a vault with vault=)".into()),
                (0, false) => {
                    let name = &sealed[0];
                    return Err(format!("vault \"{name}\" holds JWT_KEY but is sealed (bifurc vault unseal {name})"));
                }
                _ => {
                    return Err(format!(
                        "several unsealed vaults hold JWT_KEY ({}); pick one with vault=",
                        open.join(", ")
                    ));
                }
            }
        }
    };
    let secrets = crate::vault::secrets_for_variable(ws_id, &vault)?;
    let key = choose(&secrets, &vault, secret.as_deref()).map_err(|e| e.message)?;
    let now = (crate::utils::id::now_ms() / 1000) as u64;
    mint(key, alg, &header, &claims, exp, now).map(|minted| minted.token).map_err(|e| e.message)
}

/// `text` with its `{{$jwt …}}` placeholders replaced by fresh tokens. One that cannot be minted stays as written
/// and adds `{{$jwt}}: reason` to `warnings` (and goes to stderr, without key or token). With `allowed`, only
/// placeholders listed there are minted (see `vault::resolve_allowed`).
pub fn expand(ws_id: &str, text: &str, allowed: Option<&[String]>, warnings: &mut Vec<String>) -> String {
    if ws_id.is_empty() || !text.contains("{{$jwt") {
        return text.to_string();
    }
    let (mut out, mut last) = (String::with_capacity(text.len()), 0);
    for (range, args) in placeholders(text) {
        out.push_str(&text[last..range.start]);
        last = range.end;
        let whole = &text[range];
        if allowed.is_some_and(|list| !list.iter().any(|item| item == whole)) {
            out.push_str(whole);
            continue;
        }
        match resolve_placeholder(ws_id, args) {
            Ok(token) => out.push_str(&token),
            Err(reason) => {
                eprintln!("warn: {{{{$jwt}}}}: {reason}");
                let warning = format!("{{{{$jwt}}}}: {reason}");
                if !warnings.contains(&warning) {
                    warnings.push(warning);
                }
                out.push_str(whole);
            }
        }
    }
    out.push_str(&text[last..]);
    out
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
