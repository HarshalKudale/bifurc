//! `bifurc curl`: curl's own arguments, run through the engine (shared cookie jar, environments) with the output
//! of `requests send`. Our options (`--save`, `--select`, …) are told apart from curl's before the engine reads
//! the rest as a cURL command; `--bifurc-json` is our machine-readable output, because `--json` is curl's.

use base64::Engine as _;
use serde_json::{Map, Value, json};

use crate::{
    args::{ParsedArgs, parse_args},
    commands::{Context, created, find_one, id_of, list_entities, workspace_id},
    error::{CliError, Result},
    format::js_template,
    requests::{RequestType, TYPES, add_overrides, saved, text},
    response::{Response, present},
};

/// Our options that take a value, and the ones that do not. `workspace` and `data-dir` are the global ones.
const OURS_VALUE: [&str; 11] =
    ["save", "folder", "select", "max-body", "expect-status", "expect", "var", "env", "out", "data-dir", "workspace"];
const OURS_SWITCH: [&str; 11] =
    ["save-only", "update", "no-templatize", "no-jar", "body", "headers", "raw", "quiet", "reveal", "no-start", "help"];

/// curl's options the engine reads (each takes a value).
const CURL_VALUE: [&str; 16] = [
    "request",
    "header",
    "data",
    "data-raw",
    "data-binary",
    "data-ascii",
    "data-urlencode",
    "json",
    "form",
    "form-string",
    "user",
    "user-agent",
    "referer",
    "cookie",
    "url",
    "oauth2-bearer",
];

/// The long name of a short option; a cluster such as `-sSL` is read letter by letter.
fn short(letter: char) -> Option<&'static str> {
    Some(match letter {
        'X' => "request",
        'H' => "header",
        'd' => "data",
        'F' => "form",
        'u' => "user",
        'A' => "user-agent",
        'e' => "referer",
        'b' => "cookie",
        'm' => "max-time",
        'c' => "cookie-jar",
        'o' => "output",
        'G' => "get",
        'I' => "head",
        'i' => "include",
        's' => "silent",
        'S' => "show-error",
        'v' => "verbose",
        'f' => "fail",
        'k' => "insecure",
        'L' => "location",
        'h' => "help",
        _ => return None,
    })
}

fn takes_value(name: &str) -> bool {
    OURS_VALUE.contains(&name)
        || CURL_VALUE.contains(&name)
        || matches!(name, "max-time" | "connect-timeout" | "cookie-jar" | "output")
}

fn unsupported(option: &str) -> CliError {
    CliError::usage(format!("unsupported curl option {option} (see: bifurc curl --help)"))
}

/// The text of `@file` (`@-` is stdin), or `value` itself.
fn read_at(value: &str) -> Result<Option<String>> {
    let Some(path) = value.strip_prefix('@') else { return Ok(None) };
    let read = if path == "-" { std::io::read_to_string(std::io::stdin()) } else { std::fs::read_to_string(path) };
    read.map(Some).map_err(|e| CliError::usage(format!("cannot read {path}: {e}")))
}

/// What curl's arguments say, beyond our own options.
#[derive(Debug, Default)]
pub struct Curl {
    /// The words of the cURL command the engine reads (without `curl`).
    tokens: Vec<String>,
    /// `-k`, `-L`, `-m`: the request's `settings`.
    settings: Map<String, Value>,
    /// `-f`: a status of 400 or more fails the command.
    fail: bool,
    /// The files of `-F name=@file`, in order; the engine's part holds no content, so these are read here.
    files: Vec<String>,
}

impl Curl {
    fn apply(&mut self, ours: &mut Vec<String>, name: &str, value: Option<String>) -> Result<()> {
        let v = value.unwrap_or_default();
        match name {
            n if OURS_VALUE.contains(&n) => ours.push(format!("--{n}={v}")),
            "bifurc-json" => ours.push("--json=true".into()),
            // `=true` so that a switch `parse_args` does not know never takes the next word.
            n if OURS_SWITCH.contains(&n) => ours.push(format!("--{n}=true")),
            "include" => ours.push("--headers=true".into()),
            "head" => {
                self.tokens.push("--head".into());
                ours.push("--headers=true".into());
            }
            "output" if v == "-" => ours.push("--raw=true".into()),
            "output" => ours.push(format!("--out={v}")),
            // The engine's jar already keeps the cookies, and the rest only shape the transfer.
            "silent" | "show-error" | "verbose" | "compressed" | "connect-timeout" | "cookie-jar" => {}
            "fail" => self.fail = true,
            "insecure" => {
                self.settings.insert("verifyTls".into(), json!(false));
            }
            "location" => {
                self.settings.insert("followRedirects".into(), json!(true));
            }
            "max-time" => {
                let seconds = v.parse::<f64>().ok().filter(|s| s.is_finite() && *s >= 0.0);
                let seconds =
                    seconds.ok_or_else(|| CliError::usage(format!("--max-time must be seconds, not \"{v}\"")))?;
                self.settings.insert("timeoutMs".into(), json!((seconds * 1000.0).round() as u64));
            }
            "get" => self.tokens.push("--get".into()),
            // curl reads `@file` itself (`-d` drops its line breaks); the engine may run in another directory.
            "data" | "data-ascii" | "data-binary" | "json" => {
                let text = match read_at(&v)? {
                    Some(text) if name == "data" || name == "data-ascii" => text.replace(['\r', '\n'], ""),
                    Some(text) => text,
                    None => v,
                };
                let flag = if name == "json" { "--json" } else { "--data-raw" };
                self.tokens.extend([flag.into(), text]);
            }
            "data-urlencode" if v.find(['=', '@']).is_some_and(|at| v[at..].starts_with('@')) => {
                return Err(CliError::usage("--data-urlencode with a file (name@file) is not supported; inline it"));
            }
            "form" => {
                let content = v.split_once('=').map_or("", |(_, rest)| rest.split(';').next().unwrap_or(""));
                if content.starts_with('<') {
                    return Err(CliError::usage(
                        "-F name=<file is not supported; use -F name=@file or inline the text",
                    ));
                }
                if let Some(path) = content.strip_prefix('@') {
                    self.files.push(path.trim_matches('"').to_string());
                }
                self.tokens.extend(["--form".into(), v]);
            }
            // `-b file` reads cookies from a file; the jar holds them already.
            "cookie" if !v.contains('=') => {}
            n if CURL_VALUE.contains(&n) => self.tokens.extend([format!("--{n}"), v]),
            other => return Err(unsupported(&format!("--{other}"))),
        }
        Ok(())
    }
}

/// Where `curl` stands in a command line: the first word that is not a global option.
pub fn at(argv: &[String]) -> Option<usize> {
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--no-start" => i += 1,
            "--data-dir" | "--workspace" | "-w" => i += 2,
            a if a.starts_with("--data-dir=") || a.starts_with("--workspace=") => i += 1,
            _ => break,
        }
    }
    (argv.get(i).map(String::as_str) == Some("curl")).then_some(i)
}

/// A shell's words: quotes, backslashes, `$'…'` with the usual escapes. For a command given as one string.
pub(crate) fn shell_words(input: &str) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    let (mut words, mut word, mut open, mut i) = (Vec::new(), String::new(), false, 0);
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        match c {
            c if c.is_whitespace() => {
                if open {
                    words.push(std::mem::take(&mut word));
                    open = false;
                }
            }
            '\\' if chars.get(i) == Some(&'\n') => i += 1,
            '\\' if i < chars.len() => {
                word.push(chars[i]);
                (open, i) = (true, i + 1);
            }
            '\'' | '"' | '$' if c != '$' || chars.get(i) == Some(&'\'') => {
                open = true;
                let ansi = c == '$';
                let quote = if ansi { '\'' } else { c };
                i += usize::from(ansi);
                while i < chars.len() && chars[i] != quote {
                    let escaped = chars[i] == '\\'
                        && i + 1 < chars.len()
                        && (ansi || (quote == '"' && "\"\\$`".contains(chars[i + 1])));
                    if escaped {
                        i += 1;
                    }
                    word.push(match (escaped && ansi, chars[i]) {
                        (true, 'n') => '\n',
                        (true, 't') => '\t',
                        (true, 'r') => '\r',
                        (_, other) => other,
                    });
                    i += 1;
                }
                i += 1;
            }
            c => {
                word.push(c);
                open = true;
            }
        }
    }
    if open {
        words.push(word);
    }
    words
}

/// Our options as a parsed command line, and curl's arguments as they are.
pub fn split(argv: &[String]) -> Result<(ParsedArgs, Curl)> {
    let at = at(argv).expect("a curl command line");
    let mut items = argv.to_vec();
    items.remove(at);
    // Before `curl`, `-w` is ours (--workspace); after it, it is curl's --write-out, which is not supported.
    for word in items[..at].iter_mut().filter(|word| *word == "-w") {
        *word = "--workspace".to_string();
    }
    // `bifurc curl "curl -X POST …"`, and what the MCP tool sends after a `--`: the whole command is one word.
    let first = items.iter().position(|a| a == "--").map_or(at, |dashes| {
        items.remove(dashes);
        dashes
    });
    if items.get(first).is_some_and(|w| w.contains(char::is_whitespace) && !w.starts_with('-')) {
        let words = shell_words(&items[first]);
        items.splice(first..=first, words);
    }
    if items.get(first).is_some_and(|w| w == "curl") {
        items.remove(first);
    }

    let (mut call, mut ours, mut i) = (Curl::default(), vec!["curl".to_string()], 0);
    while i < items.len() {
        let item = items[i].as_str();
        i += 1;
        let mut found: Vec<(String, Option<String>)> = Vec::new();
        if let Some(long) = item.strip_prefix("--").filter(|long| !long.is_empty()) {
            match long.split_once('=') {
                Some((name, value)) if takes_value(name) => found.push((name.into(), Some(value.into()))),
                _ => found.push((long.into(), None)),
            }
        } else if item.len() > 1 && item.starts_with('-') {
            let letters: Vec<char> = item[1..].chars().collect();
            for (k, letter) in letters.iter().enumerate() {
                let name = short(*letter).ok_or_else(|| unsupported(&format!("-{letter}")))?;
                if takes_value(name) {
                    let rest: String = letters[k + 1..].iter().collect();
                    found.push((name.into(), (!rest.is_empty()).then_some(rest)));
                    break;
                }
                found.push((name.into(), None));
            }
        } else {
            call.tokens.push(item.to_string());
            continue;
        }
        for (name, inline) in found {
            let name = if name == "bifurc-json" { name } else { name.trim_start_matches('-').to_string() };
            let value = match (takes_value(&name), inline) {
                (false, _) => None,
                (true, Some(value)) => Some(value),
                (true, None) => {
                    i += 1;
                    Some(items.get(i - 1).cloned().ok_or_else(|| CliError::usage(format!("--{name} needs a value")))?)
                }
            };
            call.apply(&mut ours, &name, value)?;
        }
    }
    let args = parse_args(&ours).map_err(CliError::usage)?;
    Ok((args, call))
}

/// `value` as one word of a POSIX shell: single quotes (`'` as `'\''`), or `$'…'` when it holds a control
/// character, so the command stays on one line.
pub(crate) fn quote(value: &str) -> String {
    if !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c)) {
        return value.to_string();
    }
    if !value.chars().any(char::is_control) {
        return format!("'{}'", value.replace('\'', "'\\''"));
    }
    let mut out = String::from("$'");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out + "'"
}

/// The cURL command the engine reads.
fn command(tokens: &[String]) -> String {
    let words: Vec<String> = tokens.iter().map(|t| quote(t)).collect();
    format!("curl {}", words.join(" "))
}

/// The variables of the environment a send runs in (`--env`, else the selected one), as name and value.
fn variables(ctx: &mut Context, workspace: &str) -> Result<Vec<(String, String)>> {
    let merged = match ctx.args.flag("env") {
        None => vec![ctx.request("env.getMerged", json!({ "workspaceId": workspace }))?],
        Some(wanted) => {
            let all = list_entities(ctx, workspace, "environments")?;
            let chosen = find_one(&all, wanted, &["name"], "environment", "env")?.clone();
            all.into_iter()
                .filter(|e| e.get("id").and_then(Value::as_str) == Some("__global__"))
                .chain([chosen])
                .collect()
        }
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for variable in merged.iter().flat_map(|env| env.get("variables").and_then(Value::as_array)).flatten() {
        if let (Some(key), Some(value)) =
            (variable.get("key").and_then(Value::as_str), variable.get("value").and_then(Value::as_str))
            && !value.is_empty()
        {
            out.retain(|(name, _)| name != key);
            out.push((key.to_string(), value.to_string()));
        }
    }
    Ok(out)
}

/// A header that holds a credential, and the variable it is saved as when no variable holds its value.
fn credential(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let sensitive = ["authorization", "cookie", "proxy-authorization", "x-api-key"].contains(&lower.as_str())
        || ["token", "secret", "api-key", "apikey"].iter().any(|word| lower.contains(word));
    let slug: String = lower.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    sensitive.then(|| if lower == "authorization" { "token".to_string() } else { slug })
}

/// `word` as `{{variable}}` when a variable of 6 characters or more holds exactly it.
fn known(word: &str, vars: &[(String, String)], used: &mut Vec<String>) -> Option<String> {
    let (name, _) = vars.iter().find(|(_, value)| value.chars().count() >= 6 && value == word)?;
    note(used, name);
    Some(format!("{{{{{name}}}}}"))
}

fn note(used: &mut Vec<String>, name: &str) {
    if !used.iter().any(|u| u == name) {
        used.push(name.to_string());
    }
}

/// Put `{{variables}}` of the environment where the request holds their values: the base URL, the credentials
/// (a `{{token}}` placeholder when no variable holds them) and other values equal to a variable's. Returns the
/// variables used and the warnings to print.
fn templatize(entity: &mut Value, ty: &RequestType, vars: &[(String, String)]) -> (Vec<String>, Vec<String>) {
    let (mut used, mut warnings) = (Vec::new(), Vec::new());
    let url = text(entity, ty.url_key);
    let base = vars
        .iter()
        .filter(|(_, value)| value.starts_with("http://") || value.starts_with("https://"))
        .map(|(name, value)| (name, value.trim_end_matches('/')))
        .filter(|(_, base)| {
            url.strip_prefix(base).is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#']))
        })
        .max_by_key(|(_, base)| base.len());
    let (mut out, rest) = match base {
        Some((name, base)) => {
            note(&mut used, name);
            (format!("{{{{{name}}}}}"), &url[base.len()..])
        }
        None => (String::new(), url.as_str()),
    };
    // Path and query segments.
    let mut word = String::new();
    for c in rest.chars().chain(['/']) {
        if "/?&=#".contains(c) {
            out.push_str(&known(&word, vars, &mut used).unwrap_or_else(|| word.clone()));
            word.clear();
            out.push(c);
        } else {
            word.push(c);
        }
    }
    out.pop();
    entity[ty.url_key] = json!(out);

    if let Some(headers) = entity.get_mut(ty.headers_key).and_then(Value::as_object_mut) {
        for (name, value) in headers.iter_mut() {
            let Some(current) = value.as_str().filter(|v| !v.is_empty() && !v.contains("{{")) else { continue };
            let new = match credential(name) {
                None => known(current, vars, &mut used),
                Some(placeholder) => {
                    let (scheme, secret) = ["Bearer ", "Basic "]
                        .iter()
                        .find_map(|scheme| current.strip_prefix(scheme).map(|secret| (*scheme, secret)))
                        .unwrap_or(("", current));
                    Some(match vars.iter().find(|(_, v)| v == current || v == secret) {
                        Some((var, v)) => {
                            note(&mut used, var);
                            format!("{}{{{{{var}}}}}", if v == current { "" } else { scheme })
                        }
                        None => {
                            note(&mut used, &placeholder);
                            warnings.push(format!(
                                "warning: {name} saved as {{{{{placeholder}}}}} — set it: bifurc env set <env> {placeholder}=<value>"
                            ));
                            format!("{scheme}{{{{{placeholder}}}}}")
                        }
                    })
                }
            };
            if let Some(new) = new {
                *value = json!(new);
            }
        }
    }

    // String values of a JSON body, replaced in the text so its layout stays.
    let field = if ty.name == "graphql" { "variables" } else { "body" };
    let mut body = text(entity, field);
    if serde_json::from_str::<Value>(&body).is_ok() {
        for (name, value) in vars.iter().filter(|(_, value)| value.chars().count() >= 6) {
            let quoted = json!(value).to_string();
            if body.contains(&quoted) {
                body = body.replace(&quoted, &format!("\"{{{{{name}}}}}\""));
                note(&mut used, name);
            }
        }
        entity[field] = json!(body);
    }
    (used, warnings)
}

/// The id of the folder `path` names in the request tree, creating the folders that are missing.
fn folder_id(ctx: &mut Context, workspace: &str, parts: &[&str]) -> Result<String> {
    if ctx.args.flag("workspace").is_some() {
        return Err(CliError::msg("--folder uses the active workspace: run bifurc workspaces use <workspace>"));
    }
    let config = ctx.request("config.get", json!({}))?;
    let folders = config.get("requestFolders").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut parent: Option<String> = None;
    for part in parts {
        let hit = folders.iter().find(|f| {
            text(f, "name") == *part
                && f.get("parentId").and_then(Value::as_str) == parent.as_deref()
                && f.get("deleted").and_then(Value::as_bool) != Some(true)
        });
        parent = Some(match hit {
            Some(folder) => id_of(folder),
            None => {
                let made = json!({ "kind": "request", "name": part, "parentId": parent, "workspaceId": workspace });
                id_of(&ctx.request("folder.add", made)?)
            }
        });
    }
    Ok(parent.expect("a folder path has a part"))
}

/// Save the request as `name` (updating the one of that name with `--update`); returns its id and the line
/// that says so.
fn save(
    ctx: &mut Context,
    workspace: &str,
    ty: &RequestType,
    entity: &mut Value,
    name: &str,
) -> Result<(String, String)> {
    let args = ctx.args;
    let all: Vec<&RequestType> = TYPES.iter().collect();
    let existing = saved(ctx, workspace, &all)?
        .into_iter()
        .find(|(_, found)| text(found, "name").to_lowercase() == name.to_lowercase());
    if let Some((found_ty, _)) = &existing {
        if !args.has("update") {
            return Err(CliError::usage(format!("request \"{name}\" exists — add --update to overwrite")));
        }
        if found_ty.kind != ty.kind {
            return Err(CliError::usage(format!(
                "request \"{name}\" is a {} request, not {}; delete it first",
                found_ty.name, ty.name
            )));
        }
    }
    let (used, warnings) = if args.has("no-templatize") {
        (Vec::new(), Vec::new())
    } else {
        let vars = variables(ctx, workspace)?;
        templatize(entity, ty, &vars)
    };
    let parts: Vec<&str> =
        args.flag("folder").map(|f| f.split('/').filter(|p| !p.is_empty()).collect()).unwrap_or_default();
    if args.has("folder") && parts.is_empty() {
        return Err(CliError::usage("--folder needs a folder name"));
    }
    let folder = if parts.is_empty() { None } else { Some(folder_id(ctx, workspace, &parts)?) };
    let id = match existing {
        Some((_, mut found)) => {
            // What the command describes replaces the request's; its id, folder, scripts and tests stay.
            for (key, value) in entity.as_object().into_iter().flatten().filter(|(key, _)| *key != "name") {
                found[key] = value.clone();
            }
            if entity.get("bodyParts").is_none() {
                found.as_object_mut().map(|fields| fields.remove("bodyParts"));
            }
            if let Some(folder) = &folder {
                found["folderId"] = json!(folder);
            }
            found["workspaceId"] = json!(workspace);
            ctx.request("entity.update", json!({ "kind": ty.kind, "entity": found }))?;
            id_of(&found)
        }
        None => {
            entity["name"] = json!(name);
            if let Some(folder) = &folder {
                entity["folderId"] = json!(folder);
            }
            let made =
                ctx.request("entity.create", json!({ "kind": ty.kind, "workspaceId": workspace, "entity": entity }))?;
            id_of(&created(&made))
        }
    };
    for warning in warnings {
        eprintln!("{warning}");
    }
    let place = if parts.is_empty() { String::new() } else { format!(" ({}/)", parts.join("/")) };
    let templated = if used.is_empty() { String::new() } else { format!(" · {} templatized", used.join(", ")) };
    Ok((id, format!("saved \"{name}\"{place}{templated}")))
}

/// Run the curl command: build the request, save it if asked, send it and print the response.
pub fn run(ctx: &mut Context, call: &Curl) -> Result<String> {
    let args = ctx.args;
    let name = args.flag("save");
    if let Some(flag) =
        ["save-only", "update", "folder", "no-templatize"].iter().find(|f| name.is_none() && args.has(f))
    {
        return Err(CliError::usage(format!("--{flag} needs --save <name>")));
    }
    let workspace = workspace_id(ctx)?;
    let built = ctx.request("curl.buildRequest", json!({ "curl": command(&call.tokens) })).map_err(|e| match e {
        CliError::Engine { message, .. } => CliError::usage(message),
        other => other,
    })?;
    let kind = js_template(built.get("kind"));
    let ty = TYPES
        .iter()
        .find(|ty| ty.kind == kind && ty.send.is_some())
        .ok_or_else(|| CliError::usage("use bifurc requests send for streaming requests"))?;
    let send = ty.send.expect("checked above");
    if args.has("no-jar") && ty.name != "rest" {
        return Err(CliError::usage(format!("--no-jar applies to rest requests, not {}", ty.name)));
    }
    let mut entity = built.get("request").cloned().unwrap_or(Value::Null);
    if !call.settings.is_empty() {
        entity["settings"] = Value::Object(call.settings.clone());
    }
    let mut files = call.files.iter();
    for part in entity.get_mut("bodyParts").and_then(Value::as_array_mut).into_iter().flatten() {
        if part.get("kind").and_then(Value::as_str) == Some("file")
            && let Some(path) = files.next()
        {
            let bytes = std::fs::read(path).map_err(|e| CliError::usage(format!("cannot read {path}: {e}")))?;
            part["value"] = json!(base64::engine::general_purpose::STANDARD.encode(bytes));
        }
    }

    let (id, note) = match name {
        Some(name) => {
            let (id, note) = save(ctx, &workspace, ty, &mut entity, name)?;
            (Some(id), Some(note))
        }
        None => (None, None),
    };
    if args.has("save-only") {
        let note = note.unwrap_or_default();
        return Ok(ctx.print(&json!({ "id": id, "name": name }), || note));
    }
    let mut payload = match &id {
        Some(id) => json!({ "workspaceId": workspace, "id": id }),
        None => {
            let mut request: Map<String, Value> = ty
                .inline
                .iter()
                .chain(["bodyParts", "settings"].iter())
                .filter_map(|key| entity.get(*key).map(|value| (key.to_string(), value.clone())))
                .collect();
            request.entry("headers").or_insert_with(|| json!({}));
            let mut payload = json!({ "workspaceId": workspace, "request": request });
            if ty.name == "rest" {
                payload["scripts"] = json!({});
            }
            payload
        }
    };
    if args.has("no-jar") {
        payload["jar"] = json!(false);
    }
    add_overrides(ctx, ty, &workspace, None, &mut payload)?;
    let answer = ctx.request(send, payload)?;
    let response = if ty.name == "rest" { Response::from_rest(&answer) } else { Response::from_text(&answer) };
    let mut shown = present(ctx, &response);
    if call.fail && response.status >= 400 {
        shown = match shown {
            Ok(out) => Err(CliError::Failed(out)),
            other => other,
        };
    }
    // The line about the save comes first, unless the output is all there is to see.
    let plain = !(ctx.json || args.has("quiet") || args.has("raw"));
    match (note, shown) {
        (Some(note), Ok(out)) if plain => Ok(format!("{note}\n{out}")),
        (Some(note), Err(CliError::Failed(out))) if plain => Err(CliError::Failed(format!("{note}\n{out}"))),
        (_, shown) => shown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, payloads};

    fn words(argv: &[&str]) -> Vec<String> {
        std::iter::once("curl").chain(argv.iter().copied()).map(str::to_string).collect()
    }

    fn splitting(argv: &[&str]) -> Result<(ParsedArgs, Curl)> {
        split(&words(argv))
    }

    fn tokens(argv: &[&str]) -> Vec<String> {
        splitting(argv).unwrap().1.tokens
    }

    #[test]
    fn our_options_are_told_apart_from_curls() {
        let (args, call) = splitting(&[
            "-X",
            "POST",
            "https://h/u",
            "-H",
            "A: b",
            "--save",
            "Create user",
            "--folder",
            "auth",
            "--select",
            ".id",
            "--body",
            "--no-jar",
            "--data-dir",
            "/d",
            "--expect",
            "status == 201",
            "--expect",
            ".id != null",
        ])
        .unwrap();
        assert_eq!(call.tokens, ["--request", "POST", "https://h/u", "--header", "A: b"]);
        assert_eq!(args.flag("save"), Some("Create user"));
        assert_eq!(args.flag("folder"), Some("auth"));
        assert_eq!(args.flag("select"), Some(".id"));
        assert_eq!(args.flags("expect"), ["status == 201", ".id != null"]);
        assert!(args.has("body") && args.has("no-jar") && !args.has("data"));
        assert_eq!(args.flag("data-dir"), Some("/d"));
    }

    #[test]
    fn json_is_curls_and_bifurc_json_is_ours() {
        let (args, call) = splitting(&["--json", "{\"a\":1}", "https://h/u", "--bifurc-json"]).unwrap();
        assert_eq!(call.tokens, ["--json", "{\"a\":1}", "https://h/u"]);
        assert!(args.has("json"));
        let (args, _) = splitting(&["--json={\"a\":1}", "https://h/u"]).unwrap();
        assert!(!args.has("json"));
    }

    #[test]
    fn short_options_cluster_and_carry_their_value() {
        let (args, call) = splitting(&["-sSL", "-XPOST", "-H", "A: b", "-iko", "out.bin", "https://h/u"]).unwrap();
        assert_eq!(call.tokens, ["--request", "POST", "--header", "A: b", "https://h/u"]);
        assert_eq!(call.settings["followRedirects"], true);
        assert_eq!(call.settings["verifyTls"], false);
        assert!(args.has("headers"));
        assert_eq!(args.flag("out"), Some("out.bin"));
    }

    #[test]
    fn a_leading_curl_and_a_whole_command_string_are_read() {
        let expected = ["--request", "POST", "https://h/u", "--data-raw", "a b'c"];
        assert_eq!(tokens(&["curl", "-X", "POST", "https://h/u", "-d", "a b'c"]), expected);
        assert_eq!(tokens(&["curl -X POST https://h/u -d 'a b'\\''c'"]), expected);
        let (args, call) =
            split(&words(&["--save", "S", "--", "curl -s -X POST \"https://h/u\" -d $'a b\\'c'"])).unwrap();
        assert_eq!(call.tokens, expected);
        assert_eq!(args.flag("save"), Some("S"));
        // Global options may come before `curl`.
        let argv: Vec<String> =
            ["--no-start", "-w", "w2", "curl", "https://h/"].iter().map(|a| a.to_string()).collect();
        let (args, _) = split(&argv).unwrap();
        assert_eq!((args.flag("workspace"), args.has("no-start")), (Some("w2"), true));
    }

    #[test]
    fn what_changes_the_request_is_never_dropped() {
        for (flag, shown) in [("--proxy", "--proxy"), ("-x", "-x"), ("-w", "-w"), ("--retry", "--retry"), ("-1", "-1")]
        {
            let err = splitting(&[flag, "v", "https://h/"]).unwrap_err();
            assert_eq!(err.code(), 2);
            assert_eq!(err.to_string(), format!("unsupported curl option {shown} (see: bifurc curl --help)"));
        }
        assert_eq!(splitting(&["-sx", "https://h/"]).unwrap_err().code(), 2);
    }

    #[test]
    fn transfer_options_are_accepted_and_the_limits_become_settings() {
        let (_, call) = splitting(&[
            "-s",
            "--compressed",
            "-v",
            "--connect-timeout",
            "5",
            "-c",
            "jar.txt",
            "-b",
            "jar.txt",
            "-m",
            "1.5",
            "https://h/",
        ])
        .unwrap();
        assert_eq!(call.tokens, ["https://h/"]);
        assert_eq!(call.settings["timeoutMs"], 1500);
        // `-b` with a cookie is a cookie.
        assert_eq!(tokens(&["-b", "a=1", "https://h/"]), ["--cookie", "a=1", "https://h/"]);
    }

    #[test]
    fn head_and_include_show_the_headers_and_fail_is_noted() {
        let (args, call) = splitting(&["-I", "-f", "https://h/"]).unwrap();
        assert_eq!(call.tokens, ["--head", "https://h/"]);
        assert!(args.has("headers") && call.fail);
        assert!(splitting(&["-i", "https://h/"]).unwrap().0.has("headers"));
        assert!(splitting(&["-o", "-", "https://h/"]).unwrap().0.has("raw"));
    }

    #[test]
    fn data_at_file_is_read_by_the_cli() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("body.json");
        std::fs::write(&path, "{\n  \"a\": 1\n}\n").unwrap();
        let at = format!("@{}", path.display());
        assert_eq!(tokens(&["-d", &at, "https://h/"]), ["--data-raw", "{  \"a\": 1}", "https://h/"]);
        assert_eq!(tokens(&["--data-binary", &at, "https://h/"])[1], "{\n  \"a\": 1\n}\n");
        assert_eq!(tokens(&["--json", &at, "https://h/"])[..2], ["--json", "{\n  \"a\": 1\n}\n"]);
        assert_eq!(tokens(&["--data-raw", "@x", "https://h/"])[1], "@x");
        let missing = splitting(&["-d", "@/no/such/file", "https://h/"]).unwrap_err();
        assert_eq!(missing.code(), 2);
        let form = splitting(&["-F", "f=@a.txt;type=text/plain", "-F", "t=x", "https://h/"]).unwrap().1;
        assert_eq!(form.files, ["a.txt"]);
        assert_eq!(splitting(&["-F", "f=<a.txt", "https://h/"]).unwrap_err().code(), 2);
    }

    /// Each pair is a word and what the engine's tokenizer reads back as exactly that word (the engine has the
    /// same pairs in `utils::curl` tests).
    const QUOTED: [(&str, &str); 8] = [
        ("plain-word_1", "plain-word_1"),
        ("", "''"),
        ("a b", "'a b'"),
        ("it's", "'it'\\''s'"),
        ("$HOME `x` \"y\" \\n", "'$HOME `x` \"y\" \\n'"),
        ("héllo ✓", "'héllo ✓'"),
        ("a^\nb\t'c'\\", "$'a^\\nb\\t\\'c\\'\\\\'"),
        ("-d", "-d"),
    ];

    #[test]
    fn words_are_quoted_for_a_shell() {
        for (word, quoted) in QUOTED {
            assert_eq!(quote(word), quoted);
            assert_eq!(shell_words(quoted), [word], "{quoted}");
        }
        assert_eq!(command(&["--url".to_string(), "http://h/a b".to_string()]), "curl --url 'http://h/a b'");
    }

    fn built(url: &str, headers: Value, body: &str) -> Value {
        json!({ "kind": "requests", "request": {
            "name": "POST /users", "protocol": "rest", "method": "POST", "url": url, "headers": headers, "body": body } })
    }

    fn environment(variables: &[(&str, &str)]) -> Value {
        let list: Vec<Value> = variables.iter().map(|(k, v)| json!({ "id": k, "key": k, "value": v })).collect();
        json!({ "id": "merged", "variables": list })
    }

    fn answers(url: &str, headers: Value, body: &str, vars: &[(&str, &str)], existing: Value) -> Vec<Answer> {
        vec![
            (
                "config.get",
                Ok(json!({ "activeWorkspaceId": "w1", "requestFolders": [
                {"id": "f1", "name": "auth", "parentId": null}, {"id": "f9", "name": "admin", "parentId": "other"}] })),
            ),
            ("curl.buildRequest", Ok(built(url, headers, body))),
            ("env.getMerged", Ok(environment(vars))),
            ("entity.list", Ok(json!({ "entities": existing }))),
            ("entity.create", Ok(json!({ "entity": { "id": "new1" } }))),
            ("entity.update", Ok(json!({ "ok": true }))),
            ("folder.add", Ok(json!({ "id": "f2" }))),
            (
                "request.send",
                Ok(json!({ "status": 201, "headers": { "content-type": "application/json" },
                "body": "e30", "durationMs": 12, "tests": [] })),
            ),
        ]
    }

    fn go(argv: &[&str], answers: Vec<Answer>) -> (Result<String>, Vec<(String, Value)>) {
        let (args, call) = splitting(argv).unwrap();
        crate::testing::run_parsed(&args, answers, |ctx| run(ctx, &call))
    }

    #[test]
    fn a_plain_curl_is_sent_inline_with_the_summary_line() {
        let (done, calls) = go(
            &["-X", "POST", "https://api/users", "-k", "-m", "2", "--no-jar", "--var", "a=1"],
            answers("https://api/users", json!({}), "", &[], json!([])),
        );
        assert_eq!(done.unwrap(), "201 Created · 12ms · 2B json");
        let build = &payloads(&calls, "curl.buildRequest")[0];
        assert_eq!(build["curl"], "curl --request POST https://api/users");
        let send = &payloads(&calls, "request.send")[0];
        assert_eq!(send["request"]["method"], "POST");
        assert_eq!(send["request"]["settings"], json!({ "verifyTls": false, "timeoutMs": 2000 }));
        assert_eq!((send["jar"].clone(), send["variables"].clone()), (json!(false), json!({ "a": "1" })));
        assert_eq!(send["scripts"], json!({}));
        assert!(payloads(&calls, "entity.create").is_empty());
    }

    #[test]
    fn websockets_and_streams_are_for_requests_send() {
        let mut engine = answers("ws://h/", json!({}), "", &[], json!([]));
        engine[1] = ("curl.buildRequest", Ok(json!({ "kind": "wsConnections", "request": { "url": "ws://h/" } })));
        let (done, _) = go(&["ws://h/"], engine);
        let err = done.unwrap_err();
        assert_eq!((err.code(), err.to_string()), (2, "use bifurc requests send for streaming requests".to_string()));
    }

    #[test]
    fn save_creates_the_request_templatized_in_a_folder_and_sends_it_by_id() {
        let vars = [("baseUrl", "https://api.localhost"), ("email", "agent@example.co"), ("admin", "s3cr3t-value")];
        let (done, calls) = go(
            &[
                "-X",
                "POST",
                "https://api.localhost/users?by=agent@example.co",
                "--save",
                "Create user",
                "--folder",
                "auth/admin",
            ],
            answers(
                "https://api.localhost/users?by=agent@example.co",
                json!({ "Content-Type": "application/json", "X-Api-Key": "s3cr3t-value", "X-Who": "agent@example.co" }),
                "{\"email\": \"agent@example.co\", \"n\": 1}",
                &vars,
                json!([]),
            ),
        );
        assert_eq!(
            done.unwrap(),
            "saved \"Create user\" (auth/admin/) · baseUrl, email, admin templatized\n201 Created · 12ms · 2B json"
        );
        assert_eq!(payloads(&calls, "folder.add")[0]["parentId"], "f1");
        let create = &payloads(&calls, "entity.create")[0];
        assert_eq!(create["kind"], "requests");
        let saved = &create["entity"];
        assert_eq!(saved["name"], "Create user");
        assert_eq!(saved["folderId"], "f2");
        assert_eq!(saved["url"], "{{baseUrl}}/users?by={{email}}");
        assert_eq!(saved["headers"]["X-Api-Key"], "{{admin}}");
        assert_eq!(saved["headers"]["X-Who"], "{{email}}");
        assert_eq!(saved["headers"]["Content-Type"], "application/json");
        assert_eq!(saved["body"], "{\"email\": \"{{email}}\", \"n\": 1}");
        assert_eq!(payloads(&calls, "request.send")[0], json!({ "workspaceId": "w1", "id": "new1" }));
    }

    #[test]
    fn a_name_that_exists_needs_update_which_keeps_the_rest() {
        let existing = json!([{ "id": "r7", "name": "create USER", "folderId": "f1", "testScript": "// kept", "method": "GET",
            "url": "old", "headers": {}, "body": "" }]);
        let fresh = || answers("https://h/u", json!({}), "", &[], existing.clone());
        let (done, calls) = go(&["https://h/u", "--save", "Create user"], fresh());
        let err = done.unwrap_err();
        assert_eq!(
            (err.code(), err.to_string()),
            (2, "request \"Create user\" exists — add --update to overwrite".to_string())
        );
        assert!(payloads(&calls, "entity.update").is_empty());

        let (done, calls) = go(&["https://h/u", "--save", "Create user", "--update", "--save-only"], fresh());
        assert_eq!(done.unwrap(), "saved \"Create user\"");
        let update = &payloads(&calls, "entity.update")[0]["entity"];
        assert_eq!((update["id"].clone(), update["folderId"].clone()), (json!("r7"), json!("f1")));
        assert_eq!((update["url"].clone(), update["testScript"].clone()), (json!("https://h/u"), json!("// kept")));
        assert_eq!(update["name"], "create USER");
        assert!(payloads(&calls, "request.send").is_empty());
    }

    #[test]
    fn save_flags_need_save_and_no_templatize_saves_literally() {
        let (done, _) = go(&["https://h/u", "--update"], answers("https://h/u", json!({}), "", &[], json!([])));
        assert_eq!(done.unwrap_err().to_string(), "--update needs --save <name>");
        let vars = [("baseUrl", "https://h")];
        let (done, calls) = go(
            &["https://h/u", "--save", "S", "--no-templatize", "--save-only"],
            answers("https://h/u", json!({ "Authorization": "Bearer abc" }), "", &vars, json!([])),
        );
        assert_eq!(done.unwrap(), "saved \"S\"");
        let saved = &payloads(&calls, "entity.create")[0]["entity"];
        assert_eq!(
            (saved["url"].clone(), saved["headers"]["Authorization"].clone()),
            (json!("https://h/u"), json!("Bearer abc"))
        );
    }

    fn rest_type() -> &'static RequestType {
        TYPES.iter().find(|ty| ty.name == "rest").unwrap()
    }

    fn pairs(vars: &[(&str, &str)]) -> Vec<(String, String)> {
        vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn the_longest_base_url_wins_and_must_end_at_a_segment() {
        let vars = pairs(&[("host", "http://h"), ("api", "http://h/api/"), ("other", "http://h/ap")]);
        let mut entity = json!({ "url": "http://h/api/users", "headers": {}, "body": "" });
        let (used, _) = templatize(&mut entity, rest_type(), &vars);
        assert_eq!((entity["url"].clone(), used), (json!("{{api}}/users"), vec!["api".to_string()]));
        let mut entity = json!({ "url": "http://hx/users", "headers": {}, "body": "" });
        assert!(templatize(&mut entity, rest_type(), &vars).0.is_empty());
    }

    #[test]
    fn credentials_become_variables_or_a_token_placeholder_with_a_warning() {
        let vars = pairs(&[("jwt", "abc.def.ghi"), ("short", "abc")]);
        let mut entity = json!({ "url": "http://h/", "body": "", "headers": {
            "Authorization": "Bearer abc.def.ghi", "Cookie": "sid=1", "X-Auth-Token": "abc.def.ghi", "Accept": "abc" } });
        let (used, warnings) = templatize(&mut entity, rest_type(), &vars);
        assert_eq!(entity["headers"]["Authorization"], "Bearer {{jwt}}");
        assert_eq!(entity["headers"]["Cookie"], "{{cookie}}");
        assert_eq!(entity["headers"]["X-Auth-Token"], "{{jwt}}");
        assert_eq!(entity["headers"]["Accept"], "abc");
        assert_eq!(used, ["jwt", "cookie"]);
        assert_eq!(warnings, ["warning: Cookie saved as {{cookie}} — set it: bifurc env set <env> cookie=<value>"]);

        let mut entity = json!({ "url": "http://h/", "body": "", "headers": { "Authorization": "Bearer zzz" } });
        let (used, warnings) = templatize(&mut entity, rest_type(), &[]);
        assert_eq!(entity["headers"]["Authorization"], "Bearer {{token}}");
        assert_eq!(used, ["token"]);
        assert_eq!(
            warnings,
            ["warning: Authorization saved as {{token}} — set it: bifurc env set <env> token=<value>"]
        );
    }

    #[test]
    fn other_values_need_six_characters_and_an_exact_match() {
        let vars = pairs(&[("id", "abc123"), ("tiny", "ab"), ("who", "Ada Lovelace")]);
        let mut entity = json!({ "url": "http://h/orders/abc123/ab?x=abc1234", "headers": { "X-Id": "abc123" }, "body":
            "{\"a\": \"abc123\", \"b\": [\"Ada Lovelace\"], \"c\": \"xabc123\", \"d\": 5}" });
        let (used, _) = templatize(&mut entity, rest_type(), &vars);
        assert_eq!(entity["url"], "http://h/orders/{{id}}/ab?x=abc1234");
        assert_eq!(entity["headers"]["X-Id"], "{{id}}");
        assert_eq!(entity["body"], "{\"a\": \"{{id}}\", \"b\": [\"{{who}}\"], \"c\": \"xabc123\", \"d\": 5}");
        assert_eq!(used, ["id", "who"]);
        // A body that is not JSON is left alone.
        let mut entity = json!({ "url": "http://h/", "headers": {}, "body": "id=abc123" });
        templatize(&mut entity, rest_type(), &vars);
        assert_eq!(entity["body"], "id=abc123");
    }

    #[test]
    fn dash_f_fails_on_an_error_status_and_the_options_shape_the_output() {
        let mut engine = answers("https://h/u", json!({}), "", &[], json!([]));
        engine[7] =
            ("request.send", Ok(json!({ "status": 404, "headers": {}, "body": "", "durationMs": 1, "tests": [] })));
        let (done, _) = go(&["-f", "https://h/u"], engine.clone());
        assert_eq!(done.unwrap_err(), CliError::Failed("404 Not Found · 1ms · 0B".to_string()));
        let (done, _) = go(&["https://h/u"], engine);
        assert_eq!(done.unwrap(), "404 Not Found · 1ms · 0B");
    }
}
