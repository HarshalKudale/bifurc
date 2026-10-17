//! `bifurc changes`: what changed in the workspace since this session last asked (the engine keeps the feed
//! in memory and a cursor per session), one line each.

use serde_json::{Value, json};

use crate::{
    capture::now_ms,
    commands::{Context, workspace_id},
    error::{CliError, Result},
    format::{aligned, js_template},
};

/// The engine's feed asked for by `--since`: a sequence number (`41`), a time ago (`30s`, `2m`, `1h`, `2d`), or a
/// UTC time (`2026-10-17`, `2026-10-17T09:30[:00][Z]`).
fn since(raw: &str, now: i64) -> Result<Value> {
    if let Ok(seq) = raw.parse::<u64>() {
        return Ok(json!({ "after": seq }));
    }
    let (count, unit) = raw.split_at(raw.len().saturating_sub(1));
    let unit_ms = match unit {
        "s" => Some(1000),
        "m" => Some(60_000),
        "h" => Some(3_600_000),
        "d" => Some(86_400_000),
        _ => None,
    };
    if let (Ok(n), Some(unit_ms)) = (count.parse::<i64>(), unit_ms) {
        return Ok(json!({ "sinceTs": now - n * unit_ms }));
    }
    utc_ms(raw)
        .map(|ts| json!({ "sinceTs": ts }))
        .ok_or_else(|| CliError::usage(format!("--since is a number, 30s, 2m, 1h, 2d or a UTC time, not \"{raw}\"")))
}

/// Unix milliseconds of `YYYY-MM-DD[THH:MM[:SS]][Z]`, in UTC (Howard Hinnant's days-from-civil).
fn utc_ms(text: &str) -> Option<i64> {
    let text = text.trim_end_matches('Z');
    let (date, time) = text.split_once(['T', ' ']).unwrap_or((text, "00:00"));
    let number = |part: &str| part.parse::<i64>().ok();
    let mut d = date.split('-');
    let (year, month, day) = (number(d.next()?)?, number(d.next()?)?, number(d.next()?)?);
    let mut t = time.split(':');
    let (hour, minute) = (number(t.next()?)?, number(t.next()?)?);
    let second = t.next().map_or(Some(0), number)?;
    if d.next().is_some()
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some(((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000)
}

/// `just now`, `40s ago`, `2m ago`, `3h ago`, `2d ago`.
fn ago(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    match s {
        0..=4 => "just now".into(),
        5..=59 => format!("{s}s ago"),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86_399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86_400),
    }
}

fn line(change: &Value, now: i64) -> Vec<String> {
    let fields: Vec<&str> = change["fields"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let mut what = js_template(change.get("action"));
    if !fields.is_empty() {
        let more = fields.len().saturating_sub(4);
        what = format!(
            "{what} {}{}",
            fields[..fields.len() - more].join(", "),
            if more > 0 { format!(" +{more}") } else { String::new() }
        );
    }
    vec![
        js_template(change.get("kind")),
        format!("\"{}\"", js_template(change.get("name"))),
        what,
        format!("by {}", js_template(change.get("by"))),
        ago(now - change["ts"].as_i64().unwrap_or(now)),
    ]
}

/// `changes [--since <seq|2m|1h|time>] [--mine]`.
pub fn changes(ctx: &mut Context) -> Result<String> {
    ctx.accept_top(&["since", "mine"], 0)?;
    let mut payload = json!({ "workspaceId": workspace_id(ctx)? });
    match ctx.args.flag("since") {
        Some(raw) => payload.as_object_mut().unwrap().extend(since(raw, now_ms())?.as_object().unwrap().clone()),
        None => payload["advance"] = json!(true),
    }
    if ctx.args.has("mine") {
        payload["mine"] = json!(true);
    }
    let got = ctx.request("changes.list", payload)?;
    let now = now_ms();
    let list = got["changes"].as_array().cloned().unwrap_or_default();
    let out = json!({ "schemaVersion": 1, "changes": list, "cursor": got["cursor"] });
    Ok(ctx.print(&out, || {
        let mut lines = Vec::new();
        if let Some(dropped) = got["dropped"].as_u64().filter(|n| *n > 0) {
            lines.push(format!(
                "… {dropped} earlier change{} {} dropped",
                if dropped == 1 { "" } else { "s" },
                if dropped == 1 { "was" } else { "were" }
            ));
        }
        if list.is_empty() {
            lines.push("no changes".to_string());
        } else {
            lines.push(aligned(&list.iter().map(|c| line(c, now)).collect::<Vec<_>>()));
        }
        lines.join("\n")
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{config, payloads, run};

    fn go(argv: &[&str], feed: Value) -> (Result<String>, Vec<(String, Value)>) {
        run(argv, vec![("config.get", Ok(config())), ("changes.list", Ok(feed))], changes)
    }

    fn change(kind: &str, name: &str, action: &str, fields: &[&str], by: &str, age: i64) -> Value {
        json!({"seq": 1, "ts": now_ms() - age, "workspaceId": "w1", "kind": kind, "id": "x", "name": name,
               "action": action, "fields": fields, "by": by})
    }

    #[test]
    fn since_reads_a_number_a_time_ago_or_a_utc_time() {
        assert_eq!(since("41", 0).unwrap(), json!({"after": 41}));
        assert_eq!(since("2m", 1_000_000).unwrap(), json!({"sinceTs": 880_000}));
        assert_eq!(since("2026-10-17T09:30Z", 0).unwrap(), json!({"sinceTs": 1_792_229_400_000i64}));
        assert_eq!(since("1970-01-02", 0).unwrap(), json!({"sinceTs": 86_400_000}));
        assert_eq!(since("soon", 0).unwrap_err().code(), 2);
    }

    #[test]
    fn ages_are_short() {
        assert_eq!(
            [ago(1000), ago(40_000), ago(120_000), ago(7_300_000), ago(200_000_000)],
            ["just now", "40s ago", "2m ago", "2h ago", "2d ago"]
        );
    }

    #[test]
    fn changes_print_a_line_each_and_the_default_advances_the_cursor() {
        let feed = json!({"changes": [
            change("mock", "Get orders", "updated", &["responseBody", "responseStatus"], "session:backend", 120_000),
            change("request", "Create user", "created", &[], "app", 300_000),
        ], "cursor": 2, "oldest": 1, "dropped": 0});
        let (out, calls) = go(&["changes"], feed.clone());
        let out = out.unwrap();
        assert!(
            out.starts_with(
                "mock     \"Get orders\"   updated responseBody, responseStatus  by session:backend  2m ago"
            ),
            "{out}"
        );
        assert!(out.contains("request  \"Create user\"  created"), "{out}");
        assert_eq!(payloads(&calls, "changes.list")[0], json!({"workspaceId": "w1", "advance": true}));
        let (out, calls) = go(&["changes", "--since", "5", "--mine", "--json"], feed);
        assert_eq!(payloads(&calls, "changes.list")[0], json!({"workspaceId": "w1", "after": 5, "mine": true}));
        let out = out.unwrap();
        let shown: Value = serde_json::from_str(&out).unwrap_or_else(|_| panic!("{out}"));
        assert_eq!(
            (shown["schemaVersion"].clone(), shown["cursor"].clone(), shown["changes"].as_array().unwrap().len()),
            (json!(1), json!(2), 2)
        );
    }

    #[test]
    fn nothing_new_is_one_line_and_dropped_changes_are_said() {
        let (out, _) = go(&["changes"], json!({"changes": [], "cursor": 0, "oldest": 1, "dropped": 0}));
        assert_eq!(out.unwrap(), "no changes");
        let (out, _) = go(&["changes"], json!({"changes": [], "cursor": 9, "oldest": 7, "dropped": 6}));
        assert_eq!(out.unwrap(), "… 6 earlier changes were dropped\nno changes");
    }
}
