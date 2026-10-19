//! `policy show` and `policy check <METHOD> <url>`: what the workspace's policy file says. Nothing here writes
//! the file; a person edits it. A call that is blocked or held on the way to the network is the engine's refusal,
//! mapped to exit 7 or 6 in `session.rs`; `check` answers with the same codes without sending.

use serde_json::{Value, json};

use crate::{
    commands::{Context, need, unknown_verb, workspace_id},
    error::{CliError, Result},
    format::aligned,
};

/// `env=prod methods=DELETE,PUT`: the match keys a policy sets.
fn matches_of(policy: &Value) -> String {
    let list = |key: &str| -> Option<String> {
        let items: Vec<&str> = policy.get(key)?.as_array()?.iter().filter_map(Value::as_str).collect();
        (!items.is_empty()).then(|| format!("{key}={}", items.join(",")))
    };
    let all = ["env", "methods", "hosts", "paths"].iter().filter_map(|key| list(key)).collect::<Vec<_>>().join(" ");
    if all.is_empty() { "(everything)".into() } else { all }
}

fn show(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 0)?;
    let workspace = workspace_id(ctx)?;
    let shown = ctx.request("policy.show", json!({ "workspaceId": workspace }))?;
    if let Some(error) = shown.get("error").and_then(Value::as_str) {
        let message = format!("policy file invalid: {error}");
        return Err(if ctx.json { CliError::Failed(crate::format::json(&shown)) } else { CliError::msg(message) });
    }
    Ok(ctx.print(&shown, || {
        let Some(file) = shown.get("file").and_then(Value::as_str) else { return "no policy file".to_string() };
        let apply: Vec<&str> =
            shown["applyTo"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let rows: Vec<Vec<String>> = shown["policies"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|p| {
                let text = |key: &str| p.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
                vec![text("name"), text("kind"), matches_of(p)]
            })
            .collect();
        let head = format!("{file} · applies to {}", apply.join(", "));
        if rows.is_empty() { format!("{head}\nno policies") } else { format!("{head}\n{}", aligned(&rows)) }
    }))
}

fn check(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["env"], 2)?;
    let method = need(ctx.args.positional(2), "the method: bifurc policy check <METHOD> <url>")?;
    let url = need(ctx.args.positional(3), "the url: bifurc policy check <METHOD> <url>")?;
    let workspace = workspace_id(ctx)?;
    let mut payload = json!({ "workspaceId": workspace, "method": method, "url": url });
    if let Some(env) = ctx.args.flag("env") {
        payload["env"] = json!(env);
    }
    let answer = ctx.request("policy.check", payload)?;
    let text = ctx.print(&answer, || match answer["decision"].as_str() {
        Some("allow") => "allow".to_string(),
        _ => answer["reason"].as_str().unwrap_or_default().to_string(),
    });
    match answer["decision"].as_str() {
        Some("deny") => Err(CliError::Reported(7, text)),
        Some("require_human") => Err(CliError::Reported(6, text)),
        _ => Ok(text),
    }
}

pub fn policy(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    match sub {
        Some("show") => show(ctx),
        Some("check") => check(ctx),
        Some(other) => Err(unknown_verb("policy", other)),
        None => Err(CliError::usage("missing the verb: show | check (see: bifurc policy --help)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run as with_engine};

    fn go(argv: &[&str], answers: Vec<Answer>) -> (Result<String>, Vec<(String, Value)>) {
        with_engine(argv, answers, |ctx| policy(ctx, argv.get(1).copied()))
    }

    fn file() -> Value {
        json!({"file": "policy.yaml", "applyTo": ["cli", "mcp"], "policies": [
            {"name": "no-prod-writes", "kind": "deny", "env": ["prod"], "methods": ["DELETE", "PUT"], "hosts": [], "paths": []},
            {"name": "local-only", "kind": "allow_hosts", "env": [], "methods": [], "hosts": ["*.localhost", "127.0.0.1"], "paths": []},
            {"name": "all", "kind": "require_human", "env": [], "methods": [], "hosts": [], "paths": []},
        ]})
    }

    #[test]
    fn show_prints_a_line_per_policy_and_says_when_there_is_no_file() {
        let (out, _) = go(&["policy", "show"], vec![("config.get", Ok(config())), ("policy.show", Ok(file()))]);
        assert_eq!(
            out.unwrap(),
            "policy.yaml · applies to cli, mcp\n\
             no-prod-writes  deny           env=prod methods=DELETE,PUT\n\
             local-only      allow_hosts    hosts=*.localhost,127.0.0.1\n\
             all             require_human  (everything)"
        );
        let none = json!({"file": null, "applyTo": ["cli", "mcp"], "policies": []});
        let (out, calls) = go(&["policy", "show"], vec![("config.get", Ok(config())), ("policy.show", Ok(none))]);
        assert_eq!(out.unwrap(), "no policy file");
        assert_eq!(payloads(&calls, "policy.show"), [json!({"workspaceId": "w1"})]);
    }

    #[test]
    fn show_of_an_invalid_file_is_the_error_and_exit_1() {
        let bad =
            json!({"file": "policy.yaml", "applyTo": ["cli", "mcp"], "policies": [], "error": "unknown key \"x\""});
        let (out, _) = go(&["policy", "show"], vec![("config.get", Ok(config())), ("policy.show", Ok(bad))]);
        let err = out.unwrap_err();
        assert_eq!((err.code(), err.to_string()), (1, "policy file invalid: unknown key \"x\"".to_string()));
    }

    #[test]
    fn check_exits_0_for_allow_7_for_deny_and_6_for_a_held_call() {
        let ask = |decision: &str, reason: &str| {
            let answer = json!({"decision": decision, "policy": null, "reason": reason});
            go(
                &["policy", "check", "delete", "https://api.prod.x/orders/1", "--env", "prod"],
                vec![("config.get", Ok(config())), ("policy.check", Ok(answer))],
            )
        };
        let (out, calls) = ask("allow", "no policy blocks it");
        assert_eq!(out.unwrap(), "allow");
        assert_eq!(
            payloads(&calls, "policy.check"),
            [json!({"workspaceId": "w1", "method": "delete", "url": "https://api.prod.x/orders/1", "env": "prod"})]
        );
        let line = "blocked by policy \"no-prod-writes\": DELETE https://api.prod.x/orders/1";
        let denied = ask("deny", line).0.unwrap_err();
        assert_eq!((denied.code(), denied.to_string()), (7, line.to_string()));
        assert_eq!(ask("require_human", "needs approval: policy \"p\"").0.unwrap_err().code(), 6);
        let (out, _) = go(&["policy", "check", "GET"], vec![]);
        assert_eq!(out.unwrap_err().code(), 2);
    }
}
