//! `export.formats`, `export.create`, `import.preflight` and `import.commit`.
//!
//! Content crosses the wire as blobs: an export is published as an artifact the client pulls, an import reads
//! the blob the client uploaded. A failure to export or import is data (`{ ok: false, error }`), not an error.

use serde_json::{Value, json};

use super::{
    registry::{all_formats, get_entry},
    types::{ExportFile, ImportSource, Importer, Strategy},
};
use crate::{
    kernel::CommandRegistry,
    protocol::{EngineError, Result},
    runtime::failed,
    utils::{
        id::now_ms,
        json::{opt_str, str_of},
    },
    workspace::blob::{
        BlobError, blob_content_path, describe_blob_error, publish_artifact_checked, read_blob_bytes, stat_blob,
    },
};

fn failure(error: EngineError) -> Value {
    failed(&error)
}

/// A failure the importers and exporters report as text, keyed by what the text says. Their own sentences
/// ("Not a valid HAR file", "File not found") get a key each; the rest is the raw text of a parser or the
/// file system (`SyntaxError: ...`, `YAMLException: ...`, `Error: ...`) under a wrapper key.
// shortcut: the key is read back from the text (the importers fail with Strings), keep in step with their sentences.
fn failure_of(text: String) -> Value {
    let plain = text.strip_prefix("Error: ").unwrap_or(&text);
    let (key, params) =
        if let Some(what) = plain.strip_prefix("Not a valid ").and_then(|rest| rest.strip_suffix(" file")) {
            ("import.not_valid_file", json!({ "what": what }))
        } else {
            let key = match plain {
                "Not a valid OpenAPI specification" => "import.not_openapi",
                "Not a Postman collection" => "import.not_postman_collection",
                "File not found" => "import.file_not_found",
                "Workspace not found" => "export.workspace_not_found",
                _ if text.starts_with("SyntaxError: ") => "import.syntax_error",
                _ if text.starts_with("YAMLException: ") => "import.yaml_error",
                _ => "import.failed",
            };
            (key, json!({ "detail": text }))
        };
    failure(EngineError::bad_request(text.clone()).keyed(key, params))
}

/// A blob that could not be read or published, under a wrapper key.
fn blob_failure(err: &BlobError) -> Value {
    let text = describe_blob_error(err);
    failure(EngineError::bad_request(text.clone()).keyed("import.blob_failed", json!({ "detail": text })))
}

fn export_create(params: &Value) -> Value {
    let (kind, format) = (str_of(params, "kind"), str_of(params, "format"));
    let ws_id = str_of(params, "workspaceId");
    let Some(entry) = get_entry(kind, format) else {
        return failure(
            EngineError::bad_request(format!("No export format \"{format}\" for kind \"{kind}\"."))
                .keyed("export.format_unknown", json!({ "format": format, "kind": kind })),
        );
    };
    let Some(render) = entry.exporter else {
        return failure(
            EngineError::bad_request(format!(
                "\"{}\" is a format Bifurc imports. An export is Bifurc JSON.",
                entry.label
            ))
            .keyed("export.import_only", json!({ "label": entry.label })),
        );
    };
    let requested_name = opt_str(params, "filename").filter(|name| !name.is_empty());
    let ExportFile { content, suggested_name } = match render(ws_id) {
        Ok(file) => file,
        Err(error) => return failure_of(error),
    };
    match publish_artifact_checked(&content, requested_name.unwrap_or(suggested_name), "application/json", Some(ws_id))
    {
        Ok(artifact) => artifact,
        Err(err) => blob_failure(&err),
    }
}

/// The uploaded blob as text, with the filename the client gave it.
fn read_source(blob_id: &str, ws_id: &str, filename: &str) -> std::result::Result<ImportSource, String> {
    let bytes = read_blob_bytes(blob_id, Some(ws_id)).map_err(|e| describe_blob_error(&e))?;
    Ok(ImportSource { content: String::from_utf8_lossy(&bytes).into_owned(), filename: filename.to_string(), bytes })
}

fn import_preflight(params: &Value) -> Value {
    let (kind, format) = (str_of(params, "kind"), str_of(params, "format"));
    let (ws_id, blob_id) = (str_of(params, "workspaceId"), str_of(params, "blobId"));
    let Some(entry) = get_entry(kind, format) else {
        return failure(
            EngineError::bad_request(format!("No import format \"{format}\" for kind \"{kind}\"."))
                .keyed("import.format_unknown", json!({ "format": format, "kind": kind })),
        );
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
        Err(error) => failure_of(error),
    }
}

async fn import_commit(params: &Value) -> Value {
    let (kind, format) = (str_of(params, "kind"), str_of(params, "format"));
    let (ws_id, blob_id) = (str_of(params, "workspaceId"), str_of(params, "blobId"));
    let Some(entry) = get_entry(kind, format) else {
        return failure(
            EngineError::bad_request(format!("No import format \"{format}\" for kind \"{kind}\"."))
                .keyed("import.format_unknown", json!({ "format": format, "kind": kind })),
        );
    };
    let strategy = Strategy::parse(opt_str(params, "collisionStrategy"));

    let meta = match stat_blob(blob_id, now_ms(), Some(ws_id)) {
        Ok(meta) => meta,
        Err(err) => return blob_failure(&err),
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
        Err(error) => failure_of(error),
    }
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("contract.fromOpenApi", |params, _| {
        super::importers::requests::contract_operations(str_of(&params, "text"))
            .map_err(|text| EngineError::bad_request(text.clone()).keyed("import.failed", json!({ "detail": text })))
    });
    registry.register_sync("export.formats", |_, _| Ok(all_formats()));
    registry.register_sync("export.create", |params, _| Ok(export_create(&params)));
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
        assert_eq!(
            out,
            json!({
                "ok": false, "error": "No export format \"nope\" for kind \"mocks\".",
                "errorKey": "export.format_unknown", "errorParams": { "format": "nope", "kind": "mocks" },
            })
        );
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
        assert_eq!(formats["mocks"][0]["id"], "mocks-json");

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

    /// Imports every file of the directory `BIFURC_IMPORT_SAMPLES` names through the commands, and prints what
    /// each became. A file's format is the start of its name: `postman-…`, `insomnia-…`, `openapi-…`,
    /// `swagger…`, `har-…`, `curl-…`. Run with `cargo test --lib real_exports -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "needs a directory of real exports"]
    async fn real_exports_of_other_tools_import() {
        let Ok(dir) = std::env::var("BIFURC_IMPORT_SAMPLES") else { return };
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|entry| entry.path()).collect();
        files.sort();
        let mut failed = Vec::new();
        for file in files {
            let name = file.file_name().unwrap().to_string_lossy().into_owned();
            let (kind, format) = match name.split(['-', '.']).next().unwrap_or("") {
                "postman" if name.contains("env") => ("environments", "environments-postman"),
                "postman" => ("requests", "requests-postman"),
                "insomnia" => ("requests", "requests-insomnia"),
                "openapi" | "swagger" | "swagger2" => ("requests", "requests-openapi"),
                "har" => ("requests", "requests-har"),
                "curl" => ("requests", "requests-curl"),
                "wiremock" => ("mocks", "mocks-wiremock"),
                _ => continue,
            };
            let root = scratch_data_root();
            let ws = root.ws_id;
            let (registry, ctx) = engine();
            let bytes = std::fs::read(&file).unwrap();
            let staged = put_blob(
                &name,
                "application/octet-stream",
                bytes.len() as i64,
                &STANDARD.encode(&bytes),
                now_ms(),
                Some(ws),
            )
            .unwrap();
            let payload = json!({ "kind": kind, "format": format, "workspaceId": ws, "blobId": staged["blobId"] });
            let pre = registry.invoke("import.preflight", payload.clone(), ctx.clone()).await.unwrap();
            let done = registry.invoke("import.commit", payload, ctx.clone()).await.unwrap();
            let mut counts = Vec::new();
            for (store_kind, ty) in crate::workspace::tree::Scope::Requests.types() {
                let stored = read_all_entities(ws, store_kind);
                if !stored.is_empty() {
                    counts.push(format!("{ty}={}", stored.len()));
                }
                for request in &stored {
                    if str_of(request, "name").is_empty() {
                        failed.push(format!("{name}: a {ty} request without a name: {request}"));
                    }
                }
                if std::env::var("BIFURC_IMPORT_DUMP").is_ok_and(|wanted| name.contains(&wanted)) {
                    println!("{}", serde_json::to_string_pretty(&stored).unwrap());
                }
            }
            let folders = crate::workspace::tree::folders(ws, crate::workspace::tree::Scope::Requests).len();
            let variables = crate::workspace::fs::read_entity(ws, "environments", "__global__")
                .map_or(0, |global| crate::utils::json::array_of(&global, "variables").len());
            println!(
                "{name}: preflight={} done={done} [{}] folders={folders} variables={variables}",
                pre["itemCount"],
                counts.join(" ")
            );
            if pre["ok"] != true || done["ok"] != true || pre["itemCount"] != done["imported"] {
                failed.push(format!("{name}: preflight {pre}, commit {done}"));
            }
        }
        assert!(failed.is_empty(), "{}", failed.join("\n"));
    }

    #[tokio::test]
    async fn a_format_of_another_tool_is_imported_but_not_exported() {
        let root = scratch_data_root();
        let ws = root.ws_id;
        let (registry, ctx) = engine();
        for (kind, format) in
            [("requests", "requests-postman"), ("workspace", "workspace-zip"), ("mocks", "mocks-wiremock")]
        {
            let out = registry
                .invoke("export.create", json!({ "kind": kind, "format": format, "workspaceId": ws }), ctx.clone())
                .await
                .unwrap();
            assert_eq!(out["ok"], false, "{format}: {out}");
            assert!(out["error"].as_str().unwrap().contains("An export is Bifurc JSON"), "{out}");
        }

        // A Postman collection goes in as requests of their types, and comes out as Bifurc JSON.
        let collection = json!({ "info": { "name": "C", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
            "item": [{ "name": "Ping", "request": { "method": "GET", "url": "http://h/ping" } },
                { "name": "Feed", "request": { "method": "GET", "url": "ws://h/feed" } }] })
        .to_string();
        let blob = upload(ws, "c.json", &collection);
        let payload = json!({ "kind": "requests", "format": "requests-postman", "workspaceId": ws, "blobId": blob });
        let pre = registry.invoke("import.preflight", payload.clone(), ctx.clone()).await.unwrap();
        assert_eq!(pre, json!({ "ok": true, "itemCount": 2, "collisionIds": [] }));
        let done = registry.invoke("import.commit", payload, ctx.clone()).await.unwrap();
        assert_eq!(done, json!({ "ok": true, "imported": 2 }));

        let artifact = registry
            .invoke("export.create", json!({ "kind": "requests", "format": "requests-json", "workspaceId": ws }), ctx)
            .await
            .unwrap();
        assert_eq!(artifact["suggestedName"], "requests-export.json");
        let exported = String::from_utf8(STANDARD.decode(artifact["inline"].as_str().unwrap()).unwrap()).unwrap();
        let file: Value = serde_json::from_str(&exported).unwrap();
        let types: Vec<&str> = file["requests"].as_array().unwrap().iter().map(|r| str_of(r, "type")).collect();
        assert_eq!(types, ["rest", "websocket"]);
    }
}
