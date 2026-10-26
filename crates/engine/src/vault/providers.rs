//! Where a vault's values come from. A local vault keeps them sealed on this device (`store`) and is the only
//! kind whose secrets are edited here. Every other provider is a secret manager the vault only *pulls* from:
//! its settings name a source there (a 1Password item, an AWS secret holding JSON, an Azure key vault, …), and
//! unsealing reads every secret the source holds. Secrets are added, changed and removed in the manager.
//!
//! A source is read through the manager's own command line program. That program signs in the way its user
//! set it up (a desktop app, a profile, a token), so the engine keeps no login of its own: at most a token the
//! user gave for this device, which is passed to the program in its environment and never written anywhere
//! but this device. A program is run with fixed arguments, never through a shell, so a setting is always one
//! argument however it is written.

use std::{path::PathBuf, time::Duration};

use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

use crate::protocol::{EngineError, Result};

pub const LOCAL: &str = "local";

/// How long a program may take to answer: a manager that waits for a sign-in it cannot show would otherwise
/// hold the unseal for ever.
const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_NAME_LEN: usize = 64;

/// A setting of a provider. Settings are shared with the vault (they are in its synced file), so none of
/// them is a secret; a token is the provider's `credential`, which stays on the device.
pub struct Field {
    pub key: &'static str,
    pub required: bool,
}

pub struct Provider {
    pub id: &'static str,
    pub label: &'static str,
    /// The program the values are read with.
    pub program: &'static str,
    pub fields: &'static [Field],
    /// The environment variable the program reads a token from, when it can take one.
    pub credential_env: Option<&'static str>,
    pub credential_required: bool,
    /// Where the program comes from.
    pub install_url: &'static str,
}

const fn field(key: &'static str, required: bool) -> Field {
    Field { key, required }
}

pub static PROVIDERS: &[Provider] = &[
    Provider {
        id: "1password",
        label: "1Password",
        program: "op",
        fields: &[field("item", true), field("vault", false), field("account", false)],
        credential_env: Some("OP_SERVICE_ACCOUNT_TOKEN"),
        credential_required: false,
        install_url: "https://developer.1password.com/docs/cli/get-started/",
    },
    Provider {
        id: "aws",
        label: "AWS Secrets Manager",
        program: "aws",
        fields: &[field("secretId", true), field("region", false), field("profile", false)],
        credential_env: None,
        credential_required: false,
        install_url: "https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html",
    },
    Provider {
        id: "azure",
        label: "Azure Key Vault",
        program: "az",
        fields: &[field("vaultName", true)],
        credential_env: None,
        credential_required: false,
        install_url: "https://learn.microsoft.com/cli/azure/install-azure-cli",
    },
    Provider {
        id: "gcp",
        label: "Google Secret Manager",
        program: "gcloud",
        fields: &[field("project", false), field("filter", false)],
        credential_env: None,
        credential_required: false,
        install_url: "https://cloud.google.com/sdk/docs/install",
    },
    Provider {
        id: "hashicorp",
        label: "HashiCorp Vault",
        program: "vault",
        fields: &[field("address", true), field("path", true), field("namespace", false)],
        credential_env: Some("VAULT_TOKEN"),
        credential_required: false,
        install_url: "https://developer.hashicorp.com/vault/install",
    },
    Provider {
        id: "openbao",
        label: "OpenBao",
        program: "bao",
        fields: &[field("address", true), field("path", true), field("namespace", false)],
        credential_env: Some("BAO_TOKEN"),
        credential_required: false,
        install_url: "https://openbao.org/docs/install/",
    },
    Provider {
        id: "bitwarden",
        label: "Bitwarden Secrets Manager",
        program: "bws",
        fields: &[field("projectId", false), field("serverUrl", false)],
        credential_env: Some("BWS_ACCESS_TOKEN"),
        credential_required: true,
        install_url: "https://bitwarden.com/help/secrets-manager-cli/",
    },
    Provider {
        id: "doppler",
        label: "Doppler",
        program: "doppler",
        fields: &[field("project", false), field("config", false)],
        credential_env: Some("DOPPLER_TOKEN"),
        credential_required: false,
        install_url: "https://docs.doppler.com/docs/install-cli",
    },
];

pub fn get(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|provider| provider.id == id)
}

/// A provider id a vault may be created with: a manager's, or `local`.
pub fn check_provider(id: &str) -> Result<()> {
    if id == LOCAL || get(id).is_some() {
        return Ok(());
    }
    Err(EngineError::bad_request(format!("\"{id}\" is not a vault provider."))
        .keyed("vault.unknown_provider", json!({ "id": id })))
}

/// The program's file, found as a shell would find it: on Windows `az` and `gcloud` are `.cmd` files, which
/// `CreateProcess` does not look for on its own.
pub fn find_program(program: &str) -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(fake) = tests::FAKE_PROGRAMS.lock().unwrap().get(program) {
        return Some(fake.clone());
    }
    let extensions: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .flat_map(|dir| extensions.iter().map(move |extension| dir.join(format!("{program}{extension}"))))
        .find(|candidate| candidate.is_file())
}

/// Every provider for a client to offer, with whether its program is installed here.
pub fn catalog() -> Value {
    let managers = PROVIDERS.iter().map(|provider| {
        json!({
            "id": provider.id,
            "label": provider.label,
            "program": provider.program,
            "installed": find_program(provider.program).is_some(),
            "installUrl": provider.install_url,
            "fields": provider.fields.iter().map(|f| json!({ "key": f.key, "required": f.required })).collect::<Vec<_>>(),
            "credential": provider.credential_env.map(|env| json!({ "env": env, "required": provider.credential_required })),
        })
    });
    let local = json!({ "id": LOCAL, "label": "This device", "installed": true, "fields": [], "credential": null });
    Value::Array(std::iter::once(local).chain(managers).collect())
}

/// The settings a provider takes, from what a client sent: known keys only, trimmed, empty ones left out. A
/// required one that is missing is refused, and so is one the program would read as an option.
pub fn clean_settings(provider: &Provider, given: &Value) -> Result<Map<String, Value>> {
    let mut settings = Map::new();
    for field in provider.fields {
        let value = given.get(field.key).and_then(Value::as_str).map(str::trim).unwrap_or_default();
        if value.is_empty() {
            if field.required {
                return Err(EngineError::bad_request(format!("{} needs \"{}\".", provider.label, field.key))
                    .keyed("vault.setting_missing", json!({ "provider": provider.label, "field": field.key })));
            }
            continue;
        }
        if value.starts_with('-') {
            return Err(EngineError::bad_request(format!("\"{value}\" is not a {} setting.", provider.label))
                .keyed("vault.setting_invalid", json!({ "provider": provider.label, "field": field.key })));
        }
        settings.insert(field.key.to_string(), json!(value));
    }
    Ok(settings)
}

fn setting<'s>(settings: &'s Map<String, Value>, key: &str) -> Option<&'s str> {
    settings.get(key).and_then(Value::as_str).filter(|v| !v.is_empty())
}

/// A name a pulled secret goes by here: usable in `{{vault.NAME}}` and as an environment variable. A
/// character neither allows becomes `_` (Azure's `stripe-key` is `stripe_key`).
pub fn secret_name(raw: &str) -> Option<String> {
    let mut name: String =
        raw.trim().chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if name.is_empty() || name.chars().all(|c| c == '_') {
        return None;
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert(0, '_');
    }
    name.truncate(MAX_NAME_LEN);
    Some(name)
}

/// One run of the provider's program.
struct Run<'a> {
    provider: &'a Provider,
    settings: &'a Map<String, Value>,
    credential: Option<&'a str>,
}

impl Run<'_> {
    /// What the program printed, or why it failed (its first line of complaint).
    async fn call(&self, args: Vec<String>) -> std::result::Result<Zeroizing<String>, String> {
        let provider = self.provider;
        let program = find_program(provider.program).ok_or_else(|| {
            format!("{} is not installed: the \"{}\" program was not found", provider.label, provider.program)
        })?;
        let mut command = crate::utils::process::hidden_command(program);
        command.args(&args).stdin(std::process::Stdio::null()).kill_on_drop(true);
        if matches!(provider.id, "hashicorp" | "openbao") {
            let (address, namespace) = if provider.id == "openbao" {
                ("BAO_ADDR", "BAO_NAMESPACE")
            } else {
                ("VAULT_ADDR", "VAULT_NAMESPACE")
            };
            command.env(address, setting(self.settings, "address").unwrap_or_default());
            if let Some(value) = setting(self.settings, "namespace") {
                command.env(namespace, value);
            }
        }
        if let (Some(env), Some(token)) = (provider.credential_env, self.credential.filter(|t| !t.is_empty())) {
            command.env(env, token);
        }
        let output = match tokio::time::timeout(TIMEOUT, command.output()).await {
            Err(_) => return Err(format!("{} did not answer within {} s", provider.program, TIMEOUT.as_secs())),
            Ok(Err(e)) => return Err(format!("could not run {}: {e}", provider.program)),
            Ok(Ok(output)) => output,
        };
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let first = stderr.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("it failed");
            return Err(format!("{}: {first}", provider.program));
        }
        String::from_utf8(output.stdout).map(Zeroizing::new).map_err(|_| "the program did not print text".to_string())
    }

    async fn json(&self, args: Vec<String>) -> std::result::Result<Value, String> {
        let text = self.call(args).await?;
        serde_json::from_str(&text).map_err(|_| format!("{} did not print JSON", self.provider.program))
    }
}

fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// Adds `--flag value` for each setting that is set.
fn with(mut base: Vec<String>, settings: &Map<String, Value>, flags: &[(&str, &str)]) -> Vec<String> {
    for (key, flag) in flags {
        if let Some(value) = setting(settings, key) {
            base.extend(args(&[flag, value]));
        }
    }
    base
}

/// The text a program printed for one value, without the line break it ends with.
fn one_line(text: &str) -> String {
    text.strip_suffix('\n').map(|s| s.strip_suffix('\r').unwrap_or(s)).unwrap_or(text).to_string()
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// `(name, value)` from a JSON object of them.
fn from_object(value: &Value) -> Option<Vec<(String, String)>> {
    Some(value.as_object()?.iter().filter_map(|(k, v)| Some((k.clone(), scalar(v)?))).collect())
}

/// Reads one name each, at once.
async fn each(
    run: &Run<'_>,
    names: Vec<String>,
    args_for: impl Fn(&str) -> Vec<String>,
) -> std::result::Result<Vec<(String, String)>, String> {
    let read = futures::future::join_all(names.iter().map(|name| run.call(args_for(name)))).await;
    names.into_iter().zip(read).map(|(name, value)| value.map(|v| (name, one_line(&v)))).collect()
}

async fn pull_raw(run: &Run<'_>) -> std::result::Result<Vec<(String, String)>, String> {
    let s = run.settings;
    match run.provider.id {
        // The fields of one item: each one with a value, except its notes.
        "1password" => {
            let item = setting(s, "item").unwrap_or_default();
            let found = run
                .json(with(
                    args(&["item", "get", item, "--format", "json"]),
                    s,
                    &[("vault", "--vault"), ("account", "--account")],
                ))
                .await?;
            Ok(found["fields"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|f| f["purpose"] != "NOTES")
                .filter_map(|f| Some((f["label"].as_str()?.to_string(), f["value"].as_str()?.to_string())))
                .collect())
        }
        // One secret holding a JSON object, as the console's key/value secrets are; any other text is one
        // secret named after it.
        "aws" => {
            let id = setting(s, "secretId").unwrap_or_default();
            let base = args(&[
                "secretsmanager",
                "get-secret-value",
                "--secret-id",
                id,
                "--query",
                "SecretString",
                "--output",
                "text",
            ]);
            let text = one_line(&run.call(with(base, s, &[("region", "--region"), ("profile", "--profile")])).await?);
            let parsed = serde_json::from_str::<Value>(&text).ok();
            Ok(match parsed.as_ref().and_then(from_object) {
                Some(pairs) => pairs,
                None => vec![(id.rsplit('/').next().unwrap_or(id).to_string(), text)],
            })
        }
        // Every secret of the key vault.
        "azure" => {
            let vault = setting(s, "vaultName").unwrap_or_default();
            let listed = run
                .call(args(&["keyvault", "secret", "list", "--vault-name", vault, "--query", "[].name", "-o", "tsv"]))
                .await?;
            let names: Vec<String> =
                listed.lines().map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).collect();
            each(run, names, |name| {
                args(&[
                    "keyvault",
                    "secret",
                    "show",
                    "--vault-name",
                    vault,
                    "--name",
                    name,
                    "--query",
                    "value",
                    "-o",
                    "tsv",
                ])
            })
            .await
        }
        // Every secret of the project (or those the filter keeps), at its latest version.
        "gcp" => {
            let listed = run
                .call(with(
                    args(&["secrets", "list", "--format=value(name)"]),
                    s,
                    &[("project", "--project"), ("filter", "--filter")],
                ))
                .await?;
            let names: Vec<String> = listed
                .lines()
                .map(|line| line.trim().rsplit('/').next().unwrap_or_default().to_string())
                .filter(|n| !n.is_empty())
                .collect();
            each(run, names, |name| {
                with(
                    args(&["secrets", "versions", "access", "latest", &format!("--secret={name}")]),
                    s,
                    &[("project", "--project")],
                )
            })
            .await
        }
        // The keys of one KV path (version 2 nests them under `data.data`).
        "hashicorp" | "openbao" => {
            let path = setting(s, "path").unwrap_or_default();
            let found = run.json(args(&["kv", "get", "-format=json", path])).await?;
            let data = if found["data"]["data"].is_object() { &found["data"]["data"] } else { &found["data"] };
            from_object(data).ok_or_else(|| format!("{path} holds no keys"))
        }
        // Every secret the token may read, or those of one project.
        "bitwarden" => {
            let mut base = args(&["secret", "list"]);
            if let Some(project) = setting(s, "projectId") {
                base.push(project.to_string());
            }
            base.extend(args(&["--output", "json"]));
            let found = run.json(with(base, s, &[("serverUrl", "--server-url")])).await?;
            Ok(found
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|secret| Some((secret["key"].as_str()?.to_string(), secret["value"].as_str()?.to_string())))
                .collect())
        }
        // The config's secrets, without the ones Doppler adds about itself.
        "doppler" => {
            let base = args(&["secrets", "download", "--no-file", "--format", "json"]);
            let found = run.json(with(base, s, &[("project", "--project"), ("config", "--config")])).await?;
            Ok(from_object(&found)
                .unwrap_or_default()
                .into_iter()
                .filter(|(k, _)| !k.starts_with("DOPPLER_"))
                .collect())
        }
        other => Err(format!("\"{other}\" is not a vault provider")),
    }
}

/// Every secret the source holds, named for use here, or why it could not be read. Empty values are left out;
/// of two that come to the same name, the first is kept.
pub async fn pull(
    provider: &Provider,
    settings: &Map<String, Value>,
    credential: Option<&str>,
) -> std::result::Result<Vec<(String, Zeroizing<String>)>, String> {
    let raw = pull_raw(&Run { provider, settings, credential }).await?;
    let mut out: Vec<(String, Zeroizing<String>)> = Vec::new();
    for (name, value) in raw {
        let value = Zeroizing::new(value);
        let Some(name) = secret_name(&name) else { continue };
        if value.is_empty() || out.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)) {
            continue;
        }
        out.push((name, value));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Programs found in place of the real ones, by name.
    pub static FAKE_PROGRAMS: std::sync::Mutex<std::collections::BTreeMap<&'static str, PathBuf>> =
        std::sync::Mutex::new(std::collections::BTreeMap::new());

    /// Makes `program` a script in `dir` that answers `op item get <item> --format json` with an item whose
    /// `api key` field is `value-for-<item>` (and an empty field and notes, which are not secrets), and fails
    /// for an item with `missing` in its name.
    pub fn fake_op(dir: &std::path::Path) {
        let path = if cfg!(windows) {
            let path = dir.join("op.cmd");
            let script = "@echo off\r\necho %3| findstr missing >nul && (echo [ERROR] no such item 1>&2 & exit /b 1)\r\n\
                          echo {\"fields\":[{\"label\":\"api key\",\"value\":\"value-for-%3\"},{\"label\":\"empty\",\"value\":\"\"},{\"label\":\"notes\",\"purpose\":\"NOTES\",\"value\":\"just notes\"}]}\r\n";
            std::fs::write(&path, script).unwrap();
            path
        } else {
            let path = dir.join("op");
            let script = "#!/bin/sh\ncase \"$3\" in *missing*) echo '[ERROR] no such item' >&2; exit 1;; esac\n\
                          printf '{\"fields\":[{\"label\":\"api key\",\"value\":\"value-for-%s\"},{\"label\":\"empty\",\"value\":\"\"},{\"label\":\"notes\",\"purpose\":\"NOTES\",\"value\":\"just notes\"}]}\\n' \"$3\"\n";
            std::fs::write(&path, script).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            path
        };
        FAKE_PROGRAMS.lock().unwrap().insert("op", path);
    }

    #[test]
    fn pulled_names_work_as_references_and_environment_variables() {
        assert_eq!(secret_name("stripe-key").as_deref(), Some("stripe_key"));
        assert_eq!(secret_name(" api key ").as_deref(), Some("api_key"));
        assert_eq!(secret_name("2fa").as_deref(), Some("_2fa"));
        assert_eq!(secret_name("--"), None);
        assert_eq!(secret_name(&"x".repeat(80)).unwrap().len(), MAX_NAME_LEN);
    }

    #[test]
    fn a_json_object_gives_one_secret_per_key() {
        let pairs = from_object(&json!({ "A": "1", "B": 2, "C": null })).unwrap();
        assert_eq!(pairs, [("A".to_string(), "1".to_string()), ("B".to_string(), "2".to_string())]);
        assert_eq!(one_line("v\r\n"), "v");
        assert_eq!(one_line("a\nb\n"), "a\nb");
    }

    #[test]
    fn settings_keep_only_what_the_provider_takes() {
        let azure = get("azure").unwrap();
        assert!(clean_settings(azure, &json!({})).is_err());
        let settings = clean_settings(azure, &json!({ "vaultName": " kv1 ", "address": "https://evil" })).unwrap();
        assert_eq!(Value::Object(settings), json!({ "vaultName": "kv1" }));
        // Read as an option by the program, so never passed on.
        assert!(clean_settings(azure, &json!({ "vaultName": "--help" })).is_err());
    }

    #[tokio::test]
    async fn an_item_is_pulled_field_by_field() {
        let dir = tempfile::tempdir().unwrap();
        fake_op(dir.path());
        let op = get("1password").unwrap();
        let settings = clean_settings(op, &json!({ "item": "Stripe" })).unwrap();
        let pulled = pull(op, &settings, None).await.unwrap();
        let pulled: Vec<(&str, &str)> = pulled.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
        assert_eq!(pulled, [("api_key", "value-for-Stripe")]);
        let missing = clean_settings(op, &json!({ "item": "missing" })).unwrap();
        assert!(pull(op, &missing, None).await.unwrap_err().contains("no such item"));
    }
}
