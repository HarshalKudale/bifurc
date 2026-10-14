//! The engine's root certificate, and what a screen can do with it.
//!
//! Settings and the first-run setup both show the certificate and offer the same actions, so the state and
//! the commands live here once. The engine makes, keeps and reports the certificate (`tls.*`); the only thing
//! done on this computer is trusting it, which the engine cannot reach.

use gpui_kit::{AppContext as _, Context, Entity};
use serde_json::{Value, json};

use crate::{
    app_model::AppModel,
    components::binary::{PickedFile, pick_file},
    engine::{call, describe, failure, upload_and_call},
    i18n::{t, t_with},
    model::trust,
};

/// The certificate's bytes from a `tls.exportCert` artifact: it is a PEM of a kilobyte or so, so the engine
/// always sends it inline as base64.
fn certificate_bytes(artifact: &Value) -> Result<Vec<u8>, String> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    if artifact.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(artifact.get("error").and_then(Value::as_str).unwrap_or(t("components.certificate.no_certificate")).to_owned());
    }
    let inline = artifact.get("inline").and_then(Value::as_str).ok_or(t("components.certificate.no_engine_certificate"))?;
    STANDARD.decode(inline).map_err(|error| t_with("components.certificate.unreadable", &[("error", &error)]))
}

/// The engine's root certificate, as `tls.certStatus` reports it, and what is being done to it.
pub struct Certificate {
    model: Entity<AppModel>,
    pub generated: bool,
    pub fingerprint: Option<String>,
    /// A generate or remove is in flight.
    pub busy: bool,
    /// The user's own certificate and key are being uploaded.
    pub importing: bool,
    /// The certificate is being added to this computer's trust store.
    pub installing: bool,
    /// How the last install went: whether it worked, and what to say about it.
    pub note: Option<(bool, String)>,
}

impl Certificate {
    pub fn new(model: Entity<AppModel>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            model,
            generated: false,
            fingerprint: None,
            busy: false,
            importing: false,
            installing: false,
            note: None,
        };
        this.refresh(cx);
        this
    }

    /// A generate, remove or import is in flight.
    pub fn working(&self) -> bool {
        self.busy || self.importing
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "tls.certStatus", json!({}), |this, outcome, cx| {
            if let Ok(status) = outcome {
                this.generated = status.get("generated").and_then(Value::as_bool).unwrap_or(false);
                this.fingerprint = status.get("fingerprint").and_then(Value::as_str).map(str::to_owned);
                this.busy = false;
                cx.notify();
            }
        });
    }

    fn report(&mut self, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(message, cx));
    }

    /// The forward proxy reads the certificate when it starts, so a running one is started again to use a
    /// new certificate.
    fn reload_proxy(&mut self, cx: &mut Context<Self>) {
        if self.model.read(cx).proxy().running {
            self.model.update(cx, |model, cx| model.restart_proxy(cx));
        }
    }

    pub fn generate(&mut self, cx: &mut Context<Self>) {
        self.run_command("tls.generate", "components.certificate.generate_failed", cx);
    }

    pub fn remove(&mut self, cx: &mut Context<Self>) {
        self.run_command("tls.removeCert", "components.certificate.remove_failed", cx);
    }

    fn run_command(&mut self, action: &'static str, failed_key: &'static str, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call(&client, cx, action, json!({}), move |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.report(t_with(failed_key, &[("message", &message)]), cx);
            }
            this.busy = false;
            this.note = None;
            this.reload_proxy(cx);
            this.refresh(cx);
            cx.notify();
        });
    }

    /// Asks for the user's own certificate and then its private key, and hands both to the engine.
    pub fn import(&mut self, cx: &mut Context<Self>) {
        if self.working() {
            return;
        }
        pick_file(t("components.certificate.pick_certificate"), cx, |this, certificate, cx| {
            let certificate = match certificate {
                Ok(certificate) => certificate,
                Err(error) => return this.report(t_with("components.certificate.read_failed", &[("error", &error)]), cx),
            };
            pick_file(t("components.certificate.pick_key"), cx, move |this, key, cx| match key {
                Ok(key) => this.upload(certificate, key, cx),
                Err(error) => this.report(t_with("components.certificate.read_key_failed", &[("error", &error)]), cx),
            });
        });
    }

    fn upload(&mut self, certificate: PickedFile, key: PickedFile, cx: &mut Context<Self>) {
        self.importing = true;
        self.note = None;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        let key_client = client.clone();
        upload_and_call(
            &client,
            cx,
            None,
            &certificate.name,
            &certificate.bytes,
            "tls.importCert",
            Default::default(),
            move |this, outcome, cx| {
                if let Err(message) = outcome {
                    return this.finish_import(Some(t_with("components.certificate.import_failed", &[("message", &message)])), cx);
                }
                upload_and_call(
                    &key_client,
                    cx,
                    None,
                    &key.name,
                    &key.bytes,
                    "tls.importKey",
                    Default::default(),
                    |this, outcome, cx| {
                        let failed = outcome.err().map(|message| t_with("components.certificate.import_key_failed", &[("message", &message)]));
                        this.finish_import(failed, cx)
                    },
                );
            },
        );
    }

    fn finish_import(&mut self, failed: Option<String>, cx: &mut Context<Self>) {
        self.importing = false;
        match failed {
            Some(message) => self.report(message, cx),
            None => self.reload_proxy(cx),
        }
        self.refresh(cx);
        cx.notify();
    }

    /// Reads the certificate from the engine and puts it in this computer's trust store.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        if self.installing {
            return;
        }
        self.installing = true;
        self.note = None;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "tls.exportCert", json!({}), |this, outcome, cx| {
            let pem = match outcome {
                Ok(artifact) => certificate_bytes(&artifact),
                Err(error) => Err(describe(&error)),
            };
            let pem = match pem {
                Ok(pem) => pem,
                Err(message) => return this.finish_install(Err(message), cx),
            };
            let installing = cx.background_spawn(async move { trust::install(&pem) });
            cx.spawn(async move |this, cx| {
                let outcome = installing.await;
                this.update(cx, |this, cx| this.finish_install(outcome, cx)).ok();
            })
            .detach();
        });
    }

    fn finish_install(&mut self, outcome: Result<String, String>, cx: &mut Context<Self>) {
        self.installing = false;
        self.note = Some(match outcome {
            Ok(message) => (true, message),
            Err(message) => (false, message),
        });
        cx.notify();
    }
}
