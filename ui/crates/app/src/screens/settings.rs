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
    button::{Button, ButtonVariants as _},
    group_box::GroupBoxVariant,
    h_flex,
    link::Link,
    setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Axis, Context, Entity, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div,
};
use serde_json::{Value, json};

use crate::{
    app_model::{AppModel, ModelEvent, ProxyStatus},
    components::{certificate::Certificate, dialogs::confirm_danger},
    engine::{call, call_in, failure},
    model::config::AccountInfo,
    preferences::Preferences,
    screens::workspace_setup::{Purpose, open_setup_dialog},
    theme,
};

const REPOSITORY_URL: &str = "https://github.com/HarshalKudale/bifurc";
const RELEASES_URL: &str = "https://github.com/HarshalKudale/bifurc/releases";

/// A workspace that syncs as an account, and what it holds that its repository does not.
#[derive(Debug, Clone, PartialEq)]
struct AccountWorkspace {
    name: String,
    /// `owner/name`; `None` while the workspace was never published.
    repo: Option<String>,
    /// Files that differ from the last sync.
    changes: u64,
    /// Commits the repository has not got.
    unpushed: bool,
}

impl AccountWorkspace {
    /// What would be lost with the workspace, if anything would.
    fn unsynced(&self) -> Option<String> {
        match (&self.repo, self.changes, self.unpushed) {
            (None, _, _) => Some("never published".to_owned()),
            (_, 0, false) => None,
            (_, 0, true) => Some("unpushed commits".to_owned()),
            (_, 1, _) => Some("1 unsynced change".to_owned()),
            (_, changes, _) => Some(format!("{changes} unsynced changes")),
        }
    }
}

/// A signed-in account and the workspaces that use it, as `account.list` reports them.
#[derive(Debug, Clone, PartialEq)]
struct AccountRow {
    info: AccountInfo,
    workspaces: Vec<AccountWorkspace>,
}

fn accounts_from(value: &Value) -> Vec<AccountRow> {
    let list = |value: &Value, key: &str| value.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    list(value, "accounts")
        .iter()
        .filter_map(|account| {
            let workspaces = list(account, "workspaces")
                .iter()
                .map(|ws| AccountWorkspace {
                    name: ws.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
                    repo: ws.get("repo").and_then(Value::as_str).filter(|repo| !repo.is_empty()).map(str::to_owned),
                    changes: ws.get("changes").and_then(Value::as_u64).unwrap_or(0),
                    unpushed: ws.get("unpushed").and_then(Value::as_bool).unwrap_or(false),
                })
                .collect();
            Some(AccountRow { info: AccountInfo::from_value(account)?, workspaces })
        })
        .collect()
}

/// What signing the account out does, beyond what the question itself says.
fn sign_out_consequence(account: &AccountRow) -> String {
    if account.workspaces.is_empty() {
        return "Bifurc forgets this account on this computer. No workspace uses it.".to_owned();
    }
    let names: Vec<String> = account.workspaces.iter().map(|ws| format!("“{}”", ws.name)).collect();
    let lost: Vec<String> = account
        .workspaces
        .iter()
        .filter_map(|ws| Some(format!("“{}” ({})", ws.name, ws.unsynced()?)))
        .collect();
    let removed = format!("Its workspaces are removed from this computer: {}.", names.join(", "));
    if lost.is_empty() {
        format!("{removed} They stay on GitHub and can be imported again.")
    } else {
        format!("{removed} What was never synced is lost with them: {}.", lost.join(", "))
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

pub struct SettingsScreen {
    model: Entity<AppModel>,
    certificate: Entity<Certificate>,
    accounts: Vec<AccountRow>,
    /// The account being signed out.
    signing_out: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsScreen {
    pub fn new(model: Entity<AppModel>, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe(&model, |this, _, event: &ModelEvent, cx| {
                // What an account's workspaces hold unsynced changes with every sync.
                if matches!(
                    event,
                    ModelEvent::AccountChanged(_)
                        | ModelEvent::SyncStatus
                        | ModelEvent::WorkspaceSwitched
                        | ModelEvent::EngineRestarted
                ) {
                    this.refresh_accounts(cx);
                }
            }),
        ];
        let certificate = cx.new(|cx| Certificate::new(model.clone(), cx));
        subscriptions.push(cx.observe(&certificate, |_, _, cx| cx.notify()));
        let mut this =
            Self { model, certificate, accounts: Vec::new(), signing_out: None, _subscriptions: subscriptions };
        this.refresh_accounts(cx);
        this
    }

    fn refresh_accounts(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "account.list", json!({}), |this, outcome, cx| {
            if let Ok(listed) = outcome {
                this.accounts = accounts_from(&listed);
                cx.notify();
            }
        });
    }

    /// Asks before signing out, with what the account's workspaces hold right now: signing out removes them.
    fn ask_sign_out(&mut self, account_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call_in(&client, window, cx, "account.list", json!({}), move |this, outcome, window, cx| {
            if let Ok(listed) = outcome {
                this.accounts = accounts_from(&listed);
                cx.notify();
            }
            let Some(account) = this.accounts.iter().find(|account| account.info.id == account_id) else { return };
            let ok_text = if account.workspaces.is_empty() { "Sign out" } else { "Sign out and remove" };
            let id = account_id.clone();
            confirm_danger(
                cx.weak_entity(),
                window,
                cx,
                format!("Sign out @{}?", account.info.login),
                sign_out_consequence(account),
                ok_text,
                move |this, _, cx| this.sign_out(id.clone(), cx),
            );
        });
    }

    fn sign_out(&mut self, account_id: String, cx: &mut Context<Self>) {
        if self.signing_out.is_some() {
            return;
        }
        self.signing_out = Some(account_id.clone());
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "account.logout", json!({ "accountId": account_id }), |this, outcome, cx| {
            this.signing_out = None;
            if let Some(message) = failure(&outcome) {
                this.report(format!("Couldn't sign out. {message}"), cx);
            }
            this.model.update(cx, |model, cx| {
                model.refresh_setup(cx);
                model.refresh(cx);
            });
            this.refresh_accounts(cx);
        });
    }

    fn report(&mut self, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(message, cx));
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

    fn https_page(&self) -> SettingPage {
        let tls_read = self.model.clone();
        let tls_write = self.model.clone();
        let control = self.certificate.clone();
        let install_control = control.clone();
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
                        let (generate, import, remove) = (control.clone(), control.clone(), control.clone());
                        let certificate = control.read(cx);
                        let working = certificate.working();
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
                                    .on_click(move |_, _, cx| generate.update(cx, |certificate, cx| certificate.generate(cx))),
                            )
                            .child(
                                Button::new("import-certificate")
                                    .with_size(options.size())
                                    .label("Import…")
                                    .loading(certificate.importing)
                                    .disabled(working)
                                    .tooltip("Use your own certificate and private key instead")
                                    .on_click(move |_, _, cx| import.update(cx, |certificate, cx| certificate.import(cx))),
                            )
                            .child(
                                Button::new("remove-certificate")
                                    .with_size(options.size())
                                    .label("Remove")
                                    .disabled(!certificate.generated || working)
                                    .on_click(move |_, _, cx| remove.update(cx, |certificate, cx| certificate.remove(cx))),
                            )
                    })
                    .keywords(["ca", "root", "fingerprint", "import", "own", "generate"]),
                )
                .item(
                    SettingItem::render(move |options, _, cx| {
                        let theme = cx.theme().clone();
                        let install = install_control.clone();
                        let certificate = install_control.read(cx);
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
                                    .on_click(move |_, _, cx| install.update(cx, |certificate, cx| certificate.install(cx))),
                            )
                    })
                    .keywords(["trust", "install", "keychain", "store"]),
                )
                .footer(|_, _| {
                    "The engine signs the sites it intercepts with this certificate, so install it to avoid warnings. Firefox keeps its own store and needs the certificate imported there."
                }),
        ])
    }

    /// The GitHub accounts signed in on this computer: one group each, with the workspaces that sync as it.
    fn accounts_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let page = SettingPage::new("Accounts").icon(IconName::Github);
        if self.accounts.is_empty() {
            return page.groups(vec![
                SettingGroup::new()
                    .title("GitHub")
                    .item(SettingItem::render(|_, _, cx| {
                        div().text_color(cx.theme().muted_foreground).child("No account is signed in.")
                    }))
                    .footer(|_, _| {
                        "An account is only needed to sync a workspace with GitHub. Sign in when you add a workspace, or when you publish one from its Config."
                    }),
            ]);
        }
        let groups = self.accounts.iter().enumerate().map(|(index, account)| {
            let this = cx.weak_entity();
            let model = self.model.clone();
            let (id, signed_in) = (account.info.id.clone(), account.info.signed_in);
            let host = account.info.host.clone();
            let busy = self.signing_out.as_deref() == Some(account.info.id.as_str());
            let blocked = self.signing_out.is_some();
            let count = account.workspaces.len();
            let header = SettingItem::render(move |options, _, cx| {
                let theme = cx.theme().clone();
                let (this, id, model) = (this.clone(), id.clone(), model.clone());
                let relogin_id = id.clone();
                let state =
                    if signed_in { Tag::success().child("Signed in") } else { Tag::warning().child("Sign-in expired") };
                let detail = match count {
                    0 => format!("{host} · no workspaces"),
                    1 => format!("{host} · 1 workspace"),
                    count => format!("{host} · {count} workspaces"),
                };
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(div().flex_1().min_w_0().text_color(theme.muted_foreground).child(detail))
                    .child(state)
                    .children((!signed_in).then(|| {
                        Button::new(("sign-in-again", index)).with_size(options.size()).primary().label("Sign in again").on_click(
                            move |_, window, cx| {
                                let purpose = Purpose::SignIn { account_id: relogin_id.clone() };
                                open_setup_dialog(model.clone(), purpose, window, cx);
                            },
                        )
                    }))
                    .child(
                        Button::new(("sign-out", index))
                            .with_size(options.size())
                            .label("Sign out")
                            .loading(busy)
                            .disabled(blocked)
                            .on_click(move |_, window, cx| {
                                this.update(cx, |this, cx| this.ask_sign_out(id.clone(), window, cx)).ok();
                            }),
                    )
            })
            .keywords(["github", "login", "logout", "sign out"]);
            let workspaces = account.workspaces.iter().cloned().map(|workspace| {
                SettingItem::render(move |_, _, cx| {
                    let theme = cx.theme().clone();
                    let state = match workspace.unsynced() {
                        Some(unsynced) => Tag::warning().child(capitalized(&unsynced)),
                        None => Tag::secondary().child("Synced"),
                    };
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().overflow_hidden().text_ellipsis().child(workspace.name.clone()))
                                .children(workspace.repo.clone().map(|repo| {
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .font_family(theme.mono_font_family.clone())
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(repo)
                                })),
                        )
                        .child(state)
                })
            });
            SettingGroup::new()
                .title(format!("@{}", account.info.login))
                .item(header)
                .items(workspaces)
                .footer(|_, _| "Signing out removes the account and its workspaces from this computer.")
        });
        page.groups(groups.collect::<Vec<_>>())
    }

    fn about_page(&self, cx: &App) -> SettingPage {
        let engine = self.model.read(cx).engine().clone();
        let (telemetry_read, telemetry_write) = (self.model.clone(), self.model.clone());
        let policy_url = self.model.read(cx).setup().privacy_policy_url.clone();
        let terms_url = self.model.read(cx).setup().terms_of_service_url.clone();
        let fact =|title: &'static str, value: String| {
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
                .title("Privacy")
                .item(
                    SettingItem::new(
                        "Anonymous usage analytics",
                        SettingField::switch(
                            move |cx: &App| telemetry_read.read(cx).config().settings.telemetry_enabled,
                            move |value: bool, cx: &mut App| {
                                telemetry_write.update(cx, |model, cx| {
                                    model.update_settings(json!({ "telemetryEnabled": value }), cx)
                                })
                            },
                        ),
                    )
                    .description(
                        "On by default. What kind of thing you do, never its content. Off, only that Bifurc was installed on this device is reported, once.",
                    )
                    .keywords(["telemetry", "tracking", "posthog"]),
                )
                .item(
                    SettingItem::new(
                        "Terms of service",
                        SettingField::render(move |_, _, _| {
                            Link::new("terms-link")
                                .href(terms_url.clone())
                                .child(terms_url.trim_start_matches("https://").to_owned())
                        }),
                    )
                    .layout(Axis::Horizontal),
                )
                .item(
                    SettingItem::new(
                        "Privacy policy",
                        SettingField::render(move |_, _, _| {
                            Link::new("privacy-link")
                                .href(policy_url.clone())
                                .child(policy_url.trim_start_matches("https://").to_owned())
                        }),
                    )
                    .layout(Axis::Horizontal),
                )
                .footer(|_, _| "Turning analytics back on applies when the engine next starts."),
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
        let pages = vec![
            self.appearance_page(cx),
            self.accounts_page(cx),
            self.servers_page(),
            self.https_page(),
            self.about_page(cx),
        ];
        div()
            .size_full()
            .bg(cx.theme().background)
            .child(Settings::new("settings").with_group_variant(GroupBoxVariant::Outline).pages(pages))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_out_says_which_workspaces_go_and_what_is_lost_with_them() {
        let accounts = accounts_from(&json!({ "accounts": [
            { "id": "github.com:1", "login": "octo", "host": "github.com", "signedIn": true, "workspaces": [
                { "id": "a", "name": "Synced", "repo": "octo/a", "changes": 0, "unpushed": false },
                { "id": "b", "name": "Edited", "repo": "octo/b", "changes": 2, "unpushed": false },
                { "id": "c", "name": "Local", "repo": null, "changes": 0, "unpushed": false },
            ]},
            { "id": "github.com:2", "login": "idle", "host": "github.com", "signedIn": true, "workspaces": [] },
        ]}));
        assert_eq!(accounts.len(), 2);
        let unsynced: Vec<Option<String>> = accounts[0].workspaces.iter().map(AccountWorkspace::unsynced).collect();
        assert_eq!(unsynced, [None, Some("2 unsynced changes".to_owned()), Some("never published".to_owned())]);
        let warning = sign_out_consequence(&accounts[0]);
        assert!(warning.contains("“Synced”, “Edited”, “Local”"), "{warning}");
        assert!(warning.contains("“Edited” (2 unsynced changes), “Local” (never published)"), "{warning}");
        assert!(sign_out_consequence(&accounts[1]).contains("No workspace uses it"));

        let synced = AccountRow { workspaces: accounts[0].workspaces[..1].to_vec(), ..accounts[0].clone() };
        assert!(sign_out_consequence(&synced).contains("can be imported again"));
    }
}
