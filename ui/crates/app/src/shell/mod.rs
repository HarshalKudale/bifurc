//! The window: title bar, module rail, dock area and status bar, and the commands that act on them.
//!
//! The shell owns layout and open tabs and nothing about the workspace itself. It composes the dockable
//! panels (explorer, request tabs, response, details, activity), keeps the response and details panels
//! pointed at the active tab, and turns what the user does in them into engine commands. The workspace
//! comes from [`AppModel`]; an open request is a [`Session`].

mod chrome;
mod dialogs;
mod module;
mod palette;
mod tabs;

use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _,
    command::CommandState,
    dock::{DockArea, DockLayout, DockPlacement, DockSkin, panel_handle},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Window, div, px,
};

pub use self::module::Module;
use self::tabs::OpenTab;
use crate::{
    app_model::{AppModel, ModelEvent},
    commands::ShellHandle,
    panels::{
        activity::ActivityPanel,
        details::DetailsPanel,
        explorer::{ExplorerEvent, ExplorerPanel},
        response::ResponsePanel,
        welcome::WelcomePanel,
    },
    session::Session,
};

pub struct Shell {
    pub(crate) model: Entity<AppModel>,
    dock: Entity<DockArea>,
    /// Kept so the dock's appearance can be changed later; the area holds the same skin.
    _skin: Rc<DockSkin>,
    explorer: Entity<ExplorerPanel>,
    response: Entity<ResponsePanel>,
    details: Entity<DetailsPanel>,
    activity: Entity<ActivityPanel>,
    tabs: Vec<OpenTab>,
    active: Option<Session>,
    module: Module,
    palette: Entity<CommandState>,
    /// Numbers drafts so each gets a stable identity before it has an id from the engine.
    next_draft: u64,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (dock, skin) = DockSkin::dock_area("bifurc-main", Some(1), window, cx);
        let explorer = cx.new(|cx| ExplorerPanel::new(model.clone(), window, cx));
        let response = cx.new(ResponsePanel::new);
        let details = cx.new(|cx| DetailsPanel::new(model.clone(), cx));
        let activity = cx.new(|cx| ActivityPanel::new(model.clone(), cx));
        let home = cx.new(WelcomePanel::new);
        let palette = cx.new(|cx| CommandState::new(window, cx));

        // Dock sizes are physical starting points the user resizes; they are not part of the rem scale.
        dock.update(cx, |area, cx| {
            area.set_center(
                DockLayout::v_split()
                    .child(DockLayout::tabs().panel_view(panel_handle(home.clone()), cx), None)
                    .child(DockLayout::tabs().panel_view(panel_handle(response.clone()), cx), Some(px(300.))),
                window,
                cx,
            );
            area.set_dock(
                DockPlacement::Left,
                DockLayout::tabs().panel_view(panel_handle(explorer.clone()), cx),
                window,
                cx,
            );
            area.set_dock_size(DockPlacement::Left, px(300.), window, cx);
            area.set_dock(
                DockPlacement::Right,
                DockLayout::tabs().panel_view(panel_handle(details.clone()), cx),
                window,
                cx,
            );
            area.set_dock_size(DockPlacement::Right, px(300.), window, cx);
            area.set_dock(
                DockPlacement::Bottom,
                DockLayout::tabs().panel_view(panel_handle(activity.clone()), cx),
                window,
                cx,
            );
            area.set_dock_size(DockPlacement::Bottom, px(220.), window, cx);
            // Activity is a reference, not the work: it starts folded away.
            if area.is_dock_open(DockPlacement::Bottom) {
                area.toggle_dock(DockPlacement::Bottom, window, cx);
            }
        });

        let subscriptions = vec![
            cx.subscribe_in(&model, window, Self::on_model_event),
            cx.subscribe_in(&explorer, window, Self::on_explorer_event),
        ];

        let handle = ShellHandle(cx.weak_entity());
        cx.set_global(handle);

        Self {
            model,
            dock,
            _skin: skin,
            explorer,
            response,
            details,
            activity,
            tabs: Vec::new(),
            active: None,
            module: Module::Requests,
            palette,
            next_draft: 1,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    fn on_model_event(
        &mut self,
        _: &Entity<AppModel>,
        event: &ModelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::component::{WindowExt as _, notification::Notification};
        match event {
            ModelEvent::ConfigChanged => {
                self.close_vanished_tabs(window, cx);
                cx.notify();
            }
            ModelEvent::WorkspaceSwitched => self.close_all_tabs(window, cx),
            ModelEvent::Notice(text) => window.push_notification(Notification::warning(text.clone()), cx),
            ModelEvent::SocketStatus(_) | ModelEvent::SocketMessage(_) | ModelEvent::Traffic(_) => {}
        }
    }

    fn on_explorer_event(
        &mut self,
        _: &Entity<ExplorerPanel>,
        event: &ExplorerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ExplorerEvent::Open(node) => self.open_node(node, window, cx),
            ExplorerEvent::NewRequest { kind, folder } => self.new_request(*kind, folder.clone(), window, cx),
            ExplorerEvent::NewFolder { kind, parent } => self.prompt_new_folder(*kind, parent.clone(), window, cx),
            ExplorerEvent::Rename(node) => self.prompt_rename(node.clone(), window, cx),
            ExplorerEvent::Delete(node) => self.confirm_delete(node.clone(), window, cx),
            ExplorerEvent::NewMapping => self.prompt_new_mapping(window, cx),
            ExplorerEvent::DeleteMapping(id) => self.confirm_delete_mapping(id.clone(), window, cx),
        }
    }

    pub fn show_module(&mut self, module: Module, window: &mut Window, cx: &mut Context<Self>) {
        self.module = module;
        self.explorer.update(cx, |explorer, cx| explorer.set_module(module, cx));
        self.dock.update(cx, |area, cx| {
            if !area.is_dock_open(DockPlacement::Left) {
                area.toggle_dock(DockPlacement::Left, window, cx);
            }
        });
        cx.notify();
    }

    fn toggle(&mut self, placement: DockPlacement, window: &mut Window, cx: &mut Context<Self>) {
        self.dock.update(cx, |area, cx| area.toggle_dock(placement, window, cx));
        cx.notify();
    }

    pub fn toggle_explorer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle(DockPlacement::Left, window, cx);
    }

    pub fn toggle_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle(DockPlacement::Right, window, cx);
    }

    pub fn toggle_activity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle(DockPlacement::Bottom, window, cx);
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.refresh(cx));
    }
}

impl Focusable for Shell {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .key_context("Shell")
            .track_focus(&self.focus_handle)
            .child(self.render_title_bar(window, cx))
            .child(
                gpui_kit::component::h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(self.render_rail(cx))
                    .child(div().flex_1().min_w_0().child(self.dock.clone())),
            )
            .child(self.render_status_bar(cx))
    }
}
