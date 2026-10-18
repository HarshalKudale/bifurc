//! `jwt decode <token | --from-jar host>` and `jwt mint --key-from-vault <vault>`.
//!
//! Decoding reads the token as text and never verifies the signature. Cookie values of the jar stay masked
//! everywhere else; `--from-jar` prints the claims of the JWT-shaped ones, never the raw token. Signing is the
//! engine's `jwt.mint` (the same code the `{{$jwt …}}` variable uses); the vault is unsealed here first, the
//! way every vault-using command does it.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Map, Value, json};

use crate::{
    commands::{Context, unknown_verb, workspace_id},
    error::{CliError, Result},
    webhooks::format_time,
};

/// A token read as text: header and claims, whatever the signature says.
struct Decoded {
    header: Value,
    claims: Value,
}

fn part(text: &str) -> Option<Value> {
    // Padding is tolerated, standard-alphabet characters are not.
    let bytes = URL_SAFE_NO_PAD.decode(text.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok().filter(Value::is_object)
}

fn decode_token(token: &str) -> Result<Decoded> {
    let pieces: Vec<&str> = token.trim().split('.').collect();
    if pieces.len() == 5 {
        return Err(CliError::usage("encrypted JWT (JWE) — cannot decode without the key"));
    }
    let decoded = match pieces.as_slice() {
        [header, claims, _signature] => part(header).zip(part(claims)),
        _ => None,
    };
    let (header, claims) = decoded.ok_or_else(|| CliError::usage("not a JWT (expected three base64url parts)"))?;
    Ok(Decoded { header, claims })
}

// shortcut: decode never verifies the signature or checks aud/iss; add `--key-from-vault` verification when a test needs it.
/// `4m 12s`, `3h 2m`, `2d 5h`: the two largest units.
fn span(seconds: i64) -> String {
    let s = seconds.abs();
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {}s", s / 60, s % 60),
        3600..=86_399 => format!("{}h {}m", s / 3600, s % 3600 / 60),
        _ => format!("{}d {}h", s / 86_400, s % 86_400 / 3600),
    }
}

/// `(in 4m 12s)` for a time ahead, `(5m 0s ago)` for one behind; `expired` says so for a lapsed `exp`.
fn relative(at: i64, now: i64, expiry: bool) -> String {
    match (at - now, expiry) {
        (ahead, _) if ahead > 0 => format!("(in {})", span(ahead)),
        (behind, true) => format!("(expired {} ago)", span(behind)),
        (behind, false) => format!("({} ago)", span(behind)),
    }
}

/// The numeric time claims present, as `(name, unix seconds)`.
fn times(claims: &Value) -> Vec<(&'static str, i64)> {
    ["exp", "nbf", "iat"].iter().filter_map(|name| Some((*name, claims.get(*name)?.as_f64()? as i64))).collect()
}

/// Why `--check` refuses this token, if it does.
fn refusal(claims: &Value, now: i64) -> Option<String> {
    let at = |name| times(claims).into_iter().find(|(n, _)| *n == name).map(|(_, t)| t);
    if let Some(exp) = at("exp").filter(|exp| *exp <= now) {
        return Some(format!("✗ expired {} ago", span(now - exp)));
    }
    at("nbf")
        .filter(|nbf| *nbf > now)
        .map(|nbf| format!("✗ not valid before {} {}", format_time(nbf as u64), relative(nbf, now, false)))
}

fn describe(decoded: &Decoded, now: i64) -> String {
    let mut lines =
        vec![format!("header  {}", decoded.header), serde_json::to_string_pretty(&decoded.claims).unwrap_or_default()];
    for (name, at) in times(&decoded.claims) {
        lines.push(format!("{name}  {}  {}", format_time(at.max(0) as u64), relative(at, now, name == "exp")));
    }
    lines.push("signature not verified".to_string());
    lines.join("\n")
}

fn as_json(decoded: &Decoded, now: i64, source: &str) -> Value {
    let times: Map<String, Value> = times(&decoded.claims)
        .into_iter()
        .map(|(name, at)| (name.to_string(), json!({ "at": format_time(at.max(0) as u64), "inSeconds": at - now })))
        .collect();
    let expired = times_expired(&decoded.claims, now);
    json!({
        "schemaVersion": 1, "header": decoded.header, "claims": decoded.claims, "times": times,
        "expired": expired, "source": source,
    })
}

fn times_expired(claims: &Value, now: i64) -> bool {
    times(claims).iter().any(|(name, at)| *name == "exp" && *at <= now)
}

/// `host` as the bare host name: a pasted URL is cut down to it.
fn bare_host(host: &str) -> String {
    let rest = host.split_once("://").map_or(host, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    authority
        .rsplit_once(':')
        .filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit()))
        .map_or(authority, |(h, _)| h)
        .to_lowercase()
}

/// The JWT-shaped cookies of `jar` whose domain is `host` or a parent of it, as `(source, token)`.
fn jar_tokens(jar: &Value, host: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for domain in jar["domains"].as_array().into_iter().flatten() {
        let name = domain["domain"].as_str().unwrap_or_default().trim_start_matches('.').to_lowercase();
        if name.is_empty() || !(host == name || host.ends_with(&format!(".{name}"))) {
            continue;
        }
        for cookie in domain["cookies"].as_array().into_iter().flatten() {
            let value = cookie["value"].as_str().unwrap_or_default();
            if decode_token(value).is_ok() {
                found.push((format!("cookie {}", cookie["name"].as_str().unwrap_or_default()), value.to_string()));
            }
        }
    }
    found
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs() as i64)
}

/// `jwt decode <token> | --from-jar <host> [--check] [--json]`.
fn decode(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["from-jar", "check"], 1)?;
    let tokens: Vec<(String, String)> = match (ctx.args.positional(2), ctx.args.flag("from-jar")) {
        (Some(_), Some(_)) => return Err(CliError::usage("give a token or --from-jar <host>, not both")),
        (Some(token), None) => vec![("argument".to_string(), token.to_string())],
        (None, Some(host)) => {
            let host = bare_host(host);
            let workspace = workspace_id(ctx)?;
            let jar = ctx.request("cookies.list", json!({ "workspaceId": workspace }))?;
            let found = jar_tokens(&jar, &host);
            if found.is_empty() {
                return Err(CliError::not_found(format!("no JWT cookie for {host}")));
            }
            found
        }
        (None, None) => {
            return Err(CliError::usage("missing the token (or --from-jar <host>): bifurc jwt decode <token>"));
        }
    };
    let now = now_secs();
    let check = ctx.args.has("check");
    let mut blocks = Vec::new();
    let mut objects = Vec::new();
    let mut refused = false;
    for (source, token) in &tokens {
        let decoded = decode_token(token)?;
        let why = check.then(|| refusal(&decoded.claims, now)).flatten();
        refused |= why.is_some();
        objects.push(as_json(&decoded, now, source));
        let mut block = describe(&decoded, now);
        if source != "argument" {
            block = format!("{source}\n{block}");
        }
        if let Some(why) = why {
            block = format!("{block}\n{why}");
        }
        blocks.push(block);
    }
    let value = if objects.len() == 1 { objects.remove(0) } else { Value::Array(objects) };
    let text = ctx.print(&value, || blocks.join("\n\n"));
    if refused { Err(CliError::Failed(text)) } else { Ok(text) }
}

fn object_flag(ctx: &Context, name: &str) -> Result<Option<Value>> {
    let Some(raw) = ctx.args.flag(name) else { return Ok(None) };
    match serde_json::from_str::<Value>(raw) {
        Ok(object @ Value::Object(_)) => Ok(Some(object)),
        _ => Err(CliError::usage(format!("--{name} must be a JSON object, e.g. --{name} '{{\"role\":\"admin\"}}'"))),
    }
}

/// `jwt mint [--claims '<json>'] [--exp 5m|none] --key-from-vault <vault> [--secret K] [--alg A] [--header '<json>']`.
fn mint(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["claims", "exp", "key-from-vault", "secret", "alg", "header", "passphrase-stdin"], 0)?;
    let Some(vault) = ctx.args.flag("key-from-vault").map(str::to_string) else {
        return Err(CliError::usage("jwt mint needs --key-from-vault <vault>: the vault whose secret signs the token"));
    };
    let (claims, header) = (object_flag(ctx, "claims")?, object_flag(ctx, "header")?);
    let mut payload = json!({ "vault": vault });
    for (key, value) in [("claims", claims), ("header", header)] {
        if let Some(value) = value {
            payload[key] = value;
        }
    }
    for (key, flag) in [("secret", "secret"), ("exp", "exp"), ("alg", "alg")] {
        if let Some(value) = ctx.args.flag(flag) {
            payload[key] = json!(value);
        }
    }
    let workspace = workspace_id(ctx)?;
    payload["workspaceId"] = json!(workspace);
    crate::vault::unseal_named(ctx, &workspace, &vault)?;
    // A bad exp, alg or secret choice is the caller's mistake: a usage error, not a failed run.
    let minted = ctx.request("jwt.mint", payload).map_err(|err| match err {
        CliError::Engine { code, message } if code == "BAD_REQUEST" => CliError::usage(message),
        other => other,
    })?;
    Ok(ctx.print(&minted, || minted["token"].as_str().unwrap_or_default().to_string()))
}

pub fn jwt(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    match sub {
        Some("decode") => decode(ctx),
        Some("mint") => mint(ctx),
        Some(other) => Err(unknown_verb("jwt", other)),
        None => Err(CliError::usage("missing the verb: decode | mint (see: bifurc jwt --help)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run as with_engine};

    fn go(argv: &[&str], answers: Vec<Answer>) -> (Result<String>, Vec<(String, Value)>) {
        with_engine(argv, answers, |ctx| jwt(ctx, argv.get(1).copied()))
    }

    fn token(header: Value, claims: Value) -> String {
        let part = |v: &Value| URL_SAFE_NO_PAD.encode(v.to_string());
        format!("{}.{}.c2ln", part(&header), part(&claims))
    }

    fn token_of(claims: Value) -> String {
        token(json!({"alg": "HS256", "typ": "JWT"}), claims)
    }

    #[test]
    fn spans_are_the_two_largest_units() {
        assert_eq!([span(5), span(252), span(10_920), span(190_000)], ["5s", "4m 12s", "3h 2m", "2d 4h"]);
        assert_eq!(relative(1_252, 1_000, true), "(in 4m 12s)");
        assert_eq!(relative(0, 10_920, true), "(expired 3h 2m ago)");
        assert_eq!(relative(700, 1_000, false), "(5m 0s ago)");
    }

    #[test]
    fn decode_prints_header_claims_and_times() {
        let now = now_secs();
        let t = token_of(json!({"sub": "7", "exp": now + 252, "iat": now - 300}));
        let (out, calls) = go(&["jwt", "decode", &t], vec![]);
        let out = out.unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], r#"header  {"alg":"HS256","typ":"JWT"}"#);
        assert!(out.contains("\"sub\": \"7\""), "{out}");
        assert!(lines.iter().any(|l| l.starts_with("exp  ") && l.contains("Z  (in 4m ")), "{out}");
        assert!(lines.iter().any(|l| l.starts_with("iat  ") && l.ends_with("(5m 0s ago)")), "{out}");
        assert_eq!(lines.last(), Some(&"signature not verified"));
        assert!(calls.is_empty());
    }

    #[test]
    fn padding_is_tolerated_and_check_refuses_expired_and_not_yet_valid() {
        let now = now_secs();
        let t = token_of(json!({"exp": now - 10_920}));
        let (head, rest) = t.split_once('.').unwrap();
        let (body, sig) = rest.split_once('.').unwrap();
        let with_padding = format!("{head}==.{body}{}.{sig}", "=".repeat((4 - body.len() % 4) % 4));
        assert!(go(&["jwt", "decode", &with_padding], vec![]).0.is_ok());

        let (out, _) = go(&["jwt", "decode", &t, "--check"], vec![]);
        let err = out.unwrap_err();
        assert_eq!(err.code(), 1);
        assert!(err.to_string().contains("(expired 3h 2m ago)"), "{err}");
        assert!(err.to_string().ends_with("✗ expired 3h 2m ago"), "{err}");

        let future = token_of(json!({"nbf": now + 3_700}));
        let err = go(&["jwt", "decode", &future, "--check"], vec![]).0.unwrap_err();
        assert_eq!(err.code(), 1);
        assert!(err.to_string().contains("✗ not valid before 20") && err.to_string().ends_with("(in 1h 1m)"), "{err}");
        // Without --check an expired token only says so.
        assert!(go(&["jwt", "decode", &t], vec![]).0.is_ok());
    }

    #[test]
    fn not_a_jwt_and_jwe_are_usage_errors() {
        for (input, message) in [
            ("hello", "not a JWT (expected three base64url parts)"),
            ("a.b.c", "not a JWT (expected three base64url parts)"),
            ("a.b.c.d.e", "encrypted JWT (JWE) — cannot decode without the key"),
        ] {
            let err = go(&["jwt", "decode", input], vec![]).0.unwrap_err();
            assert_eq!((err.code(), err.to_string().as_str()), (2, message));
        }
        assert_eq!(go(&["jwt", "decode"], vec![]).0.unwrap_err().code(), 2);
        assert_eq!(go(&["jwt", "nope"], vec![]).0.unwrap_err().code(), 2);
    }

    #[test]
    fn json_has_the_times_and_whether_it_expired() {
        let now = now_secs();
        let t = token_of(json!({"exp": now - 60}));
        let out = go(&["jwt", "decode", &t, "--json"], vec![]).0.unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            (v["schemaVersion"].clone(), v["expired"].clone(), v["source"].clone()),
            (json!(1), json!(true), json!("argument"))
        );
        assert!(v["times"]["exp"]["inSeconds"].as_i64().unwrap() <= -59);
        assert_eq!(v["header"]["alg"], "HS256");
    }

    fn jar(token: &str) -> Vec<Answer> {
        let jar = json!({"domains": [
            {"domain": "localhost", "cookies": [{"name": "sid", "value": "plain"}]},
            {"domain": ".example.com", "cookies": [{"name": "auth", "value": token}]},
            {"domain": "other.com", "cookies": [{"name": "auth", "value": token}]},
        ]});
        vec![("config.get", Ok(config())), ("cookies.list", Ok(jar))]
    }

    #[test]
    fn from_jar_decodes_the_jwt_cookies_of_the_host_and_its_parents() {
        let t = token_of(json!({"role": "admin"}));
        let (out, calls) = go(&["jwt", "decode", "--from-jar", "api.example.com"], jar(&t));
        let out = out.unwrap();
        assert!(out.starts_with("cookie auth\nheader  "), "{out}");
        assert!(out.contains("\"role\": \"admin\"") && !out.contains(&t), "{out}");
        assert_eq!(payloads(&calls, "cookies.list"), [json!({"workspaceId": "w1"})]);
        // A pasted URL is cut to its host; two matching cookies are two blocks.
        let jar2 = json!({"domains": [{"domain": "a.test", "cookies": [
            {"name": "one", "value": t}, {"name": "two", "value": t}, {"name": "x", "value": "plain"}]}]});
        let answers = vec![("config.get", Ok(config())), ("cookies.list", Ok(jar2))];
        let out = go(&["jwt", "decode", "--from-jar", "https://a.test:8080/x"], answers).0.unwrap();
        assert_eq!(out.matches("signature not verified").count(), 2);
        assert!(out.contains("\n\ncookie two\n"), "{out}");
    }

    #[test]
    fn from_jar_with_nothing_for_the_host_is_not_found() {
        let t = token_of(json!({}));
        let err = go(&["jwt", "decode", "--from-jar", "api.localhost"], jar(&t)).0.unwrap_err();
        assert_eq!((err.code(), err.to_string().as_str()), (3, "no JWT cookie for api.localhost"));
        let err = go(&["jwt", "decode", "--from-jar", "xample.com"], jar(&t)).0.unwrap_err();
        assert_eq!(err.code(), 3);
    }

    fn vault_engine(state: &str) -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            ("vault.list", Ok(json!([{"id": "a1", "name": "jwt_dev", "state": state, "secrets": ["JWT_KEY"]}]))),
            ("vault.unseal", Ok(json!({"id": "a1", "state": "unsealed"}))),
            ("jwt.mint", Ok(json!({"token": "tok.en.sig", "alg": "HS256", "header": {}, "claims": {"role": "admin"}}))),
        ]
    }

    #[test]
    fn mint_prints_only_the_token_and_passes_the_options_on() {
        let argv = ["jwt", "mint", "--key-from-vault", "jwt_dev", "--claims", r#"{"role":"admin"}"#, "--exp", "5m"];
        let (out, calls) = go(&argv, vault_engine("unsealed"));
        assert_eq!(out.unwrap(), "tok.en.sig");
        assert_eq!(
            payloads(&calls, "jwt.mint"),
            [json!({"workspaceId": "w1", "vault": "jwt_dev", "claims": {"role": "admin"}, "exp": "5m"})]
        );
        assert!(payloads(&calls, "vault.unseal").is_empty());
        let json_out = go(&[&argv[..], &["--json"]].concat(), vault_engine("unsealed")).0.unwrap();
        assert_eq!(serde_json::from_str::<Value>(&json_out).unwrap()["claims"]["role"], "admin");
    }

    #[test]
    fn mint_needs_a_vault_and_an_object_and_a_sealed_vault_needs_a_human() {
        let (out, calls) = go(&["jwt", "mint", "--claims", "{}"], vault_engine("unsealed"));
        assert_eq!(out.unwrap_err().code(), 2);
        assert!(calls.is_empty());
        for bad in ["[1]", "nope", "5"] {
            let (out, calls) = go(&["jwt", "mint", "--key-from-vault", "v", "--claims", bad], vault_engine("unsealed"));
            assert_eq!(out.unwrap_err().code(), 2, "{bad}");
            assert!(payloads(&calls, "jwt.mint").is_empty());
        }
        if crate::prompt::interactive() || std::env::var_os("BIFURC_VAULT_PASSPHRASE").is_some() {
            return;
        }
        let (out, calls) = go(&["jwt", "mint", "--key-from-vault", "jwt_dev"], vault_engine("sealed"));
        assert_eq!(out.unwrap_err().code(), 6);
        assert!(payloads(&calls, "jwt.mint").is_empty());
    }

    #[test]
    fn an_engine_bad_request_is_a_usage_error() {
        let mut answers = vault_engine("unsealed");
        answers.insert(
            0,
            ("jwt.mint", Err(CliError::Engine { code: "BAD_REQUEST".into(), message: "exp is 30s, 5m".into() })),
        );
        let err = go(&["jwt", "mint", "--key-from-vault", "jwt_dev", "--exp", "x"], answers).0.unwrap_err();
        assert_eq!((err.code(), err.to_string().as_str()), (2, "exp is 30s, 5m"));
    }
}
