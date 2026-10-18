use std::{sync::Arc, time::Duration};

use ring::{
    rand::SystemRandom,
    signature::{
        ECDSA_P256_SHA256_FIXED, ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair, RSA_PKCS1_2048_8192_SHA256,
        RsaKeyPair, UnparsedPublicKey,
    },
};
use serde_json::{Value, json};

use super::*;
use crate::{
    kernel::{CommandContext, CommandRegistry},
    protocol::ErrorCode,
    testing::scratch_data_root,
};

const RSA_PKCS8: &str = include_str!("../../tests/data/jwt_rsa_pkcs8.pem");
const RSA_PKCS1: &str = include_str!("../../tests/data/jwt_rsa_pkcs1.pem");

fn obj(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

fn mint_with(key: &str, alg: Option<Alg>, claims: Value, exp: Option<Duration>) -> Result<Minted> {
    mint(key, alg, &Map::new(), &obj(claims), exp, 1_000)
}

fn parts(token: &str) -> (String, Vec<u8>) {
    let (input, sig) = token.rsplit_once('.').unwrap();
    (input.to_string(), URL_SAFE_NO_PAD.decode(sig).unwrap())
}

fn pem(label: &str, der: &[u8]) -> String {
    format!("-----BEGIN {label}-----\n{}\n-----END {label}-----\n", STANDARD.encode(der))
}

#[test]
fn hs256_matches_the_jwt_io_vector() {
    let claims = json!({"sub": "1234567890", "name": "John Doe", "iat": 1516239022});
    let minted = mint_with("your-256-bit-secret", None, claims, None).unwrap();
    assert_eq!(
        minted.token,
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"
    );
    assert_eq!(minted.alg, Alg::HS256);
}

#[test]
fn hs384_and_hs512_have_their_signature_lengths() {
    for (alg, len) in [(Alg::HS384, 48), (Alg::HS512, 64)] {
        let minted = mint_with("k", Some(alg), json!({}), None).unwrap();
        assert_eq!(parts(&minted.token).1.len(), len);
        assert_eq!(minted.header["alg"], alg.as_str());
    }
}

#[test]
fn rs256_verifies_with_the_public_key_for_both_pem_shapes() {
    for key in [RSA_PKCS8, RSA_PKCS1] {
        let minted = mint_with(key, None, json!({"a": 1}), None).unwrap();
        assert_eq!(minted.alg, Alg::RS256);
        let (input, signature) = parts(&minted.token);
        let pair = RsaKeyPair::from_pkcs8(&pem_der(RSA_PKCS8).unwrap().1).unwrap();
        UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, pair.public_key().as_ref())
            .verify(input.as_bytes(), &signature)
            .expect("a valid RS256 signature");
    }
}

#[test]
fn es256_verifies_and_is_detected_from_a_pkcs8_pem() {
    let rng = SystemRandom::new();
    let der = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, der.as_ref(), &rng).unwrap();
    let key = pem("PRIVATE KEY", der.as_ref());
    let minted = mint_with(&key, None, json!({}), None).unwrap();
    assert_eq!(minted.alg, Alg::ES256);
    let (input, signature) = parts(&minted.token);
    assert_eq!(signature.len(), 64);
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, pair.public_key().as_ref())
        .verify(input.as_bytes(), &signature)
        .expect("a valid ES256 signature");
}

#[test]
fn a_key_that_does_not_fit_the_alg_or_is_sec1_is_an_error() {
    let err = |key: &str, alg| mint_with(key, alg, json!({}), None).err().expect("an error").message;
    assert!(err("secret", Some(Alg::RS256)).contains("needs a PEM private key"));
    assert!(err(RSA_PKCS8, Some(Alg::ES256)).contains("use RS256"));
    assert!(err(RSA_PKCS8, Some(Alg::HS256)).contains("use RS256"));
    assert!(err(&pem("EC PRIVATE KEY", b"x"), None).contains("openssl pkcs8 -topk8 -nocrypt"));
    assert!(err("-----BEGIN PRIVATE KEY-----\n!!\n-----END PRIVATE KEY-----", None).contains("base64"));
    assert!(err(&pem("PRIVATE KEY", b"nonsense"), None).contains("neither an RSA"));
    assert!("HS999".parse::<Alg>().is_err());
    assert_eq!("hs512".parse::<Alg>().unwrap(), Alg::HS512);
}

#[test]
fn iat_and_exp_are_added_unless_the_claims_say_otherwise() {
    let c = |claims, exp| mint_with("k", None, claims, exp).unwrap().claims;
    let minute = Some(Duration::from_secs(60));
    assert_eq!(c(json!({}), minute), obj(json!({"iat": 1000, "exp": 1060})));
    assert_eq!(c(json!({}), None), obj(json!({"iat": 1000})));
    assert_eq!(c(json!({"exp": 5, "iat": 4}), minute), obj(json!({"exp": 5, "iat": 4})));
    let extra = obj(json!({"kid": "k1", "alg": "none"}));
    let minted = mint("k", None, &extra, &Map::new(), None, 1).unwrap();
    assert_eq!(Value::Object(minted.header), json!({"alg": "HS256", "typ": "JWT", "kid": "k1"}));
}

#[test]
fn durations() {
    for (raw, secs) in [("30s", 30), ("5m", 300), ("1h", 3600), ("2d", 172_800), ("90", 90)] {
        assert_eq!(parse_duration(raw).unwrap(), Duration::from_secs(secs), "{raw}");
    }
    for raw in ["", "m", "-5m", "1.5h", "soon"] {
        assert!(parse_duration(raw).is_err(), "{raw}");
    }
}

#[test]
fn the_secret_is_the_named_one_the_only_one_or_jwt_key() {
    let s = |pairs: &[(&str, &str)]| pairs.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect::<Vec<_>>();
    assert_eq!(choose(&s(&[("A", "1")]), "v", None).unwrap(), "1");
    assert_eq!(choose(&s(&[("A", "1"), ("JWT_KEY", "2")]), "v", None).unwrap(), "2");
    assert_eq!(choose(&s(&[("A", "1"), ("B", "2")]), "v", Some("B")).unwrap(), "2");
    let many = s(&[("A", "1"), ("B", "2")]);
    let err = choose(&many, "v", None).unwrap_err();
    assert_eq!((err.code, err.message.contains("A, B")), (ErrorCode::BadRequest, true));
    assert_eq!(choose(&many, "v", Some("C")).unwrap_err().code, ErrorCode::NotFound);
}

async fn call(registry: &Arc<CommandRegistry>, action: &str, payload: Value) -> Result<Value> {
    registry.invoke(action, payload, CommandContext { registry: registry.clone(), session: None }).await
}

#[tokio::test]
async fn the_command_signs_with_a_vault_secret_and_refuses_a_sealed_vault() {
    let root = scratch_data_root();
    let ws = root.ws_id;
    let mut registry = CommandRegistry::new();
    crate::vault::register(&mut registry);
    register(&mut registry);
    let registry = Arc::new(registry);

    let new_vault = json!({"workspaceId": ws, "name": "jwt_dev", "passphrase": "correct horse"});
    let created = call(&registry, "vault.create", new_vault).await.unwrap();
    let id = created["id"].clone();
    let set = |name: &str, value: &str| json!({"workspaceId": ws, "id": id, "name": name, "value": value});
    call(&registry, "vault.setSecret", set("JWT_KEY", "dev-signing-key")).await.unwrap();

    let mint = |extra: Value| {
        let mut p = json!({"workspaceId": ws, "vault": "JWT_DEV", "claims": {"role": "admin"}});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        p
    };
    let out = call(&registry, "jwt.mint", mint(json!({}))).await.unwrap();
    assert_eq!(out["alg"], "HS256");
    assert_eq!(out["claims"]["role"], "admin");
    assert_eq!(out["claims"]["exp"].as_i64().unwrap() - out["claims"]["iat"].as_i64().unwrap(), 900);
    let (input, signature) = parts(out["token"].as_str().unwrap());
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, b"dev-signing-key");
    assert_eq!(signature, ring::hmac::sign(&key, input.as_bytes()).as_ref());

    let none = call(&registry, "jwt.mint", mint(json!({"exp": "none"}))).await.unwrap();
    assert!(none["claims"].get("exp").is_none());
    let five = call(&registry, "jwt.mint", mint(json!({"exp": "5m", "alg": "HS512"}))).await.unwrap();
    assert_eq!(five["claims"]["exp"].as_i64().unwrap() - five["claims"]["iat"].as_i64().unwrap(), 300);
    assert_eq!(five["alg"], "HS512");

    call(&registry, "vault.setSecret", set("OTHER", "another-key")).await.unwrap();
    assert!(call(&registry, "jwt.mint", mint(json!({"secret": "OTHER"}))).await.is_ok());
    let missing = call(&registry, "jwt.mint", mint(json!({"secret": "NOPE"}))).await;
    assert_eq!(missing.unwrap_err().code, ErrorCode::NotFound);
    let not_object = call(&registry, "jwt.mint", mint(json!({"claims": 5}))).await;
    assert_eq!(not_object.unwrap_err().code, ErrorCode::BadRequest);
    let nope = call(&registry, "jwt.mint", json!({"workspaceId": ws, "vault": "nope"})).await;
    assert_eq!(nope.unwrap_err().code, ErrorCode::NotFound);

    call(&registry, "vault.seal", json!({"workspaceId": ws, "id": id})).await.unwrap();
    let sealed = call(&registry, "jwt.mint", mint(json!({}))).await.unwrap_err();
    assert_eq!(sealed.code, ErrorCode::Forbidden);
    assert_eq!(sealed.details.unwrap()["reason"], "vault_sealed");
}
