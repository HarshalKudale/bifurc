//! The loaded interception CA and the per-host certificates it signs for the proxy.

use std::{
    net::IpAddr,
    path::Path,
    str::FromStr,
    sync::{Arc, LazyLock},
};

use indexmap::IndexMap;
use parking_lot::Mutex;
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    SanType,
};
use rsa::{RsaPrivateKey, pkcs1::DecodeRsaPrivateKey, pkcs8::EncodePrivateKey};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use super::manager::{first_cert_der, mkcert_serial};

/// The most host certificates kept; the oldest is dropped to make room.
pub const MAX_CACHE_SIZE: usize = 500;
const HOST_VALIDITY_DAYS: i64 = 365;

struct LoadedCa {
    issuer: Issuer<'static, KeyPair>,
}

struct State {
    ca: Option<Arc<LoadedCa>>,
    cache: IndexMap<String, Arc<rustls::ServerConfig>>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new(State { ca: None, cache: IndexMap::new() }));

/// The CA key as the PKCS#8 DER ring signs with, from PKCS#1 or PKCS#8 PEM.
fn key_der_from_pem(pem: &str) -> Option<Vec<u8>> {
    let mut reader = pem.as_bytes();
    while let Ok(Some(item)) = rustls_pemfile::read_one(&mut reader) {
        match item {
            rustls_pemfile::Item::Pkcs8Key(key) => return Some(key.secret_pkcs8_der().to_vec()),
            rustls_pemfile::Item::Pkcs1Key(key) => {
                let rsa = RsaPrivateKey::from_pkcs1_der(key.secret_pkcs1_der()).ok()?;
                return Some(rsa.to_pkcs8_der().ok()?.as_bytes().to_vec());
            }
            _ => {}
        }
    }
    None
}

fn parse_ca(cert_pem: &str, key_pem: &str) -> Option<LoadedCa> {
    let cert_der = first_cert_der(cert_pem)?;
    let key_pair = KeyPair::try_from(key_der_from_pem(key_pem)?.as_slice()).ok()?;
    let issuer = Issuer::from_ca_cert_der(&CertificateDer::from(cert_der), key_pair).ok()?;
    Some(LoadedCa { issuer })
}

/// Load the interception CA from PEM files, replacing any loaded one and clearing the cache. `false` (with
/// no CA loaded) if the files cannot be read or are not a certificate with a usable RSA or EC key.
pub fn load_ca(cert_path: &Path, key_path: &Path) -> bool {
    let loaded = match (std::fs::read_to_string(cert_path), std::fs::read_to_string(key_path)) {
        (Ok(cert), Ok(key)) => parse_ca(&cert, &key),
        _ => None,
    };
    let mut state = STATE.lock();
    state.cache.clear();
    state.ca = loaded.map(Arc::new);
    state.ca.is_some()
}

/// Forget the loaded CA and every host certificate.
pub fn unload_ca() {
    let mut state = STATE.lock();
    state.ca = None;
    state.cache.clear();
}

pub fn is_ca_loaded() -> bool {
    STATE.lock().ca.is_some()
}

/// Forget every generated host certificate.
pub fn clear_cert_cache() {
    STATE.lock().cache.clear();
}

fn build_server_config(ca: &LoadedCa, hostname: &str) -> Result<rustls::ServerConfig, String> {
    let san = match IpAddr::from_str(hostname) {
        Ok(ip) => SanType::IpAddress(ip),
        Err(_) => SanType::DnsName(hostname.try_into().map_err(|e: rcgen::Error| e.to_string())?),
    };
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, hostname);
    params.distinguished_name = dn;
    params.subject_alt_names = vec![san];
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature, KeyUsagePurpose::KeyEncipherment];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth, ExtendedKeyUsagePurpose::ClientAuth];
    params.serial_number = Some(mkcert_serial());
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now;
    params.not_after = now + time::Duration::days(HOST_VALIDITY_DAYS);

    let key_pair = KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = params.signed_by(&key_pair, &ca.issuer).map_err(|e| e.to_string())?;
    let chain = vec![CertificateDer::from(cert.der().to_vec())];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|e| e.to_string())?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

/// [`host_server_config`] with the cache limit given, for tests.
pub(super) fn host_server_config_limited(
    hostname: &str,
    max_cache: usize,
) -> Result<Arc<rustls::ServerConfig>, String> {
    let mut state = STATE.lock();
    if let Some(config) = state.cache.get(hostname) {
        return Ok(config.clone());
    }
    let ca = state.ca.clone().ok_or_else(|| "CA not loaded".to_string())?;
    let config = Arc::new(build_server_config(&ca, hostname)?);
    if state.cache.len() >= max_cache {
        state.cache.shift_remove_index(0);
    }
    state.cache.insert(hostname.to_string(), config.clone());
    Ok(config)
}

/// A TLS server configuration presenting a certificate for `hostname`, signed by the loaded CA. Generated on
/// first use and cached.
pub fn host_server_config(hostname: &str) -> Result<Arc<rustls::ServerConfig>, String> {
    host_server_config_limited(hostname, MAX_CACHE_SIZE)
}

/// The hostnames with a cached certificate, oldest first.
#[cfg(test)]
pub(super) fn cached_hosts() -> Vec<String> {
    STATE.lock().cache.keys().cloned().collect()
}
