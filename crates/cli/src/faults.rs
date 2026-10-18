//! `faults add|list|clear`: fault-injection rules of the engine's proxies (latency, a status, dropped or reset
//! connections, at a rate), so retry, timeout and error UI can be checked against the real backend. Engine-wide
//! and in memory: a restart forgets them, and `bifurc status` lists them so they are not forgotten.

use serde_json::{Value, json};

use crate::{
    capture::tails,
    commands::{Context, need, unknown_verb},
    error::{CliError, Result},
    format::{aligned, js_template},
};

pub fn faults(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    match sub {
        Some("add") => add(ctx),
        Some("list") => list(ctx),
        Some("clear") => clear(ctx),
        other => Err(unknown_verb("faults", other.unwrap_or(""))),
    }
}

/// `30%` or `30`: a percent above 0 and at most 100.
fn percent(raw: &str, flag: &str) -> Result<f64> {
    raw.trim_end_matches('%').parse::<f64>().ok().filter(|n| *n > 0.0 && *n <= 100.0).ok_or_else(|| {
        CliError::usage(format!("--{flag} is a percent above 0 and at most 100 (30% or 30), not \"{raw}\""))
    })
}

/// Milliseconds: `2000`, or a duration (`2s`, `500ms`).
fn latency_ms(raw: &str) -> Result<u64> {
    if !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()) {
        return raw.parse().map_err(|_| CliError::usage(format!("--latency is too long: \"{raw}\"")));
    }
    Ok(crate::wait::duration(raw, "latency")?.as_millis() as u64)
}

fn add(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["latency", "status", "rate", "drop", "reset"], 1)?;
    let target = need(ctx.args.positional(2), "<target> (host[:port][/path])")?;
    let mut payload = json!({ "target": target });
    if let Some(raw) = ctx.args.flag("latency") {
        payload["latencyMs"] = json!(latency_ms(raw)?);
    }
    if let Some(raw) = ctx.args.flag("status") {
        let code =
            raw.parse::<u16>().map_err(|_| CliError::usage(format!("--status is a status code, not \"{raw}\"")))?;
        payload["status"] = json!(code);
    }
    let mut rated = false;
    for kind in ["drop", "reset"] {
        if let Some(raw) = ctx.args.flag(kind) {
            payload[kind] = json!(true);
            if raw != "true" {
                payload["rate"] = json!(percent(raw, kind)?);
                rated = true;
            }
        }
    }
    if let Some(raw) = ctx.args.flag("rate") {
        if rated {
            return Err(CliError::usage("give the rate once: --rate, or the rate after --drop or --reset"));
        }
        payload["rate"] = json!(percent(raw, "rate")?);
    }
    let added = ctx.request("fault.add", payload).map_err(|err| match err {
        CliError::Engine { code, message } if code == "BAD_REQUEST" => CliError::usage(message),
        other => other,
    })?;
    Ok(ctx.print(&added, || {
        let id = js_template(added.get("id"));
        let mut lines = vec![format!(
            "fault #{}: {} → {} (proxy traffic)",
            tails(&[id.as_str()])[0],
            js_template(added.get("target")),
            what(&added)
        )];
        lines.extend(added["warnings"].as_array().into_iter().flatten().map(|w| format!("! {}", js_template(Some(w)))));
        lines.join("\n")
    }))
}

/// `429 at 30%`, `+2000ms`, `drop at 10%`, `reset`.
pub fn what(fault: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(ms) = fault["latencyMs"].as_u64() {
        parts.push(format!("+{ms}ms"));
    }
    if let Some(status) = fault["status"].as_u64() {
        parts.push(status.to_string());
    }
    for kind in ["drop", "reset"] {
        if fault[kind].as_bool() == Some(true) {
            parts.push(kind.to_string());
        }
    }
    let rate = fault["rate"].as_f64().unwrap_or(100.0);
    if rate < 100.0 {
        parts.push(format!("at {rate}%"));
    }
    parts.join(" ")
}

/// The faults for `bifurc status`: `faults 2 (a.com/x 429@30%, b.com +2000ms)`, the first two, then `+N`.
pub fn brief(list: &Value) -> Option<String> {
    let all = list["faults"].as_array().filter(|f| !f.is_empty())?;
    let mut shown: Vec<String> = all
        .iter()
        .take(2)
        .map(|f| format!("{} {}", js_template(f.get("target")), what(f).replace(" at ", "@")))
        .collect();
    if all.len() > 2 {
        shown.push(format!("+{}", all.len() - 2));
    }
    Some(format!("faults {} ({})", all.len(), shown.join(", ")))
}

fn ids_of(got: &Value) -> Vec<&str> {
    got["faults"].as_array().into_iter().flatten().filter_map(|f| f["id"].as_str()).collect()
}

fn list(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 0)?;
    let got = ctx.request("fault.list", json!({}))?;
    Ok(ctx.print(&got, || {
        let all = got["faults"].as_array().cloned().unwrap_or_default();
        let tail = tails(&ids_of(&got));
        let rows: Vec<Vec<String>> = all
            .iter()
            .zip(&tail)
            .map(|(f, id)| {
                let hits = f["hits"].as_u64().unwrap_or(0);
                vec![
                    format!("#{id}"),
                    js_template(f.get("target")),
                    what(f),
                    format!("{hits} hit{}", if hits == 1 { "" } else { "s" }),
                ]
            })
            .collect();
        if rows.is_empty() { "no faults".to_string() } else { aligned(&rows) }
    }))
}

fn clear(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 1)?;
    let Some(wanted) = ctx.args.positional(2).map(str::to_string) else {
        let done = ctx.request("fault.clear", json!({}))?;
        return Ok(ctx.print(&done, || {
            let n = done["cleared"].as_u64().unwrap_or(0);
            format!("cleared {n} fault{}", if n == 1 { "" } else { "s" })
        }));
    };
    let got = ctx.request("fault.list", json!({}))?;
    let ids = ids_of(&got);
    let tail = wanted.strip_prefix('#').unwrap_or(&wanted);
    let hits: Vec<usize> = (0..ids.len()).filter(|&n| !tail.is_empty() && ids[n].ends_with(tail)).collect();
    let at = match hits.as_slice() {
        [one] => *one,
        [] => {
            let hint = if ids.is_empty() {
                "no faults".to_string()
            } else {
                format!("did you mean {}?", tails(&ids).iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(", "))
            };
            return Err(CliError::not_found(format!("no fault \"{wanted}\" — {hint} (see: bifurc faults list)")));
        }
        many => {
            let listed: Vec<&str> = many.iter().map(|&n| ids[n]).collect();
            return Err(CliError::usage(format!(
                "{} faults end with \"{tail}\": {} — use more of the id",
                many.len(),
                listed.join(", ")
            )));
        }
    };
    let done = ctx.request("fault.clear", json!({ "id": ids[at] }))?;
    Ok(ctx.print(&done, || format!("cleared #{}", tails(&ids)[at])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, payloads, run};

    fn go(argv: &[&str], answers: Vec<Answer>) -> (Result<String>, Vec<(String, Value)>) {
        let sub = argv.get(1).copied();
        run(argv, answers, |ctx| faults(ctx, sub))
    }

    fn added(extra: Value) -> Vec<Answer> {
        let mut rule = json!({"id": "m1abc1k3f", "target": "api.localhost/orders", "rate": 100.0, "hits": 0});
        for (k, v) in extra.as_object().unwrap() {
            rule[k] = v.clone();
        }
        vec![("fault.add", Ok(rule))]
    }

    fn sent(argv: &[&str], extra: Value) -> (String, Value) {
        let (out, calls) = go(argv, added(extra));
        (out.unwrap(), payloads(&calls, "fault.add")[0].clone())
    }

    #[test]
    fn add_maps_each_form_to_the_payload_and_prints_the_line() {
        let t = "api.localhost/orders";
        let (out, p) = sent(&["faults", "add", t, "--latency", "2000"], json!({"latencyMs": 2000}));
        assert_eq!(out, "fault #1k3f: api.localhost/orders → +2000ms (proxy traffic)");
        assert_eq!(p, json!({"target": t, "latencyMs": 2000}));
        let (_, p) = sent(&["faults", "add", t, "--latency", "2s"], json!({"latencyMs": 2000}));
        assert_eq!(p["latencyMs"], 2000);
        let (_, p) = sent(&["faults", "add", t, "--latency", "500ms"], json!({"latencyMs": 500}));
        assert_eq!(p["latencyMs"], 500);
        for rate in ["30%", "30"] {
            let (out, p) =
                sent(&["faults", "add", t, "--status", "429", "--rate", rate], json!({"status": 429, "rate": 30.0}));
            assert_eq!(out, "fault #1k3f: api.localhost/orders → 429 at 30% (proxy traffic)");
            assert_eq!(p, json!({"target": t, "status": 429, "rate": 30.0}));
        }
        let (out, p) = sent(&["faults", "add", t, "--drop", "10%"], json!({"drop": true, "rate": 10.0}));
        assert_eq!(out, "fault #1k3f: api.localhost/orders → drop at 10% (proxy traffic)");
        assert_eq!(p, json!({"target": t, "drop": true, "rate": 10.0}));
        let (out, p) = sent(&["faults", "add", t, "--drop"], json!({"drop": true}));
        assert_eq!(out, "fault #1k3f: api.localhost/orders → drop (proxy traffic)");
        assert_eq!(p, json!({"target": t, "drop": true}));
        let (out, p) = sent(&["faults", "add", t, "--reset", "5"], json!({"reset": true, "rate": 5.0}));
        assert_eq!(out, "fault #1k3f: api.localhost/orders → reset at 5% (proxy traffic)");
        assert_eq!(p, json!({"target": t, "reset": true, "rate": 5.0}));
    }

    #[test]
    fn add_prints_the_engine_warning_and_json_is_the_rule() {
        let warn = json!({"warnings": ["https:// traffic is only tunnelled"], "reset": true});
        let (out, _) = sent(&["faults", "add", "api.example.com", "--reset"], warn);
        assert!(out.ends_with("\n! https:// traffic is only tunnelled"), "{out}");
        let (out, _) = sent(&["faults", "add", "a.test", "--reset", "--json"], json!({"reset": true}));
        assert!(out.contains("\"id\": \"m1abc1k3f\""));
    }

    #[test]
    fn bad_values_and_conflicts_are_usage_errors() {
        let t = "a.test";
        for argv in [
            vec!["faults", "add", t, "--drop", "10%", "--rate", "20"],
            vec!["faults", "add", t, "--status", "429", "--rate", "0"],
            vec!["faults", "add", t, "--status", "429", "--rate", "101%"],
            vec!["faults", "add", t, "--status", "429", "--rate", "lots"],
            vec!["faults", "add", t, "--drop", "0"],
            vec!["faults", "add", t, "--latency", "soon"],
            vec!["faults", "add", t, "--status", "ok"],
            vec!["faults", "add", "--latency", "5"],
            vec!["faults", "add", t, "x", "--reset"],
            vec!["faults", "nope"],
        ] {
            assert_eq!(go(&argv, added(json!({}))).0.unwrap_err().code(), 2, "{argv:?}");
        }
        let bad =
            Err(CliError::Engine { code: "BAD_REQUEST".into(), message: "A fault needs one of latencyMs".into() });
        let err = go(&["faults", "add", t], vec![("fault.add", bad)]).0.unwrap_err();
        assert_eq!((err.code(), err.line()), (2, "bifurc: A fault needs one of latencyMs".to_string()));
    }

    fn two() -> Value {
        json!({"faults": [
            {"id": "aaa1k3f9", "target": "api.localhost/orders", "status": 429, "rate": 30.0, "hits": 12},
            {"id": "aaa2x1b2", "target": "b.test", "latencyMs": 2000, "rate": 100.0, "hits": 1}]})
    }

    #[test]
    fn list_prints_aligned_rows_or_none() {
        let (out, _) = go(&["faults", "list"], vec![("fault.list", Ok(two()))]);
        assert_eq!(
            out.unwrap(),
            "#k3f9  api.localhost/orders  429 at 30%  12 hits\n#x1b2  b.test                +2000ms     1 hit"
        );
        let (out, _) = go(&["faults", "list"], vec![("fault.list", Ok(json!({"faults": []})))]);
        assert_eq!(out.unwrap(), "no faults");
        let (out, _) = go(&["faults", "list", "--json"], vec![("fault.list", Ok(json!({"faults": []})))]);
        assert!(out.unwrap().contains("\"faults\": []"));
    }

    #[test]
    fn clear_all_one_and_unknown() {
        let (out, calls) = go(&["faults", "clear"], vec![("fault.clear", Ok(json!({"cleared": 2})))]);
        assert_eq!(out.unwrap(), "cleared 2 faults");
        assert_eq!(payloads(&calls, "fault.clear"), [json!({})]);
        let (out, _) = go(&["faults", "clear", "--json"], vec![("fault.clear", Ok(json!({"cleared": 1})))]);
        assert!(out.unwrap().contains("\"cleared\": 1"));
        let answers = vec![("fault.list", Ok(two())), ("fault.clear", Ok(json!({"cleared": 1})))];
        let (out, calls) = go(&["faults", "clear", "#x1b2"], answers.clone());
        assert_eq!(out.unwrap(), "cleared #x1b2");
        assert_eq!(payloads(&calls, "fault.clear"), [json!({"id": "aaa2x1b2"})]);
        let err = go(&["faults", "clear", "zzzz"], answers.clone()).0.unwrap_err();
        assert_eq!(err.code(), 3);
        assert!(err.line().contains("did you mean #k3f9, #x1b2?"), "{}", err.line());
    }

    #[test]
    fn brief_is_compact() {
        assert_eq!(brief(&json!({"faults": []})), None);
        let one = json!({"faults": [two()["faults"][0].clone()]});
        assert_eq!(brief(&one).unwrap(), "faults 1 (api.localhost/orders 429@30%)");
        let mut three = two();
        three["faults"].as_array_mut().unwrap().push(json!({"target": "c.test", "reset": true, "rate": 100.0}));
        assert_eq!(brief(&three).unwrap(), "faults 3 (api.localhost/orders 429@30%, b.test +2000ms, +1)");
    }
}
