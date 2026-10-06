//! The application's commands, each defined once.
//!
//! A command is a GPUI action. The key binding, the command palette entry and any menu item all dispatch the
//! same action, so they cannot disagree about what it is called, what key it has, or what it does. The
//! handlers are registered at application level and forward to the shell for the active window: an action
//! dispatched from the palette's dialog does not pass through the shell's own element tree, so handlers
//! attached there would never run.

use std::rc::Rc;

use gpui_kit::{Action, App, Context, Global, KeyBinding, WeakEntity, Window, actions};

use crate::{
    components::EntityAction,
    model::RequestKind,
    shell::{Screen, Shell},
};

actions!(
    bifurc,
    [
        OpenCommandPalette,
        OpenFinder,
        NewRestRequest,
        NewGraphQlRequest,
        NewGrpcRequest,
        NewSoapRequest,
        NewWebSocketRequest,
        NewSseRequest,
        NewSocketIoRequest,
        NewMcpRequest,
        ImportCurlRequest,
        ImportCurlMock,
        NewFolder,
        NewGraphQlFolder,
        NewGrpcFolder,
        NewSoapFolder,
        NewWebSocketFolder,
        NewSseFolder,
        NewSocketIoFolder,
        NewMcpFolder,
        NewMock,
        NewMockFolder,
        NewMapping,
        NewProxyRule,
        NewEnvironment,
        NewHealthCheck,
        NewWorkspace,
        RenameWorkspace,
        SaveRequest,
        RevertRequest,
        SyncEntity,
        DeleteEntity,
        RunRequest,
        CloseRequestTab,
        CheckAllHealth,
        RefreshServices,
        ToggleCapturePaused,
        ClearCapture,
        ToggleForwardProxy,
        RestartForwardProxy,
        PublishChanges,
        SyncWorkspace,
        ToggleSidebar,
        ToggleInspector,
        ToggleFolderTree,
        ShowServices,
        ShowHealthBar,
        ShowMappings,
        ShowProxyRules,
        ShowCapture,
        ShowRequests,
        ShowMocks,
        ShowEnvironments,
        ShowVaults,
        ShowWorkspaceConfig,
        ShowAuditLog,
        ShowSettings,
        ReloadWorkspace,
        Quit,
    ]
);

/// The shell of the open window, for the application-level handlers to reach.
pub struct ShellHandle(pub WeakEntity<Shell>);

impl Global for ShellHandle {}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        // The later of two bindings for one action is the one shown beside it, so the plain key comes second.
        KeyBinding::new("secondary-shift-p", OpenCommandPalette, None),
        KeyBinding::new("secondary-k", OpenCommandPalette, None),
        KeyBinding::new("secondary-p", OpenFinder, None),
        KeyBinding::new("secondary-n", NewRestRequest, None),
        KeyBinding::new("secondary-t", NewRestRequest, None),
        KeyBinding::new("secondary-shift-n", NewWebSocketRequest, None),
        KeyBinding::new("secondary-shift-m", NewMock, None),
        KeyBinding::new("secondary-s", SaveRequest, None),
        KeyBinding::new("secondary-enter", RunRequest, None),
        KeyBinding::new("secondary-w", CloseRequestTab, None),
        KeyBinding::new("secondary-b", ToggleSidebar, None),
        KeyBinding::new("secondary-shift-b", ToggleInspector, None),
        KeyBinding::new("secondary-shift-e", ToggleFolderTree, None),
        KeyBinding::new("secondary-1", ShowServices, None),
        KeyBinding::new("secondary-2", ShowHealthBar, None),
        KeyBinding::new("secondary-3", ShowMappings, None),
        KeyBinding::new("secondary-4", ShowProxyRules, None),
        KeyBinding::new("secondary-5", ShowCapture, None),
        KeyBinding::new("secondary-6", ShowRequests, None),
        KeyBinding::new("secondary-7", ShowMocks, None),
        KeyBinding::new("secondary-8", ShowEnvironments, None),
        KeyBinding::new("secondary-9", ShowVaults, None),
        KeyBinding::new("secondary-,", ShowSettings, None),
        KeyBinding::new("f5", ReloadWorkspace, None),
        KeyBinding::new("secondary-q", Quit, None),
    ]);
}

/// Runs `f` on the shell of the active window.
///
/// An action is dispatched from inside an update of the window it came from, and that window cannot be
/// updated again until the dispatch is over. The work is therefore deferred to the end of the current
/// update, where the window is free.
fn on_shell<A: Action + Clone>(cx: &mut App, f: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static) {
    let f = Rc::new(f);
    cx.on_action(move |_: &A, cx: &mut App| {
        let f = f.clone();
        cx.defer(move |cx| {
            let Some(shell) = cx.try_global::<ShellHandle>().map(|handle| handle.0.clone()) else { return };
            let Some(window) = cx.active_window() else { return };
            window
                .update(cx, |_, window, cx| {
                    shell.update(cx, |shell, cx| f(shell, window, cx)).ok();
                })
                .ok();
        });
    });
}

fn show<A: Action + Clone>(cx: &mut App, screen: Screen) {
    on_shell::<A>(cx, move |shell, window, cx| shell.show_screen(screen, window, cx));
}

fn new_request<A: Action + Clone>(cx: &mut App, kind: RequestKind) {
    on_shell::<A>(cx, move |shell, window, cx| shell.new_request(kind, window, cx));
}

fn new_folder<A: Action + Clone>(cx: &mut App, kind: RequestKind) {
    on_shell::<A>(cx, move |shell, window, cx| shell.prompt_new_folder(kind, window, cx));
}

pub fn register_actions(cx: &mut App) {
    on_shell::<OpenCommandPalette>(cx, |shell, window, cx| shell.open_command_palette(window, cx));
    on_shell::<OpenFinder>(cx, |shell, window, cx| shell.open_finder(window, cx));
    new_request::<NewRestRequest>(cx, RequestKind::Rest);
    new_request::<NewGraphQlRequest>(cx, RequestKind::GraphQl);
    new_request::<NewGrpcRequest>(cx, RequestKind::Grpc);
    new_request::<NewSoapRequest>(cx, RequestKind::Soap);
    new_request::<NewWebSocketRequest>(cx, RequestKind::WebSocket);
    new_request::<NewSseRequest>(cx, RequestKind::Sse);
    new_request::<NewSocketIoRequest>(cx, RequestKind::SocketIo);
    new_request::<NewMcpRequest>(cx, RequestKind::Mcp);
    on_shell::<ImportCurlRequest>(cx, |shell, window, cx| shell.import_curl_request(window, cx));
    on_shell::<ImportCurlMock>(cx, |shell, window, cx| shell.import_curl_mock(window, cx));
    new_folder::<NewFolder>(cx, RequestKind::Rest);
    new_folder::<NewGraphQlFolder>(cx, RequestKind::GraphQl);
    new_folder::<NewGrpcFolder>(cx, RequestKind::Grpc);
    new_folder::<NewSoapFolder>(cx, RequestKind::Soap);
    new_folder::<NewWebSocketFolder>(cx, RequestKind::WebSocket);
    new_folder::<NewSseFolder>(cx, RequestKind::Sse);
    new_folder::<NewSocketIoFolder>(cx, RequestKind::SocketIo);
    new_folder::<NewMcpFolder>(cx, RequestKind::Mcp);
    on_shell::<NewMock>(cx, |shell, window, cx| shell.new_mock(window, cx));
    on_shell::<NewMockFolder>(cx, |shell, window, cx| shell.prompt_new_mock_folder(window, cx));
    on_shell::<NewMapping>(cx, |shell, window, cx| shell.prompt_new_mapping(window, cx));
    on_shell::<NewProxyRule>(cx, |shell, window, cx| shell.new_proxy_rule(window, cx));
    on_shell::<NewEnvironment>(cx, |shell, window, cx| shell.prompt_new_environment(window, cx));
    on_shell::<NewHealthCheck>(cx, |shell, window, cx| shell.prompt_new_health_check(window, cx));
    on_shell::<NewWorkspace>(cx, |shell, window, cx| shell.prompt_new_workspace(window, cx));
    on_shell::<RenameWorkspace>(cx, |shell, window, cx| shell.prompt_rename_workspace(window, cx));
    on_shell::<SaveRequest>(cx, |shell, window, cx| shell.act_on_active(EntityAction::Save, window, cx));
    on_shell::<RevertRequest>(cx, |shell, window, cx| shell.act_on_active(EntityAction::Revert, window, cx));
    on_shell::<SyncEntity>(cx, |shell, window, cx| shell.act_on_active(EntityAction::Sync, window, cx));
    on_shell::<DeleteEntity>(cx, |shell, window, cx| shell.act_on_active(EntityAction::Delete, window, cx));
    on_shell::<CloseRequestTab>(cx, |shell, window, cx| shell.act_on_active(EntityAction::Close, window, cx));
    on_shell::<RunRequest>(cx, |shell, window, cx| shell.run_active(window, cx));
    on_shell::<CheckAllHealth>(cx, |shell, window, cx| shell.check_all_health(window, cx));
    on_shell::<RefreshServices>(cx, |shell, window, cx| shell.refresh_services(window, cx));
    on_shell::<ToggleCapturePaused>(cx, |shell, window, cx| shell.toggle_capture_paused(window, cx));
    on_shell::<ClearCapture>(cx, |shell, window, cx| shell.clear_capture(window, cx));
    on_shell::<ToggleForwardProxy>(cx, |shell, _, cx| shell.model.update(cx, |model, cx| model.toggle_proxy(cx)));
    on_shell::<RestartForwardProxy>(cx, |shell, _, cx| shell.model.update(cx, |model, cx| model.restart_proxy(cx)));
    on_shell::<PublishChanges>(cx, |shell, _, cx| shell.publish_changes(cx));
    on_shell::<SyncWorkspace>(cx, |shell, _, cx| shell.model.update(cx, |model, cx| model.sync_all(cx)));
    on_shell::<ToggleSidebar>(cx, |shell, _, cx| shell.toggle_sidebar(cx));
    on_shell::<ToggleInspector>(cx, |shell, window, cx| shell.toggle_right_sidebar(window, cx));
    on_shell::<ToggleFolderTree>(cx, |shell, _, cx| shell.toggle_folder_tree(cx));
    show::<ShowServices>(cx, Screen::Services);
    show::<ShowHealthBar>(cx, Screen::HealthBar);
    show::<ShowMappings>(cx, Screen::Mappings);
    show::<ShowProxyRules>(cx, Screen::ProxyRules);
    show::<ShowCapture>(cx, Screen::Capture);
    show::<ShowRequests>(cx, Screen::Requests);
    show::<ShowMocks>(cx, Screen::Mocks);
    show::<ShowEnvironments>(cx, Screen::Environments);
    show::<ShowVaults>(cx, Screen::Vaults);
    show::<ShowWorkspaceConfig>(cx, Screen::WorkspaceConfig);
    show::<ShowAuditLog>(cx, Screen::AuditLog);
    show::<ShowSettings>(cx, Screen::Settings);
    on_shell::<ReloadWorkspace>(cx, |shell, _, cx| shell.reload(cx));
    cx.on_action(|_: &Quit, cx: &mut App| {
        cx.quit();
    });
}
