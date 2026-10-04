//! The blob TTL sweep: reclaims blobs and staged files a client never released.
//!
//! Per entry under the blob root: a blob directory with `meta.json` goes when `createdAt + ttl <= now`; one
//! without metadata (a write that died) and a `.staging/` file go when their mtime is that old; a loose file
//! directly under the root always goes. The sweep never creates the root and never fails.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::task::JoinHandle;

use super::store::{BLOB_STAGING_DIR_NAME, BLOB_TTL_MS, blob_root, read_meta_from_dir};
use crate::utils::id::now_ms;

/// How often the periodic sweeper runs: a sixth of the TTL.
pub const BLOB_SWEEP_INTERVAL_MS: i64 = BLOB_TTL_MS / 6;

/// What one pass reclaimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlobSweepResult {
    /// Directories and files removed.
    pub removed: u64,
    /// Bytes those entries accounted for: exact for real blobs, on-disk size otherwise.
    pub reclaimed_bytes: u64,
}

/// Called for each entry that could not be removed.
pub type SweepErrorHandler<'a> = &'a dyn Fn(&std::io::Error, &Path);

#[derive(Default, Clone, Copy)]
pub struct BlobSweepOptions<'a> {
    /// Override for "now" in epoch milliseconds.
    pub now: Option<i64>,
    /// Override the lease term.
    pub ttl_ms: Option<i64>,
    pub on_error: Option<SweepErrorHandler<'a>>,
    /// Sweep only this workspace's namespace; `None` sweeps the whole blob root.
    pub workspace_id: Option<&'a str>,
}

fn directory_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else { continue };
        for entry in entries.flatten() {
            let full = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(full),
                Ok(_) => total += fs::metadata(&full).map(|m| m.len()).unwrap_or(0),
                Err(_) => {}
            }
        }
    }
    total
}

fn is_older_than(path: &Path, now: i64, ttl_ms: i64) -> bool {
    let mtime_ms = fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| {
        let since = t.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64);
        since.ok().or_else(|| SystemTime::now().duration_since(t).ok().map(|d| -(d.as_millis() as i64)))
    });
    match mtime_ms {
        Some(mtime) => mtime + ttl_ms <= now,
        None => true,
    }
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    if fs::symlink_metadata(path)?.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) }
}

/// One pass over the blob root. Returns what it reclaimed.
pub fn sweep_blobs(opts: BlobSweepOptions<'_>) -> BlobSweepResult {
    let now = opts.now.unwrap_or_else(now_ms);
    let ttl_ms = opts.ttl_ms.unwrap_or(BLOB_TTL_MS);
    let root = blob_root(opts.workspace_id);
    let Ok(entries) = fs::read_dir(&root) else { return BlobSweepResult::default() };

    let mut out = BlobSweepResult::default();
    for entry in entries.flatten() {
        let full = entry.path();
        let name = entry.file_name();

        if name == BLOB_STAGING_DIR_NAME {
            let staging = sweep_staging(&full, now, ttl_ms, opts.on_error);
            out.removed += staging.removed;
            out.reclaimed_bytes += staging.reclaimed_bytes;
            continue;
        }

        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir {
            let size = fs::metadata(&full).map(|m| m.len()).unwrap_or(0);
            match remove_any(&full) {
                Ok(()) => {
                    out.removed += 1;
                    out.reclaimed_bytes += size;
                }
                Err(err) => report(&opts.on_error, &err, &full),
            }
            continue;
        }

        let meta = read_meta_from_dir(&full);
        let expired = match &meta {
            Some(meta) => created_at(meta) + ttl_ms <= now,
            None => is_older_than(&full, now, ttl_ms),
        };
        if !expired {
            continue;
        }
        let bytes = match &meta {
            Some(meta) => meta.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
            None => directory_size(&full),
        };
        match fs::remove_dir_all(&full) {
            Ok(()) => {
                out.removed += 1;
                out.reclaimed_bytes += bytes;
            }
            Err(err) => report(&opts.on_error, &err, &full),
        }
    }
    out
}

fn created_at(meta: &serde_json::Value) -> i64 {
    meta.get("createdAt").and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))).unwrap_or(0)
}

fn report(on_error: &Option<SweepErrorHandler<'_>>, err: &std::io::Error, path: &Path) {
    if let Some(handler) = on_error {
        handler(err, path);
    }
}

fn sweep_staging(dir: &PathBuf, now: i64, ttl_ms: i64, on_error: Option<SweepErrorHandler<'_>>) -> BlobSweepResult {
    let Ok(entries) = fs::read_dir(dir) else { return BlobSweepResult::default() };
    let mut out = BlobSweepResult::default();
    for entry in entries.flatten() {
        let full = entry.path();
        if !is_older_than(&full, now, ttl_ms) {
            continue;
        }
        let size = fs::metadata(&full).map(|m| m.len()).unwrap_or(0);
        match remove_any(&full) {
            Ok(()) => {
                out.removed += 1;
                out.reclaimed_bytes += size;
            }
            Err(err) => report(&on_error, &err, &full),
        }
    }
    out
}

/// Start the periodic sweep of the whole blob root. The first pass runs one interval after the start, as a
/// timer would. Abort the handle to stop it.
pub fn start_blob_sweeper() -> JoinHandle<()> {
    start_blob_sweeper_every(Duration::from_millis(BLOB_SWEEP_INTERVAL_MS as u64))
}

/// [`start_blob_sweeper`] with an explicit interval.
pub fn start_blob_sweeper_every(interval: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut timer = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            timer.tick().await;
            if crate::workspace::paths::is_data_root_set() {
                sweep_blobs(BlobSweepOptions::default());
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        testing::scratch_data_root,
        workspace::blob::store::{BLOB_META_NAME, create_staging, put_blob_bytes, stat_blob},
    };

    fn id_of(out: &serde_json::Value) -> String {
        out["blobId"].as_str().unwrap().to_string()
    }

    #[test]
    fn removes_only_expired_blobs() {
        let _root = scratch_data_root();
        let old = id_of(&put_blob_bytes(b"old-bytes", "a", "b", 1_000, None).unwrap());
        let fresh = id_of(&put_blob_bytes(b"fresh", "a", "b", 1_000 + BLOB_TTL_MS, None).unwrap());

        let now = 1_000 + BLOB_TTL_MS - 1;
        assert_eq!(sweep_blobs(BlobSweepOptions { now: Some(now), ..Default::default() }).removed, 0);

        let now = 1_000 + BLOB_TTL_MS;
        let swept = sweep_blobs(BlobSweepOptions { now: Some(now), ..Default::default() });
        assert_eq!(swept, BlobSweepResult { removed: 1, reclaimed_bytes: 9 });
        assert!(stat_blob(&old, now, None).is_err());
        assert!(stat_blob(&fresh, now, None).is_ok());
    }

    #[test]
    fn sweeps_one_namespace_and_handles_a_missing_root() {
        let _root = scratch_data_root();
        assert_eq!(sweep_blobs(BlobSweepOptions::default()), BlobSweepResult::default());
        assert!(!blob_root(None).exists());

        let a = id_of(&put_blob_bytes(b"a", "a", "b", 0, Some("wsa")).unwrap());
        let b = id_of(&put_blob_bytes(b"b", "a", "b", 0, Some("wsb")).unwrap());
        let now = BLOB_TTL_MS;
        let opts = BlobSweepOptions { now: Some(now), workspace_id: Some("wsa"), ..Default::default() };
        assert_eq!(sweep_blobs(opts).removed, 1);
        assert!(stat_blob(&a, now, Some("wsa")).is_err());
        assert!(stat_blob(&b, now, Some("wsb")).is_ok());
    }

    #[test]
    fn orphans_staging_files_and_loose_files() {
        let _root = scratch_data_root();
        let orphan = blob_root(None).join(format!("blob_{}", "a".repeat(32)));
        fs::create_dir_all(&orphan).unwrap();
        fs::write(orphan.join("content"), b"12345").unwrap();
        let staging = create_staging(None, None).unwrap();
        fs::write(staging.path(), b"abc").unwrap();
        let loose = blob_root(None).join("stray.txt");
        fs::write(&loose, b"x").unwrap();

        // Everything is fresh by mtime: only the loose file goes.
        let now = now_ms();
        let swept = sweep_blobs(BlobSweepOptions { now: Some(now), ..Default::default() });
        assert_eq!(swept, BlobSweepResult { removed: 1, reclaimed_bytes: 1 });
        assert!(orphan.exists() && staging.path().exists() && !loose.exists());

        // An hour on, the orphan and the staged file are reclaimed.
        let swept = sweep_blobs(BlobSweepOptions { now: Some(now + BLOB_TTL_MS + 60_000), ..Default::default() });
        assert_eq!(swept, BlobSweepResult { removed: 2, reclaimed_bytes: 8 });
        assert!(!orphan.exists() && !staging.path().exists());
    }

    #[test]
    fn metadata_less_directory_uses_mtime_not_a_stale_meta_name() {
        let _root = scratch_data_root();
        let dir = blob_root(None).join(format!("blob_{}", "b".repeat(32)));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(BLOB_META_NAME), "not json").unwrap();
        let now = now_ms();
        assert_eq!(sweep_blobs(BlobSweepOptions { now: Some(now), ..Default::default() }).removed, 0);
        assert_eq!(
            sweep_blobs(BlobSweepOptions { now: Some(now + BLOB_TTL_MS + 60_000), ..Default::default() }).removed,
            1
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_sweeper_waits_one_interval_then_sweeps() {
        let _root = scratch_data_root();
        let id = id_of(&put_blob_bytes(b"x", "a", "b", 0, None).unwrap());
        let handle = start_blob_sweeper_every(Duration::from_secs(10));
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(blob_root(None).join(&id).exists());
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert!(!blob_root(None).join(&id).exists());
        handle.abort();
    }
}
