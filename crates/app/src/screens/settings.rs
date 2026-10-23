//! The Settings screen: this client's appearance, and the engine's own settings.
//!
//! Two different owners share one screen, and the pages keep them apart:
//!
//! - **Appearance** is this client's (theme, text size). It is kept in [`Preferences`] and never reaches the
//!   engine.
//! - **Servers**, **Requests**, **HTTPS** are the engine's. Every change is a `settings.update`, `server.*` or `tls.*`
//!   command, and what the page shows is what the engine reports back. The only things done on this computer
//!   are the ones the engine cannot reach: restarting its process, and trusting its certificate.
//! - **About** says what is running.
//! - **Licenses** carries the notices the libraries and assets Bifurc ships require.
//!
//! The screen is the kit's `Settings` component: pages in its own sidebar, groups of items, searchable.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    group_box::GroupBoxVariant,
    h_flex,
    input::{Input, InputEvent, InputState},
    link::Link,
    setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Axis, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, TestSupportExt as _, Window, div,
};
use serde_json::{Value, json};

use crate::{
    app_model::{AppModel, ModelEvent, ProxyStatus},
    components::{certificate::Certificate, dialogs::confirm_danger, tunnel::Tunnel},
    engine::{call, call_in, failure},
    i18n::{self, t, t_with},
    model::{
        config::{AccountInfo, EngineSettings, RequestDefaults},
        update::{UpdateAction, UpdateProgress},
    },
    preferences::Preferences,
    screens::workspace_setup::{Purpose, open_setup_dialog},
    theme,
};

/// The Microsoft Store listing, whatever way this copy was installed; its page carries the ratings.
const STORE_REVIEW_URL: &str = "https://apps.microsoft.com/detail/9ncb4fdzzqpv#ratings-and-reviews";
const UPDATE_DOWNLOAD_PAGE: &str = "https://bifurc.app/download";
const REPOSITORY_URL: &str = "https://github.com/HarshalKudale/bifurc";
const RELEASES_URL: &str = "https://github.com/HarshalKudale/bifurc-releases/releases";

// The engine's defaults for the settings below that are blank when unset (engine workspace/settings.rs).
const DEFAULT_CAPTURE_LIMIT: u64 = 200;
const DEFAULT_TRACE_HEADER: &str = "x-bifurc-trace";
const DEFAULT_MAIL_PORT: u64 = 1025;
const DEFAULT_APPROVAL_TIMEOUT_SECONDS: u64 = 300;

/// What a setting that is blank when unset accepts: the engine's own bounds.
#[derive(Clone, Copy)]
pub(crate) enum Accepts {
    Number { min: u64, max: u64 },
    HeaderName,
}

/// The value to send for what was typed: `null` for blank (the engine takes that as unset, so its default
/// applies), `None` when the text is not acceptable.
pub(crate) fn setting_value(accepts: Accepts, text: &str) -> Option<Value> {
    let text = text.trim();
    if text.is_empty() {
        return Some(Value::Null);
    }
    match accepts {
        Accepts::Number { min, max } => text.parse::<u64>().ok().filter(|n| (min..=max).contains(n)).map(Value::from),
        Accepts::HeaderName => {
            text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_').then(|| json!(text))
        }
    }
}

/// The text field of a [`SettingItem`] whose blank means "the default", which it shows as the placeholder.
struct BlankInput {
    input: Entity<InputState>,
    _subscription: Subscription,
}

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
            (None, _, _) => Some(t("settings.workspace.never_published").to_owned()),
            (_, 0, false) => None,
            (_, 0, true) => Some(t("settings.workspace.unpushed").to_owned()),
            (_, 1, _) => Some(t_with("settings.workspace.unsynced_one", &[("count", &1)])),
            (_, changes, _) => Some(t_with("settings.workspace.unsynced_other", &[("count", &changes)])),
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
        return t("settings.sign_out.none").to_owned();
    }
    let separator = t("settings.list.separator");
    let names: Vec<String> = account.workspaces.iter().map(|ws| format!("“{}”", ws.name)).collect();
    let lost: Vec<String> = account
        .workspaces
        .iter()
        .filter_map(|ws| Some(t_with("settings.sign_out.lost_item", &[("name", &ws.name), ("state", &ws.unsynced()?)])))
        .collect();
    if lost.is_empty() {
        t_with("settings.sign_out.removed_kept", &[("names", &names.join(separator))])
    } else {
        t_with("settings.sign_out.removed_lost", &[("names", &names.join(separator)), ("lost", &lost.join(separator))])
    }
}

/// A browser extension connected to the engine, as `engine.companions` reports it.
#[derive(Debug, Clone, PartialEq)]
struct CompanionRow {
    browser: Option<String>,
    browser_version: Option<String>,
    os: Option<String>,
    arch: Option<String>,
    extension_version: Option<String>,
    extension_id: Option<String>,
    /// Milliseconds since the epoch.
    connected_at: i64,
}

impl CompanionRow {
    /// "Chrome 126", or "Browser" when the extension did not say.
    fn title(&self) -> String {
        match (&self.browser, &self.browser_version) {
            (Some(name), Some(version)) => format!("{name} {version}"),
            (Some(name), None) => name.clone(),
            _ => t("settings.companion.browser").to_owned(),
        }
    }

    /// The rest of what is known, as one line.
    fn detail(&self) -> String {
        let system = match (&self.os, &self.arch) {
            (Some(os), Some(arch)) => Some(format!("{os} ({arch})")),
            (Some(os), None) => Some(os.clone()),
            _ => None,
        };
        let parts = [
            system,
            self.extension_version
                .as_ref()
                .map(|version| t_with("settings.companion.extension_version", &[("version", version)])),
            self.extension_id.as_ref().map(|id| t_with("settings.companion.extension_id", &[("id", id)])),
        ];
        let known: Vec<String> = parts.into_iter().flatten().collect();
        if known.is_empty() { t("settings.companion.no_details").to_owned() } else { known.join(" · ") }
    }
}

fn companions_from(value: &Value) -> Vec<CompanionRow> {
    let text = |companion: &Value, key: &str| companion.get(key).and_then(Value::as_str).map(str::to_owned);
    let mut rows: Vec<CompanionRow> = value
        .get("companions")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|companion| CompanionRow {
                    browser: text(companion, "browser"),
                    browser_version: text(companion, "browserVersion"),
                    os: text(companion, "os"),
                    arch: text(companion, "arch"),
                    extension_version: text(companion, "extensionVersion"),
                    extension_id: text(companion, "extensionId"),
                    connected_at: companion.get("connectedAt").and_then(Value::as_i64).unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default();
    rows.sort_by_key(|row| row.connected_at);
    rows
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

pub struct SettingsScreen {
    model: Entity<AppModel>,
    certificate: Entity<Certificate>,
    tunnel: Entity<Tunnel>,
    accounts: Vec<AccountRow>,
    /// The browser extensions connected to the companion server.
    companions: Vec<CompanionRow>,
    /// The account being signed out.
    signing_out: Option<String>,
    _subscriptions: Vec<Subscription>,
}

/// `settings.update` replaces the whole `requestDefaults` object, so a change sends all three keys.
fn set_request_default(model: &Entity<AppModel>, cx: &mut App, change: impl FnOnce(&mut RequestDefaults)) {
    model.update(cx, |model, cx| {
        let mut defaults = model.config().request_defaults.clone();
        change(&mut defaults);
        model.update_settings(json!({ "requestDefaults": defaults.to_json() }), cx)
    })
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
                if matches!(event, ModelEvent::CompanionsChanged | ModelEvent::EngineRestarted) {
                    this.refresh_companions(cx);
                }
            }),
        ];
        let certificate = cx.new(|cx| Certificate::new(model.clone(), cx));
        subscriptions.push(cx.observe(&certificate, |_, _, cx| cx.notify()));
        let tunnel = cx.new(|cx| Tunnel::new(model.clone(), cx));
        subscriptions.push(cx.observe(&tunnel, |_, _, cx| cx.notify()));
        let mut this = Self {
            model,
            certificate,
            tunnel,
            accounts: Vec::new(),
            companions: Vec::new(),
            signing_out: None,
            _subscriptions: subscriptions,
        };
        this.refresh_accounts(cx);
        this.refresh_companions(cx);
        this
    }

    fn refresh_companions(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "engine.companions", json!({}), |this, outcome, cx| {
            if let Ok(listed) = outcome {
                this.companions = companions_from(&listed);
                cx.notify();
            }
        });
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
            let ok_text = if account.workspaces.is_empty() {
                t("settings.account.sign_out")
            } else {
                t("settings.sign_out.confirm_remove")
            };
            let id = account_id.clone();
            confirm_danger(
                cx.weak_entity(),
                window,
                cx,
                t_with("settings.sign_out.title", &[("login", &account.info.login)]),
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
                this.report(t_with("settings.sign_out.failed", &[("message", &message)]), cx);
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
                let (mode, mode_name) = if is_dark {
                    ("Dark", t("settings.theme.mode.dark"))
                } else {
                    ("Light", t("settings.theme.mode.light"))
                };
                let label: SharedString = if name.contains(mode) {
                    name.clone()
                } else {
                    t_with("settings.theme.mode.label", &[("name", &name), ("mode", &mode_name)]).into()
                };
                (name, label)
            })
            .collect();
        let sizes: Vec<(SharedString, SharedString)> = theme::FONT_SIZES
            .iter()
            .map(|size| {
                (
                    SharedString::from(size.to_string()),
                    SharedString::from(t_with("settings.text.size.option", &[("size", size)])),
                )
            })
            .collect();

        SettingPage::new(t("settings.page.appearance")).icon(IconName::Palette).default_open(true).groups(vec![
            SettingGroup::new()
                .title(t("settings.theme.group"))
                .item(
                    SettingItem::new(
                        t("settings.theme.color.title"),
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
                    .description(t("settings.theme.color.description"))
                    .keywords([
                        t("settings.keywords.dark"),
                        t("settings.keywords.light"),
                        t("settings.keywords.colors"),
                        t("settings.keywords.mode"),
                    ]),
                )
                .footer(|_, _| t("settings.theme.footer")),
            SettingGroup::new().title(t("settings.language.title")).item(
                SettingItem::new(
                    t("settings.language.title"),
                    SettingField::dropdown(
                        i18n::LANGUAGES
                            .iter()
                            .map(|language| (SharedString::from(language.code), SharedString::from(language.name)))
                            .collect(),
                        |_: &App| SharedString::from(i18n::language().code),
                        |code: SharedString, cx: &mut App| {
                            i18n::set_language(&code);
                            Preferences::update(cx, |p| p.language = Some(code.to_string()));
                            cx.refresh_windows();
                        },
                    ),
                )
                .description(t("settings.language.description")),
            ),
            SettingGroup::new().title(t("settings.text.group")).item(
                SettingItem::new(
                    t("settings.text.size.title"),
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
                .description(t("settings.text.size.description"))
                .keywords([t("settings.keywords.zoom"), t("settings.keywords.font")]),
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

    /// A row whose text is committed on Enter or when the field loses focus: blank sends `null` (the default
    /// applies), a value outside the engine's bounds is refused and the field goes back to what is stored.
    /// `state_key` names the field's own state, so a field whose meaning changes can start again.
    #[allow(clippy::too_many_arguments)]
    fn blank_default_item(
        &self,
        id: &'static str,
        state_key: String,
        key: &'static str,
        title: &'static str,
        description: &'static str,
        placeholder: String,
        accepts: Accepts,
        disabled: bool,
        read: fn(&EngineSettings) -> String,
    ) -> SettingItem {
        let model = self.model.clone();
        SettingItem::new(
            t(title),
            SettingField::render(move |_, window, cx| {
                let state = window.use_keyed_state(SharedString::from(state_key.clone()), cx, {
                    let (model, placeholder) = (model.clone(), placeholder.clone());
                    move |window, cx| {
                        let shown = read(&model.read(cx).config().settings);
                        let input =
                            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(shown));
                        let _subscription = cx.subscribe_in(&input, window, {
                            let model = model.clone();
                            move |_, input, event: &InputEvent, window, cx| {
                                if !matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                                    return;
                                }
                                let text = input.read(cx).value().trim().to_owned();
                                let stored = read(&model.read(cx).config().settings);
                                if text == stored {
                                    return;
                                }
                                match setting_value(accepts, &text) {
                                    Some(value) => {
                                        model.update(cx, |model, cx| model.update_settings(json!({ key: value }), cx))
                                    }
                                    None => {
                                        input.update(cx, |input, cx| input.set_value(stored, window, cx));
                                        model.update(cx, |model, cx| {
                                            let message = t_with("settings.invalid_value", &[("setting", &t(title))]);
                                            model.notice(message, cx)
                                        });
                                    }
                                }
                            }
                        });
                        BlankInput { input, _subscription }
                    }
                });
                div().id(id).test_support().w_40().child(Input::new(&state.read(cx).input).small().disabled(disabled))
            }),
        )
        .description(t(description))
    }

    /// Capture, mail and approvals: the engine settings that are blank when unset.
    fn behavior_groups(&self, cx: &App) -> Vec<SettingGroup> {
        let number = |min, max| Accepts::Number { min, max };
        let trace_on = self.model.read(cx).config().settings.trace_header.as_deref() != Some("");
        let trace_switch = {
            let (read, write) = (self.model.clone(), self.model.clone());
            SettingItem::new(
                t("settings.trace.switch.title"),
                SettingField::switch(
                    move |cx: &App| read.read(cx).config().settings.trace_header.as_deref() != Some(""),
                    // On is "not set" (the default name), off is the empty name the engine reads as off.
                    move |on: bool, cx: &mut App| {
                        let value = if on { Value::Null } else { json!("") };
                        write.update(cx, |model, cx| model.update_settings(json!({ "traceHeader": value }), cx))
                    },
                ),
            )
            .description(t("settings.trace.switch.description"))
        };
        vec![
            SettingGroup::new()
                .title(t("settings.capture.group"))
                .item(self.blank_default_item(
                    "setting-capture-limit",
                    "setting-capture-limit".into(),
                    "captureLimit",
                    "settings.capture.limit.title",
                    "settings.capture.limit.description",
                    DEFAULT_CAPTURE_LIMIT.to_string(),
                    number(10, 10_000),
                    false,
                    |s| s.capture_limit.map(|n| n.to_string()).unwrap_or_default(),
                ))
                .item(trace_switch)
                .item(self.blank_default_item(
                    "setting-trace-header",
                    format!("setting-trace-header-{trace_on}"),
                    "traceHeader",
                    "settings.trace.name.title",
                    "settings.trace.name.description",
                    DEFAULT_TRACE_HEADER.to_owned(),
                    Accepts::HeaderName,
                    !trace_on,
                    |s| s.trace_header.clone().unwrap_or_default(),
                ))
                .footer(|_, _| t("settings.capture.footer")),
            SettingGroup::new()
                .title(t("settings.mail.group"))
                .item(self.blank_default_item(
                    "setting-mail-port",
                    "setting-mail-port".into(),
                    "mailPort",
                    "settings.mail.port.title",
                    "settings.mail.port.description",
                    DEFAULT_MAIL_PORT.to_string(),
                    number(1, 65_535),
                    false,
                    |s| s.mail_port.map(|n| n.to_string()).unwrap_or_default(),
                ))
                .footer(|_, _| t("settings.mail.footer")),
            SettingGroup::new()
                .title(t("settings.approvals.group"))
                .item(self.blank_default_item(
                    "setting-approval-timeout",
                    "setting-approval-timeout".into(),
                    "approvalTimeoutSeconds",
                    "settings.approvals.timeout.title",
                    "settings.approvals.timeout.description",
                    DEFAULT_APPROVAL_TIMEOUT_SECONDS.to_string(),
                    number(10, 3600),
                    false,
                    |s| s.approval_timeout_seconds.map(|n| n.to_string()).unwrap_or_default(),
                ))
                .footer(|_, _| t("settings.approvals.footer")),
        ]
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
                (None, true) => Tag::success().child(t_with("settings.listener.running", &[("port", &listener.port)])),
                (None, false) => Tag::secondary().child(t("settings.listener.stopped")),
            };
            h_flex().w_full().gap_2().child(div().flex_1().child(name)).child(state).child(
                Button::new(id)
                    .with_size(options.size())
                    .label(if listener.running { t("settings.listener.stop") } else { t("settings.listener.start") })
                    .loading(listener.busy)
                    .on_click(move |_, _, cx| toggle_model.update(cx, toggle)),
            )
        })
        .keywords(keywords)
    }

    /// The browser extensions connected right now, or a line saying there are none.
    fn companions_item(&self, cx: &Context<Self>) -> SettingItem {
        let companions = self.companions.clone();
        let theme = cx.theme().clone();
        SettingItem::render(move |_, _, _| {
            let rows: Vec<_> = if companions.is_empty() {
                vec![div().text_color(theme.muted_foreground).child(t("settings.companion.none")).into_any_element()]
            } else {
                companions
                    .iter()
                    .map(|companion| {
                        v_flex()
                            .w_full()
                            .child(companion.title())
                            .child(div().text_xs().text_color(theme.muted_foreground).child(companion.detail()))
                            .into_any_element()
                    })
                    .collect()
            };
            v_flex().w_full().gap_2().children(rows)
        })
        .keywords([
            t("settings.keywords.browser"),
            t("settings.keywords.extension"),
            t("settings.keywords.connected"),
            "chrome",
            "firefox",
            "edge",
        ])
    }

    fn servers_page(&self, cx: &Context<Self>) -> SettingPage {
        let engine_model = self.model.clone();
        let companions = self.companions_item(cx);
        SettingPage::new(t("settings.page.servers")).icon(IconName::Network).groups(vec![
            SettingGroup::new()
                .title(t("settings.engine.runtime_group"))
                .item(
                    SettingItem::render(move |options, _, cx| {
                        let theme = cx.theme().clone();
                        let model = engine_model.read(cx);
                        let restarting = model.is_restarting_engine();
                        let engine = model.engine();
                        let detail = t_with(
                            "settings.engine.detail",
                            &[("version", &engine.hello.engine_version), ("pid", &engine.info.pid)],
                        );
                        let restart_model = engine_model.clone();
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                gpui_kit::component::v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(t("settings.engine.name"))
                                    .child(div().text_xs().text_color(theme.muted_foreground).child(detail)),
                            )
                            .child(
                                Button::new("restart-engine")
                                    .with_size(options.size())
                                    .label(t("settings.engine.restart"))
                                    .loading(restarting)
                                    .disabled(restarting)
                                    .on_click(move |_, _, cx| restart_model.update(cx, |m, cx| m.restart_engine(cx))),
                            )
                    })
                    .keywords([
                        t("settings.keywords.restart"),
                        t("settings.keywords.kill"),
                        t("settings.keywords.respawn"),
                        t("settings.keywords.runtime"),
                        t("settings.keywords.process"),
                    ]),
                )
                .footer(|_, _| t("settings.engine.restart_footer")),
            SettingGroup::new()
                .title(t("settings.proxy.title"))
                .item(self.listener_item(
                    "proxy-start-stop",
                    t("settings.forward_proxy.title"),
                    [t("settings.keywords.start"), t("settings.keywords.stop"), t("settings.keywords.forward")],
                    AppModel::proxy,
                    AppModel::toggle_proxy,
                ))
                .item(self.port_item(
                    t("settings.port.title"),
                    t("settings.port.forward_description"),
                    "forwardProxyPort",
                ))
                .footer(|_, _| t("settings.proxy.footer")),
            SettingGroup::new()
                .title(t("settings.reverse_proxy.title"))
                .item(self.listener_item(
                    "reverse-proxy-start-stop",
                    t("settings.reverse_proxy.title"),
                    [t("settings.keywords.start"), t("settings.keywords.stop"), "localhost"],
                    AppModel::reverse_proxy,
                    AppModel::toggle_reverse_proxy,
                ))
                .item(self.port_item(t("settings.port.title"), t("settings.port.reverse_description"), "port"))
                .footer(|_, _| t("settings.reverse_proxy.footer")),
            SettingGroup::new()
                .title(t("settings.companion.title"))
                .item(self.port_item(
                    t("settings.port.title"),
                    t("settings.port.companion_description"),
                    "companionPort",
                ))
                .item(companions)
                .footer(|_, _| t("settings.companion.footer")),
        ])
    }

    /// The defaults a request's own Settings tab overrides.
    fn requests_page(&self) -> SettingPage {
        let model = &self.model;
        let tunnel = self.tunnel.clone();
        let flag = |title, description, read: fn(&RequestDefaults) -> bool, change: fn(&mut RequestDefaults, bool)| {
            let (reader, writer) = (model.clone(), model.clone());
            SettingItem::new(
                t(title),
                SettingField::switch(
                    move |cx: &App| read(&reader.read(cx).config().request_defaults),
                    move |value: bool, cx: &mut App| {
                        set_request_default(&writer, cx, |defaults| change(defaults, value))
                    },
                ),
            )
            .description(t(description))
        };
        let (timeout_reader, timeout_writer) = (model.clone(), model.clone());
        SettingPage::new(t("settings.page.requests")).icon(IconName::Globe).groups(vec![
            SettingGroup::new()
                .title(t("settings.requests.group"))
                .item(flag(
                    "settings.requests.verify_tls.title",
                    "settings.requests.verify_tls.description",
                    |defaults| defaults.verify_tls,
                    |defaults, value| defaults.verify_tls = value,
                ))
                .item(flag(
                    "settings.requests.follow_redirects.title",
                    "settings.requests.follow_redirects.description",
                    |defaults| defaults.follow_redirects,
                    |defaults, value| defaults.follow_redirects = value,
                ))
                .item(
                    SettingItem::new(
                        t("settings.requests.timeout.title"),
                        SettingField::number_input(
                            NumberFieldOptions { min: 0., max: 3_600_000., step: 1000. },
                            move |cx: &App| timeout_reader.read(cx).config().request_defaults.timeout_ms as f64,
                            move |value: f64, cx: &mut App| {
                                // The field reports every keystroke; only a whole, non-negative number is sent.
                                if value >= 0.0 && value.fract() == 0.0 {
                                    set_request_default(&timeout_writer, cx, |defaults| {
                                        defaults.timeout_ms = value as u64
                                    })
                                }
                            },
                        ),
                    )
                    .description(t("settings.requests.timeout.description")),
                )
                .footer(|_, _| t("settings.requests.footer")),
            SettingGroup::new().title(t("settings.capture.group")).item(
                SettingItem::render(move |options, _, cx| {
                    let install = tunnel.clone();
                    let state = tunnel.read(cx);
                    let status = if !state.supported {
                        t("settings.capture.unsupported")
                    } else if state.installed {
                        t("settings.capture.installed")
                    } else {
                        t("settings.capture.missing")
                    };
                    h_flex().w_full().gap_2().child(div().flex_1().min_w_0().child(status)).child(
                        Button::new("install-cloudflared")
                            .with_size(options.size())
                            .label(if state.installed {
                                t("settings.capture.reinstall")
                            } else {
                                t("settings.capture.install")
                            })
                            .loading(state.installing)
                            .disabled(!state.supported || state.installing)
                            .on_click(move |_, _, cx| install.update(cx, |tunnel, cx| tunnel.install(cx))),
                    )
                })
                .keywords(["cloudflared", "tunnel", "webhook", "capture"]),
            ),
        ])
    }

    fn https_page(&self) -> SettingPage {
        let tls_read = self.model.clone();
        let tls_write = self.model.clone();
        let control = self.certificate.clone();
        let install_control = control.clone();
        SettingPage::new(t("settings.page.https")).icon(IconName::Globe).groups(vec![
            SettingGroup::new().title(t("settings.https.interception")).item(
                SettingItem::new(
                    t("settings.https.intercept.title"),
                    SettingField::switch(
                        move |cx: &App| tls_read.read(cx).config().settings.tls_enabled,
                        move |value: bool, cx: &mut App| {
                            tls_write.update(cx, |model, cx| model.update_settings(json!({ "tlsEnabled": value }), cx))
                        },
                    ),
                )
                .description(t("settings.https.intercept.description"))
                .keywords(["tls", "ssl", t("settings.keywords.certificate")]),
            ),
            SettingGroup::new()
                .title(t("settings.certificate.root_group"))
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
                                    .child(if certificate.generated {
                                        t("settings.certificate.in_place")
                                    } else {
                                        t("settings.certificate.none")
                                    })
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
                                    .label(if certificate.generated {
                                        t("settings.certificate.regenerate")
                                    } else {
                                        t("settings.certificate.generate")
                                    })
                                    .loading(certificate.busy)
                                    .disabled(working)
                                    .on_click(move |_, _, cx| {
                                        generate.update(cx, |certificate, cx| certificate.generate(cx))
                                    }),
                            )
                            .child(
                                Button::new("import-certificate")
                                    .with_size(options.size())
                                    .label(t("settings.certificate.import"))
                                    .loading(certificate.importing)
                                    .disabled(working)
                                    .tooltip(t("settings.certificate.import_tooltip"))
                                    .on_click(move |_, _, cx| {
                                        import.update(cx, |certificate, cx| certificate.import(cx))
                                    }),
                            )
                            .child(
                                Button::new("remove-certificate")
                                    .with_size(options.size())
                                    .label(t("settings.certificate.remove"))
                                    .disabled(!certificate.generated || working)
                                    .on_click(move |_, _, cx| {
                                        remove.update(cx, |certificate, cx| certificate.remove(cx))
                                    }),
                            )
                    })
                    .keywords([
                        "ca",
                        t("settings.keywords.root"),
                        t("settings.keywords.fingerprint"),
                        t("settings.keywords.import"),
                        t("settings.keywords.own"),
                        t("settings.keywords.generate"),
                    ]),
                )
                .item(
                    SettingItem::render(move |options, _, cx| {
                        let theme = cx.theme().clone();
                        let install = install_control.clone();
                        let certificate = install_control.read(cx);
                        let note = certificate.note.clone().map(|(ok, text)| {
                            div().text_xs().text_color(if ok { theme.success } else { theme.danger }).child(text)
                        });
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                gpui_kit::component::v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(t("settings.certificate.trust"))
                                    .children(note),
                            )
                            .child(
                                Button::new("install-certificate")
                                    .with_size(options.size())
                                    .label(t("settings.certificate.install"))
                                    .loading(certificate.installing)
                                    .disabled(!certificate.generated || certificate.installing)
                                    .tooltip(t("settings.certificate.install_tooltip"))
                                    .on_click(move |_, _, cx| {
                                        install.update(cx, |certificate, cx| certificate.install(cx))
                                    }),
                            )
                    })
                    .keywords([
                        t("settings.keywords.trust"),
                        t("settings.keywords.install"),
                        t("settings.keywords.keychain"),
                        t("settings.keywords.store"),
                    ]),
                )
                .footer(|_, _| t("settings.certificate.footer")),
        ])
    }

    /// The GitHub accounts signed in on this computer: one group each, with the workspaces that sync as it.
    fn accounts_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let page = SettingPage::new(t("settings.page.accounts")).icon(IconName::Github);
        if self.accounts.is_empty() {
            return page.groups(vec![
                SettingGroup::new()
                    .title(t("settings.account.github"))
                    .item(SettingItem::render(|_, _, cx| {
                        div().text_color(cx.theme().muted_foreground).child(t("settings.account.none"))
                    }))
                    .footer(|_, _| t("settings.account.none_footer")),
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
                let state = if signed_in {
                    Tag::success().child(t("settings.account.signed_in"))
                } else {
                    Tag::warning().child(t("settings.account.sign_in_expired"))
                };
                let detail = match count {
                    0 => t_with("settings.account.no_workspaces", &[("host", &host)]),
                    1 => t_with("settings.account.workspaces_one", &[("host", &host)]),
                    count => t_with("settings.account.workspaces_other", &[("host", &host), ("count", &count)]),
                };
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(div().flex_1().min_w_0().text_color(theme.muted_foreground).child(detail))
                    .child(state)
                    .children((!signed_in).then(|| {
                        Button::new(("sign-in-again", index))
                            .with_size(options.size())
                            .primary()
                            .label(t("settings.account.sign_in_again"))
                            .on_click(move |_, window, cx| {
                                let purpose = Purpose::SignIn { account_id: relogin_id.clone() };
                                open_setup_dialog(model.clone(), purpose, window, cx);
                            })
                    }))
                    .child(
                        Button::new(("sign-out", index))
                            .with_size(options.size())
                            .label(t("settings.account.sign_out"))
                            .loading(busy)
                            .disabled(blocked)
                            .on_click(move |_, window, cx| {
                                this.update(cx, |this, cx| this.ask_sign_out(id.clone(), window, cx)).ok();
                            }),
                    )
            })
            .keywords([
                "github",
                t("settings.keywords.login"),
                t("settings.keywords.logout"),
                t("settings.keywords.sign_out"),
            ]);
            let workspaces = account.workspaces.iter().cloned().map(|workspace| {
                SettingItem::render(move |_, _, cx| {
                    let theme = cx.theme().clone();
                    let state = match workspace.unsynced() {
                        Some(unsynced) => Tag::warning().child(capitalized(&unsynced)),
                        None => Tag::secondary().child(t("settings.workspace.synced")),
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
                .footer(|_, _| t("settings.account.footer"))
        });
        page.groups(groups.collect::<Vec<_>>())
    }

    fn about_page(&self, cx: &App) -> SettingPage {
        let engine = self.model.read(cx).engine().clone();
        let (telemetry_read, telemetry_write) = (self.model.clone(), self.model.clone());
        let policy_url = self.model.read(cx).setup().privacy_policy_url.clone();
        let terms_url = self.model.read(cx).setup().terms_of_service_url.clone();
        let fact = |title: &'static str, value: String| {
            SettingItem::new(
                title,
                SettingField::render(move |_, _, cx| {
                    div().text_color(cx.theme().muted_foreground).child(value.clone())
                }),
            )
        };
        let (update_read, update_write) = (self.model.clone(), self.model.clone());
        let mut updates = SettingGroup::new().title(t("settings.updates.group")).item(self.update_item());
        if let Some(available) = self.available_update_item(cx) {
            updates = updates.item(available);
        }
        let updates = updates
            .item(
                SettingItem::new(
                    t("settings.updates.automatic_title"),
                    SettingField::switch(
                        move |cx: &App| update_read.read(cx).config().settings.update_check_enabled,
                        move |value: bool, cx: &mut App| {
                            update_write.update(cx, |model, cx| {
                                model.update_settings(json!({ "updateCheckEnabled": value }), cx)
                            })
                        },
                    ),
                )
                .description(t("settings.updates.automatic_description"))
                .keywords([
                    t("settings.keywords.update"),
                    t("settings.keywords.automatic"),
                    t("settings.keywords.network"),
                ]),
            )
            .item(
                SettingItem::new(
                    t("settings.updates.releases_title"),
                    SettingField::render(|_, _, _| {
                        Link::new("releases-link").href(RELEASES_URL).child("github.com/HarshalKudale/bifurc-releases")
                    }),
                )
                .description(t("settings.updates.releases_description"))
                .layout(Axis::Horizontal),
            )
            .item(SettingItem::new(
                t("settings.updates.source_title"),
                SettingField::render(|_, _, _| {
                    Link::new("repository-link").href(REPOSITORY_URL).child("github.com/HarshalKudale/bifurc")
                }),
            ));
        let origin = if engine.started_here() {
            t("settings.engine.started_here")
        } else {
            t("settings.engine.already_running")
        };
        SettingPage::new(t("settings.page.about")).icon(IconName::Info).groups(vec![
            SettingGroup::new().title("Bifurc").items(vec![
                fact(
                    t("settings.about.desktop_app"),
                    t_with("settings.version", &[("version", &env!("CARGO_PKG_VERSION"))]),
                ),
                fact(
                    t("settings.engine.name"),
                    t_with("settings.version", &[("version", &engine.hello.engine_version)]),
                ),
                fact(t("settings.about.protocol"), engine.hello.protocol_version.to_string()),
                fact(
                    t("settings.about.engine_process"),
                    t_with("settings.engine.process_detail", &[("pid", &engine.info.pid), ("origin", &origin)]),
                ),
            ]),
            SettingGroup::new()
                .title(t("settings.privacy.group"))
                .item(
                    SettingItem::new(
                        t("settings.telemetry.title"),
                        SettingField::switch(
                            move |cx: &App| telemetry_read.read(cx).config().settings.telemetry_enabled,
                            move |value: bool, cx: &mut App| {
                                telemetry_write.update(cx, |model, cx| {
                                    model.update_settings(json!({ "telemetryEnabled": value }), cx)
                                })
                            },
                        ),
                    )
                    .description(t("settings.telemetry.description"))
                    .keywords([
                        t("settings.keywords.telemetry"),
                        t("settings.keywords.tracking"),
                        "posthog",
                    ]),
                )
                .item(
                    SettingItem::new(
                        t("settings.about.terms"),
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
                        t("settings.about.privacy_policy"),
                        SettingField::render(move |_, _, _| {
                            Link::new("privacy-link")
                                .href(policy_url.clone())
                                .child(policy_url.trim_start_matches("https://").to_owned())
                        }),
                    )
                    .layout(Axis::Horizontal),
                ),
            SettingGroup::new().title(t("settings.review.group")).item(
                SettingItem::new(
                    t("settings.review.title"),
                    SettingField::render(|_, _, _| {
                        Link::new("review-link").href(STORE_REVIEW_URL).child(t("settings.review.link"))
                    }),
                )
                .description(t("settings.review.description"))
                .layout(Axis::Horizontal),
            ),
            updates,
        ])
    }

    /// The installed version, what the engine last found, and the button that has it look now.
    fn update_item(&self) -> SettingItem {
        let model = self.model.clone();
        SettingItem::render(move |options, _, cx| {
            let theme = cx.theme().clone();
            let status = model.read(cx).update().clone();
            let check_model = model.clone();
            let state = match (&status.available, status.checked) {
                (Some(update), _) => {
                    Tag::warning().child(t_with("settings.update.available", &[("version", &update.version)]))
                }
                (None, true) => Tag::success().child(t("settings.update.up_to_date")),
                (None, false) => Tag::secondary().child(t("settings.update.not_checked")),
            };
            let unsupported = status.unsupported.then_some(t("settings.update.unsupported"));
            let failure = status.error.as_ref().map(|error| t_with("settings.update.failed", &[("error", error)]));
            h_flex()
                .w_full()
                .gap_2()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(t_with("settings.version", &[("version", &env!("CARGO_PKG_VERSION"))]))
                        .child(div().text_xs().text_color(theme.muted_foreground).child(status.channel_note()))
                        .children(unsupported.map(|text| div().text_xs().text_color(theme.warning).child(text)))
                        .children(failure.map(|text| div().text_xs().text_color(theme.danger).child(text))),
                )
                .child(state)
                .child(
                    Button::new("check-for-update")
                        .with_size(options.size())
                        .label(t("settings.update.check"))
                        .loading(status.checking)
                        .disabled(status.checking)
                        .on_click(move |_, _, cx| check_model.update(cx, |model, cx| model.check_for_update(cx))),
                )
        })
        .keywords([
            t("settings.keywords.update"),
            t("settings.keywords.upgrade"),
            t("settings.keywords.version"),
            t("settings.keywords.release"),
            t("settings.keywords.latest"),
        ])
    }

    /// The newer version, when there is one: what it brings, the order to install in, and the way this install
    /// gets it.
    fn available_update_item(&self, cx: &App) -> Option<SettingItem> {
        let status = self.model.read(cx).update().clone();
        let (update, action) = (status.available?, status.action?);
        let progress = self.model.read(cx).update_progress().cloned();
        let model = self.model.clone();
        let item = SettingItem::render(move |options, _, cx| {
            let theme = cx.theme().clone();
            let security = update.critical.then(|| t("settings.update.security").to_owned());
            let notes = [update.summary.clone(), update.steps(), security];
            let get = match &action {
                UpdateAction::Updater => {
                    let apply_model = model.clone();
                    let busy = progress.as_ref().is_some_and(UpdateProgress::busy);
                    let failed = progress.as_ref().is_some_and(|progress| progress.state == "failed");
                    let note = progress.as_ref().and_then(|progress| match progress.state.as_str() {
                        "starting" => Some(t("settings.update.starting").to_owned()),
                        "downloading" => Some(match progress.percent() {
                            Some(percent) => t_with("settings.update.downloading", &[("percent", &percent)]),
                            None => t("settings.update.downloading_unknown").to_owned(),
                        }),
                        "installing" => Some(t("settings.update.installing").to_owned()),
                        "failed" => Some(t_with(
                            "settings.update.apply_failed",
                            &[("error", &progress.error.clone().unwrap_or_default())],
                        )),
                        _ => None,
                    });
                    v_flex()
                        .items_end()
                        .gap_1()
                        .child(
                            Button::new("update-now")
                                .with_size(options.size())
                                .label(t("settings.update.now"))
                                .loading(busy)
                                .disabled(busy)
                                .on_click(move |_, _, cx| apply_model.update(cx, |model, cx| model.apply_update(cx))),
                        )
                        .children(note.map(|note| {
                            div()
                                .text_xs()
                                .text_color(if failed { theme.danger } else { theme.muted_foreground })
                                .child(note)
                        }))
                        // The updater could not do it; the website still can.
                        .children(failed.then(|| {
                            Link::new("update-download")
                                .href(UPDATE_DOWNLOAD_PAGE)
                                .child(t_with("settings.update.download", &[("version", &update.next())]))
                        }))
                        .into_any_element()
                }
                UpdateAction::Store(url) => Link::new("update-store")
                    .href(url.clone())
                    .child(t("settings.update.open_store"))
                    .into_any_element(),
                UpdateAction::Download(url) => Link::new("update-download")
                    .href(url.clone())
                    .child(t_with("settings.update.download", &[("version", &update.next())]))
                    .into_any_element(),
                UpdateAction::Command(command) => {
                    let copied = command.clone();
                    h_flex()
                        .gap_2()
                        .child(div().font_family(theme.mono_font_family.clone()).child(command.clone()))
                        .child(
                            Button::new("copy-update-command")
                                .with_size(options.size())
                                .icon(IconName::Copy)
                                .label(t("settings.update.copy"))
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))
                                }),
                        )
                        .into_any_element()
                }
            };
            let skip = if update.critical {
                None
            } else if update.skipped {
                Some(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(t("settings.update.skipped"))
                        .into_any_element(),
                )
            } else {
                let (skip_model, version) = (model.clone(), update.version.clone());
                Some(
                    Button::new("skip-update")
                        .ghost()
                        .with_size(options.size())
                        .label(t("settings.update.skip"))
                        .on_click(move |_, _, cx| skip_model.update(cx, |model, cx| model.skip_update(&version, cx)))
                        .into_any_element(),
                )
            };
            let changes = update.notes_url.clone().map(|url| {
                Link::new("update-notes").href(url).child(t("settings.update.what_changed")).into_any_element()
            });
            h_flex()
                .w_full()
                .gap_3()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(t_with("settings.version", &[("version", &update.version)]))
                        .children(
                            notes
                                .into_iter()
                                .flatten()
                                .map(|note| div().text_xs().text_color(theme.muted_foreground).child(note)),
                        ),
                )
                .children(changes)
                .children(skip)
                .child(get)
        })
        .keywords([
            t("settings.keywords.update"),
            t("settings.keywords.upgrade"),
            t("settings.keywords.download"),
            t("settings.keywords.install"),
        ]);
        Some(item)
    }
}

impl Render for SettingsScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pages = vec![
            self.appearance_page(cx),
            self.accounts_page(cx),
            self.servers_page(cx).groups(self.behavior_groups(cx)),
            self.requests_page(),
            self.https_page(),
            self.about_page(cx),
            super::licenses::page(),
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
