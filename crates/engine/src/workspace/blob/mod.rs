//! The blob store: staging for file uploads and downloads, the `blob.*` commands, the expiry sweep and the
//! inline-or-blob publishing of artifacts.

mod commands;
mod publish;
mod store;
mod sweep;

pub use commands::register;
pub use publish::{publish_artifact, publish_artifact_bytes, publish_artifact_bytes_checked, publish_artifact_checked};
pub use store::{
    BLOB_CONTENT_NAME, BLOB_DIR_NAME, BLOB_INLINE_THRESHOLD_BYTES, BLOB_MAX_INGRESS_BYTES, BLOB_META_NAME,
    BLOB_READ_CHUNK_BYTES, BLOB_STAGING_DIR_NAME, BLOB_TTL_MS, BlobError, BlobErrorCode, BlobResult, StagingHandle,
    assert_ingress_within_limit, blob_content_path, blob_root, create_staging, decode_base64_lenient,
    describe_blob_error, put_blob, put_blob_bytes, read_blob, read_blob_bytes, read_meta_from_dir, release_blob,
    sha256_of, sha256_of_file, staging_root, stat_blob,
};
pub use sweep::{
    BLOB_SWEEP_INTERVAL_MS, BlobSweepOptions, BlobSweepResult, start_blob_sweeper, start_blob_sweeper_every,
    sweep_blobs,
};
