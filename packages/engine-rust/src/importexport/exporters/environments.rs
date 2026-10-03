//! Environment exports: Bifurc JSON, a Postman environment and dotenv.

use serde_json::{Value, json};

use crate::{
    importexport::{
        support::{in_workspace, is_js_space, js_str, pretty, text_or, workspace_name},
        types::{ExportFile, ExportOutcome},
    },
    utils::json::{array_of, is_truthy},
    workspace::config::load_config,
};

const NONE_FOUND: &str = "No environments found in this workspace";

pub fn json(ws_id: &str) -> ExportOutcome {
    let cfg = load_config();
    let payload = json!({
        "schema": "lp-environments-v1",
        "name": workspace_name(&cfg, ws_id, "Bifurc"),
        "environments": in_workspace(&cfg, "environments", ws_id),
    });
    Ok(ExportFile { content: pretty(&payload), suggested_name: "environments-export.json" })
}

fn env_to_postman(env: &Value) -> Value {
    let values: Vec<Value> = array_of(env, "variables")
        .iter()
        .map(|v| {
            json!({
                "key": v.get("key").cloned().unwrap_or(Value::Null),
                "value": v.get("value").cloned().unwrap_or(Value::Null),
                "enabled": true,
                "type": "default",
            })
        })
        .collect();
    json!({
        "id": env.get("id").cloned().unwrap_or(Value::Null),
        "name": env.get("name").cloned().unwrap_or(Value::Null),
        "values": values,
        "_postman_variable_scope": "environment",
    })
}

pub fn postman(ws_id: &str) -> ExportOutcome {
    let cfg = load_config();
    let environments = in_workspace(&cfg, "environments", ws_id);
    if environments.is_empty() {
        return Err(NONE_FOUND.to_string());
    }
    let content = if environments.len() == 1 {
        pretty(&env_to_postman(&environments[0]))
    } else {
        let converted: Vec<Value> = environments.iter().map(env_to_postman).collect();
        pretty(&json!({ "schema": "lp-postman-environments-v1", "environments": converted }))
    };
    Ok(ExportFile { content, suggested_name: "environments-export.json" })
}

fn escape_value(value: &str) -> String {
    if value.chars().any(|c| is_js_space(c) || matches!(c, '"' | '\'' | '\\' | '#')) {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

pub fn dotenv(ws_id: &str) -> ExportOutcome {
    let cfg = load_config();
    let environments = in_workspace(&cfg, "environments", ws_id);
    if environments.is_empty() {
        return Err(NONE_FOUND.to_string());
    }
    let several = environments.len() > 1;
    let mut lines: Vec<String> = Vec::new();
    for env in &environments {
        if several {
            lines.push(format!("# {}", text_or(env, "name", "undefined")));
            lines.push(String::new());
        }
        for variable in array_of(env, "variables") {
            if variable.get("key").is_some_and(is_truthy) {
                let value = variable.get("value").filter(|v| !v.is_null()).map(js_str).unwrap_or_default();
                lines.push(format!("{}={}", js_str(&variable["key"]), escape_value(&value)));
            }
        }
        if several {
            lines.push(String::new());
        }
    }
    Ok(ExportFile { content: lines.join("\n") + "\n", suggested_name: "environments-export.env" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{testing::scratch_data_root, workspace::fs::write_flat_entity};

    fn add_env(ws: &str, id: &str, name: &str, vars: Value) {
        let env = json!({ "id": id, "name": name, "variables": vars, "createdAt": 1, "workspaceId": ws });
        write_flat_entity(ws, "environments", id, &env).unwrap();
    }

    #[test]
    fn the_global_environment_alone_exports_as_one_postman_environment() {
        let root = scratch_data_root();
        let out = postman(root.ws_id).unwrap();
        let env: Value = serde_json::from_str(&out.content).unwrap();
        assert_eq!(env["id"], "__global__");
        assert_eq!(env["_postman_variable_scope"], "environment");
        assert_eq!(dotenv(root.ws_id).unwrap().content, "\n");
    }

    #[test]
    fn several_environments_are_grouped() {
        let root = scratch_data_root();
        add_env(
            root.ws_id,
            "e1",
            "Dev",
            json!([{ "id": "v1", "key": "HOST", "value": "a b" }, { "id": "v2", "key": "", "value": "x" }]),
        );
        let out: Value = serde_json::from_str(&postman(root.ws_id).unwrap().content).unwrap();
        assert_eq!(out["schema"], "lp-postman-environments-v1");
        let dev = out["environments"].as_array().unwrap().iter().find(|e| e["name"] == "Dev").unwrap();
        assert_eq!(dev["values"][0], json!({ "key": "HOST", "value": "a b", "enabled": true, "type": "default" }));

        let text = dotenv(root.ws_id).unwrap().content;
        assert!(text.contains("# Dev\n\nHOST=\"a b\"\n"), "{text}");
        assert!(text.ends_with("\n"));
        let json_out: Value = serde_json::from_str(&json(root.ws_id).unwrap().content).unwrap();
        assert_eq!(json_out["schema"], "lp-environments-v1");
    }

    #[test]
    fn dotenv_values_are_quoted_when_needed() {
        assert_eq!(escape_value("plain"), "plain");
        assert_eq!(escape_value("a#b"), "\"a#b\"");
        assert_eq!(escape_value("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(escape_value(""), "");
    }
}
