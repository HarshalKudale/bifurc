//! `mail list|show|last|clear`: the mail the engine's local SMTP sink caught (see `engine/src/mail`), and the
//! code or link in it. The engine finds the code and the link; this prints them and picks the exit code.

use serde_json::{Value, json};

use crate::{
    capture::{clock, number, tails},
    commands::{Context, need, unknown_verb},
    error::{CliError, Result},
    format::{aligned, js_template},
    wait::duration,
};

pub fn mail(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    match sub {
        Some("list") => list(ctx),
        Some("show") => show(ctx),
        Some("last") => last(ctx),
        Some("clear") => clear(ctx),
        other => Err(unknown_verb("mail", other.unwrap_or(""))),
    }
}

/// The addresses of a mail: the first, and how many more.
fn to_of(mail: &Value) -> String {
    let all: Vec<String> = mail["to"].as_array().into_iter().flatten().map(|a| js_template(Some(a))).collect();
    match all.split_first() {
        None => "-".to_string(),
        Some((first, [])) => first.clone(),
        Some((first, more)) => format!("{first} +{}", more.len()),
    }
}

fn list(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["to", "last"], 0)?;
    let mut payload = json!({ "last": number(ctx, "last")?.unwrap_or(20) });
    if let Some(to) = ctx.args.flag("to") {
        payload["to"] = json!(to);
    }
    let answer = ctx.request("mail.list", payload)?;
    Ok(ctx.print(&answer, || lines(&answer)))
}

/// `#k3f9  14:02:11Z  dana@x.test  Your code is 482913`, newest first.
fn lines(answer: &Value) -> String {
    let mails = answer["mails"].as_array().map(Vec::as_slice).unwrap_or_default();
    if mails.is_empty() {
        return "(none)".to_string();
    }
    let ids: Vec<String> = mails.iter().map(|m| js_template(m.get("id"))).collect();
    let tails = tails(&ids.iter().map(String::as_str).collect::<Vec<_>>());
    let rows: Vec<Vec<String>> = mails
        .iter()
        .zip(tails)
        .map(|(m, tail)| {
            vec![
                format!("#{tail}"),
                clock(m["timestamp"].as_i64().unwrap_or(0)),
                to_of(m),
                js_template(m.get("subject")),
            ]
        })
        .collect();
    let mut text = aligned(&rows);
    let cut = (answer["matched"].as_u64().unwrap_or(0) as usize).saturating_sub(mails.len());
    if cut > 0 {
        text.push_str(&format!("\n… {cut} more — use --last or --to"));
    }
    text
}

/// The id `wanted` names (a full id, `#tail` or `tail`) among the mails in the sink.
fn resolve(ctx: &mut Context, wanted: &str) -> Result<String> {
    let all = ctx.request("mail.list", json!({ "last": 1000 }))?;
    let ids: Vec<String> = all["mails"].as_array().into_iter().flatten().map(|m| js_template(m.get("id"))).collect();
    let tail = wanted.strip_prefix('#').unwrap_or(wanted);
    if let Some(exact) = ids.iter().find(|id| *id == tail) {
        return Ok(exact.clone());
    }
    let hits: Vec<&String> = ids.iter().filter(|id| id.ends_with(tail)).collect();
    match hits.as_slice() {
        [] => Err(CliError::not_found(format!("no mail \"{wanted}\" (see: bifurc mail list)"))),
        [one] => Ok((*one).clone()),
        many => {
            let listed: Vec<&str> = many.iter().map(|id| id.as_str()).collect();
            Err(CliError::usage(format!(
                "{} mails end with \"{tail}\": {} — use more of the id",
                many.len(),
                listed.join(", ")
            )))
        }
    }
}

/// `From:`, `To:` and `Subject:` lines, an empty line and the text of the mail.
fn render(mail: &Value) -> String {
    let all: Vec<String> = mail["to"].as_array().into_iter().flatten().map(|a| js_template(Some(a))).collect();
    let text = mail["text"].as_str().filter(|t| !t.is_empty()).unwrap_or("(no text part — use --html)");
    format!(
        "From: {}\nTo: {}\nSubject: {}\n\n{text}",
        js_template(mail.get("from")),
        all.join(", "),
        js_template(mail.get("subject"))
    )
}

fn show(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["html", "text"], 1)?;
    if ctx.args.has("html") && ctx.args.has("text") {
        return Err(CliError::usage("give --html or --text, not both"));
    }
    let id = resolve(ctx, &need(ctx.args.positional(2), "mail id")?)?;
    let mail = ctx.request("mail.get", json!({ "id": id }))?;
    Ok(ctx.print(&mail, || {
        let body = |key: &str, none: &str| mail[key].as_str().filter(|b| !b.is_empty()).unwrap_or(none).to_string();
        if ctx.args.has("html") {
            body("html", "(no html part — use --text)")
        } else if ctx.args.has("text") {
            body("text", "(no text part — use --html)")
        } else {
            render(&mail)
        }
    }))
}

fn last(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["to", "otp", "link", "link-match", "wait"], 0)?;
    let (otp, link) = (ctx.args.has("otp"), ctx.args.has("link"));
    if otp && link {
        return Err(CliError::usage("give --otp or --link, not both"));
    }
    if ctx.args.flag("link-match").is_some() && !link {
        return Err(CliError::usage("--link-match goes with --link"));
    }
    let mut asked = json!({});
    for (flag, key) in [("to", "to"), ("link-match", "linkMatch")] {
        if let Some(value) = ctx.args.flag(flag) {
            asked[key] = json!(value);
        }
    }
    let who = ctx.args.flag("to").map_or(String::new(), |to| format!(" for {to}"));
    let found = match ctx.args.flag("wait") {
        Some(raw) => {
            let wait = duration(raw, "wait")?;
            asked["timeoutMs"] = json!(wait.as_millis() as u64);
            let answer = ctx.request("mail.wait", asked)?;
            let mail = answer["mail"].clone();
            if mail.is_null() {
                Err(CliError::not_found(format!("no mail{who} arrived within {raw}")))
            } else {
                Ok(mail)
            }
        }
        None => {
            let mut listing = json!({ "last": 1 });
            if let Some(to) = asked.get("to") {
                listing["to"] = to.clone();
            }
            let newest = ctx.request("mail.list", listing)?;
            let Some(id) = newest["mails"][0]["id"].as_str() else {
                return Err(CliError::not_found(format!(
                    "no mail{who} yet (see: bifurc mail list, or use --wait 30s)"
                )));
            };
            let mut get = json!({ "id": id });
            if let Some(part) = asked.get("linkMatch") {
                get["linkMatch"] = part.clone();
            }
            ctx.request("mail.get", get)
        }
    }?;
    let (key, what) = if otp { ("otp", "code") } else { ("link", "link") };
    if !otp && !link {
        return Ok(ctx.print(&found, || render(&found)));
    }
    match found[key].as_str() {
        Some(value) => Ok(ctx.print(&found, || value.to_string())),
        None => Err(CliError::msg(format!(
            "no {what} found in the latest mail{who}: \"{}\"",
            js_template(found.get("subject"))
        ))),
    }
}

fn clear(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 0)?;
    let done = ctx.request("mail.clear", json!({}))?;
    Ok(ctx.print(&done, || format!("cleared {} mail{}", done["cleared"], if done["cleared"] == 1 { "" } else { "s" })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, payloads, run};

    fn summary(id: &str, to: &[&str], subject: &str) -> Value {
        json!({"id": id, "timestamp": 1_700_000_000_000i64, "from": "app@shop.test", "to": to, "subject": subject})
    }

    fn full(otp: Value, link: Value) -> Value {
        let mut mail = summary("aaa1k3f9", &["dana@x.test"], "Your code is 482913");
        mail["text"] = json!("Use 482913");
        mail["html"] = json!("<b>Use 482913</b>");
        (mail["otp"], mail["link"]) = (otp, link);
        mail
    }

    fn engine(mail: Value) -> Vec<Answer> {
        let list = json!({"mails": [summary("aaa1k3f9", &["dana@x.test"], "Your code is 482913")], "matched": 1});
        vec![("mail.list", Ok(list)), ("mail.get", Ok(mail.clone())), ("mail.wait", Ok(json!({"mail": mail})))]
    }

    fn go(argv: &[&str], answers: Vec<Answer>) -> (Result<String>, Vec<(String, Value)>) {
        run(argv, answers, |ctx| mail(ctx, ctx.args.positional(1)))
    }

    #[test]
    fn list_is_one_aligned_line_per_mail_with_the_shortest_unique_tail() {
        let both = json!({"mails": [
            summary("aaa2k3f9", &["dana@x.test", "b@x.test"], "Reset your password"),
            summary("aaa1k3f9", &["dana@x.test"], "Your code is 482913")], "matched": 7});
        let (out, calls) = go(&["mail", "list", "--to", "dana", "--last", "2"], vec![("mail.list", Ok(both))]);
        assert_eq!(
            out.unwrap(),
            "#2k3f9  22:13:20Z  dana@x.test +1  Reset your password\n#1k3f9  22:13:20Z  dana@x.test     Your code is 482913\n… 5 more — use --last or --to"
        );
        assert_eq!(payloads(&calls, "mail.list"), [json!({"last": 2, "to": "dana"})]);
        assert_eq!(
            go(&["mail", "list"], vec![("mail.list", Ok(json!({"mails": [], "matched": 0})))]).0.unwrap(),
            "(none)"
        );
    }

    #[test]
    fn show_prints_the_text_by_default_the_html_on_request_and_resolves_a_tail() {
        let (out, calls) = go(&["mail", "show", "#k3f9"], engine(full(json!(null), json!(null))));
        assert_eq!(out.unwrap(), "From: app@shop.test\nTo: dana@x.test\nSubject: Your code is 482913\n\nUse 482913");
        assert_eq!(payloads(&calls, "mail.get"), [json!({"id": "aaa1k3f9"})]);
        assert_eq!(
            go(&["mail", "show", "k3f9", "--html"], engine(full(json!(null), json!(null)))).0.unwrap(),
            "<b>Use 482913</b>"
        );
        assert_eq!(
            go(&["mail", "show", "k3f9", "--text"], engine(full(json!(null), json!(null)))).0.unwrap(),
            "Use 482913"
        );
        assert_eq!(go(&["mail", "show", "zzzz"], engine(full(json!(null), json!(null)))).0.unwrap_err().code(), 3);
        assert_eq!(go(&["mail", "show", "k3f9", "--html", "--text"], engine(json!({}))).0.unwrap_err().code(), 2);
        assert_eq!(go(&["mail", "show"], engine(json!({}))).0.unwrap_err().code(), 2);
    }

    #[test]
    fn last_otp_and_link_print_only_the_value() {
        let mail = full(json!("482913"), json!("https://x.test/verify?t=1"));
        let (out, calls) = go(&["mail", "last", "--to", "dana@x.test", "--otp"], engine(mail.clone()));
        assert_eq!(out.unwrap(), "482913");
        assert_eq!(payloads(&calls, "mail.list"), [json!({"last": 1, "to": "dana@x.test"})]);
        assert_eq!(payloads(&calls, "mail.get"), [json!({"id": "aaa1k3f9"})]);
        let (out, calls) = go(&["mail", "last", "--link", "--link-match", "/verify"], engine(mail));
        assert_eq!(out.unwrap(), "https://x.test/verify?t=1");
        assert_eq!(payloads(&calls, "mail.get"), [json!({"id": "aaa1k3f9", "linkMatch": "/verify"})]);
    }

    #[test]
    fn last_without_the_thing_asked_for_exits_1_and_without_any_mail_exits_3() {
        let err = go(&["mail", "last", "--otp"], engine(full(json!(null), json!(null)))).0.unwrap_err();
        assert_eq!(
            (err.code(), err.line()),
            (1, "bifurc: no code found in the latest mail: \"Your code is 482913\"".to_string())
        );
        let err = go(&["mail", "last", "--link", "--to", "x@y"], engine(full(json!("1"), json!(null)))).0.unwrap_err();
        assert_eq!(err.code(), 1);
        let none = vec![("mail.list", Ok(json!({"mails": [], "matched": 0})))];
        let err = go(&["mail", "last", "--otp"], none).0.unwrap_err();
        assert_eq!(
            (err.code(), err.line()),
            (3, "bifurc: no mail yet (see: bifurc mail list, or use --wait 30s)".to_string())
        );
        for bad in [
            vec!["mail", "last", "--otp", "--link"],
            vec!["mail", "last", "--link-match", "x"],
            vec!["mail", "last", "--wait", "soon"],
        ] {
            assert_eq!(go(&bad, engine(json!({}))).0.unwrap_err().code(), 2, "{bad:?}");
        }
    }

    #[test]
    fn last_wait_asks_the_engine_to_wait_and_a_miss_is_exit_3() {
        let mail = full(json!("482913"), json!(null));
        let (out, calls) = go(&["mail", "last", "--to", "dana@x.test", "--otp", "--wait", "20s"], engine(mail));
        assert_eq!(out.unwrap(), "482913");
        assert_eq!(payloads(&calls, "mail.wait"), [json!({"to": "dana@x.test", "timeoutMs": 20_000})]);
        let missed = vec![("mail.wait", Ok(json!({"mail": null})))];
        let err = go(&["mail", "last", "--otp", "--wait", "1s", "--to", "dana@x.test"], missed).0.unwrap_err();
        assert_eq!((err.code(), err.line()), (3, "bifurc: no mail for dana@x.test arrived within 1s".to_string()));
    }

    #[test]
    fn clear_says_how_many_and_an_unknown_verb_is_a_usage_error() {
        let cleared = |n| vec![("mail.clear", Ok(json!({"cleared": n})))];
        assert_eq!(go(&["mail", "clear"], cleared(3)).0.unwrap(), "cleared 3 mails");
        assert_eq!(go(&["mail", "clear"], cleared(1)).0.unwrap(), "cleared 1 mail");
        assert_eq!(go(&["mail", "send"], vec![]).0.unwrap_err().code(), 2);
    }
}
