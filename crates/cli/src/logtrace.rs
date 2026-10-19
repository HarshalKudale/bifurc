//! Request ↔ server log correlation: `capture show <id> --logs` and `services snippet <stack>`.
//! The proxies stamp `x-bifurc-trace: <capture id>` on what they forward; the engine searches the output of the
//! workspace's dev services (`devservice.grep`) for that id, or for the request's own `traceparent` trace-id.

use serde_json::{Value, json};

use crate::{
    capture::{entry_ids, number},
    commands::{Context, need, workspace_id},
    error::{CliError, Result},
    format::js_template,
};

const APPROXIMATE: &str = "≈ approximate (no line carries the trace id; matched by time and path)";
const HINT: &str =
    "add the trace id to your logs: bifurc services snippet <stack>  (express, fastify, next, fastapi, django, go)";

/// A middleware per stack that puts the trace header into the log line of each request.
const SNIPPETS: [(&str, &str); 6] = [
    (
        "express",
        "app.use((req, _res, next) => { console.log(req.method, req.originalUrl, 'trace=' + req.get('x-bifurc-trace')); next(); });",
    ),
    (
        "fastify",
        "const app = Fastify({ logger: true, requestIdHeader: 'x-bifurc-trace' }); // every log line carries reqId=<trace>",
    ),
    (
        "next",
        "// middleware.ts (Next 16: proxy.ts, export function proxy)\nimport { NextRequest, NextResponse } from 'next/server';\nexport function middleware(req: NextRequest) { console.log(req.method, req.nextUrl.pathname, 'trace=' + req.headers.get('x-bifurc-trace')); return NextResponse.next(); }",
    ),
    (
        "fastapi",
        "@app.middleware('http')\nasync def trace(request, call_next):\n    print(request.method, request.url.path, 'trace=' + str(request.headers.get('x-bifurc-trace')), flush=True)\n    return await call_next(request)",
    ),
    (
        "django",
        "# trace.py; add 'yourapp.trace.trace' to MIDDLEWARE\ndef trace(get_response):\n    def middleware(request):\n        print(request.method, request.path, 'trace=' + str(request.headers.get('X-Bifurc-Trace')), flush=True)\n        return get_response(request)\n    return middleware",
    ),
    (
        "go",
        "func trace(next http.Handler) http.Handler { return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { log.Printf(\"%s %s trace=%s\", r.Method, r.URL.Path, r.Header.Get(\"X-Bifurc-Trace\")); next.ServeHTTP(w, r) }) } // http.ListenAndServe(addr, trace(mux))",
    ),
];

/// `services snippet <stack>`.
pub fn snippet(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 1)?;
    let stack = need(ctx.args.positional(2), "the stack (express, fastify, next, fastapi, django, go)")?.to_lowercase();
    match SNIPPETS.iter().find(|(name, _)| *name == stack.trim_end_matches(".js")) {
        Some((_, code)) => Ok((*code).to_string()),
        None => Err(CliError::usage(format!(
            "unknown stack \"{stack}\" (known: {})",
            SNIPPETS.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(", ")
        ))),
    }
}

/// The path of a URL without its query.
fn path_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path = rest.find('/').map_or("/", |at| &rest[at..]);
    path.split(['?', '#']).next().unwrap_or("/").to_string()
}

fn has_match(found: &Value) -> bool {
    found["lines"].as_array().is_some_and(|lines| lines.iter().any(|l| l["match"] == json!(true)))
}

/// The log lines of an exchange: those that carry its id (or its `traceparent` trace-id), else those inside its
/// time span that mention its path. `search` asks the engine; returns the lines, whether they are approximate,
/// and whether any line in the logs ever carried an id.
fn find(
    entry: &Value,
    id: &str,
    base: Value,
    search: &mut dyn FnMut(Value) -> Result<Value>,
) -> Result<(Vec<Value>, bool, bool)> {
    let mut exact = base.clone();
    exact["needles"] = json!([id]);
    exact["probes"] = json!(["x-bifurc-trace", &id[..id.len().min(5)]]);
    if let Some(trace) = entry["traceId"].as_str() {
        exact["needles"].as_array_mut().expect("array").push(json!(trace));
    }
    let found = search(exact)?;
    let traced = found["traced"] == json!(true);
    if has_match(&found) {
        return Ok((found["lines"].as_array().cloned().unwrap_or_default(), false, traced));
    }
    let path = path_of(entry["url"].as_str().unwrap_or(""));
    if path.len() > 1 {
        // The request's span: a second before it came in to two after it was answered.
        let (start, ms) = (entry["ts"].as_i64().unwrap_or(0), entry["durationMs"].as_i64().unwrap_or(0));
        let mut near = base;
        near["needles"] = json!([path]);
        near["sinceTs"] = json!(start - 1000);
        near["untilTs"] = json!(start + ms + 2000);
        let found = search(near)?;
        if has_match(&found) {
            return Ok((found["lines"].as_array().cloned().unwrap_or_default(), true, traced));
        }
    }
    Ok((Vec::new(), false, traced))
}

/// The lines for printing: `--` between groups that are not adjacent, the service named when several matched.
fn render(lines: &[Value]) -> String {
    let mut services: Vec<&str> =
        lines.iter().filter(|l| l["match"] == json!(true)).filter_map(|l| l["service"].as_str()).collect();
    services.dedup();
    let named = services.len() > 1;
    let (mut out, mut last): (Vec<String>, Option<(&str, u64)>) = (Vec::new(), None);
    for line in lines {
        let (service, seq) = (line["service"].as_str().unwrap_or(""), line["seq"].as_u64().unwrap_or(0));
        if last.is_some_and(|(s, n)| s != service || n + 1 != seq) {
            out.push("--".into());
        }
        last = Some((service, seq));
        let text = js_template(line.get("text"));
        out.push(if named { format!("{service} | {text}") } else { text });
    }
    out.join("\n")
}

/// `capture show <entry> --logs [--context N] [--service <name>]`.
pub fn logs(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["logs", "context", "service"], 1)?;
    let wanted = need(ctx.args.positional(2), "the entry's id (see: bifurc capture list)")?;
    let workspace = workspace_id(ctx)?;
    let id = entry_ids(ctx, &workspace, &[wanted])?.remove(0);
    let got = ctx.request("capture.get", json!({ "workspaceId": workspace, "id": id }))?;
    let Some(entry) = got.get("entry").filter(|entry| entry.is_object()) else {
        return Err(CliError::not_found(format!("no captured entry \"{id}\" (see: bifurc capture list)")));
    };
    let mut base = json!({ "workspaceId": workspace });
    if let Some(context) = number(ctx, "context")? {
        base["context"] = json!(context);
    }
    if let Some(service) = ctx.args.flag("service") {
        base["name"] = json!(service);
    }
    let (lines, approximate, traced) = find(entry, &id, base, &mut |payload| ctx.request("devservice.grep", payload))?;
    let out = json!({ "schemaVersion": 1, "id": id, "approximate": approximate, "lines": lines.iter().map(|l| json!({
        "service": l["service"], "ts": l["ts"], "stream": l["stream"], "text": l["text"], "match": l["match"],
    })).collect::<Vec<_>>() });
    Ok(ctx.print(&out, || {
        if lines.is_empty() {
            let hint = if traced { String::new() } else { format!("\n{HINT}") };
            return format!("no log lines for #{id}{hint}");
        }
        let body = render(&lines);
        if approximate { format!("{APPROXIMATE}\n{body}") } else { body }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{config, payloads, run_with_events};

    fn line(service: &str, seq: u64, text: &str, matched: bool) -> Value {
        json!({"service": service, "seq": seq, "ts": 1, "stream": "stdout", "text": text, "match": matched})
    }

    fn entry() -> Value {
        json!({"id": "mgv2k3f9abcd", "ts": 10_000, "durationMs": 40, "url": "http://api.localhost/orders?x=1"})
    }

    /// `find` with an engine that answers each search in turn; returns the result and the searches made.
    fn found(entry: &Value, answers: Vec<Value>) -> ((Vec<Value>, bool, bool), Vec<Value>) {
        let (mut answers, mut asked) = (answers.into_iter(), Vec::new());
        let got = find(entry, "mgv2k3f9abcd", json!({"workspaceId": "w1", "context": 2}), &mut |p| {
            asked.push(p);
            Ok(answers.next().expect("one more search than expected"))
        })
        .unwrap();
        (got, asked)
    }

    #[test]
    fn groups_are_separated_and_services_named_only_when_several_matched() {
        let one = [line("api", 3, "a", true), line("api", 4, "b", false), line("api", 9, "c", true)];
        assert_eq!(render(&one), "a\nb\n--\nc");
        let two = [line("api", 3, "a", true), line("web", 1, "b", true)];
        assert_eq!(render(&two), "api | a\n--\nweb | b");
    }

    #[test]
    fn the_trace_id_and_the_traceparent_id_are_searched_with_the_context() {
        let mut with_parent = entry();
        with_parent["traceId"] = json!("4bf92f3577b34da6a3ce929d0e0e4736");
        let hit = json!({"lines": [line("api", 0, "trace=mgv2k3f9abcd boom", true)], "traced": true});
        let ((lines, approximate, traced), asked) = found(&with_parent, vec![hit]);
        assert_eq!((lines.len(), approximate, traced), (1, false, true));
        assert_eq!(asked[0]["needles"], json!(["mgv2k3f9abcd", "4bf92f3577b34da6a3ce929d0e0e4736"]));
        assert_eq!(asked[0]["context"], 2);
    }

    #[test]
    fn without_the_id_the_time_window_and_path_are_tried_and_marked_approximate() {
        let none = json!({"lines": [], "traced": false});
        let near = json!({"lines": [line("api", 5, "GET /orders 500", true)], "traced": false});
        let ((lines, approximate, _), asked) = found(&entry(), vec![none, near]);
        assert_eq!((lines.len(), approximate), (1, true));
        let second = &asked[1];
        assert_eq!(
            (&second["needles"], &second["sinceTs"], &second["untilTs"]),
            (&json!(["/orders"]), &json!(9000), &json!(12040))
        );
    }

    #[test]
    fn nothing_found_is_empty_and_the_hint_depends_on_whether_any_line_carried_an_id() {
        let none = |traced: bool| json!({"lines": [], "traced": traced});
        let ((lines, approximate, traced), asked) = found(&entry(), vec![none(true), none(true)]);
        assert_eq!((lines.len(), approximate, traced, asked.len()), (0, false, true, 2));
        // The home page has no path to match on: one search only.
        let mut root = entry();
        root["url"] = json!("http://api.localhost/");
        assert_eq!(found(&root, vec![none(false)]).1.len(), 1);
    }

    #[test]
    fn json_lists_the_lines_through_the_engine() {
        let answers = vec![
            ("config.get", Ok(config())),
            ("capture.list", Ok(json!({"entries": [{"id": "mgv2k3f9abcd"}], "matched": 1}))),
            ("capture.get", Ok(json!({"entry": entry()}))),
            ("devservice.grep", Ok(json!({"lines": [line("api", 0, "x mgv2k3f9abcd", true)], "traced": true}))),
        ];
        let argv = ["capture", "show", "abcd", "--logs", "--json", "--service", "api"];
        let (out, calls) = run_with_events(&argv, answers.clone(), vec![], logs);
        let parsed: Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!((&parsed["schemaVersion"], &parsed["approximate"]), (&json!(1), &json!(false)));
        assert_eq!(parsed["lines"][0]["service"], "api");
        assert_eq!(payloads(&calls, "devservice.grep")[0]["name"], "api");
        let (out, _) = run_with_events(&argv[..4], answers, vec![], logs);
        assert_eq!(out.unwrap(), "x mgv2k3f9abcd");
    }

    #[test]
    fn an_unknown_stack_is_a_usage_error_listing_the_known_ones() {
        let (out, _) =
            run_with_events(&["services", "snippet", "rails"], vec![("config.get", Ok(config()))], vec![], snippet);
        let message = format!("{:?}", out.unwrap_err());
        assert!(
            message.contains("unknown stack") && message.contains("rails") && message.starts_with("Exit(2"),
            "{message}"
        );
        assert!(message.contains("express, fastify, next, fastapi, django, go"), "{message}");
        let (out, _) = run_with_events(&["services", "snippet", "express"], vec![], vec![], snippet);
        assert!(out.unwrap().contains("x-bifurc-trace"));
    }

    #[test]
    fn the_path_drops_scheme_host_and_query() {
        assert_eq!(path_of("https://h:81/a/b?x=1#f"), "/a/b");
        assert_eq!(path_of("http://h"), "/");
    }
}
