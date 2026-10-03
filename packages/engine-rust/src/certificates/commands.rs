//! `tls.certStatus`, `tls.generate`, `tls.importCert`, `tls.importKey`, `tls.exportCert` and `tls.removeCert`.

use std::path::Path;

use serde_json::{Value, json};

use super::manager::{ca_cert_path, ca_dir, ca_key_path, generate_ca, get_cert_status, remove_ca};
use crate::{
    kernel::CommandRegistry,
    protocol::Result,
    utils::{id::now_ms, json::str_of},
    workspace::blob::{blob_content_path, describe_blob_error, publish_artifact, stat_blob},
};

pub fn register(registry: &mut CommandRegistry) {
    registry.register("tls.generate", |_, _| async move {
        let dir = ca_dir();
        let outcome = tokio::task::spawn_blocking(move || generate_ca(&dir))
            .await
            .map_err(|e| e.to_string())
            .and_then(|generated| generated);
        Ok(match outcome {
            Ok(generated) => json!({ "ok": true, "fingerprint": generated.identity.fingerprint }),
            Err(error) => json!({ "ok": false, "error": error }),
        })
    });

    registry.register_sync("tls.certStatus", |_, _| {
        let (generated, fingerprint) = get_cert_status(&ca_dir());
        Ok(json!({ "generated": generated, "fingerprint": fingerprint }))
    });

    registry.register_sync("tls.removeCert", |_, _| Ok(json!({ "ok": true, "engineRemoved": remove_ca(&ca_dir()) })));

    registry.register_sync("tls.exportCert", |_, _| export_cert());

    registry.register_sync("tls.importCert", |params, _| Ok(import(str_of(&params, "blobId"), &ca_cert_path())));

    registry.register_sync("tls.importKey", |params, _| Ok(import(str_of(&params, "blobId"), &ca_key_path())));
}

fn export_cert() -> Result<Value> {
    let Ok(content) = std::fs::read_to_string(ca_cert_path()) else {
        return Ok(json!({ "ok": false, "error": "No CA certificate found." }));
    };
    publish_artifact(&content, "bifurc-ca.pem", "application/x-pem-file", None)
}

/// Copy a staged blob over `dest`.
fn import(blob_id: &str, dest: &Path) -> Value {
    let copied = (|| {
        stat_blob(blob_id, now_ms(), None).map_err(|e| describe_blob_error(&e))?;
        let source = blob_content_path(blob_id, None).map_err(|e| describe_blob_error(&e))?;
        let bytes = std::fs::read(source).map_err(|e| e.to_string())?;
        std::fs::write(dest, bytes).map_err(|e| e.to_string())
    })();
    match copied {
        Ok(()) => json!({ "ok": true }),
        Err(error) => json!({ "ok": false, "error": error }),
    }
}
