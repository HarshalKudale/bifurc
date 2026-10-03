//! Bakes `BIFURC_GITHUB_CLIENT_ID` into the engine. The id comes from the build's environment, else from the
//! untracked `.env` at the repository root, so the id is in every binary built here and never in the repository.

use std::{env, fs, path::Path};

const KEY: &str = "BIFURC_GITHUB_CLIENT_ID";

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
    println!("cargo:rerun-if-env-changed={KEY}");
    // Watching a file that is not there would rebuild the engine on every build.
    if env_file.exists() {
        println!("cargo:rerun-if-changed={}", env_file.display());
    }
    if env::var(KEY).is_ok_and(|value| !value.trim().is_empty()) {
        return;
    }
    if let Some(id) = fs::read_to_string(&env_file).ok().and_then(|text| lookup(&text, KEY)) {
        println!("cargo:rustc-env={KEY}={id}");
    }
}
