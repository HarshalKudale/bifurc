//! Per-file status of a workspace against `HEAD`: `clean` | `modified` | `new` | `deleted`, keyed by the
//! file's path relative to the workspace.

use serde_json::{Map, Value, json};

use crate::workspace::{
    fs::{ALL_ENTITY_KINDS, get_pending_deletions},
    git_store::git,
    status_cache::{get_cached_status, set_cached_status},
};

pub use crate::workspace::status_cache::invalidate_cache;

/// A path that is an entity file rather than a bookkeeping file.
pub fn is_entity_path(path: &str) -> bool {
    let base = path.rsplit(['/', '\\']).next().unwrap_or("");
    let Some(stem) = base.strip_suffix(".json") else { return false };
    !stem.is_empty() && !["index", "enabled", "names", "pending-deletions"].contains(&stem)
}

/// Undo git's C-style quoting of a path (double quotes, backslash escapes, octal bytes), which git applies to
/// paths with spaces or non-ASCII characters unless `-z` is used.
pub fn unquote_git_path(path: &str) -> String {
    let Some(inner) = path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) else { return path.to_string() };
    let bytes = inner.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 >= bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        match bytes[i + 1] {
            b'"' => out.push(b'"'),
            b'\\' => out.push(b'\\'),
            b't' => out.push(b'\t'),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b'0'..=b'7' => {
                let digits = bytes[i + 1..].iter().take(3).take_while(|b| (b'0'..=b'7').contains(b)).count();
                let value = u32::from_str_radix(&inner[i + 1..i + 1 + digits], 8).unwrap_or(0);
                out.push(value as u8);
                i += 1 + digits;
                continue;
            }
            other => {
                out.push(b'\\');
                out.push(other);
            }
        }
        i += 2;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One entry of `git status --porcelain -z`: the two status letters and the path (a rename's original path
/// is dropped).
pub fn parse_porcelain_z(raw: &str) -> Vec<(String, String)> {
    let entries: Vec<&str> = raw.split('\0').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        let entry = entries[i];
        i += 1;
        if entry.chars().count() < 4 {
            continue;
        }
        let xy: String = entry.chars().take(2).collect();
        let path: String = entry.chars().skip(3).collect();
        if (xy.starts_with('R') || xy.starts_with('C')) && i < entries.len() {
            i += 1;
        }
        out.push((xy, path));
    }
    out
}

/// Every entity file of the workspace and how it differs from `HEAD`. Falls back to the last computed result
/// when git cannot be read.
pub async fn get_workspace_sync_status(ws_id: &str) -> Map<String, Value> {
    let Ok(raw) = git(ws_id, &["status", "--porcelain", "-z", "--untracked-files=all"]).await else {
        return get_cached_status(ws_id);
    };
    let mut result = Map::new();
    for (xy, path) in parse_porcelain_z(&raw) {
        let rel = path.replace('\\', "/");
        if !is_entity_path(&rel) {
            continue;
        }
        let status = if xy == "??" {
            "new"
        } else if xy.contains('D') {
            "deleted"
        } else {
            "modified"
        };
        result.insert(rel, json!(status));
    }

    let Ok(tracked) = git(ws_id, &["ls-files", "-z"]).await else { return get_cached_status(ws_id) };
    for entry in tracked.split('\0').filter(|e| !e.is_empty()) {
        let rel = entry.replace('\\', "/");
        if is_entity_path(&rel) && !result.contains_key(&rel) {
            result.insert(rel, json!("clean"));
        }
    }

    // An entity that was never committed and is now gone leaves nothing for git to report, but its
    // pending-deletion record says it is deleted.
    for kind in ALL_ENTITY_KINDS {
        for pending in get_pending_deletions(ws_id, kind) {
            let id = pending.get("id").and_then(Value::as_str).unwrap_or("");
            let root_path = format!("{kind}/{id}.json");
            if result.contains_key(&root_path) {
                continue;
            }
            let suffix = format!("/{id}.json");
            if !result.keys().any(|k| k.ends_with(&suffix) && *k != root_path) {
                result.insert(root_path, json!("deleted"));
            }
        }
    }

    set_cached_status(ws_id, result.clone());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookkeeping_files_are_not_entities() {
        assert!(is_entity_path("requests/abc.json"));
        assert!(is_entity_path("mocks/My Folder/m1.json"));
        assert!(!is_entity_path("requests/index.json"));
        assert!(!is_entity_path("requests/names.json"));
        assert!(!is_entity_path("pending-deletions.json"));
        assert!(!is_entity_path("workspace.txt"));
    }

    #[test]
    fn quoted_paths_are_unquoted() {
        assert_eq!(unquote_git_path("plain/a.json"), "plain/a.json");
        assert_eq!(unquote_git_path(r#""mocks/Solo Folder/sf-1.json""#), "mocks/Solo Folder/sf-1.json");
        assert_eq!(unquote_git_path(r#""a\"b\\c\td""#), "a\"b\\c\td");
        assert_eq!(unquote_git_path(r#""caf\303\251.json""#), "café.json");
    }

    #[test]
    fn porcelain_z_skips_a_renames_old_path() {
        let parsed = parse_porcelain_z("?? new.json\0R  b.json\0a.json\0 M c.json\0");
        let paths: Vec<&str> = parsed.iter().map(|(_, p)| p.as_str()).collect();
        assert_eq!(paths, vec!["new.json", "b.json", "c.json"]);
        assert_eq!(parsed[2].0, " M");
    }
}
