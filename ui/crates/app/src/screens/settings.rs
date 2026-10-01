//! The Settings screen: this client's appearance, and the engine's own settings.
//!
//! Two different owners share one screen, and the pages keep them apart:
//!
//! - **Appearance** is this client's (theme, text size). It is kept in [`Preferences`] and never reaches the
//!   engine.
//! - **Proxy**, **HTTPS** are the engine's. Every change is a `settings.update`, `server.*` or `tls.*`
//!   command, and what the page shows is what the engine reports back.
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
    App, Axis, Context, Entity, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription,
    Window, div,
};
use serde_json::{Value, json};

use crate::{
    app_model::AppModel,
    engine::{call, failure},
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
                };
                cx.notify();
            }
        });
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
                        write.update(cx, |model, cx| model.update_settings(json!({ key: value as u16 }), cx));
                    }
                },
            ),
        )
        .description(description)
    }

    fn proxy_page(&self) -> SettingPage {
        let status_model = self.model.clone();
        let tray_read = self.model.clone();
        let tray_write = self.model.clone();
        SettingPage::new("Proxy").icon(IconName::Network).groups(vec![
            SettingGroup::new().title("Server").item(
                SettingItem::render(move |options, _, cx| {
                    let proxy = status_model.read(cx).proxy().clone();
                    let (toggle_model, restart_model) = (status_model.clone(), status_model.clone());
                    let state = match (&proxy.error, proxy.running) {
                        (Some(error), _) => Tag::danger().child(error.clone()),
                        (None, true) => Tag::success().child(format!("Running on port {}", proxy.port)),
                        (None, false) => Tag::secondary().child("Stopped"),
                    };
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(div().flex_1().child("Forward proxy"))
                        .child(state)
                        .child(
                            Button::new("proxy-start-stop")
                                .with_size(options.size())
                                .label(if proxy.running { "Stop" } else { "Start" })
                                .loading(proxy.busy)
                                .on_click(move |_, _, cx| toggle_model.update(cx, |m, cx| m.toggle_proxy(cx))),
                        )
                        .child(
                            Button::new("proxy-restart")
                                .with_size(options.size())
                                .label("Restart")
                                .disabled(proxy.busy)
                                .on_click(move |_, _, cx| restart_model.update(cx, |m, cx| m.restart_proxy(cx))),
                        )
                })
                .keywords(["start", "stop", "restart", "server", "forward"]),
            ),
            SettingGroup::new()
                .title("Ports")
                .items(vec![
                    self.port_item(
                        "Reverse proxy port",
                        "Where .localhost names are served and sent on to their mappings. 80 lets the names work without a port.",
                        "port",
                    ),
                    self.port_item(
                        "Forward proxy port",
                        "Point a browser or a tool at this port to route its traffic through the proxy rules, mocks and capture.",
                        "forwardProxyPort",
                    ),
                    self.port_item("Companion port", "The browser extension connects here.", "companionPort"),
                ])
                .footer(|_, _| {
                    "The two proxies move to a new port at once. The companion port takes effect when the engine restarts."
                }),
            SettingGroup::new().title("Engine").item(
                SettingItem::new(
                    "Keep running in the tray",
                    SettingField::switch(
                        move |cx: &App| tray_read.read(cx).config().settings.minimize_to_tray,
                        move |value: bool, cx: &mut App| {
                            tray_write
                                .update(cx, |model, cx| model.update_settings(json!({ "minimizeToTray": value }), cx))
                        },
                    ),
                )
                .description(
                    "Closing the Electron app's window leaves the engine running. This app stops an engine it started.",
                ),
            ),
        ])
    }

    fn https_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let tls_read = self.model.clone();
        let tls_write = self.model.clone();
        let certificate = self.certificate.clone();
        let this = cx.weak_entity();
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
                        let (generate, remove) = (this.clone(), this.clone());
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                gpui_kit::component::v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(if certificate.generated { "Certificate generated" } else { "No certificate" })
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
                                Button::new("remove-certificate")
                                    .with_size(options.size())
                                    .label("Remove")
                                    .disabled(!certificate.generated || certificate.busy)
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
                    .keywords(["ca", "root", "fingerprint", "trust"]),
                )
                .footer(|_, _| {
                    "The engine signs the sites it intercepts with this certificate. Trust it in your system or browser to avoid warnings."
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
        let origin = if engine.is_owned() { "Started by this app" } else { "Shared with another client" };
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
        let pages = vec![self.appearance_page(cx), self.proxy_page(), self.https_page(cx), self.about_page(cx)];
        div()
            .size_full()
            .bg(cx.theme().background)
            .child(Settings::new("settings").with_group_variant(GroupBoxVariant::Outline).pages(pages))
    }
}
