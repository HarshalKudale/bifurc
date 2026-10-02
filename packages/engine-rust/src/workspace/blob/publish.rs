//! Inline-or-blob publishing for every command that returns an artifact.
//!
//! Content at or below `BLOB_INLINE_THRESHOLD_BYTES` goes out inline as base64; larger content is staged
//! as a blob the client pulls with `blob.read`, scoped to the same workspace. `sha256` is present on both.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use super::store::{BLOB_INLINE_THRESHOLD_BYTES, BlobResult, put_blob_bytes, sha256_of};
use crate::{protocol::Result, utils::id::now_ms};

/// [`publish_artifact`] with the blob layer's own error, for callers that report `{ ok: false, error }` via
/// `describe_blob_error`.
pub fn publish_artifact_checked(
    content: &str,
    suggested_name: &str,
    mime_type: &str,
    workspace_id: Option<&str>,
) -> BlobResult<Value> {
    publish_artifact_bytes_checked(content.as_bytes(), suggested_name, mime_type, workspace_id)
}

/// Publish text as an `ArtifactResult`: `inline` (base64) when small, `blobId` otherwise.
pub fn publish_artifact(
    content: &str,
    suggested_name: &str,
    mime_type: &str,
    workspace_id: Option<&str>,
) -> Result<Value> {
    Ok(publish_artifact_checked(content, suggested_name, mime_type, workspace_id)?)
}

/// [`publish_artifact`] for binary content.
pub fn publish_artifact_bytes(
    bytes: &[u8],
    suggested_name: &str,
    mime_type: &str,
    workspace_id: Option<&str>,
) -> Result<Value> {
    Ok(publish_artifact_bytes_checked(bytes, suggested_name, mime_type, workspace_id)?)
}

/// [`publish_artifact_bytes`] with the blob layer's own error.
pub fn publish_artifact_bytes_checked(
    bytes: &[u8],
    suggested_name: &str,
    mime_type: &str,
    workspace_id: Option<&str>,
) -> BlobResult<Value> {
    let sha256 = sha256_of(bytes);
    if bytes.len() <= BLOB_INLINE_THRESHOLD_BYTES {
        return Ok(json!({
            "ok": true,
            "inline": STANDARD.encode(bytes),
            "suggestedName": suggested_name,
            "size": bytes.len(),
            "mimeType": mime_type,
            "sha256": sha256,
        }));
    }
    let staged = put_blob_bytes(bytes, suggested_name, mime_type, now_ms(), workspace_id)?;
    Ok(json!({
        "ok": true,
        "blobId": staged["blobId"],
        "suggestedName": suggested_name,
        "size": bytes.len(),
        "mimeType": mime_type,
        "sha256": sha256,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        testing::scratch_data_root,
        workspace::blob::store::{read_blob_bytes, release_blob},
    };

    #[test]
    fn small_content_is_inline() {
        let _root = scratch_data_root();
        let out = publish_artifact("hello", "a.txt", "text/plain", None).unwrap();
        assert_eq!(
            out,
            json!({
                "ok": true,
                "inline": "aGVsbG8=",
                "suggestedName": "a.txt",
                "size": 5,
                "mimeType": "text/plain",
                "sha256": sha256_of(b"hello"),
            })
        );
        assert!(out.get("blobId").is_none());
        let keys: Vec<_> = out.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["ok", "inline", "suggestedName", "size", "mimeType", "sha256"]);
    }

    #[test]
    fn content_at_the_threshold_is_inline_and_one_byte_over_is_a_blob() {
        let _root = scratch_data_root();
        let at = "a".repeat(BLOB_INLINE_THRESHOLD_BYTES);
        let out = publish_artifact(&at, "n", "text/plain", Some("ws1")).unwrap();
        assert!(out.get("inline").is_some() && out.get("blobId").is_none());

        let over = "b".repeat(BLOB_INLINE_THRESHOLD_BYTES + 1);
        let out = publish_artifact(&over, "big.txt", "text/plain", Some("ws1")).unwrap();
        assert!(out.get("inline").is_none());
        assert_eq!(out["size"], BLOB_INLINE_THRESHOLD_BYTES + 1);
        assert_eq!(out["sha256"], sha256_of(over.as_bytes()));
        let keys: Vec<_> = out.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["ok", "blobId", "suggestedName", "size", "mimeType", "sha256"]);

        let id = out["blobId"].as_str().unwrap();
        assert_eq!(read_blob_bytes(id, Some("ws1")).unwrap(), over.as_bytes());
        assert!(read_blob_bytes(id, None).is_err());
        assert_eq!(release_blob(id, Some("ws1")).unwrap(), json!({ "ok": true }));
    }
}
