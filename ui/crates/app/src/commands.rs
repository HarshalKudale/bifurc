//! The application's commands, each defined once.
//!
//! A command is a GPUI action. The key binding, the command palette entry and any menu item all dispatch the
//! same action, so they cannot disagree about what it is called, what key it has, or what it does. The
//! handlers are registered at application level and forward to the shell for the active window: an action
//! dispatched from the palette's dialog does not pass through the shell's own element tree, so handlers
//! attached there would never run.

use std::rc::Rc;

use gpui_kit::{Action, App, Context, Global, KeyBinding, WeakEntity, Window, actions};

use crate::shell::{Module, Shell};

actions!(
    bifurc,
    [
        OpenCommandPalette,
        NewRestRequest,
        NewWebSocketRequest,
        NewFolder,
        SaveRequest,
        RevertRequest,
        RunRequest,
        CloseRequestTab,
        ToggleExplorer,
        ToggleDetails,
        ToggleActivity,
        ShowRequests,
        ShowEnvironments,
        ShowMappings,
        ReloadWorkspace,
        Quit,
    ]
);

/// The shell of the open window, for the application-level handlers to reach.
pub struct ShellHandle(pub WeakEntity<Shell>);

impl Global for ShellHandle {}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-shift-p", OpenCommandPalette, None),
        KeyBinding::new("secondary-k", OpenCommandPalette, None),
        KeyBinding::new("secondary-n", NewRestRequest, None),
        KeyBinding::new("secondary-shift-n", NewWebSocketRequest, None),
        KeyBinding::new("secondary-s", SaveRequest, None),
        KeyBinding::new("secondary-enter", RunRequest, None),
        KeyBinding::new("secondary-w", CloseRequestTab, None),
        KeyBinding::new("secondary-b", ToggleExplorer, None),
        KeyBinding::new("secondary-shift-b", ToggleDetails, None),
        KeyBinding::new("secondary-j", ToggleActivity, None),
        KeyBinding::new("secondary-1", ShowRequests, None),
        KeyBinding::new("secondary-2", ShowEnvironments, None),
        KeyBinding::new("secondary-3", ShowMappings, None),
        KeyBinding::new("f5", ReloadWorkspace, None),
        KeyBinding::new("secondary-q", Quit, None),
    ]);
}

/// Runs `f` on the shell of the active window.
///
/// An action is dispatched from inside an update of the window it came from, and that window cannot be
/// updated again until the dispatch is over. The work is therefore deferred to the end of the current
/// update, where the window is free.
fn on_shell<A: Action + Clone>(cx: &mut App, f: impl Fn(&mut Shell, &A, &mut Window, &mut Context<Shell>) + 'static) {
    let f = Rc::new(f);
    cx.on_action(move |action: &A, cx: &mut App| {
        let (action, f) = (action.clone(), f.clone());
        cx.defer(move |cx| {
            let Some(shell) = cx.try_global::<ShellHandle>().map(|handle| handle.0.clone()) else { return };
            let Some(window) = cx.active_window() else { return };
            window
                .update(cx, |_, window, cx| {
                    shell.update(cx, |shell, cx| f(shell, &action, window, cx)).ok();
                })
                .ok();
        });
    });
}

pub fn register_actions(cx: &mut App) {
    on_shell(cx, |shell, _: &OpenCommandPalette, window, cx| shell.open_command_palette(window, cx));
    on_shell(cx, |shell, _: &NewRestRequest, window, cx| {
        shell.new_request(crate::model::RequestKind::Rest, None, window, cx)
    });
    on_shell(cx, |shell, _: &NewWebSocketRequest, window, cx| {
        shell.new_request(crate::model::RequestKind::WebSocket, None, window, cx)
    });
    on_shell(cx, |shell, _: &NewFolder, window, cx| {
        shell.prompt_new_folder(crate::model::RequestKind::Rest, None, window, cx)
    });
    on_shell(cx, |shell, _: &SaveRequest, window, cx| shell.save_active(window, cx));
    on_shell(cx, |shell, _: &RevertRequest, window, cx| shell.revert_active(window, cx));
    on_shell(cx, |shell, _: &RunRequest, window, cx| shell.run_active(window, cx));
    on_shell(cx, |shell, _: &CloseRequestTab, window, cx| shell.close_active(window, cx));
    on_shell(cx, |shell, _: &ToggleExplorer, window, cx| shell.toggle_explorer(window, cx));
    on_shell(cx, |shell, _: &ToggleDetails, window, cx| shell.toggle_details(window, cx));
    on_shell(cx, |shell, _: &ToggleActivity, window, cx| shell.toggle_activity(window, cx));
    on_shell(cx, |shell, _: &ShowRequests, window, cx| shell.show_module(Module::Requests, window, cx));
    on_shell(cx, |shell, _: &ShowEnvironments, window, cx| shell.show_module(Module::Environments, window, cx));
    on_shell(cx, |shell, _: &ShowMappings, window, cx| shell.show_module(Module::Mappings, window, cx));
    on_shell(cx, |shell, _: &ReloadWorkspace, _, cx| shell.reload(cx));
    cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
}
