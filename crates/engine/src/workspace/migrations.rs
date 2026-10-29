//! Bringing the data an older Bifurc left up to this version, whichever version that was.
//!
//! `data-version.json` in the data directory names the version that last ran on it. When this engine is newer,
//! the migration of every version after that one, up to this engine's, runs in version order, so an install
//! many releases behind goes through each of them in turn. The data directory is copied to
//! `backups/<from>-to-<to>-<ms>` first, and nothing runs when that copy fails. A migration that fails stops the
//! chain: the version recorded is the last one reached, so the next start goes on from there. Data a newer
//! Bifurc left is not touched. Every release keeps all the migrations before it.

use std::path::Path;

use serde_json::json;

use crate::{
    updates::manifest::{Version, parse_version},
    utils::{
        id::now_ms,
        json::{opt_str, read_json, write_json_pretty},
    },
};

/// Changes the data in the data directory it is given into what one version reads.
type Migration = fn(&Path) -> std::io::Result<()>;

/// `(version, migration)`: what the data needs before that version reads it.
const MIGRATIONS: &[(&str, Migration)] = &[];

const VERSION_FILE: &str = "data-version.json";
const BACKUPS_DIR: &str = "backups";

fn text(version: Version) -> String {
    format!("{}.{}.{}", version.0, version.1, version.2)
}

fn record(dir: &Path, version: &str) -> std::io::Result<()> {
    write_json_pretty(&dir.join(VERSION_FILE), &json!({ "version": version }))
}

/// Everything in `dir` but the earlier backups, into `to`.
fn back_up(dir: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_name() == BACKUPS_DIR {
            continue;
        }
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            super::tree::copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Migrate the data in `dir` to `current`, this engine's version.
pub fn run(dir: &Path, current: &str) -> std::io::Result<()> {
    run_with(dir, current, MIGRATIONS)
}

fn run_with(dir: &Path, current: &str, migrations: &[(&str, Migration)]) -> std::io::Result<()> {
    let Some(now) = parse_version(current) else { return Ok(()) };
    let recorded =
        read_json(&dir.join(VERSION_FILE)).and_then(|file| opt_str(&file, "version").and_then(parse_version));
    let from = match recorded {
        Some(version) => version,
        // Data with no record was left by a version from before records were kept.
        None if dir.join("data").is_dir() => (0, 0, 0),
        None => return record(dir, current),
    };
    if from >= now {
        return Ok(());
    }
    let mut pending: Vec<(Version, Migration)> = migrations
        .iter()
        .filter_map(|(version, migrate)| parse_version(version).map(|version| (version, *migrate)))
        .filter(|(version, _)| *version > from && *version <= now)
        .collect();
    pending.sort_by_key(|(version, _)| *version);
    if !pending.is_empty() {
        back_up(dir, &dir.join(BACKUPS_DIR).join(format!("{}-to-{current}-{}", text(from), now_ms())))?;
    }
    for (version, migrate) in pending {
        migrate(dir)?;
        record(dir, &text(version))?;
    }
    record(dir, current)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(dir: &Path, name: &str) -> std::io::Result<()> {
        let log = dir.join("data").join("log");
        let before = std::fs::read_to_string(&log).unwrap_or_default();
        std::fs::write(log, format!("{before}{name} "))
    }

    const CHAIN: &[(&str, Migration)] = &[
        ("0.4.0", |dir| mark(dir, "four")),
        ("0.2.0", |dir| mark(dir, "two")),
        ("0.3.0", |dir| mark(dir, "three")),
        ("0.1.0", |dir| mark(dir, "one")),
    ];

    fn data(from: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data").join("ws1")).unwrap();
        std::fs::write(dir.path().join("data").join("ws1").join("request.json"), "{}").unwrap();
        if let Some(from) = from {
            record(dir.path(), from).unwrap();
        }
        dir
    }

    fn recorded(dir: &Path) -> String {
        opt_str(&read_json(&dir.join(VERSION_FILE)).unwrap(), "version").unwrap().to_string()
    }

    fn log(dir: &Path) -> String {
        std::fs::read_to_string(dir.join("data").join("log")).unwrap_or_default()
    }

    #[test]
    fn an_old_version_goes_through_each_migration_after_it_in_order_after_a_backup() {
        let dir = data(Some("0.1.0"));
        run_with(dir.path(), "0.3.5", CHAIN).unwrap();
        assert_eq!(log(dir.path()), "two three ");
        assert_eq!(recorded(dir.path()), "0.3.5");
        let backups: Vec<_> = std::fs::read_dir(dir.path().join(BACKUPS_DIR)).unwrap().flatten().collect();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].file_name().to_string_lossy().starts_with("0.1.0-to-0.3.5-"));
        // The copy is of the data before any migration ran.
        assert!(backups[0].path().join("data").join("ws1").join("request.json").is_file());
        assert!(!backups[0].path().join("data").join("log").exists());
    }

    #[test]
    fn data_from_before_records_were_kept_goes_through_every_migration() {
        let dir = data(None);
        run_with(dir.path(), "0.4.0", CHAIN).unwrap();
        assert_eq!(log(dir.path()), "one two three four ");
    }

    #[test]
    fn a_new_data_directory_and_data_already_migrated_are_left_alone() {
        let fresh = tempfile::tempdir().unwrap();
        run_with(fresh.path(), "0.4.0", CHAIN).unwrap();
        assert_eq!(recorded(fresh.path()), "0.4.0");
        assert!(!fresh.path().join(BACKUPS_DIR).exists());

        let newer = data(Some("0.9.0"));
        run_with(newer.path(), "0.4.0", CHAIN).unwrap();
        assert_eq!((log(newer.path()), recorded(newer.path())), (String::new(), "0.9.0".to_string()));
        assert!(!newer.path().join(BACKUPS_DIR).exists());
    }

    #[test]
    fn a_failed_migration_stops_the_chain_where_it_is() {
        let failing: &[(&str, Migration)] = &[
            ("0.2.0", |dir| mark(dir, "two")),
            ("0.3.0", |_| Err(std::io::Error::other("broken"))),
            ("0.4.0", |dir| mark(dir, "four")),
        ];
        let dir = data(Some("0.1.0"));
        assert!(run_with(dir.path(), "0.4.0", failing).is_err());
        assert_eq!((log(dir.path()), recorded(dir.path())), ("two ".to_string(), "0.2.0".to_string()));
    }
}
