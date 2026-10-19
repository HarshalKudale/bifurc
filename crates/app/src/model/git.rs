//! The workspace's git state as the engine reports it (`sync.getEntityStatus`), and the roll-ups the footer
//! and the history panel read from it.
//!
//! Every workspace is a git repository and every saved entity is a file in it. The engine decides what
//! changed; this only counts what it said, for the part of the workspace a screen is about.

use std::collections::HashMap;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    Clean,
    Modified,
    New,
    Deleted,
}

impl FileState {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "clean" => Some(Self::Clean),
            "modified" => Some(Self::Modified),
            "new" => Some(Self::New),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GitStatus {
    /// Workspace-relative path to state, for every file the engine tracks.
    pub files: HashMap<String, FileState>,
    /// Entity id to the workspace-relative path of its file.
    pub entity_paths: HashMap<String, String>,
    pub has_remote: bool,
    pub branch: Option<String>,
    pub ahead: bool,
    pub behind: bool,
}

/// What the footer offers for the files a screen covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishAction {
    /// Everything pending is new: commit it (and push, with a remote).
    Publish,
    /// There are edits or deletions, or commits that are not on the remote yet.
    Sync,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rollup {
    /// Paths that are not clean, sorted.
    pub changed: Vec<String>,
    pub action: Option<PublishAction>,
}

impl GitStatus {
    pub fn from_value(value: &Value) -> Self {
        let files = value
            .get("files")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(path, state)| Some((path.clone(), FileState::parse(state.as_str()?)?)))
                    .collect()
            })
            .unwrap_or_default();
        let entity_paths = value
            .pointer("/paths/entities")
            .and_then(Value::as_object)
            .map(|map| map.iter().filter_map(|(id, path)| Some((id.clone(), path.as_str()?.to_owned()))).collect())
            .unwrap_or_default();
        let git = value.get("git");
        let flag = |key: &str| git.and_then(|g| g.get(key)).and_then(Value::as_bool).unwrap_or(false);
        Self {
            files,
            entity_paths,
            has_remote: flag("hasRemote"),
            branch: git.and_then(|g| g.get("branch")).and_then(Value::as_str).map(str::to_owned),
            ahead: flag("ahead"),
            behind: flag("behind"),
        }
    }

    pub fn entity_path(&self, id: &str) -> Option<&str> {
        self.entity_paths.get(id).map(String::as_str)
    }

    pub fn entity_state(&self, id: &str) -> Option<FileState> {
        self.files.get(self.entity_path(id)?).copied()
    }

    /// The pending work under any of `prefixes` (workspace-relative directories, each ending in `/`).
    /// An empty `prefixes` means the whole workspace.
    pub fn rollup(&self, prefixes: &[&str]) -> Rollup {
        let mut changed: Vec<String> = self
            .files
            .iter()
            .filter(|(path, state)| {
                **state != FileState::Clean
                    && (prefixes.is_empty() || prefixes.iter().any(|prefix| path.starts_with(prefix)))
            })
            .map(|(path, _)| path.clone())
            .collect();
        changed.sort();
        let all_new = !changed.is_empty() && changed.iter().all(|path| self.files[path] == FileState::New);
        let action = if !changed.is_empty() {
            Some(if all_new { PublishAction::Publish } else { PublishAction::Sync })
        } else if self.has_remote && self.ahead {
            Some(PublishAction::Sync)
        } else {
            None
        };
        Rollup { changed, action }
    }
}

/// One commit in a file's history, as `git.history` returns it.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub commit: String,
    /// What the commit did to the entity: `create`, `update`, `delete`, ...
    pub action: String,
    /// The entity's name at that commit.
    pub entity_name: String,
    /// The commit's subject, when the user wrote it; empty otherwise.
    pub message: String,
    pub actor: String,
    /// Epoch milliseconds.
    pub ts: u64,
}

impl HistoryEntry {
    pub fn short_commit(&self) -> &str {
        &self.commit[..self.commit.len().min(7)]
    }
}

/// Reads a `git.history` result: `{ entries: [{ commitHash, ts, action, entityName, message?, actor }], total }`.
pub fn parse_history(result: &Value) -> Vec<HistoryEntry> {
    let text = |entry: &Value, key: &str| entry.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
    result
        .get("entries")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    Some(HistoryEntry {
                        commit: entry.get("commitHash")?.as_str()?.to_owned(),
                        action: text(entry, "action"),
                        entity_name: text(entry, "entityName"),
                        message: text(entry, "message"),
                        actor: text(entry, "actor"),
                        ts: entry.get("ts").and_then(Value::as_u64).unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `YYYY-MM-DD HH:MM` of an epoch-millisecond timestamp, in UTC.
pub fn date_time_utc(epoch_ms: u64) -> String {
    let seconds = epoch_ms / 1000;
    let days = seconds / 86_400;
    let (hour, minute) = (seconds % 86_400 / 3600, seconds % 3600 / 60);
    // Civil date from a day count (Howard Hinnant's algorithm), valid for every date this can receive.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn status(files: Value, git: Value) -> GitStatus {
        GitStatus::from_value(
            &json!({ "files": files, "git": git, "paths": { "entities": { "r1": "requests/r1.json" } } }),
        )
    }

    fn local() -> Value {
        json!({ "hasRemote": false, "branch": null, "ahead": false, "behind": false })
    }

    #[test]
    fn an_entity_is_found_by_id_through_its_path() {
        let s = status(json!({ "requests/r1.json": "modified" }), local());
        assert_eq!(s.entity_path("r1"), Some("requests/r1.json"));
        assert_eq!(s.entity_state("r1"), Some(FileState::Modified));
        assert_eq!(s.entity_state("nope"), None);
    }

    #[test]
    fn a_rollup_counts_only_the_files_under_its_prefixes() {
        let s = status(json!({ "requests/a.json": "new", "mocks/b.json": "new", "requests/c.json": "clean" }), local());
        let rollup = s.rollup(&["requests/"]);
        assert_eq!(rollup.changed, ["requests/a.json"]);
        assert_eq!(rollup.action, Some(PublishAction::Publish));
        assert_eq!(s.rollup(&[]).changed.len(), 2);
    }

    #[test]
    fn an_edit_or_a_deletion_among_the_changes_makes_it_a_sync() {
        let s = status(json!({ "requests/a.json": "new", "requests/b.json": "deleted" }), local());
        assert_eq!(s.rollup(&["requests/"]).action, Some(PublishAction::Sync));
    }

    #[test]
    fn nothing_changed_is_nothing_to_do_unless_commits_are_waiting_for_the_remote() {
        let clean = json!({ "requests/a.json": "clean" });
        assert_eq!(status(clean.clone(), local()).rollup(&[]).action, None);
        let ahead = json!({ "hasRemote": true, "branch": "main", "ahead": true, "behind": false });
        assert_eq!(status(clean, ahead).rollup(&[]).action, Some(PublishAction::Sync));
    }

    #[test]
    fn history_reads_the_engines_entries() {
        let entries = parse_history(&json!({ "total": 1, "entries": [{
            "commitHash": "3034b96bd696549f36655677c3dc4322ff175395", "ts": 1_790_840_885_000u64,
            "action": "create", "entity": "request", "entityName": "Get thing", "actor": "sam",
        }]}));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].short_commit(), "3034b96");
        assert_eq!(entries[0].action, "create");
        assert_eq!(entries[0].entity_name, "Get thing");
        assert_eq!(entries[0].message, "");
        assert!(parse_history(&json!({})).is_empty());
    }

    #[test]
    fn dates_are_civil_dates_in_utc() {
        assert_eq!(date_time_utc(0), "1970-01-01 00:00");
        assert_eq!(date_time_utc(951_782_400_000), "2000-02-29 00:00");
        assert_eq!(date_time_utc(1_700_000_000_000), "2023-11-14 22:13");
    }
}
