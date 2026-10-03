//! `export.formats`, `export.create`, `import.preflight` and `import.commit`.
//!
//! Content crosses the wire as blobs: an export is published as an artifact the client pulls, an import reads
//! the blob the client uploaded. A failure to export or import is data (`{ ok: false, error }`), not an error.

use serde_json::{Value, json};

use super::{
    registry::{all_formats, get_entry},
    types::{ExportFile, Exporter, FormatEntry, ImportSource, Importer, Strategy},
};
use crate::{
    kernel::CommandRegistry,
    protocol::Result,
    utils::{
        id::now_ms,
        json::{opt_str, str_of},
    },
    workspace::blob::{
        BlobError, blob_content_path, create_staging, describe_blob_error, publish_artifact_checked, read_blob_bytes,
        stat_blob,
    },
};

fn failure(error: impl Into<String>) -> Value {
    json!({ "ok": false, "error": error.into() })
}

fn mime_type_for(entry: &FormatEntry) -> &'static str {
    match entry.extensions.first().copied().unwrap_or("") {
        "json" | "har" => "application/json",
        "zip" => "application/zip",
        "yaml" | "yml" => "application/yaml",
        "env" | "txt" | "sh" | "curl" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// `proxyRules` -> `proxy-rules-export.json`.
fn default_name_for(kind: &str, entry: &FormatEntry) -> String {
    let mut kebab = String::new();
    for c in kind.chars() {
        if c.is_ascii_uppercase() {
            kebab.push('-');
        }
        kebab.push(c);
    }
    format!("{}-export.{}", kebab.to_lowercase(), entry.extensions.first().copied().unwrap_or("bin"))
}

async fn export_create(params: &Value) -> Value {
    let (kind, format) = (str_of(params, "kind"), str_of(params, "format"));
    let ws_id = str_of(params, "workspaceId");
    let Some(entry) = get_entry(kind, format) else {
        return failure(format!("No export format \"{format}\" for kind \"{kind}\"."));
    };
    let requested_name = opt_str(params, "filename").filter(|name| !name.is_empty());
    let mime_type = mime_type_for(entry);

    match &entry.exporter {
        Exporter::Text(render) => {
            let ExportFile { content, suggested_name } = match render(ws_id) {
                Ok(file) => file,
                Err(error) => return failure(error),
            };
            match publish_artifact_checked(&content, requested_name.unwrap_or(suggested_name), mime_type, Some(ws_id)) {
                Ok(artifact) => artifact,
                Err(err) => failure(describe_blob_error(&err)),
            }
        }
        Exporter::Path(write) => match publish_staged(*write, ws_id, requested_name, kind, entry, mime_type).await {
            Ok(artifact) | Err(artifact) => artifact,
        },
    }
}

/// Run a file-writing exporter into a staging file and commit it as a blob. Both arms carry the result.
async fn publish_staged(
    write: fn(&str, &std::path::Path) -> std::result::Result<(), String>,
    ws_id: &str,
    requested_name: Option<&str>,
    kind: &str,
    entry: &FormatEntry,
    mime_type: &str,
) -> std::result::Result<Value, Value> {
    let blob_failure = |err: BlobError| failure(describe_blob_error(&err));
    let mut staging = create_staging(None, Some(ws_id)).map_err(blob_failure)?;

    let (path, workspace) = (staging.path().to_path_buf(), ws_id.to_string());
    let written = tokio::task::spawn_blocking(move || write(&workspace, &path)).await;
    match written {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            staging.discard();
            return Err(failure(error));
        }
        Err(join) => {
            staging.discard();
            return Err(failure(join.to_string()));
        }
    }

    let size = match std::fs::metadata(staging.path()) {
        Ok(meta) => meta.len(),
        Err(err) => {
            staging.discard();
            return Err(failure(err.to_string()));
        }
    };
    let suggested_name = requested_name.map(str::to_string).unwrap_or_else(|| default_name_for(kind, entry));
    match staging.commit(&suggested_name, mime_type, now_ms()) {
        Ok(committed) => Ok(json!({
            "ok": true,
            "blobId": committed["blobId"],
            "suggestedName": suggested_name,
            "size": size,
            "mimeType": mime_type,
            "sha256": committed["sha256"],
        })),
        Err(err) => {
            staging.discard();
            Err(blob_failure(err))
        }
    }
}

/// The uploaded blob as text, with the filename the client gave it.
fn read_source(blob_id: &str, ws_id: &str, filename: &str) -> std::result::Result<ImportSource, String> {
    let bytes = read_blob_bytes(blob_id, Some(ws_id)).map_err(|e| describe_blob_error(&e))?;
    Ok(ImportSource { content: String::from_utf8_lossy(&bytes).into_owned(), filename: filename.to_string() })
}

fn import_preflight(params: &Value) -> Value {
    let (kind, format) = (str_of(params, "kind"), str_of(params, "format"));
    let (ws_id, blob_id) = (str_of(params, "workspaceId"), str_of(params, "blobId"));
    let Some(entry) = get_entry(kind, format) else {
        return failure(format!("No import format \"{format}\" for kind \"{kind}\"."));
    };
    let outcome = (|| {
        let meta = stat_blob(blob_id, now_ms(), Some(ws_id)).map_err(|e| describe_blob_error(&e))?;
        match &entry.importer {
            Importer::Text(importer) => {
                (importer.preflight)(ws_id, &read_source(blob_id, ws_id, str_of(&meta, "filename"))?)
            }
            Importer::Path(importer) => {
                let path = blob_content_path(blob_id, Some(ws_id)).map_err(|e| describe_blob_error(&e))?;
                (importer.preflight)(&path)
            }
        }
    })();
    match outcome {
        Ok(preflight) => preflight.to_value(),
        Err(error) => failure(error),
    }
}

async fn import_commit(params: &Value) -> Value {
    let (kind, format) = (str_of(params, "kind"), str_of(params, "format"));
    let (ws_id, blob_id) = (str_of(params, "workspaceId"), str_of(params, "blobId"));
    let Some(entry) = get_entry(kind, format) else {
        return failure(format!("No import format \"{format}\" for kind \"{kind}\"."));
    };
    let strategy = Strategy::parse(opt_str(params, "collisionStrategy"));

    let meta = match stat_blob(blob_id, now_ms(), Some(ws_id)) {
        Ok(meta) => meta,
        Err(err) => return failure(describe_blob_error(&err)),
    };
    let outcome = match &entry.importer {
        Importer::Text(importer) => match read_source(blob_id, ws_id, str_of(&meta, "filename")) {
            Ok(source) => (importer.run)(ws_id.to_string(), source, strategy).await,
            Err(error) => Err(error),
        },
        Importer::Path(importer) => match blob_content_path(blob_id, Some(ws_id)) {
            Ok(path) => (importer.run)(ws_id.to_string(), path, strategy).await,
            Err(err) => Err(describe_blob_error(&err)),
        },
    };
    match outcome {
        Ok(imported) => imported.to_value(),
        Err(error) => failure(error),
    }
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("export.formats", |_, _| Ok(all_formats()));
    registry.register("export.create", |params, _| async move { Result::Ok(export_create(&params).await) });
    registry.register_sync("import.preflight", |params, _| Ok(import_preflight(&params)));
    registry.register("import.commit", |params, _| async move { Result::Ok(import_commit(&params).await) });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base64::{Engine as _, engine::general_purpose::STANDARD};

    use super::*;
    use crate::{
        kernel::CommandContext,
        testing::scratch_data_root,
        workspace::{blob::put_blob, fs::read_all_entities},
    };

    fn engine() -> (Arc<CommandRegistry>, CommandContext) {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        let registry = Arc::new(registry);
        let ctx = CommandContext::internal(registry.clone());
        (registry, ctx)
    }

    fn upload(ws_id: &str, filename: &str, text: &str) -> String {
        let staged = put_blob(
            filename,
            "text/plain",
            text.len() as i64,
            &STANDARD.encode(text.as_bytes()),
            now_ms(),
            Some(ws_id),
        )
        .unwrap();
        staged["blobId"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn unknown_formats_are_reported_as_data() {
        let _root = scratch_data_root();
        let (registry, ctx) = engine();
        let out = registry
            .invoke("export.create", json!({ "kind": "mocks", "format": "nope", "workspaceId": "ws1" }), ctx.clone())
            .await
            .unwrap();
        assert_eq!(out, json!({ "ok": false, "error": "No export format \"nope\" for kind \"mocks\"." }));
        let out = registry
            .invoke("import.preflight", json!({ "kind": "x", "format": "y", "workspaceId": "ws1", "blobId": "b" }), ctx)
            .await
            .unwrap();
        assert_eq!(out["error"], "No import format \"y\" for kind \"x\".");
    }

    #[tokio::test]
    async fn an_export_is_published_inline_and_imports_back_through_a_blob() {
        let root = scratch_data_root();
        let ws = root.ws_id;
        let (registry, ctx) = engine();
        let formats = registry.invoke("export.formats", json!({}), ctx.clone()).await.unwrap();
        assert_eq!(formats["mocks"][1]["id"], "mocks-json");

        let mock = json!({ "id": "m1", "name": "M", "method": "GET", "urlPattern": "/x", "useRegex": false,
            "enabled": true, "responseStatus": 200, "responseHeaders": {}, "responseBody": "ok", "createdAt": 1,
            "workspaceId": ws });
        crate::workspace::fs::write_entity(ws, "mocks", "m1", &mock, None).unwrap();

        let artifact = registry
            .invoke("export.create", json!({ "kind": "mocks", "format": "mocks-json", "workspaceId": ws }), ctx.clone())
            .await
            .unwrap();
        assert_eq!(artifact["ok"], true);
        assert_eq!(artifact["suggestedName"], "mocks-export.json");
        assert_eq!(artifact["mimeType"], "application/json");
        let exported = String::from_utf8(STANDARD.decode(artifact["inline"].as_str().unwrap()).unwrap()).unwrap();
        assert!(exported.contains("\"lp-mocks-v1\""));

        let named = registry
            .invoke(
                "export.create",
                json!({ "kind": "mocks", "format": "mocks-json", "workspaceId": ws, "filename": "mine.json" }),
                ctx.clone(),
            )
            .await
            .unwrap();
        assert_eq!(named["suggestedName"], "mine.json");

        let blob = upload(ws, "mocks.json", &exported);
        let pre = registry
            .invoke(
                "import.preflight",
                json!({ "kind": "mocks", "format": "mocks-json", "workspaceId": ws, "blobId": blob }),
                ctx.clone(),
            )
            .await
            .unwrap();
        assert_eq!(pre, json!({ "ok": true, "itemCount": 1, "collisionIds": ["m1"] }));

        let done = registry
            .invoke(
                "import.commit",
                json!({ "kind": "mocks", "format": "mocks-json", "workspaceId": ws, "blobId": blob,
                    "collisionStrategy": "new" }),
                ctx.clone(),
            )
            .await
            .unwrap();
        assert_eq!(done, json!({ "ok": true, "imported": 1, "skipped": 0 }));
        assert_eq!(read_all_entities(ws, "mocks").len(), 2);

        let kept = registry
            .invoke(
                "import.commit",
                json!({ "kind": "mocks", "format": "mocks-json", "workspaceId": ws, "blobId": blob }),
                ctx.clone(),
            )
            .await
            .unwrap();
        assert_eq!(kept["skipped"], 1);

        let missing = registry
            .invoke(
                "import.commit",
                json!({ "kind": "mocks", "format": "mocks-json", "workspaceId": ws, "blobId": "0".repeat(32) }),
                ctx,
            )
            .await
            .unwrap();
        assert_eq!(missing["ok"], false);
        assert!(missing["error"].as_str().unwrap().starts_with("blob-"), "{missing}");
    }

    #[tokio::test]
    async fn the_zip_export_is_a_staged_blob() {
        let root = scratch_data_root();
        let ws = root.ws_id;
        let (registry, ctx) = engine();
        let out = registry
            .invoke("export.create", json!({ "kind": "workspace", "format": "workspace-zip", "workspaceId": ws }), ctx)
            .await
            .unwrap();
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["suggestedName"], "workspace-export.zip");
        assert_eq!(out["mimeType"], "application/zip");
        assert!(out["size"].as_u64().unwrap() > 0);
        let bytes = read_blob_bytes(out["blobId"].as_str().unwrap(), Some(ws)).unwrap();
        assert_eq!(&bytes[..2], b"PK");
        assert_eq!(out["sha256"], crate::workspace::blob::sha256_of(&bytes));
    }

    #[test]
    fn default_names_are_kebab_cased() {
        let entry = get_entry("proxyRules", "proxyrules-json").unwrap();
        assert_eq!(default_name_for("proxyRules", entry), "proxy-rules-export.json");
        assert_eq!(mime_type_for(get_entry("requests", "requests-curl").unwrap()), "text/plain");
        assert_eq!(mime_type_for(get_entry("requests", "requests-har").unwrap()), "application/json");
    }
}
