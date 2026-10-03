//! Environment imports: Bifurc JSON, a Postman environment and dotenv.

use serde_json::{Value, json};

use super::lists::{ListSpec, preflight_list, run_list};
use crate::{
    importexport::{
        support::{commit_config, io_failure, js_str, js_trim, parse_json, present, push_to, thrown},
        types::{ImportOutcome, ImportSource, Imported, Preflight, PreflightOutcome, Strategy},
    },
    utils::{
        id::{generate_id, now_ms},
        json::{array_of, is_truthy, opt_str},
    },
    workspace::{config::load_config, fs::write_flat_entity},
};

const JSON_SPEC: ListSpec = ListSpec {
    schema: "lp-environments-v1",
    list_key: "environments",
    cfg_key: "environments",
    fs_kind: "environments",
    tracks_enabled: false,
};

pub fn json_preflight(ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    preflight_list(&JSON_SPEC, ws_id, source)
}

pub async fn json_run(ws_id: String, source: ImportSource, strategy: Strategy) -> ImportOutcome {
    run_list(&JSON_SPEC, ws_id, source, strategy)
}

/// Store a new environment named `name` holding `variables`.
fn add_environment(cfg: &mut Value, ws_id: &str, name: &str, variables: Vec<Value>) -> Result<(), String> {
    let id = generate_id();
    let env = json!({ "id": id, "name": name, "variables": variables, "createdAt": now_ms(), "workspaceId": ws_id });
    push_to(cfg, "environments", env.clone());
    write_flat_entity(ws_id, "environments", &id, &env).map_err(io_failure)
}

// ── Postman ────────────────────────────────────────────────────────────────────────────────────────────

fn parse_postman_file(content: &str) -> Result<Vec<Value>, String> {
    let data = parse_json(content)?;
    if opt_str(&data, "_postman_variable_scope") == Some("environment") || data.get("values").is_some_and(is_truthy) {
        return Ok(vec![data]);
    }
    if opt_str(&data, "schema") == Some("lp-postman-environments-v1")
        && data.get("environments").is_some_and(Value::is_array)
    {
        return Ok(array_of(&data, "environments").to_vec());
    }
    Err(thrown("Not a valid Postman environment file"))
}

pub fn postman_preflight(_ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    Ok(Preflight::counted(parse_postman_file(&source.content)?.len()))
}

pub async fn postman_run(ws_id: String, source: ImportSource, _strategy: Strategy) -> ImportOutcome {
    let parsed = parse_postman_file(&source.content)?;
    let mut cfg = load_config();
    let mut imported = 0;
    for environment in &parsed {
        let variables = array_of(environment, "values")
            .iter()
            .filter(|v| v.get("enabled") != Some(&Value::Bool(false)) && v.get("key").is_some_and(is_truthy))
            .map(|v| {
                json!({
                    "id": generate_id(),
                    "key": v["key"],
                    "value": present(v, "value").cloned().unwrap_or(json!("")),
                })
            })
            .collect();
        let name = present(environment, "name").map(js_str).unwrap_or_else(|| "Imported Environment".to_string());
        add_environment(&mut cfg, &ws_id, &name, variables)?;
        imported += 1;
    }
    commit_config(&cfg)?;
    Ok(Imported::all(imported))
}

// ── dotenv ─────────────────────────────────────────────────────────────────────────────────────────────

/// `KEY=value` lines; `#` comments and lines without `=` are skipped, matching quotes around a value are dropped.
fn parse_dotenv(text: &str) -> Vec<(String, String)> {
    let mut vars: Vec<(String, String)> = Vec::new();
    for line in text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)) {
        let trimmed = js_trim(line);
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(eq) = trimmed.find('=') else { continue };
        let key = js_trim(&trimmed[..eq]);
        let mut value = js_trim(&trimmed[eq + 1..]).to_string();
        let quoted = |q: char| value.starts_with(q) && value.ends_with(q);
        if quoted('"') || quoted('\'') {
            let chars: Vec<char> = value.chars().collect();
            let inner: String =
                if chars.len() >= 2 { chars[1..chars.len() - 1].iter().collect() } else { String::new() };
            value = inner.replace("\\\"", "\"").replace("\\'", "'");
        }
        if key.is_empty() {
            continue;
        }
        match vars.iter_mut().find(|(k, _)| k == key) {
            Some(existing) => existing.1 = value,
            None => vars.push((key.to_string(), value)),
        }
    }
    vars
}

/// The environment's name from the file's: everything from the first `.env` on is dropped.
fn dotenv_env_name(filename: &str) -> String {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let stripped = match base.find(".env") {
        Some(at) => &base[..at],
        None => base,
    };
    if stripped.is_empty() { "Imported".to_string() } else { stripped.to_string() }
}

pub fn dotenv_preflight(_ws_id: &str, source: &ImportSource) -> PreflightOutcome {
    Ok(Preflight::counted(parse_dotenv(&source.content).len()))
}

pub async fn dotenv_run(ws_id: String, source: ImportSource, _strategy: Strategy) -> ImportOutcome {
    let vars = parse_dotenv(&source.content);
    let mut cfg = load_config();
    let variables =
        vars.into_iter().map(|(key, value)| json!({ "id": generate_id(), "key": key, "value": value })).collect();
    add_environment(&mut cfg, &ws_id, &dotenv_env_name(&source.filename), variables)?;
    commit_config(&cfg)?;
    Ok(Imported::all(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        testing::scratch_data_root,
        workspace::fs::{read_all_entities, read_entity},
    };

    fn source(content: &str, filename: &str) -> ImportSource {
        ImportSource { content: content.to_string(), filename: filename.to_string() }
    }

    #[test]
    fn dotenv_lines_are_parsed_like_dotenv_files() {
        let text = "# c\n\nA=1\r\nB = \"two words\"\nC='it\\'s'\nNOEQ\n=empty\nD=\"q\\\"x\"\nE=\"\nA=3\n";
        let vars = parse_dotenv(text);
        let pairs: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(pairs, [("A", "3"), ("B", "two words"), ("C", "it's"), ("D", "q\"x"), ("E", "")]);
        assert_eq!(dotenv_env_name("prod.env"), "prod");
        assert_eq!(dotenv_env_name("C:\\x\\my.env.local"), "my");
        assert_eq!(dotenv_env_name(".env"), "Imported");
        assert_eq!(dotenv_env_name("vars.txt"), "vars.txt");
    }

    #[tokio::test]
    async fn dotenv_import_creates_one_environment() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        assert_eq!(dotenv_preflight(&ws, &source("A=1\nB=2", "dev.env")).unwrap().item_count, 2);
        assert_eq!(dotenv_run(ws.clone(), source("A=1\nB=2", "dev.env"), Strategy::Keep).await.unwrap().imported, 1);
        let envs = read_all_entities(&ws, "environments");
        let dev = envs.iter().find(|e| e["name"] == "dev").unwrap();
        assert_eq!(dev["variables"][1]["key"], "B");
        assert_eq!(dev["variables"][1]["value"], "2");
        assert_eq!(dev["workspaceId"], ws.as_str());
    }

    #[tokio::test]
    async fn postman_environments_keep_enabled_variables() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let one = json!({ "id": "x", "name": "Staging", "_postman_variable_scope": "environment", "values": [
            { "key": "HOST", "value": "h", "enabled": true }, { "key": "OFF", "value": "o", "enabled": false },
            { "key": "NOVAL" }, { "value": "nokey" } ] })
        .to_string();
        assert_eq!(postman_preflight(&ws, &source(&one, "e.json")).unwrap().item_count, 1);
        assert_eq!(postman_run(ws.clone(), source(&one, "e.json"), Strategy::Keep).await.unwrap().imported, 1);
        let envs = read_all_entities(&ws, "environments");
        let staging = envs.iter().find(|e| e["name"] == "Staging").unwrap();
        assert_eq!(staging["variables"].as_array().unwrap().len(), 2);
        assert_eq!(staging["variables"][1]["value"], "");
        assert_ne!(staging["id"], "x");

        let many = json!({ "schema": "lp-postman-environments-v1", "environments": [{ "values": [] }, { "name": "B", "values": [] }] }).to_string();
        assert_eq!(postman_run(ws.clone(), source(&many, "e.json"), Strategy::Keep).await.unwrap().imported, 2);
        assert!(read_all_entities(&ws, "environments").iter().any(|e| e["name"] == "Imported Environment"));
        assert_eq!(
            postman_preflight(&ws, &source("{}", "e.json")).err().unwrap(),
            "Error: Not a valid Postman environment file"
        );
    }

    #[tokio::test]
    async fn bifurc_json_environments_honour_the_strategy() {
        let root = scratch_data_root();
        let ws = root.ws_id.to_string();
        let file = json!({ "schema": "lp-environments-v1", "environments": [
            { "id": "e1", "name": "One", "variables": [], "createdAt": 1, "workspaceId": "other" }] })
        .to_string();
        let first = json_run(ws.clone(), source(&file, "e.json"), Strategy::Keep).await.unwrap();
        assert_eq!((first.imported, first.skipped), (1, Some(0)));
        assert_eq!(read_entity(&ws, "environments", "e1").unwrap()["workspaceId"], ws.as_str());
        assert_eq!(json_preflight(&ws, &source(&file, "e.json")).unwrap().collision_ids, ["e1"]);
        let kept = json_run(ws.clone(), source(&file, "e.json"), Strategy::Keep).await.unwrap();
        assert_eq!((kept.imported, kept.skipped), (0, Some(1)));
        let replaced = json_run(ws.clone(), source(&file, "e.json"), Strategy::Override).await.unwrap();
        assert_eq!(replaced.imported, 1);
        let fresh = json_run(ws.clone(), source(&file, "e.json"), Strategy::New).await.unwrap();
        assert_eq!(fresh.imported, 1);
        assert_eq!(read_all_entities(&ws, "environments").iter().filter(|e| e["name"] == "One").count(), 2);
    }
}
