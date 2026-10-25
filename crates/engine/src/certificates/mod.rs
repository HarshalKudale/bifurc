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
        assert!(key_pem.starts_with("-----BEGIN PRIVATE KEY-----"));

        let identity = identify_cert(&cert_pem).unwrap();
        assert_eq!(identity, generated.identity);
        assert!(identity.fingerprint.starts_with("sha256:") && identity.fingerprint.len() == 7 + 64);
        assert_eq!(identity.fingerprint, identity.fingerprint.to_lowercase());
        assert_eq!(identity.thumbprint_sha1.len(), 40);
        assert!(identity.subject.starts_with("CN=Bifurc CA "), "{}", identity.subject);
        assert!(identity.subject.ends_with("\nC=US\nST=Development\nL=Local\nO=Bifurc CA"), "{}", identity.subject);
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

        // Reloading clears the cache.
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

    /// A CA written by an earlier version: RSA 2048, PKCS#1 PEM key.
    const LEGACY_RSA_CA_CERT: &str = "-----BEGIN CERTIFICATE-----
MIIDIDCCAgigAwIBAgIUDFSoeEIFONTvHVN4+CKAoxMuR/IwDQYJKoZIhvcNAQEL
BQAwGDEWMBQGA1UEAwwNTGVnYWN5IFJTQSBDQTAgFw0yNjEwMTUxNTU4MDZaGA8y
MTI2MDkyMTE1NTgwNlowGDEWMBQGA1UEAwwNTGVnYWN5IFJTQSBDQTCCASIwDQYJ
KoZIhvcNAQEBBQADggEPADCCAQoCggEBAPRUiuytxviTOXP9XzA+RM6jnhb0g8mT
Zi6nIc5z2cpIvQ+fWUCkX831czzV/ozt/8IWw0PUCfcjJ9KVUK54fgkn93oAEQ9J
PqC/Djm73e4vjYxZnB0UWGtWq+lFBD5JefSQdpizEwi+vKJUf+e05v2QKGJL5SdV
E5Lxz4LZRMm//KLY4J1BWuLWPLXR/V+9wO0Gkmuna+iYbRdnAz3Lh682vazXnsZd
tlLB0/Zx3WMBwxFHOEtcTiMgXEDJXxgH7FfKr97uAy5LyVsTyH+WukrzbpDMQAD+
WNpd4gqO91clbEZNYhgd3dJneniDV3mFlApCtPKvlySXm50VcppZbckCAwEAAaNg
MF4wHQYDVR0OBBYEFFJcOBFEZxZYfAETRLpsDxwP/XOGMB8GA1UdIwQYMBaAFFJc
OBFEZxZYfAETRLpsDxwP/XOGMA8GA1UdEwEB/wQFMAMBAf8wCwYDVR0PBAQDAgIE
MA0GCSqGSIb3DQEBCwUAA4IBAQDAULK377aYmN6MkjaSxuJdnh+63IYyMbxAo5ZV
aGtEORJFpkw1ILdhrcYXDhSDZ3uEPNEk/ezWvC9Ll18ASL7j6lzbhMbNDoWFfkBQ
S9dFFwnwU2c/i8KyT9i3GVEuy6RxKF8+jo3yboTi3IN+Myz12woAOpE1C1AThx0b
517Z98U/HQQtsHyCUVhWegVUER+IcO4Iet11D1OXfG8Al2P0M1QUr8OIZ8KD3k3g
eAFkBQehkLgfOGPQF7lw8maLPhV6HxLzP5xzS4wMstShJjYr+vWRBrpP4CH46uh2
A+jWXuWpKh3tXlOiVfsLTDL1n+NLRkEbRsjj7YxjPcPNkcF8
-----END CERTIFICATE-----
";
    const LEGACY_RSA_CA_KEY: &str = "-----BEGIN RSA PRIVATE KEY-----
MIIEowIBAAKCAQEA9FSK7K3G+JM5c/1fMD5EzqOeFvSDyZNmLqchznPZyki9D59Z
QKRfzfVzPNX+jO3/whbDQ9QJ9yMn0pVQrnh+CSf3egARD0k+oL8OObvd7i+NjFmc
HRRYa1ar6UUEPkl59JB2mLMTCL68olR/57Tm/ZAoYkvlJ1UTkvHPgtlEyb/8otjg
nUFa4tY8tdH9X73A7QaSa6dr6JhtF2cDPcuHrza9rNeexl22UsHT9nHdYwHDEUc4
S1xOIyBcQMlfGAfsV8qv3u4DLkvJWxPIf5a6SvNukMxAAP5Y2l3iCo73VyVsRk1i
GB3d0md6eINXeYWUCkK08q+XJJebnRVymlltyQIDAQABAoIBABsf3BzupWNQ99y/
PQ8uaIXevvUaL9vD4gENo6iHbr8g4GFQvdA591tWc5TGRohHf0BhNcwMyQ7vT5NW
qYJ3BLn4PoaEJaCyb1IZ58bDyx1vWMnjJFxLlw3SjUr8Mx6FmnUKySnGXpd20lr8
ocb7y7WMaJlSBuZ9EMtcTj7e5z602riOSf9400dU2fdG0d1r/P6L8kQdntP+vjAb
TLRiqDEiTGx+ftLchEUh8zY4f+8Elut76nbOOWqOJuBcpFCdcBL8DIoj779B8cU9
uXvCFBrMqOuOeK60p1M6L4tSAZlpUTDEYn4ECFsOxdoQ1F82ndKBRlviXbDsX0Jf
B/dA8iECgYEA/4PBNKiprS9JdMW+YE3VYoDok3EiWk9OnsnaGkzVSuP3viONyG7J
4O3w2Kvunf0HkctjQ1CBXspIxqHwt2Yg++FDcQjxDtBdXJDHk3rhkGNKJw5NIBS1
XQWJfFiBesD8tXkNTPmuTRV+EBal+TFC7zsQ23cA8Vil3e4lvDzqNiECgYEA9MtZ
d7Qw1rNh8v8c628PFu12gi9XpCczHP9NOJ0H0+cwQNOnBmOiYNtjxjGdlVSn/K0Z
4ryqiaHTAh7RK/w0vNbPdr1gWID4R8tgHC19jXxMrOCZ1MyQ+SBRANSOHuKBuyPO
IMKZefXr9x+Cwf5RQOYdbuqzAzAKsvBMHLGBcqkCgYA4eVF5pxGhkoMEyTPzqyBP
njhJZkb2ePLii7CmAXdfDxXRs6ABveLm0RBbg1zBFIzYv4BX/3IolkMRi/ddQbhm
k3pJgHVvFcSc0e8afJ1gQWHBUhrfzVPslG23ywQLMp5oKbIz4jeRF//95FOYyTeH
EqH+L8ZIkZRPgfRbL4eAoQKBgF67eyoZ73SHAvbYx0ZNOlezGdOToqUCDoPAQRAR
pP/aThAoWFDeqe41uMzwUn2Bv2jHTGc0gAK3P0NozaXp+jbCIpNfAjP95RdTmyQM
brQcUEfuu1lguRHP8ckSeNV27qGPX6vJ4z8Rbnm6Fg4g7yzgYK+V4CTxqJJK8AmC
oAfRAoGBAIJxZALFtImYwvRENAkGlgZ5YzsVL44DGjER28kXiIcqI64kisx3dZ0/
NKTBJxK7QCfDcsI1kk5Oc1P8vA6N8Kj1cDAlkC81V/MTKo0VQmVaQDA80BrF2oQv
FVs1an5osA4S/Ox+lQY4PPy81x+qkBQ4U8kS8dirD0Bu+cjkU2Ew
-----END RSA PRIVATE KEY-----
";

    #[test]
    fn a_legacy_rsa_ca_still_loads_and_signs_host_certificates() {
        let _root = scratch_data_root();
        std::fs::write(ca_cert_path(), LEGACY_RSA_CA_CERT).unwrap();
        std::fs::write(ca_key_path(), LEGACY_RSA_CA_KEY).unwrap();
        assert!(LEGACY_RSA_CA_KEY.starts_with("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(load_ca(&ca_cert_path(), &ca_key_path()));
        let ca_der = manager::first_cert_der(LEGACY_RSA_CA_CERT).unwrap();
        assert!(handshake(host_server_config("example.com").unwrap(), &ca_der, "example.com"));
        unload_ca();
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
            json!({
                "ok": false, "error": "No CA certificate found.", "errorKey": "tls.no_ca", "errorParams": {},
            })
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
        assert_eq!(missing["errorKey"], "tls.failed");

        assert_eq!(call("tls.removeCert", json!({})).await, json!({ "ok": true, "engineRemoved": true }));
    }
}
