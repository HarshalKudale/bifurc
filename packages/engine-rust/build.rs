//! Bakes the engine's build-time settings into the binary: `BIFURC_GITHUB_CLIENT_ID`, the PostHog project
//! `BIFURC_POSTHOG_KEY` / `BIFURC_POSTHOG_HOST`, and where the console is, `BIFURC_CONSOLE_URL`. Each comes from the build's environment, else from the
//! untracked `.env` at the repository root, so it is in every binary built here and never in the repository.

use std::{env, fs, path::Path};

const KEYS: [&str; 4] = ["BIFURC_GITHUB_CLIENT_ID", "BIFURC_POSTHOG_KEY", "BIFURC_POSTHOG_HOST", "BIFURC_CONSOLE_URL"];

/// The value of `key` in a `.env` file: `KEY=value` lines, `#` comments, optional quotes.
fn lookup(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let (name, value) = line.strip_prefix("export ").unwrap_or(line).split_once('=')?;
        if name.trim() != key {
            return None;
        }
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'').trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn main() {
    let env_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env");
    println!("cargo:rerun-if-changed=build.rs");
    // Watching a file that is not there would rebuild the engine on every build.
    if env_file.exists() {
        println!("cargo:rerun-if-changed={}", env_file.display());
    }
    let env_text = fs::read_to_string(&env_file).ok();
    for key in KEYS {
        println!("cargo:rerun-if-env-changed={key}");
        if env::var(key).is_ok_and(|value| !value.trim().is_empty()) {
            continue;
        }
        if let Some(value) = env_text.as_deref().and_then(|text| lookup(text, key)) {
            println!("cargo:rustc-env={key}={value}");
        }
    }
}
