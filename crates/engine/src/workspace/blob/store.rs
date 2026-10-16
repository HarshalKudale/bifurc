//! The blob store: the staging area for file transfers between a client and the engine.
//!
//! Layout, shared with the Node engine:
//!
//! ```text
//! <dataDir>/blobs[/<workspaceId>]/
//!   <blobId>/content     the raw bytes, a real file
//!   <blobId>/meta.json   filename, mimeType, size, sha256, createdAt
//!   .staging/stage_<hex> in-flight writes from `create_staging`
//! ```
//!
//! `meta.json` is written last, so a write that dies midway leaves a directory every read path reports as
//! not found. `createdAt` anchors a sliding lease: [`stat_blob`] reports what is left of it, [`read_blob`]
//! extends it on every read that returns bytes.

use std::{
    fmt,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use base64::{
    Engine as _, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig, general_purpose::STANDARD},
};
use rand::RngCore;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{protocol::EngineError, workspace::paths::data_dir};

/// Artifacts at or below this size are returned inline rather than as a blob.
pub const BLOB_INLINE_THRESHOLD_BYTES: usize = 1024 * 1024;
/// Hard cap on a single upload or staged payload.
pub const BLOB_MAX_INGRESS_BYTES: u64 = 100 * 1024 * 1024;
/// How long an unreleased blob survives before the sweep collects it.
pub const BLOB_TTL_MS: i64 = 60 * 60 * 1000;
/// Chunk size `blob.read` uses when the caller gives no `length`.
pub const BLOB_READ_CHUNK_BYTES: u64 = 512 * 1024;

pub const BLOB_DIR_NAME: &str = "blobs";
pub const BLOB_CONTENT_NAME: &str = "content";
pub const BLOB_META_NAME: &str = "meta.json";
/// In-flight writes live here, one level below the blob root.
pub const BLOB_STAGING_DIR_NAME: &str = ".staging";

/// The blob layer's own error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobErrorCode {
    NotFound,
    InvalidId,
    TooLarge,
    SizeMismatch,
    InvalidOffset,
}

impl BlobErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            BlobErrorCode::NotFound => "blob-not-found",
            BlobErrorCode::InvalidId => "blob-invalid-id",
            BlobErrorCode::TooLarge => "blob-too-large",
            BlobErrorCode::SizeMismatch => "blob-size-mismatch",
            BlobErrorCode::InvalidOffset => "blob-invalid-offset",
        }
    }
}

/// A failure of the blob layer. `code` is `None` for a plain failure (disk I/O, a reused staging handle).
///
/// On the wire it is a plain engine error carrying only the message; [`describe_blob_error`] is the
/// `"<code>: <message>"` form used in `{ ok: false, error }` results.
#[derive(Debug, Clone)]
pub struct BlobError {
    pub code: Option<BlobErrorCode>,
    pub message: String,
}

impl BlobError {
    pub fn new(code: BlobErrorCode, message: impl Into<String>) -> Self {
        Self { code: Some(code), message: message.into() }
    }

    pub fn plain(message: impl Into<String>) -> Self {
        Self { code: None, message: message.into() }
    }
}

impl fmt::Display for BlobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BlobError {}

impl From<std::io::Error> for BlobError {
    fn from(err: std::io::Error) -> Self {
        Self::plain(err.to_string())
    }
}

impl From<BlobError> for EngineError {
    fn from(err: BlobError) -> Self {
        let key = match err.code {
            Some(BlobErrorCode::NotFound) => "blob.not_found",
            Some(BlobErrorCode::InvalidId) => "blob.invalid_id",
            Some(BlobErrorCode::TooLarge) => "blob.too_large",
            Some(BlobErrorCode::SizeMismatch) => "blob.size_mismatch",
            Some(BlobErrorCode::InvalidOffset) => "blob.invalid_offset",
            None => "blob.failed",
        };
        EngineError::engine(err.message.clone()).keyed(key, json!({ "detail": err.message }))
    }
}

/// A blob failure as the `error` string of a result: `"<code>: <message>"`, or the bare message.
pub fn describe_blob_error(err: &BlobError) -> String {
    match err.code {
        Some(code) => format!("{}: {}", code.as_str(), err.message),
        None => err.message.clone(),
    }
}

pub type BlobResult<T> = std::result::Result<T, BlobError>;

/// The blob root, optionally namespaced by workspace: `<dataDir>/blobs` or `<dataDir>/blobs/<workspaceId>`.
pub fn blob_root(workspace_id: Option<&str>) -> PathBuf {
    let root = data_dir().join(BLOB_DIR_NAME);
    match workspace_id {
        None => root,
        Some(ws) => root.join(ws),
    }
}

/// Where [`create_staging`] puts in-flight writes.
pub fn staging_root(workspace_id: Option<&str>) -> PathBuf {
    blob_root(workspace_id).join(BLOB_STAGING_DIR_NAME)
}

fn is_blob_id(blob_id: &str) -> bool {
    blob_id
        .strip_prefix("blob_")
        .is_some_and(|hex| hex.len() == 32 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

fn assert_blob_id(blob_id: &str) -> BlobResult<()> {
    if is_blob_id(blob_id) {
        return Ok(());
    }
    Err(BlobError::new(
        BlobErrorCode::InvalidId,
        format!(
            "Not a valid blob id: {}. Expected \"blob_\" followed by 32 hex characters.",
            serde_json::to_string(blob_id).unwrap_or_default()
        ),
    ))
}

/// A workspace id becomes one path segment; anything that could leave the blob root is refused.
fn assert_workspace_segment(workspace_id: Option<&str>) -> BlobResult<()> {
    if let Some(ws) = workspace_id
        && (ws == "." || ws == ".." || ws.contains(['/', '\\', '\0']))
    {
        return Err(BlobError::new(
            BlobErrorCode::InvalidId,
            format!("Not a valid workspace id: {}.", serde_json::to_string(ws).unwrap_or_default()),
        ));
    }
    Ok(())
}

fn blob_dir(blob_id: &str, workspace_id: Option<&str>) -> BlobResult<PathBuf> {
    assert_blob_id(blob_id)?;
    assert_workspace_segment(workspace_id)?;
    Ok(blob_root(workspace_id).join(blob_id))
}

/// Absolute path of a blob's bytes, for callers that need a real file (zip import/export).
///
/// `workspace_id` must be the value the blob was staged under. An in-process read does not refresh the lease.
pub fn blob_content_path(blob_id: &str, workspace_id: Option<&str>) -> BlobResult<PathBuf> {
    Ok(blob_dir(blob_id, workspace_id)?.join(BLOB_CONTENT_NAME))
}

fn write_meta(dir: &Path, meta: &Value) -> std::io::Result<()> {
    std::fs::write(dir.join(BLOB_META_NAME), serde_json::to_string(meta)?)
}

fn meta_value(blob_id: &str, filename: &str, mime_type: &str, size: u64, sha256: &str, created_at: i64) -> Value {
    json!({
        "blobId": blob_id,
        "filename": filename,
        "mimeType": mime_type,
        "size": size,
        "sha256": sha256,
        "createdAt": created_at,
    })
}

fn not_found(message: String) -> BlobError {
    BlobError::new(BlobErrorCode::NotFound, message)
}

/// A blob's metadata, or `blob-not-found` when it is absent, unreadable, inconsistent or has lost its content.
fn read_meta(blob_id: &str, workspace_id: Option<&str>) -> BlobResult<Value> {
    let dir = blob_dir(blob_id, workspace_id)?;
    let raw = std::fs::read_to_string(dir.join(BLOB_META_NAME))
        .map_err(|_| not_found(format!("No such blob: {blob_id}. It may have been released or swept.")))?;
    if !dir.join(BLOB_CONTENT_NAME).exists() {
        return Err(not_found(format!("Blob {blob_id} has no content — it may be mid-write or damaged.")));
    }
    let meta: Value =
        serde_json::from_str(&raw).map_err(|_| not_found(format!("Blob {blob_id} has unreadable metadata.")))?;
    let consistent = meta.get("blobId").and_then(Value::as_str) == Some(blob_id)
        && meta.get("size").is_some_and(Value::is_number)
        && meta.get("sha256").is_some_and(Value::is_string);
    if !consistent {
        return Err(not_found(format!("Blob {blob_id} has inconsistent metadata.")));
    }
    Ok(meta)
}

/// Best-effort metadata read from a blob directory; `None` when absent or not blob metadata.
pub fn read_meta_from_dir(dir: &Path) -> Option<Value> {
    let meta: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(BLOB_META_NAME)).ok()?).ok()?;
    if meta.get("size").is_some_and(Value::is_number) && meta.get("sha256").is_some_and(Value::is_string) {
        Some(meta)
    } else {
        None
    }
}

fn meta_size(meta: &Value) -> u64 {
    meta.get("size").and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f.max(0.0) as u64))).unwrap_or(0)
}

fn meta_created_at(meta: &Value) -> i64 {
    meta.get("createdAt").and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))).unwrap_or(0)
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_of(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// SHA-256 of a file on disk, read in bounded chunks.
pub fn sha256_of_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn new_blob_id() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    format!("blob_{}", hex::encode(bytes))
}

/// The pre-decode half of the ingress rules: `declared` must be a non-negative integer within `max_bytes`,
/// and the base64 string short enough that it could decode to under the cap. Checked before anything is
/// allocated or decoded.
pub fn assert_ingress_within_limit(declared: i64, base64_length: usize, max_bytes: u64) -> BlobResult<()> {
    if declared < 0 {
        return Err(BlobError::new(
            BlobErrorCode::SizeMismatch,
            format!("Declared size must be a non-negative integer, got {declared}."),
        ));
    }
    if declared as u64 > max_bytes {
        return Err(BlobError::new(
            BlobErrorCode::TooLarge,
            format!("Declared size {declared} bytes exceeds the {max_bytes}-byte ingress limit."),
        ));
    }
    let max_base64_length = max_bytes.div_ceil(3) * 4 + 4;
    if base64_length as u64 > max_base64_length {
        return Err(BlobError::new(
            BlobErrorCode::TooLarge,
            format!(
                "Payload of {base64_length} base64 characters cannot decode to under the {max_bytes}-byte ingress limit."
            ),
        ));
    }
    Ok(())
}

/// Decode base64 the way Node's `Buffer.from(s, "base64")` does: characters outside the alphabet are
/// skipped, `-`/`_` count as `+`/`/`, padding ends the data, and a dangling sixth bit is dropped.
pub fn decode_base64_lenient(data: &str) -> Vec<u8> {
    let mut cleaned = String::with_capacity(data.len());
    for ch in data.chars() {
        match ch {
            '=' => break,
            'A'..='Z' | 'a'..='z' | '0'..='9' | '+' | '/' => cleaned.push(ch),
            '-' => cleaned.push('+'),
            '_' => cleaned.push('/'),
            _ => {}
        }
    }
    if cleaned.len() % 4 == 1 {
        cleaned.pop();
    }
    let config = GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true);
    GeneralPurpose::new(&alphabet::STANDARD, config).decode(cleaned).unwrap_or_default()
}

/// Stage an upload (`blob.put`) and return `{ blobId, sha256 }`.
///
/// `declared_size` is checked before the payload is decoded, and the decoded length must equal it. `now`
/// is the lease anchor; `workspace_id` selects the namespace (`None` is the host-wide root).
pub fn put_blob(
    filename: &str,
    mime_type: &str,
    declared_size: i64,
    data: &str,
    now: i64,
    workspace_id: Option<&str>,
) -> BlobResult<Value> {
    assert_ingress_within_limit(declared_size, data.encode_utf16().count(), BLOB_MAX_INGRESS_BYTES)?;
    let bytes = decode_base64_lenient(data);
    if bytes.len() as i64 != declared_size {
        return Err(BlobError::new(
            BlobErrorCode::SizeMismatch,
            format!("Declared size {declared_size} bytes but decoded {} bytes.", bytes.len()),
        ));
    }
    write_blob(&bytes, filename, mime_type, now, workspace_id)
}

fn write_blob(
    bytes: &[u8],
    filename: &str,
    mime_type: &str,
    now: i64,
    workspace_id: Option<&str>,
) -> BlobResult<Value> {
    assert_workspace_segment(workspace_id)?;
    let blob_id = new_blob_id();
    let dir = blob_root(workspace_id).join(&blob_id);
    std::fs::create_dir_all(&dir)?;
    let sha256 = sha256_of(bytes);
    std::fs::write(dir.join(BLOB_CONTENT_NAME), bytes)?;
    write_meta(&dir, &meta_value(&blob_id, filename, mime_type, bytes.len() as u64, &sha256, now))?;
    Ok(json!({ "blobId": blob_id, "sha256": sha256 }))
}

/// Stage bytes the engine produced itself. Only the size cap applies. Returns `{ blobId, sha256 }`.
pub fn put_blob_bytes(
    bytes: &[u8],
    filename: &str,
    mime_type: &str,
    now: i64,
    workspace_id: Option<&str>,
) -> BlobResult<Value> {
    if bytes.len() as u64 > BLOB_MAX_INGRESS_BYTES {
        return Err(BlobError::new(
            BlobErrorCode::TooLarge,
            format!("Artifact of {} bytes exceeds the {BLOB_MAX_INGRESS_BYTES}-byte limit.", bytes.len()),
        ));
    }
    write_blob(bytes, filename, mime_type, now, workspace_id)
}

/// Metadata plus the remaining lease (`blob.stat`). Does not extend the lease.
pub fn stat_blob(blob_id: &str, now: i64, workspace_id: Option<&str>) -> BlobResult<Value> {
    let meta = read_meta(blob_id, workspace_id)?;
    let mut out = serde_json::Map::new();
    out.insert("size".into(), meta.get("size").cloned().unwrap_or(Value::Null));
    for key in ["mimeType", "sha256", "filename"] {
        if let Some(value) = meta.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    out.insert("ttlRemainingMs".into(), json!((meta_created_at(&meta) + BLOB_TTL_MS - now).max(0)));
    Ok(Value::Object(out))
}

/// One base64 slice of a blob (`blob.read`): `{ data, eof }`. A read that returns bytes extends the lease.
pub fn read_blob(
    blob_id: &str,
    offset: Option<u64>,
    length: Option<u64>,
    now: i64,
    workspace_id: Option<&str>,
) -> BlobResult<Value> {
    let meta = read_meta(blob_id, workspace_id)?;
    let size = meta_size(&meta);
    let offset = offset.unwrap_or(0);
    let length = length.unwrap_or(BLOB_READ_CHUNK_BYTES);
    if offset > size {
        return Err(BlobError::new(
            BlobErrorCode::InvalidOffset,
            format!("Offset {offset} is past the end of blob {blob_id} ({size} bytes)."),
        ));
    }
    let want = length.min(size - offset);
    let mut file = File::open(blob_dir(blob_id, workspace_id)?.join(BLOB_CONTENT_NAME))?;
    file.seek(SeekFrom::Start(offset))?;
    let mut slice = Vec::new();
    file.take(want).read_to_end(&mut slice)?;
    let read = slice.len() as u64;
    if read > 0 {
        refresh_lease(blob_id, &meta, now, workspace_id);
    }
    Ok(json!({ "data": STANDARD.encode(&slice), "eof": offset + read >= size }))
}

/// Extend the lease by rewriting `createdAt`. Best effort: a failure never fails the read.
fn refresh_lease(blob_id: &str, meta: &Value, now: i64, workspace_id: Option<&str>) {
    let mut next = meta.clone();
    next["createdAt"] = json!(now);
    if let Ok(dir) = blob_dir(blob_id, workspace_id) {
        let _ = write_meta(&dir, &next);
    }
}

/// The whole content of a blob, for in-process consumers. Does not extend the lease.
pub fn read_blob_bytes(blob_id: &str, workspace_id: Option<&str>) -> BlobResult<Vec<u8>> {
    read_meta(blob_id, workspace_id)?;
    Ok(std::fs::read(blob_content_path(blob_id, workspace_id)?)?)
}

/// Delete a staged blob (`blob.release`): `{ ok }`, false when there was nothing to release.
pub fn release_blob(blob_id: &str, workspace_id: Option<&str>) -> BlobResult<Value> {
    let dir = blob_dir(blob_id, workspace_id)?;
    if !dir.exists() {
        return Ok(json!({ "ok": false }));
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(json!({ "ok": true }))
}

/// A file to write into, and the two ways to finish: [`commit`](StagingHandle::commit) or
/// [`discard`](StagingHandle::discard). An abandoned handle is reclaimed by the sweep.
pub struct StagingHandle {
    path: PathBuf,
    max_bytes: u64,
    workspace_id: Option<String>,
    settled: bool,
}

impl StagingHandle {
    /// Absolute path to write the payload to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Hash, measure and move the staged file into the store. Single-use. Returns `{ blobId, sha256 }`.
    pub fn commit(&mut self, filename: &str, mime_type: &str, now: i64) -> BlobResult<Value> {
        if self.settled {
            return Err(BlobError::plain("createStaging(): this handle has already been committed or discarded."));
        }
        self.settled = true;

        let size = std::fs::metadata(&self.path)?.len();
        if size > self.max_bytes {
            let _ = std::fs::remove_file(&self.path);
            return Err(BlobError::new(
                BlobErrorCode::TooLarge,
                format!("Staged payload of {size} bytes exceeds the {}-byte ingress limit.", self.max_bytes),
            ));
        }
        let sha256 = sha256_of_file(&self.path)?;
        let blob_id = new_blob_id();
        let target = blob_root(self.workspace_id.as_deref()).join(&blob_id);
        std::fs::create_dir_all(&target)?;
        std::fs::rename(&self.path, target.join(BLOB_CONTENT_NAME))?;
        write_meta(&target, &meta_value(&blob_id, filename, mime_type, size, &sha256, now))?;
        Ok(json!({ "blobId": blob_id, "sha256": sha256 }))
    }

    /// Abandon the staged file. A no-op after `commit` or a second `discard`.
    pub fn discard(&mut self) {
        if self.settled {
            return;
        }
        self.settled = true;
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Open a staging handle under `workspace_id`'s namespace. `max_bytes` overrides the ingress cap.
pub fn create_staging(max_bytes: Option<u64>, workspace_id: Option<&str>) -> BlobResult<StagingHandle> {
    assert_workspace_segment(workspace_id)?;
    let dir = staging_root(workspace_id);
    std::fs::create_dir_all(&dir)?;
    let mut random = [0u8; 16];
    rand::rng().fill_bytes(&mut random);
    Ok(StagingHandle {
        path: dir.join(format!("stage_{}", hex::encode(random))),
        max_bytes: max_bytes.unwrap_or(BLOB_MAX_INGRESS_BYTES),
        workspace_id: workspace_id.map(str::to_string),
        settled: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch_data_root;

    fn code(err: &BlobError) -> &'static str {
        err.code.map(BlobErrorCode::as_str).unwrap_or("")
    }

    fn put(bytes: &[u8], now: i64, ws: Option<&str>) -> String {
        let out = put_blob("a.bin", "application/octet-stream", bytes.len() as i64, &STANDARD.encode(bytes), now, ws)
            .unwrap();
        out["blobId"].as_str().unwrap().to_string()
    }

    #[test]
    fn put_stat_read_round_trip_in_chunks() {
        let _root = scratch_data_root();
        let bytes: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let out = put_blob("a.bin", "x/y", 1000, &STANDARD.encode(&bytes), 5_000, None).unwrap();
        let id = out["blobId"].as_str().unwrap();
        assert!(is_blob_id(id));
        assert_eq!(out["sha256"], sha256_of(&bytes));

        let stat = stat_blob(id, 5_000 + 1_000, None).unwrap();
        assert_eq!(stat["size"], 1000);
        assert_eq!(stat["filename"], "a.bin");
        assert_eq!(stat["mimeType"], "x/y");
        assert_eq!(stat["sha256"], sha256_of(&bytes));
        assert_eq!(stat["ttlRemainingMs"], BLOB_TTL_MS - 1_000);
        assert_eq!(stat_blob(id, 5_000 + 2 * BLOB_TTL_MS, None).unwrap()["ttlRemainingMs"], 0);

        let mut got = Vec::new();
        let mut offset = 0u64;
        loop {
            let part = read_blob(id, Some(offset), Some(300), 6_000, None).unwrap();
            let chunk = STANDARD.decode(part["data"].as_str().unwrap()).unwrap();
            offset += chunk.len() as u64;
            got.extend(chunk);
            if part["eof"] == true {
                break;
            }
        }
        assert_eq!(got, bytes);
        assert_eq!(sha256_of(&got), out["sha256"]);
        assert_eq!(read_blob_bytes(id, None).unwrap(), bytes);
    }

    #[test]
    fn meta_file_format_matches_the_node_store() {
        let _root = scratch_data_root();
        let id = put(b"hello", 42, None);
        let raw = std::fs::read_to_string(blob_root(None).join(&id).join(BLOB_META_NAME)).unwrap();
        assert_eq!(
            raw,
            format!(
                r#"{{"blobId":"{id}","filename":"a.bin","mimeType":"application/octet-stream","size":5,"sha256":"{}","createdAt":42}}"#,
                sha256_of(b"hello")
            )
        );
    }

    #[test]
    fn read_validates_offset_and_extends_the_lease_only_when_bytes_return() {
        let _root = scratch_data_root();
        let id = put(b"abcdef", 100, None);
        let err = read_blob(&id, Some(7), None, 200, None).unwrap_err();
        assert_eq!(code(&err), "blob-invalid-offset");
        assert!(err.message.contains("past the end"));

        let end = read_blob(&id, Some(6), None, 300, None).unwrap();
        assert_eq!(end["data"], "");
        assert_eq!(end["eof"], true);
        assert_eq!(stat_blob(&id, 100, None).unwrap()["ttlRemainingMs"], BLOB_TTL_MS);

        read_blob(&id, Some(0), Some(2), 500, None).unwrap();
        assert_eq!(stat_blob(&id, 500, None).unwrap()["ttlRemainingMs"], BLOB_TTL_MS);
        let part = read_blob(&id, Some(0), Some(2), 500, None).unwrap();
        assert_eq!(part["eof"], false);
    }

    #[test]
    fn put_validates_sizes() {
        let _root = scratch_data_root();
        let err = put_blob("a", "b", 3, &STANDARD.encode(b"abcd"), 0, None).unwrap_err();
        assert_eq!(code(&err), "blob-size-mismatch");
        assert_eq!(err.message, "Declared size 3 bytes but decoded 4 bytes.");

        let err = put_blob("a", "b", BLOB_MAX_INGRESS_BYTES as i64 + 1, "AAAA", 0, None).unwrap_err();
        assert_eq!(code(&err), "blob-too-large");

        let err = put_blob("a", "b", -1, "AAAA", 0, None).unwrap_err();
        assert_eq!(code(&err), "blob-size-mismatch");

        let err = assert_ingress_within_limit(10, 100, 10).unwrap_err();
        assert_eq!(code(&err), "blob-too-large");
        assert!(err.message.contains("base64 characters"));
        assert!(assert_ingress_within_limit(10, 16, 10).is_ok());
    }

    #[test]
    fn lenient_base64_matches_node() {
        assert_eq!(decode_base64_lenient("aGVsbG8="), b"hello");
        assert_eq!(decode_base64_lenient("aGVs bG8"), b"hello");
        assert_eq!(decode_base64_lenient("aGVsbG8=junk"), b"hello");
        assert_eq!(decode_base64_lenient("_-8"), [0xff, 0xef]);
    }

    #[test]
    fn unknown_and_invalid_ids() {
        let _root = scratch_data_root();
        let missing = format!("blob_{}", "0".repeat(32));
        let err = stat_blob(&missing, 0, None).unwrap_err();
        assert_eq!(code(&err), "blob-not-found");
        assert_eq!(err.message, format!("No such blob: {missing}. It may have been released or swept."));
        assert_eq!(
            describe_blob_error(&err),
            format!("blob-not-found: No such blob: {missing}. It may have been released or swept.")
        );
        assert_eq!(code(&read_blob(&missing, None, None, 0, None).unwrap_err()), "blob-not-found");

        let err = stat_blob("../../etc/passwd", 0, None).unwrap_err();
        assert_eq!(code(&err), "blob-invalid-id");
        assert_eq!(code(&release_blob("BLOB_x", None).unwrap_err()), "blob-invalid-id");
        assert_eq!(code(&stat_blob(&missing, 0, Some("..")).unwrap_err()), "blob-invalid-id");

        let wire: EngineError = err.into();
        assert_eq!(wire.code.as_str(), "ENGINE_ERROR");
    }

    #[test]
    fn missing_content_reads_as_not_found() {
        let _root = scratch_data_root();
        let id = put(b"abc", 0, None);
        std::fs::remove_file(blob_root(None).join(&id).join(BLOB_CONTENT_NAME)).unwrap();
        assert_eq!(code(&stat_blob(&id, 0, None).unwrap_err()), "blob-not-found");
    }

    #[test]
    fn release_reports_whether_anything_was_there() {
        let _root = scratch_data_root();
        let id = put(b"abc", 0, None);
        assert_eq!(release_blob(&id, None).unwrap(), json!({ "ok": true }));
        assert_eq!(release_blob(&id, None).unwrap(), json!({ "ok": false }));
        assert_eq!(code(&stat_blob(&id, 0, None).unwrap_err()), "blob-not-found");
    }

    #[test]
    fn blobs_are_scoped_to_their_workspace() {
        let _root = scratch_data_root();
        let id = put(b"abc", 0, Some("ws1"));
        assert!(blob_root(Some("ws1")).join(&id).is_dir());
        assert!(stat_blob(&id, 0, Some("ws1")).is_ok());
        assert_eq!(code(&stat_blob(&id, 0, Some("ws2")).unwrap_err()), "blob-not-found");
        assert_eq!(code(&stat_blob(&id, 0, None).unwrap_err()), "blob-not-found");
        assert_eq!(release_blob(&id, Some("ws2")).unwrap(), json!({ "ok": false }));
        assert_eq!(release_blob(&id, Some("ws1")).unwrap(), json!({ "ok": true }));
    }

    #[test]
    fn staging_commits_and_discards() {
        let _root = scratch_data_root();
        let mut handle = create_staging(None, None).unwrap();
        std::fs::write(handle.path(), b"zipbytes").unwrap();
        let out = handle.commit("x.zip", "application/zip", 7).unwrap();
        let id = out["blobId"].as_str().unwrap();
        assert_eq!(out["sha256"], sha256_of(b"zipbytes"));
        assert!(!handle.path().exists());
        assert_eq!(read_blob_bytes(id, None).unwrap(), b"zipbytes");
        assert!(handle.commit("x", "y", 0).is_err());
        handle.discard();

        let mut big = create_staging(Some(3), None).unwrap();
        std::fs::write(big.path(), b"toolong").unwrap();
        let err = big.commit("x", "y", 0).unwrap_err();
        assert_eq!(code(&err), "blob-too-large");
        assert!(!big.path().exists());

        let mut gone = create_staging(None, Some("ws1")).unwrap();
        std::fs::write(gone.path(), b"x").unwrap();
        gone.discard();
        assert!(!gone.path().exists());
    }
}
