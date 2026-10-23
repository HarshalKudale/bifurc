//! Test support: a scratch data root. The data root is process-wide, so tests that use the store take turns.

use std::sync::{Mutex, MutexGuard};

use serde_json::json;
use tempfile::TempDir;

use crate::workspace::{
    fs::init_workspace_dir,
    paths::set_data_root,
    settings::{load_settings, save_settings},
};

static STORE: Mutex<()> = Mutex::new(());

/// A temporary data root holding one workspace, `ws1`, which is the active one. Dropping it releases the
/// store for the next test.
pub struct ScratchRoot {
    pub dir: TempDir,
    pub ws_id: &'static str,
    _turn: MutexGuard<'static, ()>,
}

pub fn scratch_data_root() -> ScratchRoot {
    let turn = STORE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempfile::tempdir().expect("a temporary directory");
    set_data_root(dir.path()).expect("the data root is created");
    // Jars are cached in memory by workspace id, and every scratch store reuses `ws1`.
    crate::cookies::forget_all();
    crate::mail::forget_all();
    crate::vault::forget_all();
    let mut settings = load_settings();
    settings.set("workspaces", json!([{ "id": "ws1", "name": "Workspace 1", "activeEnvironmentId": null }]));
    settings.set("activeWorkspaceId", json!("ws1"));
    save_settings(&settings).expect("settings are saved");
    init_workspace_dir("ws1", "Workspace 1").expect("the workspace directory is created");
    ScratchRoot { dir, ws_id: "ws1", _turn: turn }
}

/// Turn HTTPS on in the scratch store: a CA is generated, and the client configuration returned trusts it.
pub fn enable_https(root: &ScratchRoot) -> std::sync::Arc<rustls::ClientConfig> {
    use std::sync::Arc;

    let generated = crate::certificates::generate_ca(root.dir.path()).expect("a CA is generated");
    let mut settings = load_settings();
    settings.set("tlsEnabled", json!(true));
    save_settings(&settings).expect("settings are saved");

    let pem = std::fs::read(&generated.cert_path).expect("the CA certificate is readable");
    let mut roots = rustls::RootCertStore::empty();
    for cert in <rustls::pki_types::CertificateDer<'_> as rustls::pki_types::pem::PemObject>::pem_slice_iter(&pem) {
        roots.add(cert.expect("a certificate")).expect("a usable root");
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("protocol versions")
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}
