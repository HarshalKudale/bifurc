//! The named session from the command line: `session`, `vars`, `cookies set`, and `--extract`, which keeps
//! a value of a response as a variable of the session for the `{{name}}` of later sends.

use serde_json::{Value, json};

use crate::{
    commands::{Context, id_of, list_entities, need, unknown_verb, workspace_id},
    error::{CliError, Result},
    format::{aligned, js_template},
    response::{is_sensitive_name, mask, revealed},
    select,
    session::session_name,
};

/// The payload of a session command: the workspace only when `--workspace` names one (else the active one).
fn scoped(ctx: &Context) -> Value {
    match ctx.args.flag("workspace") {
        Some(workspace) => json!({ "workspaceId": workspace }),
        None => json!({}),
    }
}

fn plural(count: usize, what: &str) -> String {
    format!("{count} {what}{}", if count == 1 { "" } else { "s" })
}

/// `session [show | reset]`.
pub fn session(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    let name = session_name(ctx.args)?;
    match sub.unwrap_or("show") {
        "show" => {
            ctx.accept(&[], 0)?;
            let got = ctx.request("session.get", scoped(ctx))?;
            // The environment goes by its name; an id that names none (deleted since) is shown as it is.
            let env = match got["environmentId"].as_str() {
                Some(id) => {
                    let workspace = workspace_id(ctx)?;
                    let named = list_entities(ctx, &workspace, "environments")?
                        .iter()
                        .find(|e| id_of(e) == id)
                        .map(|e| js_template(e.get("name")));
                    Some(named.unwrap_or_else(|| id.to_string()))
                }
                None => None,
            };
            let text = || match &name {
                None => "session default (shared with the app)".to_string(),
                Some(name) => {
                    let variables = got["variables"].as_array().map_or(0, Vec::len);
                    let mut parts = vec![format!("session {name}"), plural(variables, "variable")];
                    parts.extend(env.map(|env| format!("env {env}")));
                    let domains = got["cookieDomains"].as_u64().unwrap_or(0) as usize;
                    parts.push(format!("cookies {}", plural(domains, "domain")));
                    parts.join(" · ")
                }
            };
            Ok(ctx.print(&got, text))
        }
        "reset" => {
            ctx.accept(&[], 0)?;
            ctx.request("session.reset", scoped(ctx))?;
            let done = match &name {
                Some(name) => format!("session {name} reset: cookies, variables and environment cleared"),
                None => "session default: runtime variables cleared (the app's jar and environment are untouched)"
                    .to_string(),
            };
            Ok(ctx.print(&json!({ "ok": true }), || done))
        }
        other => Err(unknown_verb("session", other)),
    }
}

/// `vars list | set <KEY=VALUE>... | clear [<KEY>...]`.
pub fn vars(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    let args = ctx.args;
    match sub.unwrap_or("list") {
        "list" => {
            ctx.accept(&["reveal"], 0)?;
            let mut got = ctx.request("session.get", scoped(ctx))?;
            let reveal = revealed(args);
            let mut rows = Vec::new();
            for variable in got["variables"].as_array_mut().into_iter().flatten() {
                let (key, value) = (js_template(variable.get("key")), js_template(variable.get("value")));
                let shown = if reveal || !is_sensitive_name(&key) { value } else { mask(&value) };
                variable["value"] = json!(shown);
                rows.push(vec![key, shown]);
            }
            let variables = got["variables"].clone();
            Ok(ctx.print(&variables, || if rows.is_empty() { "(none)".to_string() } else { aligned(&rows) }))
        }
        "set" => {
            ctx.accept(&[], 1000)?;
            let given = args.positionals.get(2..).unwrap_or_default();
            if given.is_empty() {
                return Err(CliError::usage("missing KEY=VALUE"));
            }
            let mut variables = serde_json::Map::new();
            for raw in given {
                match raw.split_once('=') {
                    Some((key, value)) if !key.is_empty() => variables.insert(key.to_string(), json!(value)),
                    _ => return Err(CliError::usage(format!("a variable is set as KEY=VALUE, not \"{raw}\""))),
                };
            }
            let names = variables.keys().cloned().collect::<Vec<_>>().join(", ");
            let mut payload = scoped(ctx);
            payload["variables"] = Value::Object(variables);
            let done = ctx.request("session.setVariables", payload)?;
            Ok(ctx.print(&done, || format!("set {names}")))
        }
        "clear" => {
            ctx.accept(&[], 1000)?;
            let keys: Vec<&String> = args.positionals.iter().skip(2).collect();
            let mut payload = scoped(ctx);
            if !keys.is_empty() {
                payload["keys"] = json!(keys);
            }
            let done = ctx.request("session.clearVariables", payload)?;
            let count = done["cleared"].as_u64().unwrap_or(0) as usize;
            Ok(ctx.print(&done, || format!("cleared {}", plural(count, "variable"))))
        }
        other => Err(unknown_verb("vars", other)),
    }
}

/// `cookies set <domain|url> <name>=<value>`: seed a cookie in the jar this session uses.
pub fn cookies_set(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 2)?;
    let args = ctx.args;
    let target = need(args.positional(2), "domain or URL")?;
    let pair = need(args.positional(3), "NAME=VALUE")?;
    let Some((name, value)) = pair.split_once('=').filter(|(name, _)| !name.is_empty()) else {
        return Err(CliError::usage(format!("a cookie is set as NAME=VALUE, not \"{pair}\"")));
    };
    let mut payload = json!({ "workspaceId": workspace_id(ctx)?, "name": name, "value": value });
    payload[if target.contains("://") { "url" } else { "domain" }] = json!(target);
    let done = ctx.request("cookies.set", payload)?;
    Ok(ctx.print(&done, || format!("set cookie {name} for {target}")))
}

/// The pairs an extraction found, and why the first path that found nothing did not.
pub type Extracted = (Vec<(String, String)>, Option<String>);

/// What `--extract name=<path>` options (`specs`) take from a response: the pairs found, and why the first
/// one that found nothing did not. A path is evaluated against the JSON body, or is `status`.
pub fn evaluate(specs: &[String], status: u16, body: Option<&Value>) -> Result<Extracted> {
    let mut found = Vec::new();
    let mut failure = None;
    for spec in specs {
        let (name, path) = select::parse_pair(spec, "--extract").map_err(CliError::usage)?;
        let value = if path == "status" {
            Ok(vec![json!(status)])
        } else {
            match body {
                Some(body) => select::select(body, &path),
                None => Err("the body is not JSON".to_string()),
            }
        };
        match value.map(|values| values.into_iter().find(|v| !v.is_null())) {
            Ok(Some(Value::String(text))) => found.push((name, text)),
            Ok(Some(other)) => found.push((name, other.to_string())),
            Ok(None) => {
                failure.get_or_insert(format!("extract {name}: {path} matched nothing"));
            }
            Err(why) => {
                failure.get_or_insert(format!("extract {name}: {why}"));
            }
        }
    }
    Ok((found, failure))
}

/// Keep `pairs` as variables of the current session.
pub fn store(ctx: &mut Context, pairs: &[(String, String)]) -> Result<()> {
    let mut payload = scoped(ctx);
    payload["variables"] = Value::Object(pairs.iter().map(|(k, v)| (k.clone(), json!(v))).collect());
    ctx.request("session.setVariables", payload).map(|_| ())
}

/// `extracted a=1 b=••••(3)`: what was kept, secret-looking names masked.
pub fn extracted_line(pairs: &[(String, String)]) -> String {
    let shown: Vec<String> = pairs
        .iter()
        .map(|(name, value)| format!("{name}={}", if is_sensitive_name(name) { mask(value) } else { value.clone() }))
        .collect();
    format!("extracted {}", shown.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{config, payloads, run};

    fn go(argv: &[&str], answers: Vec<(&'static str, Result<Value>)>) -> (Result<String>, Vec<(String, Value)>) {
        let sub = argv.get(1).copied();
        run(argv, answers, |c| match argv[0] {
            "session" => session(c, sub),
            "vars" => vars(c, sub),
            _ => cookies_set(c),
        })
    }

    fn got() -> Value {
        json!({"name": "frontend", "variables": [{"key": "cartId", "value": "c_91"}, {"key": "apiToken", "value": "abcd"}],
               "environmentId": "e1", "cookieDomains": 1})
    }

    #[test]
    fn show_says_what_the_session_holds_and_the_default_session_is_the_apps() {
        let envs = Ok(json!({"entities": [{"id": "e1", "name": "dev"}]}));
        let answers = || vec![("session.get", Ok(got())), ("config.get", Ok(config())), ("entity.list", envs.clone())];
        let (out, _) = go(&["session", "show", "--session", "frontend"], answers());
        assert_eq!(out.unwrap(), "session frontend · 2 variables · env dev · cookies 1 domain");
        let (out, _) = go(&["session", "show"], vec![("session.get", Ok(json!({"name": "default", "variables": []})))]);
        assert_eq!(out.unwrap(), "session default (shared with the app)");
    }

    #[test]
    fn reset_vars_and_cookies_set_send_their_commands() {
        let (out, calls) =
            go(&["session", "reset", "--session", "a"], vec![("session.reset", Ok(json!({"ok": true})))]);
        assert!(out.unwrap().starts_with("session a reset"));
        assert_eq!(payloads(&calls, "session.reset"), [json!({})]);

        let (out, calls) = go(&["vars", "list"], vec![("session.get", Ok(got()))]);
        assert_eq!(
            out.unwrap(),
            "cartId    c_91
apiToken  ••••(4)"
        );
        assert!(calls.len() == 1);

        let (out, calls) =
            go(&["vars", "set", "a=1", "b=x=y", "-w", "w2"], vec![("session.setVariables", Ok(json!({"ok": true})))]);
        assert_eq!(out.unwrap(), "set a, b");
        assert_eq!(
            payloads(&calls, "session.setVariables"),
            [json!({"workspaceId": "w2", "variables": {"a": "1", "b": "x=y"}})]
        );
        assert_eq!(go(&["vars", "set", "nope"], vec![]).0.unwrap_err().code(), 2);

        let (out, calls) = go(&["vars", "clear", "a"], vec![("session.clearVariables", Ok(json!({"cleared": 1})))]);
        assert_eq!(out.unwrap(), "cleared 1 variable");
        assert_eq!(payloads(&calls, "session.clearVariables"), [json!({"keys": ["a"]})]);
        let (_, calls) = go(&["vars", "clear"], vec![("session.clearVariables", Ok(json!({"cleared": 0})))]);
        assert_eq!(payloads(&calls, "session.clearVariables"), [json!({})]);

        let answers = || vec![("config.get", Ok(config())), ("cookies.set", Ok(json!({"ok": true})))];
        let (out, calls) = go(&["cookies", "set", "api.localhost", "flag=on"], answers());
        assert_eq!(out.unwrap(), "set cookie flag for api.localhost");
        assert_eq!(
            payloads(&calls, "cookies.set"),
            [json!({"workspaceId": "w1", "name": "flag", "value": "on", "domain": "api.localhost"})]
        );
        let (_, calls) = go(&["cookies", "set", "http://a.test/x", "f=1"], answers());
        assert_eq!(payloads(&calls, "cookies.set")[0]["url"], "http://a.test/x");
    }
}
