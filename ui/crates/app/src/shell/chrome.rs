//! The window's fixed furniture: the title bar, the bar above the screen, and the footer.
//!
//! - Title bar: the app, search (the command palette) and the environment selector.
//! - Top bar: the left sidebar's trigger, where you are, and the right sidebar's trigger.
//! - Footer: the workspace's git state with its publish action, the current screen's status, the proxy and
//!   the engine connection.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Side, Sizable as _, StyledExt as _, TitleBar,
    breadcrumb::{Breadcrumb, BreadcrumbItem},
    button::{Button, ButtonVariants as _},
    h_flex,
    kbd::Kbd,
    menu::{DropdownMenu as _, PopupMenuItem},
    sidebar::SidebarToggleButton,
    status_bar::StatusBar,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div,
};

use super::{Screen, Shell};
use crate::{commands::OpenCommandPalette, model::git::PublishAction, preferences::Preferences};

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

        TitleBar::new().child(
            h_flex()
                .w_full()
                .gap_2()
                .child(div().flex_1().font_semibold().child("Bifurc"))
                // Search sits in the middle of the bar and is the palette's visible entry point.
                .child(
                    div().occlude().child(
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
                                    .child(div().flex_1().text_left().child("Search requests and commands…"))
                                    .children(shortcut),
                            )
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

    /// What the breadcrumb ends in: the thing open on the current screen, if the screen has one.
    fn active_item_title(&self, cx: &Context<Self>) -> Option<SharedString> {
        self.active_target(cx).map(|target| target.title)
    }

    pub(super) fn render_top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let collapsed = Preferences::global(cx).sidebar_collapsed;
        let workspace = self
            .model
            .read(cx)
            .config()
            .active_workspace()
            .map(|w| w.name.clone())
            .unwrap_or_else(|| "Workspace".to_owned());
        let breadcrumb = Breadcrumb::new()
            .child(BreadcrumbItem::new(workspace))
            .child(BreadcrumbItem::new(self.screen.label()))
            .when_some(self.active_item_title(cx), |this, title| this.child(BreadcrumbItem::new(title)));

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
            Screen::Requests => plural(config.requests.len(), "request"),
            Screen::Mocks => plural(config.mocks.len(), "mock"),
            Screen::Mappings => plural(config.mappings.len(), "mapping"),
            Screen::ProxyRules => plural(config.proxy_rule_count, "rule"),
            Screen::Environments => plural(config.selectable_environments().count(), "environment"),
            Screen::Services | Screen::HealthBar | Screen::Capture => self.screen_note.clone().unwrap_or_default(),
            Screen::Settings => String::new(),
        }
    }

    fn render_git_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let model = self.model.read(cx);
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
    }

    pub(super) fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let model = self.model.read(cx);
        let proxy = model.proxy().clone();
        let lost = model.is_connection_lost();
        let engine_version = model.engine().hello.engine_version.clone();
        let status = self.screen_status(cx);

        let proxy_label = match (&proxy.error, proxy.running) {
            (Some(_), _) => format!("Proxy :{} unavailable", proxy.port),
            (None, true) => format!("Proxy :{} running", proxy.port),
            (None, false) => format!("Proxy :{} stopped", proxy.port),
        };
        let proxy_tooltip: SharedString = match (&proxy.error, proxy.running) {
            (Some(error), _) => format!("{error}. Select to try starting it again.").into(),
            (None, true) => "Stop the proxy".into(),
            (None, false) => "Start the proxy".into(),
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
