//! The window: title bar, left sidebar, the current screen, right sidebar and footer.
//!
//! The shell owns layout and navigation and nothing about the workspace itself. Only the area in the middle
//! changes when the sidebar's selection changes; the bars around it stay. Each screen is its own view with
//! its own state, kept alive while another screen is in front, so coming back to Requests finds its tabs as
//! they were. The workspace comes from [`AppModel`].

mod chrome;
mod nav;
mod palette;
mod right_sidebar;

use gpui_kit::component::{
    ActiveTheme as _, WindowExt as _,
    command::CommandState,
    h_flex,
    input::InputState,
    notification::Notification,
    resizable::{h_resizable, resizable_panel},
    v_flex,
};
use gpui_kit::{
    AnyView, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div, px,
};

pub use self::nav::Screen;
use self::right_sidebar::RightSidebar;
use crate::{
    app_model::{AppModel, ModelEvent},
    commands::ShellHandle,
    components::{
        EntityAction,
        dialogs::{PromptField, confirm_danger, prompt},
    },
    model::{NodeRef, RequestKind, snippet::RequestParts},
    preferences::Preferences,
    screens::{
        audit::AuditScreen,
        capture::{CaptureEvent, CaptureScreen},
        environments::EnvironmentsScreen,
        healthbar::HealthBarScreen,
        mappings::MappingsScreen,
        mocks::MocksScreen,
        proxy_rules::ProxyRulesScreen,
        requests::RequestsScreen,
        services::ServicesScreen,
        settings::SettingsScreen,
        workspace::WorkspaceScreen,
    },
};

/// The thing open on the current screen: what the breadcrumb ends in and what the right sidebar is about.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveTarget {
    pub title: SharedString,
    /// The saved entity's id, once it has one.
    pub entity_id: Option<String>,
    /// The request as an HTTP call, when it can be written as a snippet.
    pub request: Option<RequestParts>,
    /// The field the entity's name is typed in. The breadcrumb shows it in place of the title, so the name
    /// is edited where it is read.
    pub name: Option<Entity<InputState>>,
    /// The request's type, on the screen whose entities have one.
    pub kind: Option<RequestKind>,
    /// The folders it is in, outermost first; empty at the top level and for entities not kept in folders.
    pub folders: Vec<String>,
}

impl ActiveTarget {
    /// The target for an entity that is not a request: its title, its saved id and its name field.
    pub fn named(title: SharedString, entity_id: Option<String>, name: Entity<InputState>) -> Self {
        Self { title, entity_id, request: None, name: Some(name), kind: None, folders: Vec::new() }
    }
}

pub struct Shell {
    pub(crate) model: Entity<AppModel>,
    screen: Screen,
    requests: Entity<RequestsScreen>,
    mocks: Entity<MocksScreen>,
    services: Entity<ServicesScreen>,
    healthbar: Entity<HealthBarScreen>,
    proxy_rules: Entity<ProxyRulesScreen>,
    capture: Entity<CaptureScreen>,
    mappings: Entity<MappingsScreen>,
    environments: Entity<EnvironmentsScreen>,
    settings: Entity<SettingsScreen>,
    workspace: Entity<WorkspaceScreen>,
    audit: Entity<AuditScreen>,
    right: Entity<RightSidebar>,
    right_open: bool,
    palette: Entity<CommandState>,
    finder: Entity<CommandState>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let requests = cx.new(|cx| RequestsScreen::new(model.clone(), window, cx));
        let mocks = cx.new(|cx| MocksScreen::new(model.clone(), window, cx));
        let services = cx.new(|cx| ServicesScreen::new(model.clone(), window, cx));
        let healthbar = cx.new(|cx| HealthBarScreen::new(model.clone(), window, cx));
        let proxy_rules = cx.new(|cx| ProxyRulesScreen::new(model.clone(), window, cx));
        let capture = cx.new(|cx| CaptureScreen::new(model.clone(), window, cx));
        let mappings = cx.new(|cx| MappingsScreen::new(model.clone(), window, cx));
        let environments = cx.new(|cx| EnvironmentsScreen::new(model.clone(), window, cx));
        let settings = cx.new(|cx| SettingsScreen::new(model.clone(), cx));
        let workspace = cx.new(|cx| WorkspaceScreen::new(model.clone(), window, cx));
        let audit = cx.new(|cx| AuditScreen::new(model.clone(), window, cx));
        let right = cx.new(|cx| RightSidebar::new(model.clone(), window, cx));
        let palette = cx.new(|cx| CommandState::new(window, cx));
        let finder = cx.new(|cx| CommandState::new(window, cx));

        let subscriptions = vec![
            cx.subscribe_in(&model, window, Self::on_model_event),
            cx.observe(&model, |_, _, cx| cx.notify()),
            // The breadcrumb and the right sidebar follow what is open on the screen in front.
            cx.observe_in(&requests, window, |this, _, window, cx| this.sync_right_sidebar(window, cx)),
            cx.observe_in(&mocks, window, |this, _, window, cx| this.sync_right_sidebar(window, cx)),
            cx.observe_in(&proxy_rules, window, |this, _, window, cx| this.sync_right_sidebar(window, cx)),
            cx.observe_in(&environments, window, |this, _, window, cx| this.sync_right_sidebar(window, cx)),
            // These screens give the footer a line that changes as they work.
            cx.observe(&services, |_, _, cx| cx.notify()),
            cx.observe(&healthbar, |_, _, cx| cx.notify()),
            cx.observe(&capture, |_, _, cx| cx.notify()),
            cx.subscribe_in(&capture, window, Self::on_capture_event),
        ];

        let handle = ShellHandle(cx.weak_entity());
        cx.set_global(handle);

        Self {
            model,
            screen: Screen::Requests,
            requests,
            mocks,
            services,
            healthbar,
            proxy_rules,
            capture,
            mappings,
            environments,
            settings,
            workspace,
            audit,
            right,
            right_open: false,
            palette,
            finder,
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
        match event {
            ModelEvent::ConfigChanged | ModelEvent::WorkspaceSwitched => {
                self.sync_right_sidebar(window, cx);
                cx.notify();
            }
            ModelEvent::Notice(text) => window.push_notification(Notification::warning(text.clone()), cx),
            ModelEvent::SocketStatus(_)
            | ModelEvent::SocketMessage(_)
            | ModelEvent::Traffic(_)
            | ModelEvent::RunnerProgress(_)
            | ModelEvent::RunnerFinished(_)
            | ModelEvent::SyncStatus
            | ModelEvent::EngineRestarted
            | ModelEvent::EntityReverted(_) => {}
        }
    }

    /// What is open on the current screen, if the screen has such a thing.
    pub(super) fn active_target(&self, cx: &gpui_kit::App) -> Option<ActiveTarget> {
        match self.screen {
            Screen::Requests => self.requests.read(cx).active_tab_target(cx),
            Screen::Mocks => self.mocks.read(cx).active_target(cx),
            Screen::ProxyRules => self.proxy_rules.read(cx).active_target(cx),
            Screen::Environments => self.environments.read(cx).active_target(cx),
            _ => None,
        }
    }

    /// Asks the screen in front to do `action` to the entity it has open. Screens with nothing of the kind
    /// ignore it.
    fn entity_action(&mut self, action: EntityAction, window: &mut Window, cx: &mut Context<Self>) {
        match self.screen {
            Screen::Requests => self.requests.update(cx, |screen, cx| screen.entity_action(action, window, cx)),
            Screen::Mocks => self.mocks.update(cx, |screen, cx| screen.entity_action(action, window, cx)),
            Screen::ProxyRules => self.proxy_rules.update(cx, |screen, cx| screen.entity_action(action, window, cx)),
            Screen::Environments => self.environments.update(cx, |screen, cx| screen.entity_action(action, window, cx)),
            _ => {}
        }
    }

    /// Makes the open draft request one of another type.
    pub(super) fn change_request_kind(&mut self, kind: RequestKind, window: &mut Window, cx: &mut Context<Self>) {
        self.requests.update(cx, |requests, cx| requests.change_active_kind(kind, window, cx));
    }

    fn sync_right_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.active_target(cx);
        let open = self.right_open;
        self.right.update(cx, |right, cx| right.set_target(target, open, window, cx));
        cx.notify();
    }

    fn screen_view(&self) -> Option<AnyView> {
        match self.screen {
            Screen::Requests => Some(self.requests.clone().into()),
            Screen::Mocks => Some(self.mocks.clone().into()),
            Screen::Mappings => Some(self.mappings.clone().into()),
            Screen::Environments => Some(self.environments.clone().into()),
            Screen::Settings => Some(self.settings.clone().into()),
            Screen::WorkspaceConfig => Some(self.workspace.clone().into()),
            Screen::AuditLog => Some(self.audit.clone().into()),
            Screen::Services => Some(self.services.clone().into()),
            Screen::HealthBar => Some(self.healthbar.clone().into()),
            Screen::ProxyRules => Some(self.proxy_rules.clone().into()),
            Screen::Capture => Some(self.capture.clone().into()),
        }
    }

    pub fn show_screen(&mut self, screen: Screen, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen != screen {
            self.screen = screen;
            self.sync_right_sidebar(window, cx);
        }
        cx.notify();
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        Preferences::update(cx, |p| p.sidebar_collapsed = !p.sidebar_collapsed);
        cx.notify();
    }

    pub fn toggle_right_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.right_open = !self.right_open;
        // Opening it is what loads the history it shows.
        self.sync_right_sidebar(window, cx);
    }

    pub fn toggle_folder_tree(&mut self, cx: &mut Context<Self>) {
        Preferences::update(cx, |p| p.folder_tree_open = !p.folder_tree_open);
        self.requests.update(cx, |_, cx| cx.notify());
        self.mocks.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.refresh(cx));
    }

    pub fn new_request(&mut self, kind: RequestKind, window: &mut Window, cx: &mut Context<Self>) {
        self.show_screen(Screen::Requests, window, cx);
        self.requests.update(cx, |requests, cx| requests.new_request(kind, None, window, cx));
    }

    /// Asks for the name of a new top-level folder for requests of `kind`. Each request type has its own
    /// folders.
    pub fn prompt_new_folder(&mut self, kind: RequestKind, window: &mut Window, cx: &mut Context<Self>) {
        self.show_screen(Screen::Requests, window, cx);
        self.requests.update(cx, |requests, cx| requests.prompt_new_folder(kind, None, window, cx));
    }

    /// Opens a saved request in a tab and selects it in the tree.
    pub fn open_request(&mut self, node: &NodeRef, window: &mut Window, cx: &mut Context<Self>) {
        self.show_screen(Screen::Requests, window, cx);
        self.requests.update(cx, |requests, cx| {
            requests.open_node(node, window, cx);
            requests.reveal(node, cx);
        });
    }

    /// Opens a saved mock in a tab.
    pub fn open_mock(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.show_screen(Screen::Mocks, window, cx);
        self.mocks.update(cx, |mocks, cx| mocks.open_mock(id, window, cx));
    }

    pub fn new_mock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_screen(Screen::Mocks, window, cx);
        self.mocks.update(cx, |mocks, cx| mocks.new_mock(None, window, cx));
    }

    pub fn prompt_new_mock_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_screen(Screen::Mocks, window, cx);
        self.mocks.update(cx, |mocks, cx| mocks.prompt_new_folder(None, window, cx));
    }

    pub fn prompt_new_mapping(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_screen(Screen::Mappings, window, cx);
        self.mappings.update(cx, |mappings, cx| mappings.prompt_new(window, cx));
    }

    pub fn new_proxy_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_screen(Screen::ProxyRules, window, cx);
        self.proxy_rules.update(cx, |rules, cx| rules.new_rule(window, cx));
    }

    pub fn prompt_new_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_screen(Screen::Environments, window, cx);
        self.environments.update(cx, |environments, cx| environments.prompt_new(window, cx));
    }

    pub fn prompt_new_health_check(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_screen(Screen::HealthBar, window, cx);
        self.healthbar.update(cx, |healthbar, cx| healthbar.prompt_new(window, cx));
    }

    pub fn check_all_health(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_screen(Screen::HealthBar, window, cx);
        self.healthbar.update(cx, |healthbar, cx| healthbar.check_all(window, cx));
    }

    pub fn refresh_services(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_screen(Screen::Services, window, cx);
        self.services.update(cx, |services, cx| services.refresh(cx));
    }

    pub fn toggle_capture_paused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_screen(Screen::Capture, window, cx);
        self.capture.update(cx, |capture, cx| capture.toggle_paused(window, cx));
    }

    pub fn clear_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_screen(Screen::Capture, window, cx);
        self.capture.update(cx, |capture, cx| capture.clear(cx));
    }

    /// Commits what changed on the current screen (and pushes, when the workspace has a remote).
    pub fn publish_changes(&mut self, cx: &mut Context<Self>) {
        let rollup = self.model.read(cx).git().rollup(self.screen.git_prefixes());
        if rollup.action.is_none() {
            self.model.update(cx, |model, cx| model.notice("There is nothing to publish.", cx));
            return;
        }
        let paths = rollup.changed.clone();
        self.model.update(cx, |model, cx| model.publish(paths, cx));
    }

    /// Does `action` to what is open on the screen in front: the keys and the palette ask for the same
    /// things the tab's footer does.
    pub fn act_on_active(&mut self, action: EntityAction, window: &mut Window, cx: &mut Context<Self>) {
        self.entity_action(action, window, cx);
    }

    /// Send, or connect: the active request's main verb.
    pub fn run_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen == Screen::Requests {
            self.requests.update(cx, |screen, cx| screen.run_active(window, cx));
        }
    }

    /// An exchange saved from Capture is opened where it now lives.
    fn on_capture_event(
        &mut self,
        _: &Entity<CaptureScreen>,
        event: &CaptureEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The new entity is in the engine but not yet in this window's snapshot, which the tree is built from.
        self.model.update(cx, |model, cx| model.refresh(cx));
        match event {
            CaptureEvent::SavedRequest { id } => {
                self.open_request(&NodeRef::Request { kind: RequestKind::Rest, id: id.clone() }, window, cx)
            }
            CaptureEvent::SavedMock { id } => self.open_mock(id, window, cx),
        }
    }

    pub(super) fn prompt_new_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            "New workspace",
            "Create",
            vec![PromptField::new("Name", "Workspace name")],
            |this, values, _, cx| this.model.update(cx, |model, cx| model.add_workspace(&values[0], cx)),
        );
    }

    pub(super) fn prompt_rename_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.model.read(cx).config().active_workspace().cloned() else { return };
        prompt(
            cx.weak_entity(),
            window,
            cx,
            format!("Rename “{}”", workspace.name),
            "Rename",
            vec![PromptField::new("Name", "Workspace name").initial(workspace.name.clone())],
            move |this, values, _, cx| {
                this.model.update(cx, |model, cx| model.rename_workspace(&workspace.id, &values[0], cx))
            },
        );
    }

    pub(super) fn confirm_delete_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.model.read(cx).config().active_workspace().cloned() else { return };
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            format!("Delete workspace “{}”?", workspace.name),
            "Its requests, mocks, mappings and history are deleted from this computer. This can't be undone.",
            "Delete",
            move |this, _, cx| this.model.update(cx, |model, cx| model.delete_workspace(&workspace.id, cx)),
        );
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
        let screen = match self.screen_view() {
            Some(view) => view.into_any_element(),
            None => crate::components::page::empty(
                self.screen.label(),
                "This screen is not part of the desktop UI yet. Use the Electron app or the bifurc CLI for it.",
            ),
        };
        let center = v_flex()
            .size_full()
            .child(self.render_top_bar(cx))
            .child(div().flex_1().min_h_0().bg(theme.background).child(screen));

        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .key_context("Shell")
            .track_focus(&self.focus_handle)
            .child(self.render_title_bar(window, cx))
            .child(
                h_flex().flex_1().min_h_0().items_stretch().child(self.render_sidebar(window, cx)).child(
                    // The right sidebar's width is a physical starting point the user resizes.
                    div().flex_1().min_w_0().child(
                        h_resizable("shell-split").child(resizable_panel().child(center)).child(
                            resizable_panel()
                                .visible(self.right_open)
                                .size(px(340.))
                                .size_range(px(260.)..px(640.))
                                .child(self.right.clone()),
                        ),
                    ),
                ),
            )
            .child(self.render_footer(cx))
    }
}
