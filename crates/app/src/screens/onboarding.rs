//! The first run: what the window shows until the app is set up and has a workspace.
//!
//! It is a short walk in a fixed order. The terms of service come first and the privacy policy second, and
//! nothing else is reachable until both are accepted; then what the app does, how it looks, HTTPS, the two
//! proxies' ports, and last the first workspace. Everything after the policy can be left as it is and changed
//! later in Settings. Usage analytics are not asked about here: accepting the terms and the policy is what
//! allows them, and the switch that turns them off is in Settings, under About.
//!
//! Nothing here is this view's to keep. That the terms and the policy were accepted and that the walk was
//! finished are the engine's (`setup.acceptTerms`, `setup.acceptPrivacy`, `setup.complete`), as are the ports,
//! the certificate and the workspace;
//! only the theme and text size are this client's own, kept in [`Preferences`].

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _, TitleBar,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    link::Link,
    menu::{DropdownMenu as _, PopupMenuItem},
    spinner::Spinner,
    stepper::{Stepper, StepperItem},
    switch::Switch,
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Div, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, rems,
};
use serde_json::{Value, json};

use crate::{
    app_model::{AppModel, Load, ModelEvent, ProxyStatus, SetupState},
    components::certificate::Certificate,
    engine::{call, call_in, failure},
    i18n::{self, t, t_with},
    preferences::Preferences,
    screens::workspace_setup::{Purpose, WorkspaceSetup},
    theme,
};

/// The walk's width in rems: room for a row's text beside its control.
const WIDTH: f32 = 44.;

/// The port a reverse proxy needs for `.localhost` names to work without one in the address.
const HTTP_PORT: u16 = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Language,
    Terms,
    Privacy,
    Overview,
    Preferences,
    Https,
    Proxies,
    Workspace,
}

impl Stage {
    const ALL: [Stage; 8] = [
        Stage::Language,
        Stage::Terms,
        Stage::Privacy,
        Stage::Overview,
        Stage::Preferences,
        Stage::Https,
        Stage::Proxies,
        Stage::Workspace,
    ];

    fn label(self) -> &'static str {
        t(match self {
            Stage::Language => "onboarding.stage.language.label",
            Stage::Terms => "onboarding.stage.terms.label",
            Stage::Privacy => "onboarding.stage.privacy.label",
            Stage::Overview => "onboarding.stage.overview.label",
            Stage::Preferences => "onboarding.stage.preferences.label",
            Stage::Https => "onboarding.stage.https.label",
            Stage::Proxies => "onboarding.stage.proxies.label",
            Stage::Workspace => "onboarding.stage.workspace.label",
        })
    }

    fn previous(self) -> Option<Stage> {
        let index = Self::ALL.iter().position(|stage| *stage == self)?;
        index.checked_sub(1).map(|index| Self::ALL[index])
    }
}

/// Where the walk starts. One that was left half way picks up after the terms and the policy, neither of
/// which is asked twice; one that was finished is only here because the last workspace is gone.
fn first_stage(setup: &SetupState, language_chosen: bool) -> Stage {
    match (setup.terms_accepted, setup.privacy_accepted, setup.onboarded) {
        (false, _, false) if !language_chosen => Stage::Language,
        (false, _, _) => Stage::Terms,
        (true, false, _) => Stage::Privacy,
        (true, true, false) => Stage::Overview,
        (true, true, true) => Stage::Workspace,
    }
}

/// A port as it was typed, if it is one.
fn parse_port(text: &str) -> Option<u16> {
    text.trim().parse::<u16>().ok().filter(|port| *port != 0)
}

/// How a mapping named `api.localhost` is opened when the reverse proxy listens on `port`.
fn mapping_address(port: u16) -> String {
    if port == HTTP_PORT { "http://api.localhost".to_owned() } else { format!("http://api.localhost:{port}") }
}

/// Ports below this one need permission to listen on, on Linux.
const FIRST_UNRESERVED_PORT: u16 = 1024;

/// Which of the two proxies `setup.startServers` could not start, and why, if one could not.
fn start_problem(result: &Value) -> Option<String> {
    let problems: Vec<String> = [("reverse", "onboarding.proxy.reverse"), ("forward", "onboarding.proxy.forward")]
        .into_iter()
        .filter_map(|(key, name)| {
            let listener = result.get(key)?;
            let error = listener.get("error").and_then(Value::as_str)?;
            let port = listener.get("port").and_then(Value::as_u64).unwrap_or_default();
            Some(t_with(
                "onboarding.problem.server_start",
                &[("name", &t(name)), ("port", &port), ("error", &error.trim_end_matches('.'))],
            ))
        })
        .collect();
    (!problems.is_empty()).then(|| problems.join(" "))
}

/// What is wrong with the two ports as typed, if something is.
fn ports_problem(reverse: Option<u16>, forward: Option<u16>) -> Option<&'static str> {
    match (reverse, forward) {
        (None, _) | (_, None) => Some(t("onboarding.problem.port_range")),
        (Some(reverse), Some(forward)) if reverse == forward => Some(t("onboarding.problem.ports_differ")),
        _ => None,
    }
}

pub struct Onboarding {
    model: Entity<AppModel>,
    certificate: Entity<Certificate>,
    workspace: Entity<WorkspaceSetup>,
    /// `None` until the engine has said how far this data directory already is.
    stage: Option<Stage>,
    /// The terms' checkbox is ticked.
    terms_accepted: bool,
    /// The policy's checkbox is ticked.
    accepted: bool,
    /// The acceptance or the completion is on its way to the engine.
    busy: bool,
    error: Option<String>,
    reverse_port: Entity<InputState>,
    forward_port: Entity<InputState>,
    /// The port fields show the engine's ports; before that, what they hold is not something the user typed.
    ports_filled: bool,
    /// The ports (reverse, forward) a proxy last failed to start with. Going on with the same two is the
    /// user's choice to sort it out later.
    servers_failed_on: Option<(u16, u16)>,
    /// The ports (reverse, forward) the engine last asked for permission to listen on.
    permission_asked_on: Option<(u16, u16)>,
    /// The engine was given that permission and is starting again to have it.
    awaiting_restart: bool,
    _subscriptions: Vec<Subscription>,
}

impl Onboarding {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let certificate = cx.new(|cx| Certificate::new(model.clone(), cx));
        let this = cx.weak_entity();
        let workspace = cx.new(|cx| {
            WorkspaceSetup::new(model.clone(), Purpose::First, window, cx).on_back(move |window, cx| {
                this.update(cx, |this, cx| this.go_to(Stage::Proxies, window, cx)).ok();
            })
        });
        let reverse_port = cx.new(|cx| InputState::new(window, cx).placeholder("80"));
        let forward_port = cx.new(|cx| InputState::new(window, cx).placeholder("8080"));
        let subscriptions = vec![
            cx.observe_in(&model, window, |this, _, window, cx| this.sync(window, cx)),
            // The engine is back with the permission it was given: the servers step picks up where it stopped.
            cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| {
                if matches!(event, ModelEvent::EngineRestarted) && this.awaiting_restart {
                    this.awaiting_restart = false;
                    this.busy = false;
                    this.start_servers(window, cx);
                }
            }),
            cx.observe(&certificate, |_, _, cx| cx.notify()),
            cx.subscribe(&reverse_port, |this, _, event: &InputEvent, cx| this.on_port_event(event, cx)),
            cx.subscribe(&forward_port, |this, _, event: &InputEvent, cx| this.on_port_event(event, cx)),
        ];
        let mut this = Self {
            model,
            certificate,
            workspace,
            stage: None,
            terms_accepted: false,
            accepted: false,
            busy: false,
            error: None,
            reverse_port,
            forward_port,
            ports_filled: false,
            servers_failed_on: None,
            permission_asked_on: None,
            awaiting_restart: false,
            _subscriptions: subscriptions,
        };
        this.sync(window, cx);
        this
    }

    fn has_workspace(&self, cx: &Context<Self>) -> bool {
        !self.model.read(cx).config().workspaces.is_empty()
    }

    /// The stages this walk has. A data directory that already holds a workspace is not asked for a first one.
    fn stages(&self, cx: &Context<Self>) -> Vec<Stage> {
        let has_workspace = self.has_workspace(cx);
        Stage::ALL.into_iter().filter(|stage| !(has_workspace && *stage == Stage::Workspace)).collect()
    }

    /// Follows the engine: where the walk starts, the ports it has, and the workspace that ends the walk.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let setup = model.setup().clone();
        let ready = *model.load() == Load::Ready;
        let settings = model.config().settings.clone();
        // The engine did not come back from the restart: nothing is on its way any more.
        if self.awaiting_restart && model.is_connection_lost() {
            self.awaiting_restart = false;
            self.busy = false;
        }
        if self.stage.is_none() && setup.known {
            self.stage = Some(first_stage(&setup, Preferences::global(cx).language.is_some()));
            self.terms_accepted = setup.terms_accepted;
            self.accepted = setup.privacy_accepted;
        }
        if !self.ports_filled && ready {
            self.ports_filled = true;
            self.reverse_port.update(cx, |input, cx| input.set_value(settings.proxy_port.to_string(), window, cx));
            self.forward_port
                .update(cx, |input, cx| input.set_value(settings.forward_proxy_port.to_string(), window, cx));
        }
        // The first workspace was made or imported: that is the walk's last step.
        if self.stage == Some(Stage::Workspace) && self.has_workspace(cx) && setup.known && !setup.onboarded {
            self.complete(cx);
        }
        cx.notify();
    }

    fn go_to(&mut self, stage: Stage, window: &mut Window, cx: &mut Context<Self>) {
        self.stage = Some(stage);
        self.error = None;
        if stage == Stage::Workspace {
            self.workspace.update(cx, |workspace, cx| workspace.focus(window, cx));
        }
        cx.notify();
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = self.stage.and_then(Stage::previous) {
            self.go_to(previous, window, cx);
        }
    }

    /// The stage's main action: on to the next one, through whatever the stage has to settle first.
    fn advance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        match self.stage {
            Some(Stage::Language) => {
                let code = i18n::language().code.to_owned();
                Preferences::update(cx, |p| p.language = Some(code));
                self.go_to(Stage::Terms, window, cx);
            }
            Some(Stage::Terms) => self.accept_terms(window, cx),
            Some(Stage::Privacy) => self.accept_privacy(window, cx),
            Some(Stage::Overview) => self.go_to(Stage::Preferences, window, cx),
            Some(Stage::Preferences) => self.go_to(Stage::Https, window, cx),
            Some(Stage::Https) => self.go_to(Stage::Proxies, window, cx),
            Some(Stage::Proxies) => self.start_servers(window, cx),
            Some(Stage::Workspace) | None => {}
        }
    }

    /// Hands the engine the two ports and has it start both proxies; from then on every run of the engine
    /// starts them by itself. A proxy that could not start keeps the walk here once, so its port can be
    /// changed; going on with the same ports leaves it for Settings.
    fn start_servers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ports_filled {
            return;
        }
        let (Some(reverse), Some(forward)) = self.typed_ports(cx) else { return };
        if reverse == forward {
            return;
        }
        let acknowledged = self.servers_failed_on == Some((reverse, forward));
        self.busy = true;
        self.error = None;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "port": reverse, "forwardProxyPort": forward });
        call_in(&client, window, cx, "setup.startServers", payload, move |this, outcome, window, cx| {
            this.busy = false;
            this.model.update(cx, |model, cx| {
                model.refresh(cx);
                model.refresh_setup(cx);
            });
            if let Some(message) = failure(&outcome) {
                this.error = Some(t_with("onboarding.error.start_proxies", &[("message", &message)]));
                cx.notify();
                return;
            }
            let result = outcome.ok();
            let denied = result.as_ref().and_then(|result| result.get("lowPortsDenied")).and_then(Value::as_bool);
            match result.as_ref().and_then(start_problem) {
                // The engine may be given the permission it lacks. It is asked for once for a pair of ports.
                Some(problem) if denied == Some(true) && this.permission_asked_on != Some((reverse, forward)) => {
                    this.permission_asked_on = Some((reverse, forward));
                    this.allow_low_ports(problem, cx);
                }
                Some(problem) if !acknowledged => {
                    this.servers_failed_on = Some((reverse, forward));
                    this.error = Some(t_with("onboarding.problem.choose_other", &[("problem", &problem)]));
                    cx.notify();
                }
                _ => {
                    this.servers_failed_on = None;
                    if this.has_workspace(cx) { this.complete(cx) } else { this.go_to(Stage::Workspace, window, cx) }
                }
            }
        });
    }

    /// Has the engine ask for the password that lets it listen below 1024. It has the permission from its
    /// next start, so it is restarted, and the proxies are started again once it is back. Refused, the
    /// walk stays here with `problem`, the port that could not be opened, as any other that failed.
    fn allow_low_ports(&mut self, problem: String, cx: &mut Context<Self>) {
        self.busy = true;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "setup.allowLowPorts", json!({}), move |this, outcome, cx| match failure(&outcome) {
            None => {
                this.awaiting_restart = true;
                this.model.update(cx, |model, cx| model.restart_engine(cx));
            }
            Some(why) => {
                this.busy = false;
                this.servers_failed_on = this.permission_asked_on;
                this.error = Some(t_with(
                    "onboarding.problem.low_ports_refused",
                    &[
                        ("problem", &problem),
                        ("reserved", &FIRST_UNRESERVED_PORT),
                        ("why", &why.trim_end_matches('.')),
                    ],
                ));
                cx.notify();
            }
        });
    }

    fn accept_terms(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.terms_accepted {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call_in(&client, window, cx, "setup.acceptTerms", json!({}), |this, outcome, window, cx| {
            this.busy = false;
            match failure(&outcome) {
                Some(message) => {
                    this.error = Some(t_with("onboarding.error.accepted", &[("message", &message)]));
                    cx.notify();
                }
                None => {
                    // What comes next is where the walk would start with the terms accepted.
                    let mut setup = this.model.read(cx).setup().clone();
                    setup.terms_accepted = true;
                    this.model.update(cx, |model, cx| model.refresh_setup(cx));
                    this.go_to(first_stage(&setup, true), window, cx);
                }
            }
        });
    }

    fn accept_privacy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.accepted {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call_in(&client, window, cx, "setup.acceptPrivacy", json!({}), |this, outcome, window, cx| {
            this.busy = false;
            match failure(&outcome) {
                Some(message) => {
                    this.error = Some(t_with("onboarding.error.accepted", &[("message", &message)]));
                    cx.notify();
                }
                None => {
                    let mut setup = this.model.read(cx).setup().clone();
                    setup.privacy_accepted = true;
                    this.model.update(cx, |model, cx| model.refresh_setup(cx));
                    this.go_to(first_stage(&setup, true), window, cx);
                }
            }
        });
    }

    /// Tells the engine the walk is done. The shell shows the app once the engine says so.
    fn complete(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "setup.complete", json!({}), |this, outcome, cx| {
            this.busy = false;
            if let Some(message) = failure(&outcome) {
                this.error = Some(t_with("onboarding.error.finish", &[("message", &message)]));
            }
            this.model.update(cx, |model, cx| model.refresh_setup(cx));
            cx.notify();
        });
    }

    fn typed_ports(&self, cx: &Context<Self>) -> (Option<u16>, Option<u16>) {
        (parse_port(&self.reverse_port.read(cx).value()), parse_port(&self.forward_port.read(cx).value()))
    }

    /// Both ports as typed, when both are ports.
    fn typed_port_pair(&self, cx: &Context<Self>) -> Option<(u16, u16)> {
        let (reverse, forward) = self.typed_ports(cx);
        reverse.zip(forward)
    }

    /// A port is sent when its field is left or Enter is pressed in it, not on every key: each new port is a
    /// listener the engine opens.
    fn on_port_event(&mut self, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Blur | InputEvent::PressEnter { .. } => {
                self.apply_ports(cx);
            }
            InputEvent::Change | InputEvent::Focus => {}
        }
        cx.notify();
    }

    /// Sends the ports that differ from the engine's. `false` when what is typed cannot be used.
    fn apply_ports(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.ports_filled {
            return false;
        }
        let (Some(reverse), Some(forward)) = self.typed_ports(cx) else { return false };
        if reverse == forward {
            return false;
        }
        let settings = self.model.read(cx).config().settings.clone();
        let mut patch = serde_json::Map::new();
        if reverse != settings.proxy_port {
            patch.insert("port".to_owned(), json!(reverse));
        }
        if forward != settings.forward_proxy_port {
            patch.insert("forwardProxyPort".to_owned(), json!(forward));
        }
        if !patch.is_empty() {
            self.model.update(cx, |model, cx| model.update_settings(patch.into(), cx));
        }
        true
    }

    /// A label and what it means on the leading side, its control on the trailing edge.
    fn row(
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        control: impl IntoElement,
        cx: &Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        h_flex()
            .w_full()
            .gap_4()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(div().text_sm().font_medium().child(title.into()))
                    .child(div().text_sm().text_color(theme.muted_foreground).child(description.into())),
            )
            .child(h_flex().flex_none().gap_2().child(control))
    }

    /// Rows that belong together.
    fn group(cx: &Context<Self>) -> Div {
        let theme = cx.theme();
        v_flex().w_full().p_4().gap_4().rounded(theme.radius).border_1().border_color(theme.border)
    }

    fn render_terms(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let url = self.model.read(cx).setup().terms_of_service_url.clone();
        let shown = url.trim_start_matches("https://").to_owned();
        v_flex()
            .gap_4()
            .child(t("onboarding.terms.body"))
            .child(
                h_flex()
                    .gap_1()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(t("onboarding.terms.link_label"))
                    .child(Link::new("terms-of-service").href(url).child(shown)),
            )
            .child(
                Checkbox::new("accept-terms")
                    .label(t("onboarding.terms.accept_label"))
                    .checked(self.terms_accepted)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        this.terms_accepted = *checked;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn render_privacy(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let url = self.model.read(cx).setup().privacy_policy_url.clone();
        let shown = url.trim_start_matches("https://").to_owned();
        v_flex()
            .gap_4()
            .child(t("onboarding.privacy.body"))
            .child(
                h_flex()
                    .gap_1()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(t("onboarding.privacy.link_label"))
                    .child(Link::new("privacy-policy").href(url).child(shown)),
            )
            .child(
                Checkbox::new("accept-privacy")
                    .label(t("onboarding.privacy.accept_label"))
                    .checked(self.accepted)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        this.accepted = *checked;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn render_overview(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let parts: [(IconName, &str, &str); 6] = [
            (
                IconName::Globe,
                t("onboarding.overview.requests.title"),
                t("onboarding.overview.requests.description"),
            ),
            (
                IconName::GalleryVerticalEnd,
                t("onboarding.overview.mocks.title"),
                t("onboarding.overview.mocks.description"),
            ),
            (
                IconName::Replace,
                t("onboarding.overview.mappings.title"),
                t("onboarding.overview.mappings.description"),
            ),
            (
                IconName::Network,
                t("onboarding.overview.proxy_rules.title"),
                t("onboarding.overview.proxy_rules.description"),
            ),
            (
                IconName::Asterisk,
                t("onboarding.overview.environments.title"),
                t("onboarding.overview.environments.description"),
            ),
            (
                IconName::Folder,
                t("onboarding.overview.workspaces.title"),
                t("onboarding.overview.workspaces.description"),
            ),
        ];
        Self::group(cx)
            .children(parts.into_iter().map(|(icon, title, description)| {
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_start()
                    .child(
                        h_flex()
                            .flex_none()
                            .size_6()
                            .justify_center()
                            .child(Icon::new(icon).text_color(theme.muted_foreground)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(div().text_sm().font_medium().child(title))
                            .child(div().text_sm().text_color(theme.muted_foreground).child(description)),
                    )
            }))
            .into_any_element()
    }

    fn render_language(&self, cx: &mut Context<Self>) -> AnyElement {
        let chosen = i18n::language().code;
        let menu = Button::new("onboarding-language")
            .outline()
            .label(i18n::language().name)
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                i18n::LANGUAGES.iter().fold(menu, |menu, language| {
                    let code = language.code;
                    menu.item(PopupMenuItem::new(language.name).checked(code == chosen).on_click(move |_, _, cx| {
                        i18n::set_language(code);
                        Preferences::update(cx, |p| p.language = Some(code.to_owned()));
                        cx.refresh_windows();
                    }))
                })
            });
        Self::group(cx)
            .child(Self::row(t("onboarding.language.row.title"), t("onboarding.language.row.description"), menu, cx))
            .into_any_element()
    }

    fn render_preferences(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = theme::current(cx);
        let themes = theme::available(cx);
        let chosen = current.clone();
        let theme_menu = Button::new("onboarding-theme").outline().label(current).dropdown_caret(true).dropdown_menu(
            move |menu, window, _| {
                let menu = menu.scrollable(true).max_h(rems(20.).to_pixels(window.rem_size()));
                themes.iter().fold(menu, |menu, (name, is_dark)| {
                    // Most themes say which they are in their name; the rest get it appended.
                    let mode = t(if *is_dark { "onboarding.preferences.theme.dark" } else { "onboarding.preferences.theme.light" });
                    let label: SharedString =
                        if name.contains(&mode) { name.clone() } else { format!("{name} ({mode})").into() };
                    let name = name.clone();
                    menu.item(PopupMenuItem::new(label).checked(name == chosen).on_click(move |_, _, cx| {
                        if theme::apply(&name, cx) {
                            Preferences::update(cx, |p| p.theme = Some(name.to_string()));
                        }
                    }))
                })
            },
        );

        let size = Preferences::global(cx).font_size;
        let size_menu =
            Button::new("onboarding-size").outline().label(format!("{size} px")).dropdown_caret(true).dropdown_menu(
                move |menu, _, _| {
                    theme::FONT_SIZES.iter().fold(menu, |menu, choice| {
                        let choice = *choice;
                        menu.item(PopupMenuItem::new(format!("{choice} px")).checked(choice == size).on_click(
                            move |_, _, cx| {
                                theme::set_font_size(choice, cx);
                                Preferences::update(cx, |p| p.font_size = choice);
                                cx.refresh_windows();
                            },
                        ))
                    })
                },
            );

        Self::group(cx)
            .child(Self::row(t("onboarding.preferences.theme.title"), t("onboarding.preferences.theme.description"), theme_menu, cx))
            .child(Self::row(t("onboarding.preferences.size.title"), t("onboarding.preferences.size.description"), size_menu, cx))
            .into_any_element()
    }

    fn render_https(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let certificate = self.certificate.read(cx);
        let working = certificate.working();
        let (generate, import, install) =
            (self.certificate.clone(), self.certificate.clone(), self.certificate.clone());
        let state: SharedString = match (&certificate.fingerprint, certificate.generated) {
            (Some(fingerprint), true) => fingerprint.clone().into(),
            (None, true) => t("onboarding.https.certificate.in_place").into(),
            (_, false) => t("onboarding.https.certificate.none").into(),
        };
        let certificate_actions = h_flex()
            .gap_2()
            .child(
                Button::new("onboarding-generate")
                    .outline()
                    .label(if certificate.generated {
                        t("onboarding.https.regenerate")
                    } else {
                        t("onboarding.https.generate")
                    })
                    .loading(certificate.busy)
                    .disabled(working)
                    .on_click(move |_, _, cx| generate.update(cx, |certificate, cx| certificate.generate(cx))),
            )
            .child(
                Button::new("onboarding-import")
                    .outline()
                    .label(t("onboarding.https.import"))
                    .loading(certificate.importing)
                    .disabled(working)
                    .tooltip(t("onboarding.https.import_tooltip"))
                    .on_click(move |_, _, cx| import.update(cx, |certificate, cx| certificate.import(cx))),
            );
        let install_action = Button::new("onboarding-install")
            .outline()
            .label(t("onboarding.https.install"))
            .loading(certificate.installing)
            .disabled(!certificate.generated || certificate.installing)
            .on_click(move |_, _, cx| install.update(cx, |certificate, cx| certificate.install(cx)));
        let note = certificate
            .note
            .clone()
            .map(|(ok, text)| div().text_sm().text_color(if ok { theme.success } else { theme.danger }).child(text));

        let model = self.model.clone();
        let intercept = Switch::new("onboarding-intercept")
            .checked(self.model.read(cx).config().settings.tls_enabled)
            .on_change(move |checked: &bool, _, cx| {
                let checked = *checked;
                model.update(cx, |model, cx| model.update_settings(json!({ "tlsEnabled": checked }), cx));
            });

        Self::group(cx)
            .child(Self::row(t("onboarding.https.root.title"), state, certificate_actions, cx))
            .child(
                v_flex()
                    .gap_2()
                    .child(Self::row(
                        t("onboarding.https.trust.title"),
                        t("onboarding.https.trust.description"),
                        install_action,
                        cx,
                    ))
                    .children(note),
            )
            .child(Self::row(
                t("onboarding.https.intercept.title"),
                t("onboarding.https.intercept.description"),
                intercept,
                cx,
            ))
            .into_any_element()
    }

    /// A listener's state and the button that starts or stops it.
    fn listener_controls(
        id: &'static str,
        listener: &ProxyStatus,
        model: Entity<AppModel>,
        toggle: fn(&mut AppModel, &mut Context<AppModel>),
    ) -> Div {
        let state = match (&listener.error, listener.running) {
            (Some(error), _) => Tag::danger().child(error.clone()),
            (None, true) => Tag::success().child(t_with("onboarding.listener.running", &[("port", &listener.port)])),
            (None, false) => Tag::secondary().child(t("onboarding.listener.stopped")),
        };
        h_flex().gap_2().child(state).child(
            Button::new(id)
                .outline()
                .small()
                .label(if listener.running { t("onboarding.listener.stop") } else { t("onboarding.listener.start") })
                .loading(listener.busy)
                .on_click(move |_, _, cx| model.update(cx, |model, cx| toggle(model, cx))),
        )
    }

    fn render_proxies(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let model = self.model.read(cx);
        let (reverse_status, forward_status) = (model.reverse_proxy().clone(), model.proxy().clone());
        let (reverse, forward) = self.typed_ports(cx);
        let problem = ports_problem(reverse, forward).filter(|_| self.ports_filled);
        let port_field = |input: &Entity<InputState>| div().w_24().child(Input::new(input));
        let muted = |text: &'static str| div().text_sm().text_color(theme.muted_foreground).child(text);

        let reverse_group = Self::group(cx)
            .child(Self::row(
                t("onboarding.reverse.title"),
                t("onboarding.reverse.description"),
                Self::listener_controls(
                    "onboarding-reverse-toggle",
                    &reverse_status,
                    self.model.clone(),
                    AppModel::toggle_reverse_proxy,
                ),
                cx,
            ))
            .child(Self::row(
                t("onboarding.port.title"),
                t_with(
                    "onboarding.reverse.port_description",
                    &[
                        ("port", &HTTP_PORT),
                        ("plain", &mapping_address(HTTP_PORT)),
                        ("example", &mapping_address(3000)),
                    ],
                ),
                port_field(&self.reverse_port),
                cx,
            ))
            .children(reverse.filter(|port| *port != HTTP_PORT).map(|port| {
                Alert::warning(
                    "reverse-port-note",
                    t_with(
                        "onboarding.reverse.port_note",
                        &[
                            ("port", &port),
                            ("address", &mapping_address(port)),
                            ("plain", &mapping_address(HTTP_PORT)),
                        ],
                    ),
                )
                .title(t_with("onboarding.reverse.port_names_need", &[("port", &port)]))
            }))
            .children(reverse.filter(|port| cfg!(target_os = "linux") && *port < FIRST_UNRESERVED_PORT).map(|port| {
                Alert::warning(
                    "reverse-port-permission",
                    t_with(
                        "onboarding.reverse.permission_note",
                        &[("reserved", &FIRST_UNRESERVED_PORT), ("port", &port)],
                    ),
                )
                .title(t_with("onboarding.reverse.permission_title", &[("port", &port)]))
            }));

        let forward_group = Self::group(cx)
            .child(Self::row(
                t("onboarding.forward.title"),
                t("onboarding.forward.description"),
                Self::listener_controls(
                    "onboarding-forward-toggle",
                    &forward_status,
                    self.model.clone(),
                    AppModel::toggle_proxy,
                ),
                cx,
            ))
            .child(Self::row(
                t("onboarding.port.title"),
                match forward {
                    Some(port) => t_with("onboarding.forward.address_known", &[("port", &port)]),
                    None => t("onboarding.forward.address_unknown").to_owned(),
                },
                port_field(&self.forward_port),
                cx,
            ));

        v_flex()
            .gap_4()
            .child(reverse_group)
            .child(forward_group)
            .children(problem.map(|problem| div().text_sm().text_color(theme.danger).child(problem)))
            .child(muted(if model.setup().servers_confirmed {
                t("onboarding.proxies.takes_effect")
            } else {
                t("onboarding.proxies.not_started")
            }))
            .into_any_element()
    }

    /// Where the walk is, among the stages it has. A stage that was passed can be gone back to.
    fn render_steps(&self, stages: &[Stage], current: usize, cx: &mut Context<Self>) -> AnyElement {
        let stages_for_click = stages.to_vec();
        Stepper::new("onboarding-steps")
            .small()
            .selected_index(current)
            .items(
                stages.iter().enumerate().map(|(index, stage)| {
                    StepperItem::new().disabled(index > current || self.busy).child(stage.label())
                }),
            )
            .on_click(cx.listener(move |this, step: &usize, window, cx| {
                if let Some(stage) = stages_for_click.get(*step).copied()
                    && *step < current
                    && !this.busy
                {
                    this.go_to(stage, window, cx);
                }
            }))
            .into_any_element()
    }

    fn render_actions(&self, stage: Stage, cx: &mut Context<Self>) -> AnyElement {
        let label = t(match stage {
            Stage::Terms | Stage::Privacy => "onboarding.action.accept",
            Stage::Proxies if self.servers_failed_on.is_some() && self.servers_failed_on == self.typed_port_pair(cx) => {
                "onboarding.action.continue_anyway"
            }
            Stage::Proxies if self.has_workspace(cx) => "onboarding.action.finish",
            _ => "onboarding.action.continue",
        });
        let blocked = match stage {
            Stage::Terms => !self.terms_accepted,
            Stage::Privacy => !self.accepted,
            Stage::Proxies => {
                let (reverse, forward) = self.typed_ports(cx);
                !self.ports_filled || ports_problem(reverse, forward).is_some()
            }
            _ => false,
        };
        h_flex()
            .w_full()
            .gap_2()
            .when(stage.previous().is_some(), |this| {
                this.child(
                    Button::new("onboarding-back")
                        .outline()
                        .label(t("onboarding.action.back"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.back(window, cx))),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("onboarding-continue")
                    .primary()
                    .label(label)
                    .loading(self.busy)
                    .disabled(blocked || self.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.advance(window, cx))),
            )
            .into_any_element()
    }

    fn render_stage(&self, stage: Stage, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (title, description) = match stage {
            Stage::Language => (t("onboarding.stage.language.title"), t("onboarding.stage.language.description")),
            Stage::Terms => (t("onboarding.stage.terms.title"), t("onboarding.stage.terms.description")),
            Stage::Privacy => (t("onboarding.stage.privacy.title"), t("onboarding.stage.privacy.description")),
            Stage::Overview => (t("onboarding.stage.overview.title"), t("onboarding.stage.overview.description")),
            Stage::Preferences => {
                (t("onboarding.stage.preferences.title"), t("onboarding.stage.preferences.description"))
            }
            Stage::Https => (t("onboarding.stage.https.title"), t("onboarding.stage.https.description")),
            Stage::Proxies => (t("onboarding.stage.proxies.title"), t("onboarding.stage.proxies.description")),
            Stage::Workspace => (t("onboarding.stage.workspace.title"), t("onboarding.stage.workspace.description")),
        };
        let heading = v_flex()
            .gap_1()
            .child(div().text_xl().font_semibold().child(title))
            .child(div().text_color(theme.muted_foreground).child(description));
        let body = match stage {
            Stage::Language => self.render_language(cx),
            Stage::Terms => self.render_terms(cx),
            Stage::Privacy => self.render_privacy(cx),
            Stage::Overview => self.render_overview(cx),
            Stage::Preferences => self.render_preferences(cx),
            Stage::Https => self.render_https(cx),
            Stage::Proxies => self.render_proxies(cx),
            // The workspace view draws its own actions: its main one is what makes the workspace.
            Stage::Workspace => self.workspace.clone().into_any_element(),
        };
        v_flex()
            .w_full()
            .gap_6()
            .child(heading)
            .child(body)
            .children(self.error.clone().map(|error| div().text_sm().text_color(theme.danger).child(error)))
            .when(stage != Stage::Workspace, |this| this.child(self.render_actions(stage, cx)))
            .into_any_element()
    }

    fn render_waiting(text: &'static str, cx: &Context<Self>) -> AnyElement {
        h_flex()
            .gap_2()
            .child(Spinner::new().small())
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(text))
            .into_any_element()
    }
}

impl Render for Onboarding {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let stages = self.stages(cx);
        // A walk that was finished before is only here for a workspace: there are no steps to show.
        let finished_before = self.model.read(cx).setup().onboarded;
        let content = match self.stage {
            None => Self::render_waiting(t("onboarding.waiting.starting"), cx),
            Some(stage) => match stages.iter().position(|listed| *listed == stage) {
                // The first workspace exists and the engine is being told the walk is over.
                None => v_flex()
                    .gap_4()
                    .child(Self::render_waiting(t("onboarding.waiting.finishing"), cx))
                    .children(self.error.clone().map(|error| div().text_sm().text_color(theme.danger).child(error)))
                    .into_any_element(),
                Some(current) => v_flex()
                    .w_full()
                    .gap_8()
                    .when(!finished_before, |this| this.child(self.render_steps(&stages, current, cx)))
                    .child(self.render_stage(stage, cx))
                    .into_any_element(),
            },
        };
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(TitleBar::new().child("Bifurc"))
            .child(
                div().id("onboarding").flex_1().min_h_0().overflow_y_scroll().child(
                    v_flex()
                        .w_full()
                        .items_center()
                        .py_12()
                        .px_4()
                        .child(v_flex().w_full().max_w(rems(WIDTH)).child(content)),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(terms_accepted: bool, privacy_accepted: bool, onboarded: bool) -> SetupState {
        SetupState { known: true, terms_accepted, privacy_accepted, onboarded, ..SetupState::default() }
    }

    #[test]
    fn the_walk_starts_at_the_terms_then_the_policy_until_both_are_accepted() {
        assert_eq!(first_stage(&setup(false, false, false), true), Stage::Terms);
        assert_eq!(first_stage(&setup(false, true, true), true), Stage::Terms);
        assert_eq!(first_stage(&setup(true, false, false), true), Stage::Privacy);
        assert_eq!(first_stage(&setup(true, true, false), true), Stage::Overview);
        assert_eq!(first_stage(&setup(true, true, true), true), Stage::Workspace);
        assert_eq!(first_stage(&setup(false, false, false), false), Stage::Language);
        assert_eq!(Stage::Privacy.previous(), Some(Stage::Terms));
    }

    #[test]
    fn a_name_needs_the_port_unless_the_reverse_proxy_is_on_80() {
        assert_eq!(mapping_address(80), "http://api.localhost");
        assert_eq!(mapping_address(3000), "http://api.localhost:3000");
    }

    #[test]
    fn ports_are_numbers_and_the_two_proxies_differ() {
        assert_eq!(parse_port(" 8080 "), Some(8080));
        assert_eq!(parse_port("0"), None);
        assert_eq!(parse_port("70000"), None);
        assert_eq!(parse_port("http"), None);
        assert!(ports_problem(Some(80), Some(8080)).is_none());
        assert!(ports_problem(Some(8080), Some(8080)).is_some());
        assert!(ports_problem(None, Some(8080)).is_some());
    }

    #[test]
    fn a_proxy_that_could_not_start_is_named_with_its_port() {
        let running = json!({ "running": true, "port": 8080, "error": null });
        let refused = json!({ "running": false, "port": 80, "error": "Permission denied." });
        assert_eq!(start_problem(&json!({ "ok": true, "forward": running, "reverse": running })), None);
        assert_eq!(start_problem(&json!({ "ok": true, "forward": null, "reverse": null })), None);
        assert_eq!(
            start_problem(&json!({ "ok": true, "forward": running, "reverse": refused })).as_deref(),
            Some("The reverse proxy could not start on port 80: Permission denied.")
        );
    }
}
