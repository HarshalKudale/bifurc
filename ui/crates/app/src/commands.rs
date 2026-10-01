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
    model::RequestKind,
    shell::{Screen, Shell},
};

actions!(
    bifurc,
    [
        OpenCommandPalette,
        NewRestRequest,
        NewGraphQlRequest,
        NewGrpcRequest,
        NewSoapRequest,
        NewWebSocketRequest,
        NewFolder,
        SaveRequest,
        RevertRequest,
        RunRequest,
        CloseRequestTab,
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
        // The later of two bindings for one action is the one shown beside it, so Ctrl+K comes second.
        KeyBinding::new("secondary-shift-p", OpenCommandPalette, None),
        KeyBinding::new("secondary-k", OpenCommandPalette, None),
        KeyBinding::new("secondary-n", NewRestRequest, None),
        KeyBinding::new("secondary-shift-n", NewWebSocketRequest, None),
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

pub fn register_actions(cx: &mut App) {
    on_shell::<OpenCommandPalette>(cx, |shell, window, cx| shell.open_command_palette(window, cx));
    new_request::<NewRestRequest>(cx, RequestKind::Rest);
    new_request::<NewGraphQlRequest>(cx, RequestKind::GraphQl);
    new_request::<NewGrpcRequest>(cx, RequestKind::Grpc);
    new_request::<NewSoapRequest>(cx, RequestKind::Soap);
    new_request::<NewWebSocketRequest>(cx, RequestKind::WebSocket);
    on_shell::<NewFolder>(cx, |shell, window, cx| shell.prompt_new_folder(window, cx));
    on_shell::<SaveRequest>(cx, |shell, window, cx| shell.save_active(window, cx));
    on_shell::<RevertRequest>(cx, |shell, window, cx| shell.revert_active(window, cx));
    on_shell::<RunRequest>(cx, |shell, window, cx| shell.run_active(window, cx));
    on_shell::<CloseRequestTab>(cx, |shell, window, cx| shell.close_active(window, cx));
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
    show::<ShowSettings>(cx, Screen::Settings);
    on_shell::<ReloadWorkspace>(cx, |shell, _, cx| shell.reload(cx));
    cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
}
