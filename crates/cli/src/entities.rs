//! `mappings`, `rules` and `mocks`: the saved items that can be switched on and off. Each has the same verbs
//! (`list`, `show`, `create`, `update`, `enable`, `disable`); only what `create` and `update` take differs.
//!
//! An update loads the item, changes the fields that were given and writes it back, so what the command line
//! has no option for (a rule's scripts, a mock's captured request) is kept as it is.

use serde_json::{Map, Value, json};

use crate::{
    args::ParsedArgs,
    commands::{
        Context, body_from, created, find_one, header_flags, id_of, list_entities, merge_headers, need, switch,
        unknown_verb, with_field, workspace_id,
    },
    error::{CliError, Result},
    format::{js_template, json, table, truthy},
};

mod scenarios;

/// One kind of item that can be switched on and off.
struct Kind {
    /// The command group, which is also what `(see: bifurc <group> list)` names.
    group: &'static str,
    /// What one of them is called in a message.
    what: &'static str,
    /// The entity kind of `entity.create`, `entity.list`, `entity.load` and `entity.update`.
    kind: &'static str,
    /// The kind `entity.setEnabled` knows it by.
    toggle_kind: &'static str,
    /// The fields an item can be named by instead of its id.
    name_keys: &'static [&'static str],
    columns: &'static [&'static str],
}

const MAPPINGS: Kind = Kind {
    group: "mappings",
    what: "mapping",
    kind: "mappings",
    toggle_kind: "mappings",
    name_keys: &["domain", "label"],
    columns: &["id", "enabled", "domain", "target", "label"],
};

const RULES: Kind = Kind {
    group: "rules",
    what: "rule",
    kind: "proxyRules",
    toggle_kind: "rules",
    name_keys: &["name"],
    columns: &["id", "enabled", "name", "pattern", "regex", "target"],
};

const MOCKS: Kind = Kind {
    group: "mocks",
    what: "mock",
    kind: "mocks",
    toggle_kind: "mocks",
    name_keys: &["name"],
    columns: &["id", "enabled", "method", "name", "urlPattern", "status"],
};

/// The item as `entity.load` gives it: its file plus whether it is enabled.
fn load(ctx: &mut Context, kind: &Kind, workspace: &str, id: &str) -> Result<Value> {
    let response = ctx.request("entity.load", json!({ "workspaceId": workspace, "kind": kind.kind, "id": id }))?;
    match response.get("entity").filter(|entity| entity.is_object()) {
        Some(entity) if response.get("ok").is_some_and(truthy) => Ok(entity.clone()),
        _ => Err(CliError::not_found(format!("no {} \"{id}\" (see: bifurc {} list)", kind.what, kind.group))),
    }
}

/// The workspace and the item the command's first operand names.
fn resolve(ctx: &mut Context, kind: &Kind) -> Result<(String, Value)> {
    let wanted = need(ctx.args.positional(2), &format!("{} id or name", kind.what))?;
    let workspace = workspace_id(ctx)?;
    let rows = list_entities(ctx, &workspace, kind.kind)?;
    let id = id_of(find_one(&rows, &wanted, kind.name_keys, kind.what, kind.group)?);
    let entity = load(ctx, kind, &workspace, &id)?;
    Ok((workspace, entity))
}

/// `list`: every item with whether it is enabled. `column` adds what the table shows and the file does not hold.
fn list(
    ctx: &mut Context,
    kind: &Kind,
    mut column: impl FnMut(&mut Context, &str, Value) -> Result<Value>,
) -> Result<String> {
    ctx.accept(&[], 0)?;
    let workspace = workspace_id(ctx)?;
    let mut rows = Vec::new();
    for row in list_entities(ctx, &workspace, kind.kind)? {
        let entity = load(ctx, kind, &workspace, &id_of(&row))?;
        rows.push(column(ctx, &workspace, entity)?);
    }
    Ok(ctx.print(&Value::Array(rows.clone()), || table(&rows, kind.columns)))
}

fn show(ctx: &mut Context, kind: &Kind) -> Result<String> {
    ctx.accept(&[], 1)?;
    let (_, entity) = resolve(ctx, kind)?;
    Ok(ctx.print(&entity, || json(&entity)))
}

fn set_enabled_by_id(ctx: &mut Context, kind: &Kind, workspace: &str, id: &str, enabled: bool) -> Result<()> {
    ctx.request(
        "entity.setEnabled",
        json!({ "workspaceId": workspace, "kind": kind.toggle_kind, "id": id, "enabled": enabled }),
    )?;
    Ok(())
}

fn set_enabled(ctx: &mut Context, kind: &Kind, enabled: bool) -> Result<String> {
    ctx.accept(&[], 1)?;
    let (workspace, entity) = resolve(ctx, kind)?;
    let id = id_of(&entity);
    set_enabled_by_id(ctx, kind, &workspace, &id, enabled)?;
    let entity = with_field(&entity, "enabled", json!(enabled));
    Ok(ctx.print(&entity, || format!("{} {} {id}", if enabled { "Enabled" } else { "Disabled" }, kind.what)))
}

/// Create `entity` and return it as the engine saved it.
fn create(ctx: &mut Context, kind: &Kind, workspace: &str, entity: Value) -> Result<Value> {
    let response =
        ctx.request("entity.create", json!({ "kind": kind.kind, "workspaceId": workspace, "entity": entity }))?;
    Ok(created(&response))
}

/// Write a changed item back. `changed` is whether an option changed anything; `options` names them for the
/// error when none did.
fn update(
    ctx: &mut Context,
    kind: &Kind,
    workspace: &str,
    mut entity: Value,
    changed: bool,
    options: &str,
) -> Result<String> {
    if !changed {
        return Err(CliError::usage(format!("nothing to change: give at least one of {options}")));
    }
    if entity.get("workspaceId").is_none_or(Value::is_null) {
        entity["workspaceId"] = json!(workspace);
    }
    ctx.request("entity.update", json!({ "kind": kind.kind, "entity": entity }))?;
    let id = id_of(&entity);
    Ok(ctx.print(&entity, || format!("Updated {} {id}", kind.what)))
}

/// Set `key` from `--<flag>` when it is given. Returns whether it was.
fn set_text(entity: &mut Value, key: &str, args: &ParsedArgs, flag: &str) -> bool {
    match args.flag(flag) {
        Some(value) => {
            entity[key] = json!(value);
            true
        }
        None => false,
    }
}

/// `api` or `api.localhost` as the domain a mapping is saved with.
fn localhost_domain(name: &str) -> String {
    let name = name.to_lowercase();
    if name.ends_with(".localhost") { name } else { format!("{name}.localhost") }
}

/// `mappings list|show|create|update|enable|disable`.
pub fn mappings(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    let args = ctx.args;
    match sub.unwrap_or("list") {
        "list" => list(ctx, &MAPPINGS, |_, _, entity| Ok(entity)),
        "show" => show(ctx, &MAPPINGS),
        "create" | "add" => {
            ctx.accept(&["label", "disabled"], 2)?;
            let domain = localhost_domain(&need(args.positional(2), "domain, e.g. api or api.localhost")?);
            let target = need(args.positional(3), "target, e.g. 127.0.0.1:3000 or http://host:port")?;
            let workspace = workspace_id(ctx)?;
            let entity = json!({
                "domain": domain,
                "target": target,
                "label": args.flag("label").unwrap_or_default(),
                "enabled": !args.has("disabled"),
            });
            let saved = create(ctx, &MAPPINGS, &workspace, entity)?;
            Ok(ctx.print(&saved, || format!("Mapped http://{domain} → {target}  (id {})", id_of(&saved))))
        }
        "update" => {
            ctx.accept(&["domain", "target", "label"], 1)?;
            let (workspace, mut entity) = resolve(ctx, &MAPPINGS)?;
            let mut changed = set_text(&mut entity, "target", args, "target");
            changed |= set_text(&mut entity, "label", args, "label");
            if let Some(domain) = args.flag("domain") {
                entity["domain"] = json!(localhost_domain(domain));
                changed = true;
            }
            update(ctx, &MAPPINGS, &workspace, entity, changed, "--domain, --target, --label")
        }
        "enable" => set_enabled(ctx, &MAPPINGS, true),
        "disable" => set_enabled(ctx, &MAPPINGS, false),
        other => Err(unknown_verb("mappings", other)),
    }
}

/// Point a rule where `--mapping` or `--target` says. Returns whether either was given.
fn set_rule_target(ctx: &mut Context, workspace: &str, rule: &mut Value) -> Result<bool> {
    let args = ctx.args;
    match (args.flag("mapping"), args.flag("target")) {
        (Some(_), Some(_)) => Err(CliError::usage("--mapping and --target cannot both be given")),
        (Some(wanted), None) => {
            let mappings = list_entities(ctx, workspace, MAPPINGS.kind)?;
            let mapping = find_one(&mappings, wanted, MAPPINGS.name_keys, MAPPINGS.what, MAPPINGS.group)?;
            rule["targetType"] = json!("mapping");
            rule["targetMappingId"] = json!(id_of(mapping));
            Ok(true)
        }
        (None, Some(target)) => {
            rule["targetType"] = json!("external");
            rule["targetExternal"] = json!(target);
            Ok(true)
        }
        (None, None) => Ok(false),
    }
}

/// `rules list|show|create|update|enable|disable`: the forward proxy's rules.
pub fn rules(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    let args = ctx.args;
    match sub.unwrap_or("list") {
        "list" => {
            // The mappings are read once, and only when a rule points at one.
            let mut mappings: Option<Vec<Value>> = None;
            list(ctx, &RULES, |ctx, workspace, rule| {
                let target = if rule.get("targetType").and_then(Value::as_str) == Some("mapping") {
                    if mappings.is_none() {
                        mappings = Some(list_entities(ctx, workspace, MAPPINGS.kind)?);
                    }
                    let id = rule.get("targetMappingId");
                    let mapping = mappings.iter().flatten().find(|mapping| mapping.get("id") == id);
                    mapping.map_or_else(|| js_template(id), |mapping| js_template(mapping.get("domain")))
                } else {
                    rule.get("targetExternal").and_then(Value::as_str).unwrap_or_default().to_string()
                };
                let regex = if rule.get("useRegex").is_some_and(truthy) { "regex" } else { "" };
                Ok(with_field(&with_field(&rule, "regex", json!(regex)), "target", json!(target)))
            })
        }
        "show" => show(ctx, &RULES),
        "create" | "add" => {
            ctx.accept(&["mapping", "target", "name", "regex", "disabled"], 1)?;
            let pattern = need(args.positional(2), "pattern, e.g. api.example.com/v1")?;
            if args.flag("mapping").is_none() && args.flag("target").is_none() {
                return Err(CliError::usage(
                    "missing where to send what matches: --mapping <id|domain> or --target <url>",
                ));
            }
            let workspace = workspace_id(ctx)?;
            let mut rule = json!({
                "name": args.flag("name").unwrap_or(&pattern),
                "pattern": pattern,
                "useRegex": args.has("regex"),
                "enabled": !args.has("disabled"),
                "targetType": "external",
                "targetMappingId": "",
                "targetExternal": "",
                "requestScript": "",
                "responseScript": "",
            });
            set_rule_target(ctx, &workspace, &mut rule)?;
            let saved = create(ctx, &RULES, &workspace, rule)?;
            Ok(ctx.print(&saved, || format!("Saved rule {}", id_of(&saved))))
        }
        "update" => {
            ctx.accept(&["pattern", "mapping", "target", "name", "regex", "no-regex"], 1)?;
            let regex = switch(args, "regex", "no-regex")?;
            let (workspace, mut rule) = resolve(ctx, &RULES)?;
            let mut changed = set_text(&mut rule, "name", args, "name");
            changed |= set_text(&mut rule, "pattern", args, "pattern");
            if let Some(regex) = regex {
                rule["useRegex"] = json!(regex);
                changed = true;
            }
            changed |= set_rule_target(ctx, &workspace, &mut rule)?;
            update(
                ctx,
                &RULES,
                &workspace,
                rule,
                changed,
                "--pattern, --regex, --no-regex, --mapping, --target, --name",
            )
        }
        "enable" => set_enabled(ctx, &RULES, true),
        "disable" => set_enabled(ctx, &RULES, false),
        other => Err(unknown_verb("rules", other)),
    }
}

/// A mock that is the whole response to calls of `pattern`: an empty `200` until the options say otherwise.
fn new_mock(pattern: &str) -> Value {
    json!({
        "name": "",
        "method": "GET",
        "urlPattern": pattern,
        "useRegex": false,
        "standalone": true,
        "responseStatus": 200,
        "responseStatusMocked": true,
        "responseHeaders": {},
        "mockedResponseHeaders": [],
        "responseBody": "",
        "responseBodyMocked": true,
        "responseDelay": 0,
        "responseDelayMocked": true,
        "streamingMode": "none",
        "streamingChunkDelay": 0,
    })
}

/// `--<flag>` as a whole number from `min` to `max`, when it is given.
fn number(args: &ParsedArgs, flag: &str, min: u64, max: u64, what: &str) -> Result<Option<u64>> {
    let Some(raw) = args.flag(flag) else { return Ok(None) };
    match raw.parse::<u64>() {
        Ok(value) if (min..=max).contains(&value) => Ok(Some(value)),
        _ => Err(CliError::usage(format!("--{flag} must be {what}, not \"{raw}\""))),
    }
}

/// Put the response options into a mock. A part of the response an option sets is a part the mock replaces.
/// Returns whether any option was given.
fn apply_mock_options(mock: &mut Value, args: &ParsedArgs) -> Result<bool> {
    let regex = switch(args, "regex", "no-regex")?;
    let status = number(args, "status", 100, 599, "a status from 100 to 599")?;
    let delay = number(args, "delay", 0, u64::MAX, "a whole number of milliseconds")?;
    let headers = header_flags(args)?;
    let body = body_from(args)?;

    let mut changed = set_text(mock, "name", args, "name");
    if let Some(method) = args.flag("method") {
        mock["method"] =
            json!(if method.eq_ignore_ascii_case("any") { "*".to_string() } else { method.to_uppercase() });
        changed = true;
    }
    if let Some(regex) = regex {
        mock["useRegex"] = json!(regex);
        changed = true;
    }
    if let Some(status) = status {
        mock["responseStatus"] = json!(status);
        mock["responseStatusMocked"] = json!(true);
        changed = true;
    }
    if let Some(delay) = delay {
        mock["responseDelay"] = json!(delay);
        mock["responseDelayMocked"] = json!(true);
        changed = true;
    }
    if let Some(body) = body {
        mock["responseBody"] = json!(body);
        mock["responseBodyMocked"] = json!(true);
        // The body is text now, whatever it was.
        if let Some(fields) = mock.as_object_mut() {
            fields.remove("responseBodyEncoding");
        }
        changed = true;
    }
    if !headers.is_empty() {
        let mut response_headers: Map<String, Value> =
            mock.get("responseHeaders").and_then(Value::as_object).cloned().unwrap_or_default();
        merge_headers(&mut response_headers, &headers);
        mock["responseHeaders"] = Value::Object(response_headers);
        // The names of the headers the mock sets. The type of the body is not one of them: it is always the
        // mock's own when the mock replaces the body.
        let mut mocked: Vec<Value> =
            mock.get("mockedResponseHeaders").and_then(Value::as_array).cloned().unwrap_or_default();
        for (name, value) in &headers {
            mocked.retain(|other| !other.as_str().is_some_and(|other| other.eq_ignore_ascii_case(name)));
            if !value.is_empty() && !name.eq_ignore_ascii_case("content-type") {
                mocked.push(json!(name));
            }
        }
        mock["mockedResponseHeaders"] = Value::Array(mocked);
        changed = true;
    }
    Ok(changed)
}

/// The type a new mock's body is served as when no `-H "Content-Type: …"` says: JSON when it is JSON.
fn default_content_type(mock: &mut Value) {
    let body = mock.get("responseBody").and_then(Value::as_str).unwrap_or_default();
    let headers = mock.get("responseHeaders").and_then(Value::as_object);
    if body.is_empty() || headers.is_some_and(|h| h.keys().any(|name| name.eq_ignore_ascii_case("content-type"))) {
        return;
    }
    let is_json = serde_json::from_str::<Value>(body).is_ok_and(|parsed| parsed.is_object() || parsed.is_array());
    mock["responseHeaders"]["Content-Type"] = json!(if is_json { "application/json" } else { "text/plain" });
}

const MOCK_RESPONSE_OPTIONS: [&str; 9] =
    ["method", "name", "status", "header", "data", "body-file", "delay", "regex", "no-regex"];

/// `mocks list|show|create|update|enable|disable|scenario|status|hits`.
pub fn mocks(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    let args = ctx.args;
    match sub.unwrap_or("list") {
        "list" => list(ctx, &MOCKS, |_, _, mock| {
            let status = mock.get("responseStatus").cloned().unwrap_or(Value::Null);
            Ok(with_field(&mock, "status", status))
        }),
        "show" => show(ctx, &MOCKS),
        "scenario" => scenarios::scenario(ctx),
        "status" => scenarios::status(ctx),
        "hits" => scenarios::hits(ctx),
        "create" | "add" if args.has("from-request") || args.has("from-capture") => scenarios::create_from(ctx),
        "create" | "add" => {
            let mut options = MOCK_RESPONSE_OPTIONS.to_vec();
            options.extend(["curl", "url", "disabled"]);
            ctx.accept(&options, 1)?;
            let pattern = args.positional(2).or(args.flag("url"));
            let mut mock = match (args.flag("curl"), pattern) {
                (Some(_), Some(_)) => {
                    return Err(CliError::usage(
                        "--curl already says what the mock answers to; leave out the URL pattern",
                    ));
                }
                (Some(curl), None) => {
                    let built = ctx.request("curl.buildMock", json!({ "curl": curl }))?;
                    built.get("mock").cloned().unwrap_or(Value::Null)
                }
                (None, pattern) => new_mock(&need(pattern, "URL pattern, e.g. https://api.example.com/users")?),
            };
            apply_mock_options(&mut mock, args)?;
            default_content_type(&mut mock);
            if mock.get("name").and_then(Value::as_str).unwrap_or_default().is_empty() {
                mock["name"] =
                    json!(format!("{} {}", js_template(mock.get("method")), js_template(mock.get("urlPattern"))));
            }
            mock["enabled"] = json!(!args.has("disabled"));
            let workspace = workspace_id(ctx)?;
            let saved = create(ctx, &MOCKS, &workspace, mock)?;
            Ok(ctx.print(&saved, || format!("Saved mock {}", id_of(&saved))))
        }
        "update" => {
            let mut options = MOCK_RESPONSE_OPTIONS.to_vec();
            options.push("url");
            ctx.accept(&options, 1)?;
            // The options are read before the engine is asked for anything, so a bad one changes nothing.
            apply_mock_options(&mut json!({}), args)?;
            let (workspace, mut mock) = resolve(ctx, &MOCKS)?;
            let mut changed = apply_mock_options(&mut mock, args)?;
            changed |= set_text(&mut mock, "urlPattern", args, "url");
            update(
                ctx,
                &MOCKS,
                &workspace,
                mock,
                changed,
                "--url, --method, --name, --status, -H, -d/--data, --body-file, --delay, --regex, --no-regex",
            )
        }
        "enable" => set_enabled(ctx, &MOCKS, true),
        "disable" => set_enabled(ctx, &MOCKS, false),
        other => Err(unknown_verb("mocks", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run, sub};

    fn listing(entities: Value) -> Result<Value> {
        Ok(json!({ "entities": entities }))
    }

    fn loaded(entity: Value) -> Result<Value> {
        Ok(json!({ "ok": true, "entity": entity }))
    }

    fn mapping() -> Value {
        json!({"id": "m1", "domain": "api.localhost", "target": "127.0.0.1:3000", "label": "API", "workspaceId": "w1"})
    }

    /// An engine with one mapping, which `entity.load` reports as enabled.
    fn one_mapping() -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            ("entity.list:mappings", listing(json!([mapping()]))),
            ("entity.load:mappings", loaded(with_field(&mapping(), "enabled", json!(true)))),
            ("entity.update", Ok(json!({"ok": true}))),
            ("entity.setEnabled", Ok(json!({"ok": true}))),
        ]
    }

    #[test]
    fn mappings_list_shows_whether_each_is_enabled() {
        let argv = ["mappings"];
        let (out, calls) = run(&argv, one_mapping(), |c| mappings(c, None));
        assert_eq!(
            out.unwrap(),
            "ID  ENABLED  DOMAIN         TARGET          LABEL\nm1  true     api.localhost  127.0.0.1:3000  API"
        );
        assert_eq!(calls[1].1, json!({"workspaceId": "w1", "kind": "mappings"}));
        assert_eq!(calls[2], ("entity.load".to_string(), json!({"workspaceId": "w1", "kind": "mappings", "id": "m1"})));
    }

    #[test]
    fn mappings_show_takes_an_id_a_domain_or_a_label() {
        for wanted in ["m1", "API.localhost", "api"] {
            let argv = ["mappings", "show", wanted];
            let (out, _) = run(&argv, one_mapping(), |c| mappings(c, sub(&argv)));
            let shown: Value = serde_json::from_str(&out.unwrap()).unwrap();
            assert_eq!(shown["enabled"], true, "{wanted}");
            assert_eq!(shown["target"], "127.0.0.1:3000");
        }
        let argv = ["mappings", "show", "nope"];
        let (out, _) = run(&argv, one_mapping(), |c| mappings(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::not_found("no mapping \"nope\" (see: bifurc mappings list)"));
        let argv = ["mappings", "show"];
        let (out, _) = run(&argv, vec![], |c| mappings(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("missing mapping id or name"));
    }

    #[test]
    fn mappings_create_appends_localhost_and_lowercases() {
        let argv = ["mappings", "create", "API", "127.0.0.1:3000", "--label", "My API"];
        let answers =
            vec![("config.get", Ok(config())), ("entity.create", Ok(json!({"id": "m9", "entity": {"id": "m9"}})))];
        let (out, calls) = run(&argv, answers, |c| mappings(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Mapped http://api.localhost → 127.0.0.1:3000  (id m9)");
        assert_eq!(
            calls[1].1,
            json!({"kind": "mappings", "workspaceId": "w1",
                   "entity": {"domain": "api.localhost", "target": "127.0.0.1:3000", "label": "My API", "enabled": true}})
        );
    }

    #[test]
    fn mappings_create_keeps_a_full_domain_can_start_disabled_and_prints_the_entity_as_json() {
        let argv = ["mappings", "add", "Api.LOCALHOST", "t", "-w", "w2", "--disabled", "--json"];
        let answers =
            vec![("entity.create", Ok(json!({"id": "n1", "entity": {"id": "n1", "domain": "api.localhost"}})))];
        let (out, calls) = run(&argv, answers, |c| mappings(c, sub(&argv)));
        assert_eq!(
            serde_json::from_str::<Value>(&out.unwrap()).unwrap(),
            json!({"id": "n1", "domain": "api.localhost"})
        );
        assert_eq!(calls[0].1["workspaceId"], "w2");
        assert_eq!(calls[0].1["entity"]["label"], "");
        assert_eq!(calls[0].1["entity"]["enabled"], false);
    }

    #[test]
    fn mappings_create_needs_domain_and_target() {
        let argv = ["mappings", "create"];
        let (out, _) = run(&argv, vec![], |c| mappings(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("missing domain, e.g. api or api.localhost"));
        let argv = ["mappings", "create", "api"];
        let (out, _) = run(&argv, vec![], |c| mappings(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("missing target, e.g. 127.0.0.1:3000 or http://host:port"));
    }

    #[test]
    fn mappings_update_changes_only_what_is_given() {
        let argv = ["mappings", "update", "api.localhost", "--target", "127.0.0.1:4000", "--domain", "Web"];
        let (out, calls) = run(&argv, one_mapping(), |c| mappings(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Updated mapping m1");
        assert_eq!(
            payloads(&calls, "entity.update"),
            [json!({"kind": "mappings", "entity": {
                "id": "m1", "domain": "web.localhost", "target": "127.0.0.1:4000", "label": "API", "workspaceId": "w1",
                "enabled": true}})]
        );

        let argv = ["mappings", "update", "m1"];
        let (out, calls) = run(&argv, one_mapping(), |c| mappings(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::usage("nothing to change: give at least one of --domain, --target, --label")
        );
        assert!(payloads(&calls, "entity.update").is_empty());
    }

    #[test]
    fn enable_and_disable_go_by_id_whatever_named_the_item() {
        for (verb, word, enabled) in [("enable", "Enabled", true), ("disable", "Disabled", false)] {
            let argv = ["mappings", verb, "api.localhost"];
            let (out, calls) = run(&argv, one_mapping(), |c| mappings(c, sub(&argv)));
            assert_eq!(out.unwrap(), format!("{word} mapping m1"));
            assert_eq!(
                payloads(&calls, "entity.setEnabled"),
                [json!({"workspaceId": "w1", "kind": "mappings", "id": "m1", "enabled": enabled})]
            );
        }
    }

    fn rule() -> Value {
        json!({"id": "r1", "name": "Users", "pattern": "api.example.com/users", "useRegex": false,
               "targetType": "mapping", "targetMappingId": "m1", "targetExternal": "",
               "requestScript": "// kept", "responseScript": "", "workspaceId": "w1"})
    }

    fn one_rule() -> Vec<Answer> {
        let mut answers = one_mapping();
        answers.extend([
            ("entity.list:proxyRules", listing(json!([rule()]))),
            ("entity.load:proxyRules", loaded(with_field(&rule(), "enabled", json!(false)))),
            ("entity.create", Ok(json!({"id": "r9", "entity": {"id": "r9"}}))),
        ]);
        answers
    }

    #[test]
    fn rules_list_names_the_mapping_a_rule_points_at() {
        let argv = ["rules"];
        let (out, _) = run(&argv, one_rule(), |c| rules(c, None));
        assert_eq!(
            out.unwrap(),
            "ID  ENABLED  NAME   PATTERN                REGEX  TARGET\n\
             r1  false    Users  api.example.com/users         api.localhost"
        );
    }

    #[test]
    fn rules_create_points_at_a_mapping_or_at_another_server() {
        let argv = ["rules", "create", "^https://x/", "--mapping", "api.localhost", "--regex", "--name", "X"];
        let (out, calls) = run(&argv, one_rule(), |c| rules(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Saved rule r9");
        assert_eq!(
            payloads(&calls, "entity.create"),
            [json!({"kind": "proxyRules", "workspaceId": "w1", "entity": {
                "name": "X", "pattern": "^https://x/", "useRegex": true, "enabled": true, "targetType": "mapping",
                "targetMappingId": "m1", "targetExternal": "", "requestScript": "", "responseScript": ""}})]
        );

        let argv = ["rules", "create", "x.test", "--target", "http://localhost:9000", "--disabled"];
        let (_, calls) = run(&argv, one_rule(), |c| rules(c, sub(&argv)));
        let entity = &payloads(&calls, "entity.create")[0]["entity"];
        assert_eq!(entity["name"], "x.test");
        assert_eq!(entity["targetType"], "external");
        assert_eq!(entity["targetExternal"], "http://localhost:9000");
        assert_eq!(entity["enabled"], false);
    }

    #[test]
    fn rules_create_needs_a_pattern_and_exactly_one_target() {
        let argv = ["rules", "create"];
        let (out, _) = run(&argv, vec![], |c| rules(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("missing pattern, e.g. api.example.com/v1"));
        let argv = ["rules", "create", "x"];
        let (out, calls) = run(&argv, vec![], |c| rules(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::usage("missing where to send what matches: --mapping <id|domain> or --target <url>")
        );
        assert!(calls.is_empty());
        let argv = ["rules", "create", "x", "--mapping", "m1", "--target", "http://y"];
        let (out, calls) = run(&argv, one_rule(), |c| rules(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("--mapping and --target cannot both be given"));
        assert!(payloads(&calls, "entity.create").is_empty());
        let argv = ["rules", "create", "x", "--mapping", "nope"];
        let (out, _) = run(&argv, one_rule(), |c| rules(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::not_found("no mapping \"nope\" (see: bifurc mappings list)"));
    }

    #[test]
    fn rules_update_keeps_the_scripts_and_has_no_option_for_them() {
        let argv = ["rules", "update", "users", "--target", "http://localhost:9000", "--regex"];
        let (out, calls) = run(&argv, one_rule(), |c| rules(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Updated rule r1");
        let entity = &payloads(&calls, "entity.update")[0]["entity"];
        assert_eq!(entity["targetType"], "external");
        assert_eq!(entity["targetExternal"], "http://localhost:9000");
        assert_eq!(entity["useRegex"], true);
        assert_eq!(entity["requestScript"], "// kept");
        assert_eq!(entity["name"], "Users");

        let argv = ["rules", "update", "r1", "--request-script", "x"];
        let (out, calls) = run(&argv, one_rule(), |c| rules(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::usage("unknown option --request-script for \"rules update\" (see: bifurc rules --help)")
        );
        assert!(calls.is_empty());

        let argv = ["rules", "update", "r1", "--regex", "--no-regex"];
        let (out, calls) = run(&argv, one_rule(), |c| rules(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("--regex and --no-regex cannot both be given"));
        assert!(calls.is_empty());
    }

    #[test]
    fn rules_are_switched_under_the_name_the_engine_toggles_them_by() {
        let argv = ["rules", "enable", "r1"];
        let (out, calls) = run(&argv, one_rule(), |c| rules(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Enabled rule r1");
        assert_eq!(
            payloads(&calls, "entity.setEnabled"),
            [json!({"workspaceId": "w1", "kind": "rules", "id": "r1", "enabled": true})]
        );
        assert_eq!(payloads(&calls, "entity.load")[0]["kind"], "proxyRules");
    }

    fn mock() -> Value {
        let mut mock = new_mock("https://api.test/users");
        mock["id"] = json!("k1");
        mock["name"] = json!("Users");
        mock["workspaceId"] = json!("w1");
        mock["capturedBody"] = json!("a2VwdA==");
        mock["responseHeaders"] = json!({"Content-Type": "application/json", "X-Old": "1"});
        mock["mockedResponseHeaders"] = json!(["X-Old"]);
        mock
    }

    fn one_mock() -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            ("entity.list:mocks", listing(json!([mock()]))),
            ("entity.load:mocks", loaded(with_field(&mock(), "enabled", json!(true)))),
            ("entity.create", Ok(json!({"id": "k9", "entity": {"id": "k9"}}))),
            ("entity.update", Ok(json!({"ok": true}))),
            ("entity.setEnabled", Ok(json!({"ok": true}))),
        ]
    }

    #[test]
    fn mocks_list_is_a_table() {
        let argv = ["mocks", "list"];
        let (out, _) = run(&argv, one_mock(), |c| mocks(c, sub(&argv)));
        assert_eq!(
            out.unwrap(),
            "ID  ENABLED  METHOD  NAME   URLPATTERN              STATUS\n\
             k1  true     GET     Users  https://api.test/users  200"
        );
    }

    #[test]
    fn mocks_create_is_the_whole_response() {
        let argv = [
            "mocks",
            "create",
            "https://api.test/users",
            "-X",
            "post",
            "--status",
            "201",
            "-d",
            "{\"id\":1}",
            "-H",
            "X-Trace: abc",
            "--delay",
            "250",
        ];
        let (out, calls) = run(&argv, one_mock(), |c| mocks(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Saved mock k9");
        assert_eq!(
            payloads(&calls, "entity.create"),
            [json!({"kind": "mocks", "workspaceId": "w1", "entity": {
                "name": "POST https://api.test/users", "method": "POST", "urlPattern": "https://api.test/users",
                "useRegex": false, "standalone": true, "responseStatus": 201, "responseStatusMocked": true,
                "responseHeaders": {"X-Trace": "abc", "Content-Type": "application/json"},
                "mockedResponseHeaders": ["X-Trace"], "responseBody": "{\"id\":1}", "responseBodyMocked": true,
                "responseDelay": 250, "responseDelayMocked": true, "streamingMode": "none", "streamingChunkDelay": 0,
                "enabled": true}})]
        );
    }

    #[test]
    fn mocks_create_defaults_to_an_empty_200_and_takes_any_method_and_a_regex() {
        let argv = ["mocks", "create", "^https://api\\.test/", "--regex", "--method", "any", "--disabled"];
        let (_, calls) = run(&argv, one_mock(), |c| mocks(c, sub(&argv)));
        let entity = &payloads(&calls, "entity.create")[0]["entity"];
        assert_eq!(entity["method"], "*");
        assert_eq!(entity["useRegex"], true);
        assert_eq!(entity["enabled"], false);
        assert_eq!(entity["responseStatus"], 200);
        assert_eq!(entity["responseHeaders"], json!({}));
        assert_eq!(entity["name"], "* ^https://api\\.test/");
    }

    #[test]
    fn mocks_create_keeps_a_given_content_type_and_calls_plain_text_plain() {
        let argv = ["mocks", "create", "/a", "-d", "hello"];
        let (_, calls) = run(&argv, one_mock(), |c| mocks(c, sub(&argv)));
        assert_eq!(
            payloads(&calls, "entity.create")[0]["entity"]["responseHeaders"],
            json!({"Content-Type": "text/plain"})
        );

        let argv = ["mocks", "create", "/a", "-d", "<a/>", "-H", "content-type: text/xml"];
        let (_, calls) = run(&argv, one_mock(), |c| mocks(c, sub(&argv)));
        let entity = &payloads(&calls, "entity.create")[0]["entity"];
        assert_eq!(entity["responseHeaders"], json!({"content-type": "text/xml"}));
        assert_eq!(entity["mockedResponseHeaders"], json!([]));
    }

    #[test]
    fn mocks_create_from_a_curl_command_asks_the_engine_what_it_mocks() {
        let argv = ["mocks", "create", "--curl", "curl https://h.test/a", "--status", "404", "--name", "Gone"];
        let built = json!({"mock": {"name": "GET /a", "method": "GET", "urlPattern": "https://h.test/a",
                                    "responseStatus": 200, "responseHeaders": {}, "mockedResponseHeaders": []}});
        let mut answers = one_mock();
        answers.push(("curl.buildMock", Ok(built)));
        let (_, calls) = run(&argv, answers, |c| mocks(c, sub(&argv)));
        assert_eq!(payloads(&calls, "curl.buildMock"), [json!({"curl": "curl https://h.test/a"})]);
        let entity = &payloads(&calls, "entity.create")[0]["entity"];
        assert_eq!(entity["urlPattern"], "https://h.test/a");
        assert_eq!(entity["responseStatus"], 404);
        assert_eq!(entity["name"], "Gone");
        assert_eq!(entity["enabled"], true);

        let argv = ["mocks", "create", "/a", "--curl", "curl https://h.test/a"];
        let (out, calls) = run(&argv, one_mock(), |c| mocks(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::usage("--curl already says what the mock answers to; leave out the URL pattern")
        );
        assert!(calls.is_empty());
    }

    #[test]
    fn mocks_create_refuses_a_bad_status_delay_or_missing_pattern_before_asking_the_engine() {
        for (argv, message) in [
            (vec!["mocks", "create", "/a", "--status", "99"], "--status must be a status from 100 to 599, not \"99\""),
            (vec!["mocks", "create", "/a", "--status", "ok"], "--status must be a status from 100 to 599, not \"ok\""),
            (
                vec!["mocks", "create", "/a", "--delay", "-1"],
                "--delay must be a whole number of milliseconds, not \"-1\"",
            ),
            (vec!["mocks", "create"], "missing URL pattern, e.g. https://api.example.com/users"),
        ] {
            let (out, calls) = run(&argv, one_mock(), |c| mocks(c, sub(&argv)));
            assert_eq!(out.unwrap_err(), CliError::usage(message));
            assert!(calls.is_empty(), "{argv:?}");
        }
    }

    #[test]
    fn mocks_update_changes_what_is_given_and_keeps_the_rest() {
        let argv = [
            "mocks",
            "update",
            "users",
            "--status",
            "500",
            "-H",
            "x-old:",
            "-H",
            "X-New: 2",
            "--url",
            "https://api.test/v2",
        ];
        let (out, calls) = run(&argv, one_mock(), |c| mocks(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Updated mock k1");
        let entity = &payloads(&calls, "entity.update")[0]["entity"];
        assert_eq!(entity["responseStatus"], 500);
        assert_eq!(entity["urlPattern"], "https://api.test/v2");
        assert_eq!(entity["responseHeaders"], json!({"Content-Type": "application/json", "X-New": "2"}));
        assert_eq!(entity["mockedResponseHeaders"], json!(["X-New"]));
        assert_eq!(entity["capturedBody"], "a2VwdA==");
        assert_eq!(entity["name"], "Users");
        assert_eq!(entity["id"], "k1");
    }

    #[test]
    fn mocks_update_makes_a_data_body_text_and_refuses_a_bad_option_before_reading_the_mock() {
        let mut data = mock();
        data["responseBodyEncoding"] = json!("base64");
        let mut answers = one_mock();
        answers.insert(0, ("entity.load:mocks", loaded(data)));
        let argv = ["mocks", "update", "k1", "-d", "now text"];
        let (_, calls) = run(&argv, answers, |c| mocks(c, sub(&argv)));
        let entity = &payloads(&calls, "entity.update")[0]["entity"];
        assert_eq!(entity["responseBody"], "now text");
        assert!(entity.get("responseBodyEncoding").is_none());

        let argv = ["mocks", "update", "k1", "--status", "9000"];
        let (out, calls) = run(&argv, one_mock(), |c| mocks(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::usage("--status must be a status from 100 to 599, not \"9000\""));
        assert!(calls.is_empty());

        let argv = ["mocks", "update", "k1"];
        let (out, _) = run(&argv, one_mock(), |c| mocks(c, sub(&argv)));
        assert!(out.unwrap_err().to_string().starts_with("nothing to change: give at least one of --url, --method"));
    }

    #[test]
    fn mocks_enable_and_disable() {
        let argv = ["mocks", "disable", "Users"];
        let (out, calls) = run(&argv, one_mock(), |c| mocks(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Disabled mock k1");
        assert_eq!(
            payloads(&calls, "entity.setEnabled"),
            [json!({"workspaceId": "w1", "kind": "mocks", "id": "k1", "enabled": false})]
        );
    }
}
