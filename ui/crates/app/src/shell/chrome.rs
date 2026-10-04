//! The window's fixed furniture: the title bar, the bar above the screen, and the footer.
//!
//! - Title bar: the app, search (the finder), the command center and the environment selector.
//! - Top bar: the left sidebar's trigger, where you are, and the right sidebar's trigger.
//! - Footer: the workspace's git state with its publish action, the current screen's status, the proxy and
//!   the engine connection.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Side, Sizable as _, StyledExt as _, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    kbd::Kbd,
    menu::{DropdownMenu as _, PopupMenuItem},
    sidebar::SidebarToggleButton,
    status_bar::StatusBar,
    tag::Tag,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _, Window,
    div,
};

use super::{Screen, Shell};
use crate::{
    commands::{OpenCommandPalette, OpenFinder},
    model::{RequestKind, git::PublishAction},
    preferences::Preferences,
};

fn plural(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

impl Shell {
    pub(super) fn render_title_bar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let config = self.model.read(cx).config();
        let environment_name =
            config.active_environment().map(|e| e.name.clone()).unwrap_or_else(|| "No environment".to_owned());
        let environments: Vec<(Option<String>, String, bool)> =
            std::iter::once((None, "No environment".to_owned(), config.active_environment_id.is_none()))
                .chain(config.selectable_environments().map(|e| {
                    (Some(e.id.clone()), e.name.clone(), config.active_environment_id.as_deref() == Some(e.id.as_str()))
                }))
                .collect();
        let model = self.model.clone();
        let shell = cx.weak_entity();
        let shortcut = Kbd::binding_for_action(&OpenCommandPalette, None, window);
        let finder_shortcut = Kbd::binding_for_action(&OpenFinder, None, window);

        TitleBar::new().child(
            h_flex()
                .w_full()
                .gap_2()
                .child(div().flex_1().font_semibold().child("Bifurc"))
                // Search sits in the middle of the bar and finds things to open; the button beside it opens
                // the command center, which is only actions.
                .child(
                    h_flex()
                        .occlude()
                        .gap_1p5()
                        .child(
                            Button::new("open-search")
                                .outline()
                                .small()
                                .w_96()
                                .child(
                                    h_flex()
                                        .w_full()
                                        .gap_2()
                                        .text_color(theme.muted_foreground)
                                        .child(Icon::new(IconName::Search).small())
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_left()
                                                .child("Search requests, mocks and environments…"),
                                        )
                                        .children(finder_shortcut),
                                )
                                .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenFinder), cx)),
                        )
                        .child(
                            Button::new("open-commands")
                                .outline()
                                .small()
                                .icon(IconName::SquareTerminal)
                                .label("Commands")
                                .tooltip("Make something new, go to a screen or change the view")
                                .children(shortcut)
                                .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenCommandPalette), cx)),
                        ),
                )
                .child(
                    h_flex().flex_1().justify_end().child(
                        div().occlude().child(
                            Button::new("environment-menu")
                                .ghost()
                                .small()
                                .icon(IconName::Asterisk)
                                .label(environment_name)
                                .tooltip("Environment that variables resolve against")
                                .dropdown_menu(move |menu, _, _| {
                                    let menu = environments.iter().fold(menu, |menu, (id, name, active)| {
                                        let (model, id) = (model.clone(), id.clone());
                                        menu.item(PopupMenuItem::new(name.clone()).checked(*active).on_click(
                                            move |_, _, cx| {
                                                model.update(cx, |m, cx| m.set_active_environment(id.as_deref(), cx))
                                            },
                                        ))
                                    });
                                    let shell = shell.clone();
                                    menu.separator().item(PopupMenuItem::new("Manage environments").on_click(
                                        move |_, window, cx| {
                                            shell
                                                .update(cx, |shell, cx| {
                                                    shell.show_screen(Screen::Environments, window, cx)
                                                })
                                                .ok();
                                        },
                                    ))
                                }),
                        ),
                    ),
                ),
        )
    }

    /// The request's type, in the breadcrumb. A draft's type can still be chosen; a saved request's is fixed,
    /// because its type is which kind of entity it was stored as.
    fn render_request_kind(&self, kind: RequestKind, is_draft: bool, cx: &mut Context<Self>) -> AnyElement {
        if !is_draft {
            return Tag::secondary().small().child(kind.label()).into_any_element();
        }
        let shell = cx.weak_entity();
        Button::new("request-kind")
            .outline()
            .xsmall()
            .label(kind.label())
            .icon(IconName::ChevronDown)
            .tooltip("The type can be changed until the request is saved")
            .dropdown_menu(move |menu, _, _| {
                RequestKind::ALL.iter().fold(menu, |menu, option| {
                    let (shell, option) = (shell.clone(), *option);
                    menu.item(PopupMenuItem::new(option.label()).checked(option == kind).on_click(
                        move |_, window, cx| {
                            shell.update(cx, |shell, cx| shell.change_request_kind(option, window, cx)).ok();
                        },
                    ))
                })
            })
            .into_any_element()
    }

    /// Where the user is: the workspace, the screen, and what is open on it. The last part is the open
    /// entity's own name field, so the name is changed where it is read.
    fn render_breadcrumb(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let workspace = self
            .model
            .read(cx)
            .config()
            .active_workspace()
            .map(|w| w.name.clone())
            .unwrap_or_else(|| "Workspace".to_owned());
        let target = self.active_target(cx);
        let separator = || Icon::new(IconName::ChevronRight).text_color(theme.muted_foreground).size_3p5();
        let screen = div()
            .flex_none()
            .when(target.is_none(), |this| this.text_color(theme.foreground))
            .child(self.screen.label());

        let trail = h_flex()
            .w_full()
            .gap_1p5()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(div().flex_none().child(workspace))
            .child(separator())
            .child(screen);
        let Some(target) = target else { return trail.into_any_element() };
        let kind = target.kind.map(|kind| self.render_request_kind(kind, target.entity_id.is_none(), cx));
        // The folders the entity is filed under, so where it lives is read here and not only in the tree.
        let folders = target.folders.iter().flat_map(|name| {
            [separator().into_any_element(), div().flex_none().child(name.clone()).into_any_element()]
        });
        trail
            .children(folders)
            .child(separator())
            .children(kind)
            .child(match target.name {
                Some(name) => div()
                    .flex_1()
                    .min_w_0()
                    .text_color(theme.foreground)
                    .child(Input::new(&name).small().appearance(false))
                    .into_any_element(),
                None => div().text_color(theme.foreground).child(target.title).into_any_element(),
            })
            .into_any_element()
    }

    pub(super) fn render_top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let collapsed = Preferences::global(cx).sidebar_collapsed;
        let breadcrumb = self.render_breadcrumb(cx);

        h_flex()
            .flex_none()
            .h_10()
            .px_2()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.background)
            // Each trigger is the same kit button with the same inner id, so each needs its own scope.
            .child(
                div().id("left-sidebar-trigger").child(
                    SidebarToggleButton::new()
                        .collapsed(collapsed)
                        .accessibility_label(if collapsed { "Expand the sidebar" } else { "Collapse the sidebar" })
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                ),
            )
            .child(div().flex_1().min_w_0().overflow_hidden().child(breadcrumb))
            .child(
                div().id("right-sidebar-trigger").child(
                    SidebarToggleButton::new()
                        .side(Side::Right)
                        .collapsed(!self.right_open)
                        .accessibility_label(if self.right_open { "Hide the inspector" } else { "Show the inspector" })
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_right_sidebar(window, cx))),
                ),
            )
    }

    /// One line about the current screen, for the footer.
    fn screen_status(&self, cx: &Context<Self>) -> String {
        let config = self.model.read(cx).config();
        match self.screen {
            Screen::Requests => plural(config.requests.iter().filter(|r| !r.deleted).count(), "request"),
            Screen::Mocks => plural(config.mocks.iter().filter(|m| !m.deleted).count(), "mock"),
            Screen::Mappings => plural(config.mappings.iter().filter(|m| !m.deleted).count(), "mapping"),
            Screen::ProxyRules => plural(config.proxy_rule_count, "rule"),
            Screen::Environments => plural(config.selectable_environments().count(), "environment"),
            Screen::Vaults => plural(self.model.read(cx).vaults().len(), "vault"),
            Screen::Services => self.services.read(cx).status_line(),
            Screen::HealthBar => self.healthbar.read(cx).status_line(),
            Screen::Capture => self.capture.read(cx).status_line(cx),
            Screen::Settings | Screen::WorkspaceConfig | Screen::AuditLog => String::new(),
        }
    }

    fn render_git_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let model = self.model.read(cx);
        // Without git nothing is tracked, so there is nothing to publish and no state to report.
        if model.setup().known && !model.setup().git_installed {
            return h_flex()
                .gap_2()
                .text_color(theme.muted_foreground)
                .child(Icon::new(IconName::TriangleAlert).small())
                .child("Git is not installed. Changes are saved, not tracked.")
                .into_any_element();
        }
        let git = model.git();
        let rollup = git.rollup(self.screen.git_prefixes());
        let publishing = model.is_publishing();
        let has_remote = git.has_remote;
        let place = match (&git.branch, has_remote) {
            (Some(branch), _) => branch.clone(),
            (None, true) => "remote".to_owned(),
            (None, false) => "Local".to_owned(),
        };
        let scope = if self.screen.git_prefixes().is_empty() { "in this workspace" } else { "on this screen" };
        let state = match (rollup.changed.len(), rollup.action) {
            (0, Some(_)) => "Commits waiting to be pushed".to_owned(),
            (0, None) => "Up to date".to_owned(),
            (count, _) => format!("{} {scope}", plural(count, "unpublished change")),
        };
        let action_label = match rollup.action {
            Some(PublishAction::Publish) => "Publish",
            Some(PublishAction::Sync) | None => "Sync",
        };
        let tooltip = if has_remote { "Commit these changes and push them" } else { "Commit these changes" };
        let paths = rollup.changed.clone();
        let app_model = self.model.clone();

        h_flex()
            .gap_2()
            .child(Icon::new(IconName::Github).small().text_color(theme.muted_foreground))
            .child(div().child(place))
            .child(div().text_color(theme.muted_foreground).child(state))
            .when(rollup.action.is_some(), |this| {
                this.child(
                    Button::new("publish")
                        .outline()
                        .xsmall()
                        .label(action_label)
                        .loading(publishing)
                        .disabled(publishing)
                        .tooltip(tooltip)
                        .on_click(move |_, _, cx| {
                            let paths = paths.clone();
                            app_model.update(cx, |model, cx| model.publish(paths, cx));
                        }),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let model = self.model.read(cx);
        let proxy = model.proxy().clone();
        let lost = model.is_connection_lost();
        let engine_version = model.engine().hello.engine_version.clone();
        let status = self.screen_status(cx);

        let proxy_label = match (&proxy.error, proxy.running) {
            (Some(_), _) => format!("Forward proxy :{} unavailable", proxy.port),
            (None, true) => format!("Forward proxy :{} running", proxy.port),
            (None, false) => format!("Forward proxy :{} stopped", proxy.port),
        };
        let proxy_tooltip: SharedString = match (&proxy.error, proxy.running) {
            (Some(error), _) => format!("{error}. Select to try starting it again.").into(),
            (None, true) => "Stop the forward proxy".into(),
            (None, false) => "Start the forward proxy".into(),
        };
        let app_model = self.model.clone();

        StatusBar::new().left(self.render_git_status(cx)).right(
            h_flex()
                .gap_3()
                .when(!status.is_empty(), |this| this.child(div().text_color(theme.muted_foreground).child(status)))
                .child(
                    Button::new("proxy-toggle")
                        .ghost()
                        .xsmall()
                        .icon(if proxy.error.is_some() {
                            IconName::TriangleAlert
                        } else if proxy.running {
                            IconName::CircleCheck
                        } else {
                            IconName::Pause
                        })
                        .label(proxy_label)
                        .loading(proxy.busy)
                        .tooltip(proxy_tooltip)
                        .on_click(move |_, _, cx| app_model.update(cx, |model, cx| model.toggle_proxy(cx))),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .text_color(if lost { theme.danger } else { theme.muted_foreground })
                        .child(Icon::new(if lost { IconName::CircleX } else { IconName::HardDrive }).small())
                        .child(if lost {
                            "Engine disconnected".to_owned()
                        } else {
                            format!("Engine {engine_version}")
                        }),
                ),
        )
    }
}
