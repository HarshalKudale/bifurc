//! The format registry: every kind of entity and the file formats it is exported to and imported from.
//! Bifurc JSON is the one format a kind is exported to; the formats of other tools are only read. The order
//! here is the order `export.formats` lists them in.

use std::sync::LazyLock;

use serde_json::{Map, Value};

use super::{
    exporters, importers,
    types::{FormatEntry, Importer, PathImporter, TextImporter},
};

macro_rules! text_import {
    ($preflight:path, $run:path) => {
        Importer::Text(TextImporter {
            preflight: $preflight,
            run: |ws, source, strategy| Box::pin($run(ws, source, strategy)),
        })
    };
}

macro_rules! entry {
    ($kind:expr, $id:expr, $label:expr, $ext:expr, $exporter:expr, $importer:expr) => {
        FormatEntry { kind: $kind, id: $id, label: $label, extensions: &$ext, exporter: $exporter, importer: $importer }
    };
}

static FORMATS: LazyLock<Vec<FormatEntry>> = LazyLock::new(|| {
    use exporters::{environments as env_x, kinds as kinds_x, mocks as mocks_x, requests as req_x, workspace as ws_x};
    use importers::{environments as env_i, kinds as kinds_i, mocks as mocks_i, requests as req_i, workspace as ws_i};
    vec![
        entry!(
            "workspace",
            "workspace-json",
            "Bifurc JSON",
            ["json"],
            Some(ws_x::json),
            text_import!(ws_i::json_preflight, ws_i::json_run)
        ),
        entry!(
            "workspace",
            "workspace-zip",
            "ZIP Archive",
            ["zip"],
            None,
            Importer::Path(PathImporter {
                preflight: ws_i::zip_preflight,
                run: |ws, path, strategy| Box::pin(ws_i::zip_run(ws, path, strategy)),
            })
        ),
        entry!(
            "requests",
            "requests-json",
            "Bifurc JSON",
            ["json"],
            Some(req_x::json),
            text_import!(req_i::json_preflight, req_i::json_run)
        ),
        entry!(
            "requests",
            "requests-postman",
            "Postman Collection",
            ["json", "zip", "yaml", "yml"],
            None,
            text_import!(req_i::postman_preflight, req_i::postman_run)
        ),
        entry!(
            "requests",
            "requests-insomnia",
            "Insomnia",
            ["json", "yaml", "yml"],
            None,
            text_import!(req_i::insomnia_preflight, req_i::insomnia_run)
        ),
        entry!(
            "requests",
            "requests-openapi",
            "OpenAPI / Swagger",
            ["json", "yaml", "yml"],
            None,
            text_import!(req_i::openapi_preflight, req_i::openapi_run)
        ),
        entry!(
            "requests",
            "requests-curl",
            "cURL Commands",
            ["sh", "txt", "curl"],
            None,
            text_import!(req_i::curl_preflight, req_i::curl_run)
        ),
        entry!(
            "requests",
            "requests-har",
            "HAR 1.2",
            ["har", "json"],
            None,
            text_import!(req_i::har_preflight, req_i::har_run)
        ),
        entry!(
            "mocks",
            "mocks-json",
            "Bifurc JSON",
            ["json"],
            Some(mocks_x::json),
            text_import!(mocks_i::json_preflight, mocks_i::json_run)
        ),
        entry!(
            "mocks",
            "mocks-postman",
            "Postman Collection v2.1",
            ["json"],
            None,
            text_import!(mocks_i::postman_preflight, mocks_i::postman_run)
        ),
        entry!(
            "mocks",
            "mocks-wiremock",
            "WireMock Stubs",
            ["json"],
            None,
            text_import!(mocks_i::wiremock_preflight, mocks_i::wiremock_run)
        ),
        entry!(
            "environments",
            "environments-json",
            "Bifurc JSON",
            ["json"],
            Some(env_x::json),
            text_import!(env_i::json_preflight, env_i::json_run)
        ),
        entry!(
            "environments",
            "environments-postman",
            "Postman Environment",
            ["json"],
            None,
            text_import!(env_i::postman_preflight, env_i::postman_run)
        ),
        entry!(
            "environments",
            "environments-dotenv",
            "dotenv (.env)",
            ["env", "txt"],
            None,
            text_import!(env_i::dotenv_preflight, env_i::dotenv_run)
        ),
        entry!(
            "mappings",
            "mappings-json",
            "Bifurc JSON",
            ["json"],
            Some(kinds_x::mappings),
            text_import!(kinds_i::mappings_preflight, kinds_i::mappings_run)
        ),
        entry!(
            "proxyRules",
            "proxyrules-json",
            "Bifurc JSON",
            ["json"],
            Some(kinds_x::proxy_rules),
            text_import!(kinds_i::proxy_rules_preflight, kinds_i::proxy_rules_run)
        ),
    ]
});

/// The format `format_id` of entity kind `kind`.
pub fn get_entry(kind: &str, format_id: &str) -> Option<&'static FormatEntry> {
    FORMATS.iter().find(|entry| entry.kind == kind && entry.id == format_id)
}

/// Every kind with its formats (`export.formats`), kinds and formats in registration order.
pub fn all_formats() -> Value {
    let mut out: Map<String, Value> = Map::new();
    for entry in FORMATS.iter() {
        let list = out.entry(entry.kind.to_string()).or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(list) = list {
            list.push(entry.definition());
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_listed_by_kind_in_registration_order() {
        let all = all_formats();
        let kinds: Vec<_> = all.as_object().unwrap().keys().cloned().collect();
        assert_eq!(kinds, ["workspace", "requests", "mocks", "environments", "mappings", "proxyRules"]);
        let ids = |kind: &str| -> Vec<String> {
            all[kind].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap().to_string()).collect()
        };
        assert_eq!(ids("workspace"), ["workspace-json", "workspace-zip"]);
        assert_eq!(
            ids("requests"),
            ["requests-json", "requests-postman", "requests-insomnia", "requests-openapi", "requests-curl", "requests-har"]
        );
        assert_eq!(ids("mocks"), ["mocks-json", "mocks-postman", "mocks-wiremock"]);
        assert_eq!(ids("environments"), ["environments-json", "environments-postman", "environments-dotenv"]);
        assert_eq!(
            all["requests"][3],
            serde_json::json!({ "id": "requests-openapi", "label": "OpenAPI / Swagger", "extensions": ["json", "yaml", "yml"],
                "supportsExport": false, "supportsImport": true })
        );
        assert!(get_entry("mocks", "mocks-json").is_some());
        assert!(get_entry("mocks", "requests-curl").is_none());
    }

    #[test]
    fn only_bifurc_json_is_exported() {
        let all = all_formats();
        for (kind, formats) in all.as_object().unwrap() {
            let exported: Vec<&str> = formats
                .as_array()
                .unwrap()
                .iter()
                .filter(|f| f["supportsExport"] == true)
                .map(|f| f["label"].as_str().unwrap())
                .collect();
            assert_eq!(exported, ["Bifurc JSON"], "{kind}");
            assert!(formats.as_array().unwrap().iter().all(|f| f["supportsImport"] == true), "{kind}");
        }
    }
}
