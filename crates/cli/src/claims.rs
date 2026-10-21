//! `bifurc claim <kind> <name>` and `bifurc claims`: mark an item as the one this session is editing. A claim is a
//! lock (the engine keeps it in memory for 30 minutes): another session's update, delete or enable is refused
//! (exit 2) until the claim is released or lapses; `claim --force` takes it over.

use serde_json::json;

use crate::{
    capture::now_ms,
    changes::ago,
    commands::{Context, find_one, id_of, list_entities, need, workspace_id},
    error::{CliError, Result},
    format::{aligned, js_template},
};

/// The engine kind, the singular the rows are found under and the group that lists them, for a CLI kind word.
fn kind_of(word: &str) -> Result<(&'static str, &'static str, &'static str)> {
    Ok(match word.trim_end_matches('s') {
        "request" => ("requests", "request", "requests"),
        "mock" => ("mocks", "mock", "mocks"),
        "flow" => ("flows", "flow", "flows"),
        "env" | "environment" => ("environments", "environment", "env"),
        _ => return Err(CliError::usage(format!("unknown kind \"{word}\": request, mock, flow or env"))),
    })
}

/// The kind word an engine kind goes by on the command line.
fn word_of(engine_kind: &str) -> &str {
    match engine_kind {
        "environments" => "env",
        other => other.strip_suffix('s').unwrap_or(other),
    }
}

/// `claim [--release] <kind> <name>`.
pub fn claim(ctx: &mut Context) -> Result<String> {
    ctx.accept_top(&["release", "force"], 2)?;
    let args = ctx.args;
    let (kind, what, group) = kind_of(&need(args.positional(1), "a kind: request, mock, flow or env")?)?;
    let wanted = need(args.positional(2), "a name")?;
    let workspace = workspace_id(ctx)?;
    let rows = list_entities(ctx, &workspace, kind)?;
    let row = find_one(&rows, &wanted, &["name"], what, group)?;
    let (id, name) = (id_of(row), js_template(row.get("name")));
    let release = args.has("release");
    let got = ctx.request(
        "claims.set",
        json!({ "workspaceId": workspace, "kind": kind, "id": id, "release": release, "force": args.has("force") }),
    )?;
    let out = json!({ "schemaVersion": 1, "claim": got["claim"], "previous": got["previous"] });
    Ok(ctx.print(&out, || {
        let word = word_of(kind);
        if release {
            return format!("released {word} \"{name}\"");
        }
        match got["previous"].as_object() {
            Some(prev) => format!(
                "claimed {word} \"{name}\" — was held by {} {}",
                js_template(prev.get("by")),
                ago(now_ms() - prev["ts"].as_i64().unwrap_or_default())
            ),
            None => format!("claimed {word} \"{name}\""),
        }
    }))
}

/// `claims`: every live claim of the workspace, oldest first.
pub fn claims(ctx: &mut Context) -> Result<String> {
    ctx.accept_top(&[], 0)?;
    let workspace = workspace_id(ctx)?;
    let got = ctx.request("claims.list", json!({ "workspaceId": workspace }))?;
    let list = got["claims"].as_array().cloned().unwrap_or_default();
    let out = json!({ "schemaVersion": 1, "claims": list });
    Ok(ctx.print(&out, || {
        if list.is_empty() {
            return "no claims".to_string();
        }
        let now = now_ms();
        let rows: Vec<Vec<String>> = list
            .iter()
            .map(|c| {
                vec![
                    word_of(&js_template(c.get("kind"))).to_string(),
                    format!("\"{}\"", js_template(c.get("name"))),
                    format!("by {}", js_template(c.get("by"))),
                    ago(now - c["ts"].as_i64().unwrap_or(now)),
                ]
            })
            .collect();
        aligned(&rows)
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::testing::{config, payloads, run};

    fn mocks() -> Value {
        json!({"entities": [{"id": "m1", "name": "Get orders"}, {"id": "m2", "name": "Get user"}]})
    }

    fn go(argv: &[&str], reply: Value) -> (Result<String>, Vec<(String, Value)>) {
        let answers = vec![("config.get", Ok(config())), ("entity.list", Ok(mocks())), ("claims.set", Ok(reply))];
        run(argv, answers, claim)
    }

    #[test]
    fn claim_names_the_previous_holder_and_release_says_released() {
        let (out, calls) = go(&["claim", "mock", "get orders"], json!({"claim": {"id": "m1"}, "previous": null}));
        assert_eq!(out.unwrap(), "claimed mock \"Get orders\"");
        assert_eq!(
            payloads(&calls, "claims.set")[0],
            json!({"workspaceId": "w1", "kind": "mocks", "id": "m1", "release": false, "force": false})
        );
        let held = json!({"claim": {}, "previous": {"by": "session:frontend", "ts": now_ms() - 180_000}});
        let (out, _) = go(&["claim", "mock", "Get orders"], held);
        assert_eq!(out.unwrap(), "claimed mock \"Get orders\" — was held by session:frontend 3m ago");
        let (out, calls) = go(&["claim", "--release", "mock", "Get orders"], json!({"claim": null, "previous": null}));
        assert_eq!(out.unwrap(), "released mock \"Get orders\"");
        assert_eq!(payloads(&calls, "claims.set")[0]["release"], true);
        assert_eq!(go(&["claim", "widget", "x"], json!({})).0.unwrap_err().code(), 2);
    }

    #[test]
    fn a_claim_over_a_live_one_is_refused_with_exit_2_unless_forced() {
        let refused = CliError::Engine {
            code: "CONFLICT".into(),
            message: "\"Get orders\" is claimed by session:a and locked against edits. Take it over with --force"
                .into(),
        };
        let answers = vec![("config.get", Ok(config())), ("entity.list", Ok(mocks())), ("claims.set", Err(refused))];
        let err = run(&["claim", "mock", "Get orders"], answers, claim).0.unwrap_err();
        assert_eq!(err.code(), 2);
        let (out, calls) = go(
            &["claim", "--force", "mock", "Get orders"],
            json!({"claim": {}, "previous": {"by": "session:a", "ts": now_ms()}}),
        );
        assert!(out.unwrap().starts_with("claimed mock"));
        assert_eq!(payloads(&calls, "claims.set")[0]["force"], true);
    }

    #[test]
    fn claims_prints_a_line_each_or_none() {
        let list = |rows: Value| {
            let answers = vec![("config.get", Ok(config())), ("claims.list", Ok(json!({ "claims": rows })))];
            run(&["claims"], answers, claims)
        };
        assert_eq!(list(json!([])).0.unwrap(), "no claims");
        let one = json!([{"id": "m1", "kind": "mocks", "name": "Get orders", "by": "session:backend", "ts": now_ms() - 180_000}]);
        assert_eq!(list(one).0.unwrap(), "mock  \"Get orders\"  by session:backend  3m ago");
    }
}
