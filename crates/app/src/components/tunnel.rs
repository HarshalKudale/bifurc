//! cloudflared, the program webhook capture needs to be reachable from the internet: whether it is installed,
//! and installing it. The engine finds, downloads and checks the program (`tunnel.*`); this keeps what a
//! screen shows about it. Onboarding and Settings install it; a webhook request's Capture button needs it.

use gpui_kit::{Context, Entity};
use serde_json::{Value, json};

use crate::{
    app_model::AppModel,
    engine::{call, failure},
    i18n::t,
};

pub struct Tunnel {
    model: Entity<AppModel>,
    /// The engine has said whether the program is there.
    pub known: bool,
    pub installed: bool,
    /// The engine has a build of the program for this computer.
    pub supported: bool,
    pub installing: bool,
    /// How the last install went: whether it worked, and what to say about it.
    pub note: Option<(bool, String)>,
}

impl Tunnel {
    pub fn new(model: Entity<AppModel>, cx: &mut Context<Self>) -> Self {
        let mut this = Self { model, known: false, installed: false, supported: true, installing: false, note: None };
        this.refresh(cx);
        this
    }

    fn apply(&mut self, status: &Value) {
        self.known = true;
        self.installed = status.get("installed").and_then(Value::as_bool).unwrap_or(false);
        self.supported = status.get("supported").and_then(Value::as_bool).unwrap_or(true);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "tunnel.status", json!({}), |this, outcome, cx| {
            if let Ok(status) = outcome {
                this.apply(&status);
                cx.notify();
            }
        });
    }

    pub fn install(&mut self, cx: &mut Context<Self>) {
        if self.installing {
            return;
        }
        self.installing = true;
        self.note = None;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "tunnel.install", json!({}), |this, outcome, cx| {
            this.installing = false;
            this.note = Some(match failure(&outcome) {
                Some(message) => (false, message),
                None => {
                    if let Ok(status) = &outcome {
                        this.apply(status);
                    }
                    (true, t("components.tunnel.installed").to_owned())
                }
            });
            cx.notify();
        });
    }
}
