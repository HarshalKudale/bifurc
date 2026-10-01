//! The window's fixed furniture: title bar, module rail and status bar.

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, StyledExt as _, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    sidebar::{Sidebar, SidebarMenu, SidebarMenuItem},
    status_bar::StatusBar,
    tag::Tag,
};
use gpui_kit::{Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, Window, div};

use super::{Module, Shell};
use crate::commands::OpenCommandPalette;

impl Shell {
    pub(super) fn render_title_bar(&self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let config = model.config();
        let workspace_name =
            config.active_workspace().map(|w| w.name.clone()).unwrap_or_else(|| "Workspace".to_owned());
        let environment_name =
            config.active_environment().map(|e| e.name.clone()).unwrap_or_else(|| "No environment".to_owned());

        let workspaces: Vec<(String, String, bool)> = config
            .workspaces
            .iter()
            .map(|w| (w.id.clone(), w.name.clone(), w.id == config.active_workspace_id))
            .collect();
        let environments: Vec<(Option<String>, String, bool)> =
            std::iter::once((None, "No environment".to_owned(), config.active_environment_id.is_none()))
                .chain(config.selectable_environments().map(|e| {
                    (Some(e.id.clone()), e.name.clone(), config.active_environment_id.as_deref() == Some(e.id.as_str()))
                }))
                .collect();

        let workspace_model = self.model.clone();
        let environment_model = self.model.clone();

        TitleBar::new().child(
            h_flex()
                .w_full()
                .gap_2()
                .child(div().font_semibold().child("Bifurc"))
                .child(
                    div().occlude().child(
                        Button::new("workspace-menu")
                            .ghost()
                            .small()
                            .label(workspace_name)
                            .icon(IconName::ChevronDown)
                            .tooltip("Switch workspace")
                            .dropdown_menu(move |menu, _, _| {
                                workspaces.iter().fold(menu, |menu, (id, name, active)| {
                                    let model = workspace_model.clone();
                                    let id = id.clone();
                                    menu.item(PopupMenuItem::new(name.clone()).checked(*active).on_click(
                                        move |_, _, cx| model.update(cx, |m, cx| m.set_active_workspace(&id, cx)),
                                    ))
                                })
                            }),
                    ),
                )
                .child(
                    div().occlude().child(
                        Button::new("environment-menu")
                            .ghost()
                            .small()
                            .label(environment_name)
                            .icon(IconName::Globe)
                            .tooltip("Switch environment")
                            .dropdown_menu(move |menu, _, _| {
                                environments.iter().fold(menu, |menu, (id, name, active)| {
                                    let model = environment_model.clone();
                                    let id = id.clone();
                                    menu.item(PopupMenuItem::new(name.clone()).checked(*active).on_click(
                                        move |_, _, cx| {
                                            model.update(cx, |m, cx| m.set_active_environment(id.as_deref(), cx))
                                        },
                                    ))
                                })
                            }),
                    ),
                )
                .child(div().flex_1())
                .child(
                    div().occlude().child(
                        Button::new("open-palette")
                            .ghost()
                            .small()
                            .icon(IconName::Search)
                            .label("Commands…")
                            .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenCommandPalette), cx)),
                    ),
                ),
        )
    }

    pub(super) fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let items = Module::ALL.map(|module| {
            SidebarMenuItem::new(module.label())
                .icon(module.icon())
                .active(self.module == module)
                .on_click(cx.listener(move |this, _, window, cx| this.show_module(module, window, cx)))
        });
        Sidebar::<SidebarMenu>::new("module-rail")
            .collapsed(true)
            .collapsible(false)
            .child(SidebarMenu::new().children(items))
    }

    pub(super) fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let model = self.model.read(cx);
        let engine = model.engine();
        let config = model.config();

        let connection = if model.is_connection_lost() {
            Tag::danger().child("Engine disconnected")
        } else {
            Tag::success().child("Engine connected")
        };
        let origin = if engine.is_owned() { "started by this app" } else { "shared with the CLI" };
        let ports = match (config.proxy_port, config.forward_proxy_port) {
            (0, 0) => String::new(),
            (edge, forward) => format!("Edge port {edge} · Forward proxy {forward}"),
        };

        StatusBar::new()
            .left(
                h_flex().gap_2().child(connection).child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child(format!("Engine {} · pid {} · {origin}", engine.hello.engine_version, engine.info.pid)),
                ),
            )
            .right(div().text_color(theme.muted_foreground).child(ports))
    }
}
