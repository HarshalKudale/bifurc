//! The error keys the engine names exist in the UI's locale files, and the languages agree on their keys.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

fn locale_keys(language: &str) -> BTreeSet<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../app/locales").join(language).join("errors.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&text).expect("errors.json parses");
    map.keys().cloned().collect()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("src is readable").flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Every `keyed("key"` literal in the engine's source (`EngineError::keyed` and `GitError::keyed` both take
/// the key first).
fn keys_in_source() -> BTreeSet<String> {
    let mut files = Vec::new();
    rust_files(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let mut keys = BTreeSet::new();
    for file in files {
        let text = fs::read_to_string(&file).expect("source is readable");
        // `said("key", params, message)` (flows) takes the key first too.
        for (at, opener) in text.match_indices("keyed(").chain(text.match_indices("said(")) {
            let rest = text[at + opener.len()..].trim_start();
            if let Some(rest) = rest.strip_prefix('"')
                && let Some(end) = rest.find('"')
            {
                keys.insert(rest[..end].to_owned());
            }
        }
        // `send_failure(code, "area.failed", message)` keys a raw transport message under its wrapper key.
        for (at, _) in text.match_indices("send_failure(") {
            let rest = &text[at + "send_failure(".len()..];
            if let Some(start) = rest.find('"')
                && let Some(end) = rest[start + 1..].find('"')
            {
                let literal = &rest[start + 1..start + 1 + end];
                if literal.contains('.') && !literal.contains(' ') {
                    keys.insert(literal.to_owned());
                }
            }
        }
    }
    keys
}

#[test]
fn every_key_the_engine_names_has_english_text() {
    let english = locale_keys("en");
    let used = keys_in_source();
    assert!(used.len() > 30, "the scan found only {} keys", used.len());
    let missing: Vec<_> = used.difference(&english).collect();
    assert!(missing.is_empty(), "keys with no text in locales/en/errors.json: {missing:?}");
}

#[test]
fn the_languages_have_the_same_error_keys() {
    let english = locale_keys("en");
    for language in ["de", "ja"] {
        let other = locale_keys(language);
        let missing: Vec<_> = english.difference(&other).collect();
        let extra: Vec<_> = other.difference(&english).collect();
        assert!(missing.is_empty() && extra.is_empty(), "{language}: missing {missing:?}, extra {extra:?}");
    }
}
