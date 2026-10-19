//! Where the user can be: the screens, and the left sidebar that moves between them.
//!
//! The sidebar is the only thing that changes which screen is shown. It carries the workspace switcher at
//! its top, the screens in groups, and Settings pinned at its foot; collapsed, it keeps the icons.

use gpui_kit::component::{
    ActiveTheme as _, Collapsible as _, Icon, Sizable as _, StyledExt as _, h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    sidebar::{
        Sidebar, SidebarCollapsible, SidebarGroup, SidebarHeader, SidebarItem as _, SidebarMenu, SidebarMenuItem,
    },
    v_flex,
};
// The full icon catalog: the component crate's own enum has no mail icon.
use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{Context, IntoElement, ParentElement as _, Styled as _, Window, div};

use super::Shell;
use crate::{i18n::t, model::ConfigSnapshot, preferences::Preferences};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    Services,
    HealthBar,
    Mappings,
    ProxyRules,
    Capture,
    Mail,
    Requests,
    Mocks,
    Flows,
    Webhooks,
    Environments,
    Vaults,
    WorkspaceConfig,
    AuditLog,
    Settings,
}

impl Screen {
    pub fn label(self) -> &'static str {
        t(match self {
            Screen::Services => "shell.screen.services",
            Screen::HealthBar => "shell.screen.health_bar",
            Screen::Mappings => "shell.screen.mappings",
            Screen::ProxyRules => "shell.screen.proxy_rules",
            Screen::Capture => "shell.screen.capture",
            Screen::Mail => "shell.screen.mail",
            Screen::Requests => "shell.screen.requests",
            Screen::Mocks => "shell.screen.mocks",
            Screen::Flows => "shell.screen.flows",
            Screen::Webhooks => "shell.screen.webhooks",
            Screen::Environments => "shell.screen.environments",
            Screen::Vaults => "shell.screen.vaults",
            Screen::WorkspaceConfig => "shell.screen.config",
            Screen::AuditLog => "shell.screen.audit_log",
            Screen::Settings => "shell.screen.settings",
        })
    }

    pub fn icon(self) -> IconName {
        match self {
            Screen::Services => IconName::Cpu,
            Screen::HealthBar => IconName::Heart,
            Screen::Mappings => IconName::Replace,
            Screen::ProxyRules => IconName::Network,
            Screen::Capture => IconName::Inbox,
            Screen::Mail => IconName::Mail,
            Screen::Requests => IconName::Globe,
            Screen::Mocks => IconName::GalleryVerticalEnd,
            Screen::Flows => IconName::LayoutDashboard,
            Screen::Webhooks => IconName::Webhook,
            Screen::Environments => IconName::Asterisk,
            Screen::Vaults => IconName::EyeOff,
            Screen::WorkspaceConfig => IconName::Settings2,
            Screen::AuditLog => IconName::FileText,
            Screen::Settings => IconName::Settings,
        }
    }

    /// The workspace directories this screen's entities are stored in, for the footer's publish roll-up.
    /// A screen that is not about saved entities covers the whole workspace.
    pub fn git_prefixes(self) -> &'static [&'static str] {
        match self {
            Screen::Requests => &[
                "requests/",
                "graphqlRequests/",
                "soapRequests/",
                "grpcRequests/",
                "sseRequests/",
                "socketioRequests/",
                "mcpRequests/",
                "sockets/",
            ],
            Screen::Mocks => &["mocks/"],
            Screen::Flows => &["flows/"],
            Screen::Mappings => &["mappings/"],
            Screen::ProxyRules => &["rules/"],
            Screen::Environments => &["environments/"],
            Screen::Vaults => &["vaults/"],
            Screen::HealthBar => &["healthbar/"],
            Screen::Services
            | Screen::Capture
            | Screen::Mail
            | Screen::Webhooks
            | Screen::Settings
            | Screen::WorkspaceConfig
            | Screen::AuditLog => &[],
        }
    }

    /// How many of this screen's things the workspace holds, for the badge beside its name.
    fn count(self, config: &ConfigSnapshot) -> Option<usize> {
        let count = match self {
            Screen::Mappings => config.mappings.iter().filter(|m| !m.deleted).count(),
            Screen::ProxyRules => config.proxy_rule_count,
            Screen::Requests => config.requests.iter().filter(|r| !r.deleted).count(),
            Screen::Mocks => config.mocks.iter().filter(|m| !m.deleted).count(),
            Screen::Flows => config.flows.iter().filter(|f| !f.deleted).count(),
            Screen::Environments => config.selectable_environments().count(),
            Screen::Services
            | Screen::HealthBar
            | Screen::Vaults
            | Screen::Capture
            | Screen::Mail
            | Screen::Webhooks
            | Screen::Settings
            | Screen::WorkspaceConfig
            | Screen::AuditLog => 0,
        };
        (count > 0).then_some(count)
    }
}

/// The sidebar's groups, in order. Settings is not here: it is pinned to the foot.
const GROUPS: [(&str, &[Screen]); 4] = [
    ("shell.group.applications", &[Screen::Services, Screen::HealthBar]),
    ("shell.group.routing", &[Screen::Mappings, Screen::ProxyRules, Screen::Capture, Screen::Mail]),
    ("shell.group.api", &[Screen::Requests, Screen::Mocks, Screen::Flows, Screen::Webhooks]),
    ("shell.group.workspace", &[Screen::Environments, Screen::Vaults, Screen::WorkspaceConfig, Screen::AuditLog]),
];

impl Shell {
    fn nav_item(&self, screen: Screen, config: &ConfigSnapshot, cx: &mut Context<Self>) -> SidebarMenuItem {
        let item = SidebarMenuItem::new(screen.label())
            .icon(screen.icon())
            .active(self.screen == screen)
            .on_click(cx.listener(move |this, _, window, cx| this.show_screen(screen, window, cx)));
        let count = match screen {
            Screen::Vaults => Some(self.model.read(cx).vaults().len()).filter(|n| *n > 0),
            _ => screen.count(config),
        };
        match count {
            Some(count) => item
                .suffix(move |_, cx| div().text_xs().text_color(cx.theme().muted_foreground).child(count.to_string())),
            None => item,
        }
    }

    fn render_workspace_switcher(&self, collapsed: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let config = self.model.read(cx).config();
        let name = config
            .active_workspace()
            .map(|w| w.name.clone())
            .unwrap_or_else(|| t("shell.workspace.default_name").to_owned());
        let active_id = config.active_workspace_id.clone();
        let workspaces: Vec<(String, String)> =
            config.workspaces.iter().map(|w| (w.id.clone(), w.name.clone())).collect();
        let can_delete = workspaces.len() > 1;
        let model = self.model.clone();
        let shell = cx.weak_entity();

        let switcher = SidebarHeader::new()
            .when(collapsed, |this| this.p_0().justify_center())
            .child(
                h_flex()
                    .gap_2()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size_8()
                            .rounded(theme.radius)
                            .bg(theme.sidebar_primary)
                            .text_color(theme.sidebar_primary_foreground)
                            .child(Icon::new(IconName::Building2).small()),
                    )
                    .when(!collapsed, |this| {
                        this.child(
                            v_flex()
                                .min_w_0()
                                .child(div().text_sm().font_medium().overflow_hidden().text_ellipsis().child(name))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(t("shell.workspace.caption")),
                                ),
                        )
                    }),
            )
            .when(!collapsed, |this| this.child(Icon::new(IconName::ChevronsUpDown).small()))
            .dropdown_menu(move |menu, _, _| {
                let menu = workspaces.iter().fold(menu, |menu, (id, name)| {
                    let (model, id) = (model.clone(), id.clone());
                    menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(id == active_id)
                            .on_click(move |_, _, cx| model.update(cx, |m, cx| m.set_active_workspace(&id, cx))),
                    )
                });
                let on = |shell: &gpui_kit::WeakEntity<Shell>, f: fn(&mut Shell, &mut Window, &mut Context<Shell>)| {
                    let shell = shell.clone();
                    move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut gpui_kit::App| {
                        shell.update(cx, |shell, cx| f(shell, window, cx)).ok();
                    }
                };
                menu.separator()
                    .item(
                        PopupMenuItem::new(t("shell.workspace.add")).on_click(on(&shell, Shell::prompt_new_workspace)),
                    )
                    .item(
                        PopupMenuItem::new(t("shell.workspace.rename"))
                            .on_click(on(&shell, Shell::prompt_rename_workspace)),
                    )
                    .item(
                        PopupMenuItem::new(t("shell.workspace.delete"))
                            .disabled(!can_delete)
                            .on_click(on(&shell, Shell::confirm_delete_workspace)),
                    )
            });
        // The menu wraps its trigger in a box as wide as the trigger's content, and the sidebar lays its
        // header out in a row. This column is what gives the switcher the sidebar's full width, like the
        // items under it.
        v_flex().flex_1().min_w_0().child(switcher)
    }

    pub(super) fn render_sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let collapsed = Preferences::global(cx).sidebar_collapsed;
        let config = self.model.read(cx).config().clone();
        let groups: Vec<SidebarGroup<SidebarMenu>> = GROUPS
            .iter()
            .map(|(label, screens)| {
                SidebarGroup::new(t(label)).child(
                    SidebarMenu::new().children(screens.iter().map(|screen| self.nav_item(*screen, &config, cx))),
                )
            })
            .collect();
        // Settings stays at the foot whatever the list above it does, so it is always in the same place.
        let settings = SidebarMenu::new()
            .child(self.nav_item(Screen::Settings, &config, cx))
            .collapsed(collapsed)
            .render("nav-settings", window, cx)
            .into_any_element();

        Sidebar::<SidebarGroup<SidebarMenu>>::new("nav")
            .collapsible(SidebarCollapsible::Icon)
            .collapsed(collapsed)
            .header(self.render_workspace_switcher(collapsed, cx))
            .children(groups)
            .footer(div().w_full().child(settings))
    }
}
