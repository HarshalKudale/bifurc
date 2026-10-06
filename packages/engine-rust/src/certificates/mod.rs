//! The interception CA: generated, stored, reported and exchanged by the `tls.*` commands, and loaded for the
//! proxy to sign per-host certificates with.

mod commands;
mod host_certs;
mod manager;

pub use host_certs::{
    MAX_CACHE_SIZE, apply_settings, clear_cert_cache, host_server_config, is_ca_loaded, listener_server_config,
    load_ca, unload_ca,
};
pub use manager::{
    CertIdentity, GeneratedCa, ca_cert_path, ca_dir, ca_key_path, fingerprint_of, generate_ca, get_cert_status,
    identify_cert, remove_ca,
};

use crate::kernel::CommandRegistry;

pub fn register(registry: &mut CommandRegistry) {
    commands::register(registry);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rustls::{ClientConfig, ClientConnection, RootCertStore, ServerConnection, pki_types::ServerName};
    use serde_json::json;

    use super::{host_certs::cached_hosts, manager::write_ca, *};
    use crate::{
        kernel::CommandContext,
        testing::scratch_data_root,
        workspace::blob::{BLOB_INLINE_THRESHOLD_BYTES, put_blob_bytes},
    };

    /// Complete a TLS handshake in memory; `true` when the client accepted the server's certificate.
    fn handshake(server: Arc<rustls::ServerConfig>, ca_der: &[u8], name: &str) -> bool {
        let mut roots = RootCertStore::empty();
        roots.add(ca_der.to_vec().into()).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let client = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let mut client =
            ClientConnection::new(Arc::new(client), ServerName::try_from(name.to_string()).unwrap()).unwrap();
        let mut server = ServerConnection::new(server).unwrap();
        for _ in 0..10 {
            let mut buf = Vec::new();
            client.write_tls(&mut buf).unwrap();
            if !buf.is_empty() {
                server.read_tls(&mut buf.as_slice()).unwrap();
                if server.process_new_packets().is_err() {
                    return false;
                }
            }
            let mut buf = Vec::new();
            server.write_tls(&mut buf).unwrap();
            if !buf.is_empty() {
                client.read_tls(&mut buf.as_slice()).unwrap();
                if client.process_new_packets().is_err() {
                    return false;
                }
            }
            if !client.is_handshaking() && !server.is_handshaking() {
                return true;
            }
        }
        false
    }

    #[test]
    fn a_generated_ca_is_stored_identified_loaded_and_signs_host_certificates() {
        let root = scratch_data_root();
        let dir = root.dir.path().to_path_buf();
        assert_eq!(get_cert_status(&dir), (false, None));

        let key = manager::generate_ca_key().unwrap();
        let generated = write_ca(&dir, &key).unwrap();
        assert_eq!(generated.cert_path, ca_cert_path());
        assert_eq!(generated.key_path, ca_key_path());

        let cert_pem = std::fs::read_to_string(&generated.cert_path).unwrap();
        let key_pem = std::fs::read_to_string(&generated.key_path).unwrap();
        assert!(cert_pem.starts_with("-----BEGIN CERTIFICATE-----"));
        assert!(key_pem.starts_with("-----BEGIN RSA PRIVATE KEY-----"));

        let identity = identify_cert(&cert_pem).unwrap();
        assert_eq!(identity, generated.identity);
        assert!(identity.fingerprint.starts_with("sha256:") && identity.fingerprint.len() == 7 + 64);
        assert_eq!(identity.fingerprint, identity.fingerprint.to_lowercase());
        assert_eq!(identity.thumbprint_sha1.len(), 40);
        assert_eq!(identity.subject, "CN=Bifurc CA\nC=US\nST=Development\nL=Local\nO=Bifurc CA");
        assert_eq!(get_cert_status(&dir), (true, Some(identity.fingerprint.clone())));

        let ca_der = manager::first_cert_der(&cert_pem).unwrap();
        assert!(!is_ca_loaded());
        assert!(matches!(host_server_config("example.com"), Err(e) if e == "CA not loaded"));
        assert!(load_ca(&ca_cert_path(), &ca_key_path()));
        assert!(is_ca_loaded());

        let config = host_server_config("example.com").unwrap();
        assert!(Arc::ptr_eq(&config, &host_server_config("example.com").unwrap()));
        assert!(handshake(config.clone(), &ca_der, "example.com"));
        assert!(!handshake(config, &ca_der, "other.com"));
        assert!(handshake(host_server_config("127.0.0.1").unwrap(), &ca_der, "127.0.0.1"));

        // A PKCS#8 key (what other tools write) loads as well.
        let pkcs8 = key.to_pkcs8_pem(LineEnding::LF).unwrap();
        std::fs::write(ca_key_path(), pkcs8.as_bytes()).unwrap();
        assert!(load_ca(&ca_cert_path(), &ca_key_path()));
        assert!(cached_hosts().is_empty());
        assert!(handshake(host_server_config("example.com").unwrap(), &ca_der, "example.com"));

        // The cache drops its oldest entry at the limit and can be cleared.
        host_certs::host_server_config_limited("a.test", 2).unwrap();
        host_certs::host_server_config_limited("b.test", 2).unwrap();
        host_certs::host_server_config_limited("c.test", 2).unwrap();
        assert_eq!(cached_hosts(), ["b.test", "c.test"]);
        clear_cert_cache();
        assert!(cached_hosts().is_empty());

        unload_ca();
        assert!(!is_ca_loaded());
        assert!(host_server_config("example.com").is_err());
        assert!(!load_ca(&dir.join("nope.pem"), &ca_key_path()));

        assert!(remove_ca(&dir));
        assert!(!remove_ca(&dir));
        assert_eq!(get_cert_status(&dir), (false, None));
    }

    #[test]
    fn status_reports_unreadable_certificates_without_a_fingerprint() {
        let root = scratch_data_root();
        std::fs::write(ca_cert_path(), "not a certificate").unwrap();
        std::fs::write(ca_key_path(), "x").unwrap();
        assert_eq!(get_cert_status(root.dir.path()), (true, None));
        assert!(identify_cert("not a certificate").is_none());
    }

    #[tokio::test]
    async fn commands_report_status_import_export_and_remove() {
        let root = scratch_data_root();
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        let registry = Arc::new(registry);
        let ctx = CommandContext::internal(registry.clone());
        let call = |action: &'static str, params: serde_json::Value| {
            let (registry, ctx) = (registry.clone(), ctx.clone());
            async move { registry.invoke(action, params, ctx).await.unwrap() }
        };

        assert_eq!(call("tls.certStatus", json!({})).await, json!({ "generated": false, "fingerprint": null }));
        assert_eq!(
            call("tls.exportCert", json!({})).await,
            json!({ "ok": false, "error": "No CA certificate found." })
        );
        assert_eq!(call("tls.removeCert", json!({})).await, json!({ "ok": true, "engineRemoved": false }));

        let key = manager::generate_ca_key().unwrap();
        let generated = write_ca(root.dir.path(), &key).unwrap();
        let status = call("tls.certStatus", json!({})).await;
        assert_eq!(status, json!({ "generated": true, "fingerprint": generated.identity.fingerprint }));

        let exported = call("tls.exportCert", json!({})).await;
        assert_eq!(exported["suggestedName"], "bifurc-ca.pem");
        assert_eq!(exported["mimeType"], "application/x-pem-file");
        assert!(exported["size"].as_u64().unwrap() <= BLOB_INLINE_THRESHOLD_BYTES as u64);

        let blob = put_blob_bytes(b"imported cert", "c.pem", "application/x-pem-file", 1, None).unwrap();
        let blob_id = blob["blobId"].as_str().unwrap();
        assert_eq!(call("tls.importCert", json!({ "blobId": blob_id })).await, json!({ "ok": true }));
        assert_eq!(std::fs::read(ca_cert_path()).unwrap(), b"imported cert");
        assert_eq!(call("tls.importKey", json!({ "blobId": blob_id })).await, json!({ "ok": true }));
        assert_eq!(std::fs::read(ca_key_path()).unwrap(), b"imported cert");

        let missing = call("tls.importCert", json!({ "blobId": format!("blob_{}", "0".repeat(32)) })).await;
        assert_eq!(missing["ok"], false);
        assert!(missing["error"].as_str().unwrap().contains("No such blob"));

        assert_eq!(call("tls.removeCert", json!({})).await, json!({ "ok": true, "engineRemoved": true }));
    }
}
