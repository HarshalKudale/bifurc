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

// ── {{$jwt …}} ──────────────────────────────────────────────────────────────────────────────────────────

fn args(text: &str) -> Vec<(String, String, bool)> {
    parse_args(text).unwrap()
}

#[test]
fn placeholder_arguments_are_pairs_with_optional_quotes() {
    let pair = |k: &str, v: &str, q| (k.to_string(), v.to_string(), q);
    assert_eq!(args("  role=admin  exp=5m "), [pair("role", "admin", false), pair("exp", "5m", false)]);
    assert_eq!(
        args(r#"name="John Doe" n="a \"b\" c" p="x\\y""#),
        [pair("name", "John Doe", true), pair("n", r#"a "b" c"#, true), pair("p", r"x\y", true)]
    );
    assert_eq!(args(r#"roles=["a","b"] e="""#), [pair("roles", r#"["a","b"]"#, false), pair("e", "", true)]);
    assert!(args("").is_empty());
    for bad in ["role", "=x", "a=\"open", "a=\"x\"y", "a b=1"] {
        assert!(parse_args(bad).is_err(), "{bad}");
    }
}

#[test]
fn claim_values_are_json_when_unquoted_and_text_otherwise() {
    assert_eq!(claim_value("42", false), json!(42));
    assert_eq!(claim_value("42", true), json!("42"));
    assert_eq!(claim_value("-1.5", false), json!(-1.5));
    assert_eq!(claim_value("true", false), json!(true));
    assert_eq!(claim_value("null", false), json!(null));
    assert_eq!(claim_value(r#"["a",1]"#, false), json!(["a", 1]));
    assert_eq!(claim_value(r#"{"a":1}"#, false), json!({"a": 1}));
    for text in ["admin", "007", "[oops", "1e999", "True"] {
        assert_eq!(claim_value(text, false), json!(text));
    }
}

#[test]
fn placeholders_are_found_with_their_ranges() {
    let text = "a {{$jwt role=admin}} b {{$jwt}} c {{$jwtx}} {{$jwt open";
    let found: Vec<_> = placeholders(text).into_iter().map(|(r, a)| (&text[r], a)).collect();
    assert_eq!(found, [("{{$jwt role=admin}}", " role=admin"), ("{{$jwt}}", "")]);
}

fn payload(token: &str) -> Value {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(token.split('.').nth(1).unwrap()).unwrap()).unwrap()
}

fn header_of(token: &str) -> Value {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(token.split('.').next().unwrap()).unwrap()).unwrap()
}

fn verified(token: &str, key: &str) -> bool {
    let (input, signature) = parts(token);
    ring::hmac::sign(&ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key.as_bytes()), input.as_bytes()).as_ref()
        == signature
}

async fn new_vault(registry: &Arc<CommandRegistry>, ws: &str, name: &str, secrets: &[(&str, &str)]) -> Value {
    let new = json!({"workspaceId": ws, "name": name, "passphrase": "correct horse"});
    let created = call(registry, "vault.create", new).await.unwrap();
    for (secret, value) in secrets {
        let set = json!({"workspaceId": ws, "id": created["id"], "name": secret, "value": value});
        call(registry, "vault.setSecret", set).await.unwrap();
    }
    created["id"].clone()
}

fn echoed_headers(sent: &Value) -> Value {
    let body = crate::utils::replay::decode_base64_lenient(sent["body"].as_str().unwrap());
    serde_json::from_slice::<Value>(&body).unwrap()["headers"].clone()
}

#[tokio::test]
async fn the_variable_picks_its_vault_and_reports_what_it_cannot_do() {
    let root = scratch_data_root();
    let ws = root.ws_id;
    let mut registry = CommandRegistry::new();
    crate::vault::register(&mut registry);
    crate::env::register(&mut registry);
    let registry = Arc::new(registry);
    let env = crate::env::load_merged_environment(ws);
    let resolve = |text: &str| crate::env::resolve_vars(text, env.as_ref());
    let reason = |text: &str| {
        let mut warnings = Vec::new();
        assert_eq!(expand(ws, text, None, &mut warnings), text);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        warnings.remove(0)
    };

    assert!(reason("{{$jwt}}").starts_with("{{$jwt}}: no vault holds a JWT_KEY"));
    let first = new_vault(&registry, ws, "first", &[("JWT_KEY", "key-one"), ("OTHER", "key-other")]).await;
    let token = resolve("{{$jwt role=admin sub=42 id=\"7\" admin=true tags=[1,2] kid=k1 exp=5m}}");
    assert!(verified(&token, "key-one"), "the only vault with JWT_KEY signs");
    let (claims, header) = (payload(&token), header_of(&token));
    assert_eq!((&claims["role"], &claims["sub"], &claims["id"]), (&json!("admin"), &json!(42), &json!("7")));
    assert_eq!((&claims["admin"], &claims["tags"]), (&json!(true), &json!([1, 2])));
    assert_eq!(claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap(), 300);
    assert_eq!((&header["kid"], &header["alg"]), (&json!("k1"), &json!("HS256")));

    let default = payload(&resolve("{{$jwt}}"));
    assert_eq!(default["exp"].as_u64().unwrap() - default["iat"].as_u64().unwrap(), 900);
    assert!(payload(&resolve("{{$jwt exp=none}}")).get("exp").is_none());
    assert!(verified(&resolve("{{$jwt vault=first secret=OTHER}}"), "key-other"));
    let both = resolve("a {{$jwt name=\"John Doe\"}} b {{$jwt}} {{x}}");
    assert!(both.starts_with("a ") && both.ends_with(" {{x}}") && !both.contains("{{$jwt"), "{both}");

    let second = new_vault(&registry, ws, "second", &[("JWT_KEY", "key-two")]).await;
    assert!(reason("{{$jwt}}").contains("several unsealed vaults hold JWT_KEY (first, second)"));
    assert!(verified(&resolve("{{$jwt vault=second}}"), "key-two"));
    assert!(reason("{{$jwt vault=nope}}").contains("no vault named \"nope\""));
    assert!(reason("{{$jwt vault=first secret=NOPE}}").contains("NOPE"));
    assert!(reason("{{$jwt exp=soon}}").contains("exp is 30s"));
    assert!(reason("{{$jwt vault=first alg=RS256}}").contains("PEM"));
    assert!(reason("{{$jwt role}}").contains("expected key=value"));

    call(&registry, "vault.seal", json!({"workspaceId": ws, "id": first})).await.unwrap();
    assert!(verified(&resolve("{{$jwt}}"), "key-two"), "a sealed vault is not a candidate");
    assert!(reason("{{$jwt vault=first}}").contains("vault \"first\" is sealed (bifurc vault unseal first)"));
    call(&registry, "vault.seal", json!({"workspaceId": ws, "id": second})).await.unwrap();
    assert!(reason("{{$jwt}}").contains("holds JWT_KEY but is sealed (bifurc vault unseal first)"));

    // Without a workspace (an environment without one, none at all, env.resolveText) it stays as written.
    let blank = json!({ "variables": [], "workspaceId": "" });
    assert_eq!(crate::env::resolve_vars("{{$jwt}}", Some(&blank)), "{{$jwt}}");
    assert_eq!(crate::env::resolve_vars("{{$jwt}}", None), "{{$jwt}}");
    call(&registry, "vault.unseal", json!({"workspaceId": ws, "id": second, "passphrase": "correct horse"}))
        .await
        .unwrap();
    assert!(verified(&resolve("{{$jwt}}"), "key-two"));
    let params = json!({"workspaceId": ws, "text": "{{$jwt}} {{random.uuid}}"});
    let text = call(&registry, "env.resolveText", params).await.unwrap();
    let text = text["text"].as_str().unwrap();
    assert!(text.starts_with("{{$jwt}} ") && !text.contains("random"), "{text}");
}

#[tokio::test]
async fn a_send_without_an_environment_mints_into_the_wire_header_only() {
    let root = scratch_data_root();
    let ws = root.ws_id;
    let mut registry = CommandRegistry::new();
    crate::vault::register(&mut registry);
    crate::rest::register(&mut registry);
    let registry = Arc::new(registry);
    let id = new_vault(&registry, ws, "dev", &[("JWT_KEY", "wire-key")]).await;
    let base = crate::rest::test_server::start(Default::default()).await;
    let send = || {
        let params = json!({"workspaceId": ws, "request": {"method": "GET", "url": format!("{base}/me"), "body": "",
            "headers": {"Authorization": "Bearer {{$jwt role=admin name=\"John Doe\" exp=5m}}"}}, "scripts": {}});
        call(&registry, "request.send", params)
    };

    let sent = send().await.unwrap();
    let headers = echoed_headers(&sent);
    let bearer = headers["authorization"].as_str().unwrap().strip_prefix("Bearer ").unwrap();
    assert!(verified(bearer, "wire-key"));
    let claims = payload(bearer);
    assert_eq!((&claims["role"], &claims["name"]), (&json!("admin"), &json!("John Doe")));
    assert_eq!(claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap(), 300);
    // What comes back to the client names the placeholder, as it does for a vault reference.
    assert_eq!(sent["request"]["headers"]["Authorization"], "Bearer {{$jwt role=admin name=\"John Doe\" exp=5m}}");
    assert!(sent.get("warnings").is_none());

    call(&registry, "vault.seal", json!({"workspaceId": ws, "id": id})).await.unwrap();
    let sent = send().await.unwrap();
    assert!(echoed_headers(&sent)["authorization"].as_str().unwrap().contains("{{$jwt role=admin"));
    let warnings = sent["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].as_str().unwrap().starts_with("{{$jwt}}: vault \"dev\" holds JWT_KEY but is sealed"));
}
