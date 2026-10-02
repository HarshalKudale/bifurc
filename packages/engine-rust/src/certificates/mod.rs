use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{kernel::CommandRegistry, workspace::settings::app_data_dir};

pub fn register(_registry: &mut CommandRegistry) {}

/// Where the engine keeps its own interception CA certificate.
pub fn ca_cert_path() -> PathBuf {
    app_data_dir().join("ca-cert.pem")
}

/// Where the engine keeps its own interception CA key.
pub fn ca_key_path() -> PathBuf {
    app_data_dir().join("ca-key.pem")
}

/// Load the interception CA from PEM files, replacing any loaded one. `false` if they cannot be used.
pub fn load_ca(_cert_path: &Path, _key_path: &Path) -> bool {
    false
}

pub fn unload_ca() {}

pub fn is_ca_loaded() -> bool {
    false
}

/// Forget every generated host certificate.
pub fn clear_cert_cache() {}

/// A TLS server configuration presenting a certificate for `hostname`, signed by the loaded CA. Generated on
/// first use and cached.
pub fn host_server_config(_hostname: &str) -> Result<Arc<rustls::ServerConfig>, String> {
    Err("CA not loaded".to_string())
}
