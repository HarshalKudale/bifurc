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
use serde_json::json;

use crate::{
    app_model::{AppModel, Load, ProxyStatus, SetupState},
    components::certificate::Certificate,
    engine::{call, call_in, failure},
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
    Terms,
    Privacy,
    Overview,
    Preferences,
    Https,
    Proxies,
    Workspace,
}

impl Stage {
    const ALL: [Stage; 7] = [
        Stage::Terms,
        Stage::Privacy,
        Stage::Overview,
        Stage::Preferences,
        Stage::Https,
        Stage::Proxies,
        Stage::Workspace,
    ];

    fn label(self) -> &'static str {
        match self {
            Stage::Terms => "Terms",
            Stage::Privacy => "Privacy",
            Stage::Overview => "Overview",
            Stage::Preferences => "Preferences",
            Stage::Https => "HTTPS",
            Stage::Proxies => "Proxies",
            Stage::Workspace => "Workspace",
        }
    }

    fn previous(self) -> Option<Stage> {
        let index = Self::ALL.iter().position(|stage| *stage == self)?;
        index.checked_sub(1).map(|index| Self::ALL[index])
    }
}

/// Where the walk starts. One that was left half way picks up after the terms and the policy, neither of
/// which is asked twice; one that was finished is only here because the last workspace is gone.
fn first_stage(setup: &SetupState) -> Stage {
    match (setup.terms_accepted, setup.privacy_accepted, setup.onboarded) {
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

/// What is wrong with the two ports as typed, if something is.
fn ports_problem(reverse: Option<u16>, forward: Option<u16>) -> Option<&'static str> {
    match (reverse, forward) {
        (None, _) | (_, None) => Some("A port is a number from 1 to 65535."),
        (Some(reverse), Some(forward)) if reverse == forward => Some("The two proxies need different ports."),
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
        if self.stage.is_none() && setup.known {
            self.stage = Some(first_stage(&setup));
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
            Some(Stage::Terms) => self.accept_terms(window, cx),
            Some(Stage::Privacy) => self.accept_privacy(window, cx),
            Some(Stage::Overview) => self.go_to(Stage::Preferences, window, cx),
            Some(Stage::Preferences) => self.go_to(Stage::Https, window, cx),
            Some(Stage::Https) => self.go_to(Stage::Proxies, window, cx),
            Some(Stage::Proxies) => {
                if !self.apply_ports(cx) {
                    return;
                }
                if self.has_workspace(cx) { self.complete(cx) } else { self.go_to(Stage::Workspace, window, cx) }
            }
            Some(Stage::Workspace) | None => {}
        }
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
                    this.error = Some(format!("Couldn't record that you accepted. {message}"));
                    cx.notify();
                }
                None => {
                    // What comes next is where the walk would start with the terms accepted.
                    let mut setup = this.model.read(cx).setup().clone();
                    setup.terms_accepted = true;
                    this.model.update(cx, |model, cx| model.refresh_setup(cx));
                    this.go_to(first_stage(&setup), window, cx);
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
                    this.error = Some(format!("Couldn't record that you accepted. {message}"));
                    cx.notify();
                }
                None => {
                    let mut setup = this.model.read(cx).setup().clone();
                    setup.privacy_accepted = true;
                    this.model.update(cx, |model, cx| model.refresh_setup(cx));
                    this.go_to(first_stage(&setup), window, cx);
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
                this.error = Some(format!("Couldn't finish the setup. {message}"));
            }
            this.model.update(cx, |model, cx| model.refresh_setup(cx));
            cx.notify();
        });
    }

    fn typed_ports(&self, cx: &Context<Self>) -> (Option<u16>, Option<u16>) {
        (parse_port(&self.reverse_port.read(cx).value()), parse_port(&self.forward_port.read(cx).value()))
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
            .child(
                "The terms of service are the agreement between you and Bifurc: what you may use the app for, \
                 what it comes with and without, and who is responsible for what. They also say that anonymous \
                 usage analytics are on by default and can be turned off in Settings, under About. Read them \
                 before you go on.",
            )
            .child(
                h_flex()
                    .gap_1()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("The full terms:")
                    .child(Link::new("terms-of-service").href(url).child(shown)),
            )
            .child(
                Checkbox::new("accept-terms")
                    .label("I have read and accept the terms of service")
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
            .child(
                "Your workspaces, requests and captured traffic stay on this computer. Bifurc sends anonymous \
                 usage analytics: what kind of thing you do, from which interface and on which platform. Never \
                 a name, a URL, a path or anything you typed. They are on by default once you accept, and you \
                 can turn them off at any time in Settings, under About; that Bifurc was installed on this \
                 device is still reported, once.",
            )
            .child(
                h_flex()
                    .gap_1()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("The full policy:")
                    .child(Link::new("privacy-policy").href(url).child(shown)),
            )
            .child(
                Checkbox::new("accept-privacy")
                    .label("I have read and accept the privacy policy")
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
                "Requests",
                "Send REST, GraphQL, gRPC, SOAP and WebSocket requests, one at a time or a folder in a run.",
            ),
            (
                IconName::GalleryVerticalEnd,
                "Mocks",
                "Answer a call yourself, for an endpoint that is missing or that you want to behave differently.",
            ),
            (
                IconName::Replace,
                "Mappings",
                "Give local services names: api.localhost reaches the service on port 3000.",
            ),
            (
                IconName::Network,
                "Proxy rules and capture",
                "Send a browser or a tool through the forward proxy to redirect, mock and record its traffic.",
            ),
            (
                IconName::Asterisk,
                "Environments and vaults",
                "Keep variables for each environment, and secrets sealed behind a passphrase.",
            ),
            (
                IconName::Folder,
                "Workspaces",
                "Everything you save is a file in a workspace with its history. Syncing it with GitHub is optional.",
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

    fn render_preferences(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = theme::current(cx);
        let themes = theme::available(cx);
        let chosen = current.clone();
        let theme_menu = Button::new("onboarding-theme").outline().label(current).dropdown_caret(true).dropdown_menu(
            move |menu, window, _| {
                let menu = menu.scrollable(true).max_h(rems(20.).to_pixels(window.rem_size()));
                themes.iter().fold(menu, |menu, (name, is_dark)| {
                    // Most themes say which they are in their name; the rest get it appended.
                    let mode = if *is_dark { "Dark" } else { "Light" };
                    let label: SharedString =
                        if name.contains(mode) { name.clone() } else { format!("{name} ({mode})").into() };
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
            .child(Self::row("Color theme", "Each theme is light or dark.", theme_menu, cx))
            .child(Self::row("Interface size", "Scales text, spacing and controls together.", size_menu, cx))
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
            (None, true) => "A certificate is in place.".into(),
            (_, false) => "No certificate yet.".into(),
        };
        let certificate_actions = h_flex()
            .gap_2()
            .child(
                Button::new("onboarding-generate")
                    .outline()
                    .label(if certificate.generated { "Regenerate" } else { "Generate" })
                    .loading(certificate.busy)
                    .disabled(working)
                    .on_click(move |_, _, cx| generate.update(cx, |certificate, cx| certificate.generate(cx))),
            )
            .child(
                Button::new("onboarding-import")
                    .outline()
                    .label("Import…")
                    .loading(certificate.importing)
                    .disabled(working)
                    .tooltip("Use your own certificate and private key instead")
                    .on_click(move |_, _, cx| import.update(cx, |certificate, cx| certificate.import(cx))),
            );
        let install_action = Button::new("onboarding-install")
            .outline()
            .label("Install")
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
            .child(Self::row("Root certificate", state, certificate_actions, cx))
            .child(
                v_flex()
                    .gap_2()
                    .child(Self::row(
                        "Trust on this computer",
                        "Adds the certificate to your trusted roots, so intercepted sites open without warnings. \
                         Firefox keeps its own store and needs the certificate imported there.",
                        install_action,
                        cx,
                    ))
                    .children(note),
            )
            .child(Self::row(
                "Intercept HTTPS",
                "Decrypt HTTPS traffic through the forward proxy so rules, mocks and capture apply to it.",
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
            (None, true) => Tag::success().child(format!("Running on port {}", listener.port)),
            (None, false) => Tag::secondary().child("Stopped"),
        };
        h_flex().gap_2().child(state).child(
            Button::new(id)
                .outline()
                .small()
                .label(if listener.running { "Stop" } else { "Start" })
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
        let muted = |text: String| div().text_sm().text_color(theme.muted_foreground).child(text);

        let reverse_group = Self::group(cx)
            .child(Self::row(
                "Reverse proxy",
                "Serves your .localhost names and sends each one on to its mapping.",
                Self::listener_controls(
                    "onboarding-reverse-toggle",
                    &reverse_status,
                    self.model.clone(),
                    AppModel::toggle_reverse_proxy,
                ),
                cx,
            ))
            .child(Self::row(
                "Port",
                format!(
                    "Keep {HTTP_PORT} for names that work without a port, like {}. On any other port, every \
                     address needs that port, like {}.",
                    mapping_address(HTTP_PORT),
                    mapping_address(3000),
                ),
                port_field(&self.reverse_port),
                cx,
            ))
            .children(reverse.filter(|port| *port != HTTP_PORT).map(|port| {
                Alert::warning(
                    "reverse-port-note",
                    format!(
                        "Add :{port} to every .localhost address you open or call: {} instead of {}.",
                        mapping_address(port),
                        mapping_address(HTTP_PORT),
                    ),
                )
                .title(format!("Names need the port while the reverse proxy is on {port}"))
            }));

        let forward_group = Self::group(cx)
            .child(Self::row(
                "Forward proxy",
                "Point a browser or a tool at it to apply proxy rules, mocks and capture to its traffic.",
                Self::listener_controls(
                    "onboarding-forward-toggle",
                    &forward_status,
                    self.model.clone(),
                    AppModel::toggle_proxy,
                ),
                cx,
            ))
            .child(Self::row(
                "Port",
                match forward {
                    Some(port) => format!("The proxy address to give a browser or a tool is http://localhost:{port}."),
                    None => "The proxy address to give a browser or a tool uses this port.".to_owned(),
                },
                port_field(&self.forward_port),
                cx,
            ));

        v_flex()
            .gap_4()
            .child(reverse_group)
            .child(forward_group)
            .children(problem.map(|problem| div().text_sm().text_color(theme.danger).child(problem)))
            .child(muted("A new port takes effect at once while its proxy is running.".to_owned()))
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
        let label = match stage {
            Stage::Terms | Stage::Privacy => "Accept and continue",
            Stage::Proxies if self.has_workspace(cx) => "Finish",
            _ => "Continue",
        };
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
                        .label("Back")
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
            Stage::Terms => ("Terms of service", "Accept the terms to use Bifurc."),
            Stage::Privacy => ("Privacy policy", "Accept the policy to use Bifurc."),
            Stage::Overview => ("What Bifurc does", "One place for the calls your work makes and receives."),
            Stage::Preferences => ("Preferences", "How the app looks. Change it any time in Settings."),
            Stage::Https => (
                "HTTPS",
                "Optional. To work on HTTPS traffic, the forward proxy signs the sites it intercepts with a root \
                 certificate made on this computer. You can set this up later in Settings.",
            ),
            Stage::Proxies => ("Proxy ports", "The ports Bifurc's two proxies listen on."),
            Stage::Workspace => {
                ("Set up your first workspace", "A workspace holds your requests, mocks, mappings and environments.")
            }
        };
        let heading = v_flex()
            .gap_1()
            .child(div().text_xl().font_semibold().child(title))
            .child(div().text_color(theme.muted_foreground).child(description));
        let body = match stage {
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
            None => Self::render_waiting("Starting", cx),
            Some(stage) => match stages.iter().position(|listed| *listed == stage) {
                // The first workspace exists and the engine is being told the walk is over.
                None => v_flex()
                    .gap_4()
                    .child(Self::render_waiting("Finishing the setup", cx))
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
        assert_eq!(first_stage(&setup(false, false, false)), Stage::Terms);
        assert_eq!(first_stage(&setup(false, true, true)), Stage::Terms);
        assert_eq!(first_stage(&setup(true, false, false)), Stage::Privacy);
        assert_eq!(first_stage(&setup(true, true, false)), Stage::Overview);
        assert_eq!(first_stage(&setup(true, true, true)), Stage::Workspace);
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
}
