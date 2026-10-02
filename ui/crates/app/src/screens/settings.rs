//! The Settings screen: this client's appearance, and the engine's own settings.
//!
//! Two different owners share one screen, and the pages keep them apart:
//!
//! - **Appearance** is this client's (theme, text size). It is kept in [`Preferences`] and never reaches the
//!   engine.
//! - **Servers**, **HTTPS** are the engine's. Every change is a `settings.update`, `server.*` or `tls.*`
//!   command, and what the page shows is what the engine reports back. The only things done on this computer
//!   are the ones the engine cannot reach: restarting its process, and trusting its certificate.
//! - **About** says what is running.
//!
//! The screen is the kit's `Settings` component: pages in its own sidebar, groups of items, searchable.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    button::Button,
    group_box::GroupBoxVariant,
    h_flex,
    link::Link,
    setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    tag::Tag,
};
use gpui_kit::{
    App, AppContext as _, Axis, Context, Entity, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div,
};
use serde_json::{Value, json};

use crate::{
    app_model::{AppModel, ProxyStatus},
    components::binary::{PickedFile, pick_file},
    engine::{call, describe, failure, upload_and_call},
    model::trust,
    preferences::Preferences,
    theme,
};

const REPOSITORY_URL: &str = "https://github.com/HarshalKudale/bifurc";
const RELEASES_URL: &str = "https://github.com/HarshalKudale/bifurc/releases";

/// The engine's root certificate, as `tls.certStatus` reports it.
#[derive(Debug, Clone, Default, PartialEq)]
struct Certificate {
    generated: bool,
    fingerprint: Option<String>,
    /// A generate or remove is in flight.
    busy: bool,
    /// The user's own certificate and key are being uploaded.
    importing: bool,
    /// The certificate is being added to this computer's trust store.
    installing: bool,
    /// How the last install went: whether it worked, and what to say about it.
    note: Option<(bool, String)>,
}

/// The certificate's bytes from a `tls.exportCert` artifact: it is a PEM of a kilobyte or so, so the engine
/// always sends it inline as base64.
fn certificate_bytes(artifact: &Value) -> Result<Vec<u8>, String> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    if artifact.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(artifact.get("error").and_then(Value::as_str).unwrap_or("There is no certificate.").to_owned());
    }
    let inline = artifact.get("inline").and_then(Value::as_str).ok_or("The engine sent no certificate.")?;
    STANDARD.decode(inline).map_err(|error| format!("The certificate could not be read. {error}"))
}

pub struct SettingsScreen {
    model: Entity<AppModel>,
    certificate: Certificate,
    _subscription: Subscription,
}

impl SettingsScreen {
    pub fn new(model: Entity<AppModel>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&model, |_, _, cx| cx.notify());
        let mut this = Self { model, certificate: Certificate::default(), _subscription: subscription };
        this.refresh_certificate(cx);
        this
    }

    fn refresh_certificate(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "tls.certStatus", json!({}), |this, outcome, cx| {
            if let Ok(status) = outcome {
                this.certificate = Certificate {
                    generated: status.get("generated").and_then(Value::as_bool).unwrap_or(false),
                    fingerprint: status.get("fingerprint").and_then(Value::as_str).map(str::to_owned),
                    busy: false,
                    importing: this.certificate.importing,
                    installing: this.certificate.installing,
                    note: this.certificate.note.clone(),
                };
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

    /// Asks for the user's own certificate and then its private key, and hands both to the engine.
    fn import_certificate(&mut self, cx: &mut Context<Self>) {
        if self.certificate.busy || self.certificate.importing {
            return;
        }
        pick_file("Choose the CA certificate (.pem or .crt)", cx, |this, certificate, cx| {
            let certificate = match certificate {
                Ok(certificate) => certificate,
                Err(error) => return this.report(format!("Couldn't read the certificate. {error}"), cx),
            };
            pick_file("Now choose its private key (.pem or .key)", cx, move |this, key, cx| match key {
                Ok(key) => this.upload_certificate(certificate, key, cx),
                Err(error) => this.report(format!("Couldn't read the private key. {error}"), cx),
            });
        });
    }

    fn upload_certificate(&mut self, certificate: PickedFile, key: PickedFile, cx: &mut Context<Self>) {
        self.certificate.importing = true;
        self.certificate.note = None;
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
                    return this.finish_import(Some(format!("Couldn't import the certificate. {message}")), cx);
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
                        let failed = outcome.err().map(|message| format!("Couldn't import the private key. {message}"));
                        this.finish_import(failed, cx)
                    },
                );
            },
        );
    }

    fn finish_import(&mut self, failed: Option<String>, cx: &mut Context<Self>) {
        self.certificate.importing = false;
        match failed {
            Some(message) => self.report(message, cx),
            None => self.reload_proxy(cx),
        }
        self.refresh_certificate(cx);
    }

    /// Reads the certificate from the engine and puts it in this computer's trust store.
    fn install_certificate(&mut self, cx: &mut Context<Self>) {
        if self.certificate.installing {
            return;
        }
        self.certificate.installing = true;
        self.certificate.note = None;
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
        self.certificate.installing = false;
        self.certificate.note = Some(match outcome {
            Ok(message) => (true, message),
            Err(message) => (false, message),
        });
        cx.notify();
    }

    fn run_certificate_command(&mut self, action: &'static str, what: &'static str, cx: &mut Context<Self>) {
        if self.certificate.busy {
            return;
        }
        self.certificate.busy = true;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call(&client, cx, action, json!({}), move |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.model.update(cx, |model, cx| model.notice(format!("{what} {message}"), cx));
            }
            this.certificate.busy = false;
            this.certificate.note = None;
            this.reload_proxy(cx);
            this.refresh_certificate(cx);
        });
    }

    fn appearance_page(&self, cx: &App) -> SettingPage {
        let themes: Vec<(SharedString, SharedString)> = theme::available(cx)
            .into_iter()
            .map(|(name, is_dark)| {
                // Most themes say which they are in their name; the rest get it appended.
                let mode = if is_dark { "Dark" } else { "Light" };
                let label: SharedString =
                    if name.contains(mode) { name.clone() } else { format!("{name} ({mode})").into() };
                (name, label)
            })
            .collect();
        let sizes: Vec<(SharedString, SharedString)> = theme::FONT_SIZES
            .iter()
            .map(|size| (SharedString::from(size.to_string()), SharedString::from(format!("{size} px"))))
            .collect();

        SettingPage::new("Appearance").icon(IconName::Palette).default_open(true).groups(vec![
            SettingGroup::new()
                .title("Theme")
                .item(
                    SettingItem::new(
                        "Color theme",
                        SettingField::scrollable_dropdown(
                            themes,
                            |cx: &App| theme::current(cx),
                            |name: SharedString, cx: &mut App| {
                                if theme::apply(&name, cx) {
                                    Preferences::update(cx, |p| p.theme = Some(name.to_string()));
                                }
                            },
                        ),
                    )
                    .description("The predefined GPUI Kit themes. Each one is light or dark.")
                    .keywords(["dark", "light", "colors", "mode"]),
                )
                .footer(|_, _| "Applies to this app on this computer only."),
            SettingGroup::new().title("Text").item(
                SettingItem::new(
                    "Interface size",
                    SettingField::dropdown(
                        sizes,
                        |cx: &App| SharedString::from(Preferences::global(cx).font_size.to_string()),
                        |size: SharedString, cx: &mut App| {
                            if let Ok(size) = size.parse::<f32>() {
                                theme::set_font_size(size, cx);
                                Preferences::update(cx, |p| p.font_size = size);
                            }
                        },
                    ),
                )
                .description("Scales text, spacing and controls together.")
                .keywords(["zoom", "font"]),
            ),
        ])
    }

    fn port_item(&self, title: &'static str, description: &'static str, key: &'static str) -> SettingItem {
        let read = self.model.clone();
        let write = self.model.clone();
        SettingItem::new(
            title,
            SettingField::number_input(
                NumberFieldOptions { min: 1., max: 65535., step: 1. },
                move |cx: &App| {
                    let settings = &read.read(cx).config().settings;
                    f64::from(match key {
                        "port" => settings.proxy_port,
                        "forwardProxyPort" => settings.forward_proxy_port,
                        _ => settings.companion_port,
                    })
                },
                move |value: f64, cx: &mut App| {
                    // The field reports every keystroke; only a whole, valid port is sent.
                    if (1.0..=65535.0).contains(&value) && value.fract() == 0.0 {
                        write.update(cx, |model, cx| model.update_settings(json!({ key: value as u16 }), cx))
                    }
                },
            ),
        )
        .description(description)
    }

    /// A listener's name, whether it is up, and the button that starts or stops it.
    fn listener_item(
        &self,
        id: &'static str,
        name: &'static str,
        keywords: [&'static str; 3],
        status: fn(&AppModel) -> &ProxyStatus,
        toggle: fn(&mut AppModel, &mut Context<AppModel>),
    ) -> SettingItem {
        let model = self.model.clone();
        SettingItem::render(move |options, _, cx| {
            let listener = status(model.read(cx)).clone();
            let toggle_model = model.clone();
            let state = match (&listener.error, listener.running) {
                (Some(error), _) => Tag::danger().child(error.clone()),
                (None, true) => Tag::success().child(format!("Running on port {}", listener.port)),
                (None, false) => Tag::secondary().child("Stopped"),
            };
            h_flex().w_full().gap_2().child(div().flex_1().child(name)).child(state).child(
                Button::new(id)
                    .with_size(options.size())
                    .label(if listener.running { "Stop" } else { "Start" })
                    .loading(listener.busy)
                    .on_click(move |_, _, cx| toggle_model.update(cx, |m, cx| toggle(m, cx))),
            )
        })
        .keywords(keywords)
    }

    fn servers_page(&self) -> SettingPage {
        let engine_model = self.model.clone();
        SettingPage::new("Servers").icon(IconName::Network).groups(vec![
            SettingGroup::new()
                .title("Engine runtime")
                .item(
                    SettingItem::render(move |options, _, cx| {
                        let theme = cx.theme().clone();
                        let model = engine_model.read(cx);
                        let restarting = model.is_restarting_engine();
                        let engine = model.engine();
                        let detail = format!("Version {} · pid {}", engine.hello.engine_version, engine.info.pid);
                        let restart_model = engine_model.clone();
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                gpui_kit::component::v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child("Engine")
                                    .child(div().text_xs().text_color(theme.muted_foreground).child(detail)),
                            )
                            .child(
                                Button::new("restart-engine")
                                    .with_size(options.size())
                                    .label("Restart")
                                    .loading(restarting)
                                    .disabled(restarting)
                                    .on_click(move |_, _, cx| restart_model.update(cx, |m, cx| m.restart_engine(cx))),
                            )
                    })
                    .keywords(["restart", "kill", "respawn", "runtime", "process"]),
                )
                .footer(|_, _| {
                    "The engine keeps running behind its tray icon after this window closes; quit it from the icon. Restart stops the engine and starts a new one. Connections it holds, such as open WebSockets and captured traffic, are lost."
                }),
            SettingGroup::new()
                .title("Proxy")
                .item(self.listener_item(
                    "proxy-start-stop",
                    "Forward proxy",
                    ["start", "stop", "forward"],
                    AppModel::proxy,
                    AppModel::toggle_proxy,
                ))
                .item(self.port_item(
                    "Port",
                    "Point a browser or a tool at this port to route its traffic through the proxy rules, mocks and capture.",
                    "forwardProxyPort",
                ))
                .footer(|_, _| "A new port takes effect at once while the proxy is running."),
            SettingGroup::new()
                .title("Reverse proxy")
                .item(self.listener_item(
                    "reverse-proxy-start-stop",
                    "Reverse proxy",
                    ["start", "stop", "localhost"],
                    AppModel::reverse_proxy,
                    AppModel::toggle_reverse_proxy,
                ))
                .item(self.port_item(
                    "Port",
                    "Where .localhost names are served and sent on to their mappings. 80 lets the names work without a port.",
                    "port",
                ))
                .footer(|_, _| "A new port takes effect at once while the reverse proxy is running."),
            SettingGroup::new()
                .title("Companion")
                .item(self.port_item("Port", "The browser extension connects here.", "companionPort"))
                .footer(|_, _| "The companion server always runs. A new port takes effect after the engine restarts."),
        ])
    }

    fn https_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let tls_read = self.model.clone();
        let tls_write = self.model.clone();
        let certificate = self.certificate.clone();
        let this = cx.weak_entity();
        let install_this = this.clone();
        let install_certificate = certificate.clone();
        SettingPage::new("HTTPS").icon(IconName::Globe).groups(vec![
            SettingGroup::new().title("Interception").item(
                SettingItem::new(
                    "Intercept HTTPS",
                    SettingField::switch(
                        move |cx: &App| tls_read.read(cx).config().settings.tls_enabled,
                        move |value: bool, cx: &mut App| {
                            tls_write.update(cx, |model, cx| model.update_settings(json!({ "tlsEnabled": value }), cx))
                        },
                    ),
                )
                .description("Decrypt HTTPS traffic through the forward proxy so rules, mocks and capture apply to it.")
                .keywords(["tls", "ssl", "certificate"]),
            ),
            SettingGroup::new()
                .title("Root certificate")
                .item(
                    SettingItem::render(move |options, _, cx| {
                        let theme = cx.theme().clone();
                        let (generate, import, remove) = (this.clone(), this.clone(), this.clone());
                        let working = certificate.busy || certificate.importing;
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                gpui_kit::component::v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(if certificate.generated { "Certificate in place" } else { "No certificate" })
                                    .children(certificate.fingerprint.clone().map(|fingerprint| {
                                        div()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .font_family(theme.mono_font_family.clone())
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .child(fingerprint)
                                    })),
                            )
                            .child(
                                Button::new("generate-certificate")
                                    .with_size(options.size())
                                    .label(if certificate.generated { "Regenerate" } else { "Generate" })
                                    .loading(certificate.busy)
                                    .disabled(working)
                                    .on_click(move |_, _, cx| {
                                        generate
                                            .update(cx, |this, cx| {
                                                this.run_certificate_command(
                                                    "tls.generate",
                                                    "Couldn't generate the certificate.",
                                                    cx,
                                                )
                                            })
                                            .ok();
                                    }),
                            )
                            .child(
                                Button::new("import-certificate")
                                    .with_size(options.size())
                                    .label("Import…")
                                    .loading(certificate.importing)
                                    .disabled(working)
                                    .tooltip("Use your own certificate and private key instead")
                                    .on_click(move |_, _, cx| {
                                        import.update(cx, |this, cx| this.import_certificate(cx)).ok();
                                    }),
                            )
                            .child(
                                Button::new("remove-certificate")
                                    .with_size(options.size())
                                    .label("Remove")
                                    .disabled(!certificate.generated || working)
                                    .on_click(move |_, _, cx| {
                                        remove
                                            .update(cx, |this, cx| {
                                                this.run_certificate_command(
                                                    "tls.removeCert",
                                                    "Couldn't remove the certificate.",
                                                    cx,
                                                )
                                            })
                                            .ok();
                                    }),
                            )
                    })
                    .keywords(["ca", "root", "fingerprint", "import", "own", "generate"]),
                )
                .item(
                    SettingItem::render(move |options, _, cx| {
                        let theme = cx.theme().clone();
                        let install = install_this.clone();
                        let certificate = install_certificate.clone();
                        let note = certificate.note.clone().map(|(ok, text)| {
                            div()
                                .text_xs()
                                .text_color(if ok { theme.success } else { theme.danger })
                                .child(text)
                        });
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                gpui_kit::component::v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child("Trust on this computer")
                                    .children(note),
                            )
                            .child(
                                Button::new("install-certificate")
                                    .with_size(options.size())
                                    .label("Install")
                                    .loading(certificate.installing)
                                    .disabled(!certificate.generated || certificate.installing)
                                    .tooltip("Add the certificate to this computer's trusted roots")
                                    .on_click(move |_, _, cx| {
                                        install.update(cx, |this, cx| this.install_certificate(cx)).ok();
                                    }),
                            )
                    })
                    .keywords(["trust", "install", "keychain", "store"]),
                )
                .footer(|_, _| {
                    "The engine signs the sites it intercepts with this certificate, so install it to avoid warnings. Firefox keeps its own store and needs the certificate imported there."
                }),
        ])
    }

    fn about_page(&self, cx: &App) -> SettingPage {
        let engine = self.model.read(cx).engine().clone();
        let fact = |title: &'static str, value: String| {
            SettingItem::new(
                title,
                SettingField::render(move |_, _, cx| {
                    div().text_color(cx.theme().muted_foreground).child(value.clone())
                }),
            )
        };
        let origin = if engine.started_here() { "Started by this app" } else { "Already running" };
        SettingPage::new("About").icon(IconName::Info).groups(vec![
            SettingGroup::new().title("Bifurc").items(vec![
                fact("Desktop app", format!("Version {}", env!("CARGO_PKG_VERSION"))),
                fact("Engine", format!("Version {}", engine.hello.engine_version)),
                fact("Protocol", engine.hello.protocol_version.to_string()),
                fact("Engine process", format!("pid {} · {origin}", engine.info.pid)),
            ]),
            SettingGroup::new()
                .title("Updates")
                .item(
                    SettingItem::new(
                        "Releases",
                        SettingField::render(|_, _, _| {
                            Link::new("releases-link")
                                .href(RELEASES_URL)
                                .child("github.com/HarshalKudale/bifurc/releases")
                        }),
                    )
                    .description("New versions are published on GitHub.")
                    .layout(Axis::Horizontal),
                )
                .item(SettingItem::new(
                    "Source",
                    SettingField::render(|_, _, _| {
                        Link::new("repository-link").href(REPOSITORY_URL).child("github.com/HarshalKudale/bifurc")
                    }),
                ))
                .footer(|_, _| "This app does not update itself yet. Download a new version from the releases page."),
        ])
    }
}

impl Render for SettingsScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pages = vec![self.appearance_page(cx), self.servers_page(), self.https_page(cx), self.about_page(cx)];
        div()
            .size_full()
            .bg(cx.theme().background)
            .child(Settings::new("settings").with_group_variant(GroupBoxVariant::Outline).pages(pages))
    }
}
