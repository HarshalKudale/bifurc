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
    preferences::Preferences,
    shell::{Screen, Shell},
    theme,
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
        NewMock,
        NewMockFolder,
        NewMapping,
        NewProxyRule,
        NewEnvironment,
        NewFlow,
        NewHealthCheck,
        NewWorkspace,
        RenameWorkspace,
        SaveRequest,
        RevertRequest,
        SyncEntity,
        DeleteEntity,
        DuplicateEntity,
        RenameEntity,
        FocusUrl,
        DuplicateNode,
        RunRequest,
        CancelRequest,
        CloseRequestTab,
        ReopenClosedTab,
        NextTab,
        PreviousTab,
        NextItem,
        PreviousItem,
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
        ShowFlows,
        ShowEnvironments,
        ShowVaults,
        ShowWorkspaceConfig,
        ShowAuditLog,
        ShowSettings,
        ReloadWorkspace,
        ZoomIn,
        ZoomOut,
        ResetZoom,
        Quit,
    ]
);

/// The shell of the open window, for the application-level handlers to reach.
pub struct ShellHandle(pub WeakEntity<Shell>);

impl Global for ShellHandle {}

/// Where the keys that act on the open tab (save, send, close, new, step) apply: inside the shell, not in a
/// dialog or the palette above it, where Ctrl+S or Ctrl+W would otherwise reach the editor behind. The shell
/// takes focus when it opens (see `Shell::new`) so these work before the first click.
const IN_SHELL: Option<&str> = Some("Shell");

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys(keys());
}

fn keys() -> Vec<KeyBinding> {
    let mut bindings = vec![
        // The later of two bindings for one action is the one shown beside it, so the plain key comes second.
        KeyBinding::new("secondary-shift-p", OpenCommandPalette, None),
        KeyBinding::new("secondary-k", OpenCommandPalette, None),
        KeyBinding::new("secondary-p", OpenFinder, None),
        KeyBinding::new("secondary-n", NewRestRequest, IN_SHELL),
        KeyBinding::new("secondary-t", NewRestRequest, IN_SHELL),
        KeyBinding::new("secondary-shift-n", NewWebSocketRequest, IN_SHELL),
        KeyBinding::new("secondary-shift-m", NewMock, IN_SHELL),
        KeyBinding::new("secondary-shift-g", NewFlow, IN_SHELL),
        KeyBinding::new("secondary-s", SaveRequest, IN_SHELL),
        KeyBinding::new("secondary-d", DuplicateEntity, IN_SHELL),
        // The flow canvas takes Ctrl+D for its selected node: its context is deeper than the shell's, so it wins.
        KeyBinding::new("secondary-d", DuplicateNode, Some("Shell > FlowCanvas")),
        KeyBinding::new("f2", RenameEntity, IN_SHELL),
        KeyBinding::new("secondary-l", FocusUrl, IN_SHELL),
        KeyBinding::new("secondary-enter", RunRequest, IN_SHELL),
        // A text field binds Ctrl+Enter itself (a newline in a body, a submit in a line), and a deeper context
        // outranks `Shell`. This one sits at the field's own depth and comes later, so Send still wins there.
        KeyBinding::new("secondary-enter", RunRequest, Some("Shell > Input")),
        // Esc cancels a send only from inside a request editor; elsewhere it closes what is open.
        KeyBinding::new("escape", CancelRequest, Some("RestEditor")),
        KeyBinding::new("secondary-w", CloseRequestTab, IN_SHELL),
        KeyBinding::new("secondary-shift-t", ReopenClosedTab, IN_SHELL),
        // Not Ctrl+arrows: macOS takes those for Spaces, and in text they jump by word.
        KeyBinding::new("ctrl-pagedown", NextTab, IN_SHELL),
        KeyBinding::new("ctrl-pageup", PreviousTab, IN_SHELL),
        KeyBinding::new("ctrl-shift-tab", PreviousItem, IN_SHELL),
        KeyBinding::new("ctrl-tab", NextItem, IN_SHELL),
        KeyBinding::new("secondary-b", ToggleSidebar, None),
        KeyBinding::new("secondary-shift-b", ToggleInspector, None),
        KeyBinding::new("secondary-shift-e", ToggleFolderTree, None),
        // The sidebar's screens from the top, 1 to 9. Vaults, Config, Audit log and Settings have no key: Ctrl+0 is zoom reset.
        KeyBinding::new("secondary-1", ShowServices, None),
        KeyBinding::new("secondary-2", ShowHealthBar, None),
        KeyBinding::new("secondary-3", ShowMappings, None),
        KeyBinding::new("secondary-4", ShowProxyRules, None),
        KeyBinding::new("secondary-5", ShowCapture, None),
        KeyBinding::new("secondary-6", ShowRequests, None),
        KeyBinding::new("secondary-7", ShowMocks, None),
        KeyBinding::new("secondary-8", ShowFlows, None),
        KeyBinding::new("secondary-9", ShowEnvironments, None),
        KeyBinding::new("secondary-,", ShowSettings, None),
        KeyBinding::new("f5", ReloadWorkspace, None),
        // Zoom steps through the interface sizes Settings offers. Not Ctrl+Shift+0 for reset: Windows keeps that
        // for switching input languages.
        KeyBinding::new("secondary-=", ZoomIn, None),
        KeyBinding::new("secondary-+", ZoomIn, None),
        KeyBinding::new("secondary--", ZoomOut, None),
        KeyBinding::new("secondary-0", ResetZoom, None),
        KeyBinding::new("secondary-q", Quit, None),
    ];
    // Alt+1..9 (Cmd+Option+1..9 on macOS) start the kinds of request in the order the empty Requests screen lists
    // them, 9 being the cURL import. Not Shift+digit: every platform names that key by the character the layout
    // shifts it to (`@` on a US layout, `"` on a German one), and macOS keeps Cmd+Shift+3/4/5 for screenshots. Not
    // Ctrl+Alt+digit, which is AltGr+digit and types `{` or `]` on many layouts.
    let chord = if cfg!(target_os = "macos") { "secondary-alt" } else { "alt" };
    let mut digit = 0;
    let mut bind = |action: &dyn Fn(&str, Option<&str>) -> Vec<KeyBinding>| {
        digit += 1;
        bindings.extend(action(&format!("{chord}-{digit}"), IN_SHELL));
    };
    bind(&|k, c| vec![KeyBinding::new(k, NewRestRequest, c)]);
    bind(&|k, c| vec![KeyBinding::new(k, NewGraphQlRequest, c)]);
    bind(&|k, c| vec![KeyBinding::new(k, NewGrpcRequest, c)]);
    bind(&|k, c| vec![KeyBinding::new(k, NewSoapRequest, c)]);
    bind(&|k, c| vec![KeyBinding::new(k, NewWebSocketRequest, c)]);
    bind(&|k, c| vec![KeyBinding::new(k, NewSseRequest, c)]);
    bind(&|k, c| vec![KeyBinding::new(k, NewSocketIoRequest, c)]);
    bind(&|k, c| vec![KeyBinding::new(k, NewMcpRequest, c)]);
    bind(&|k, c| vec![KeyBinding::new(k, ImportCurlRequest, c)]);
    bindings
}

/// The actions on Alt+1..9 (Cmd+Option+1..9 on macOS), in the order the empty Requests screen lists what it can start.
pub fn request_actions() -> [Box<dyn Action>; 9] {
    [
        Box::new(NewRestRequest),
        Box::new(NewGraphQlRequest),
        Box::new(NewGrpcRequest),
        Box::new(NewSoapRequest),
        Box::new(NewWebSocketRequest),
        Box::new(NewSseRequest),
        Box::new(NewSocketIoRequest),
        Box::new(NewMcpRequest),
        Box::new(ImportCurlRequest),
    ]
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
    on_shell::<NewFolder>(cx, |shell, window, cx| shell.prompt_new_folder(window, cx));
    on_shell::<NewMock>(cx, |shell, window, cx| shell.new_mock(window, cx));
    on_shell::<NewMockFolder>(cx, |shell, window, cx| shell.prompt_new_mock_folder(window, cx));
    on_shell::<NewMapping>(cx, |shell, window, cx| shell.prompt_new_mapping(window, cx));
    on_shell::<NewProxyRule>(cx, |shell, window, cx| shell.new_proxy_rule(window, cx));
    on_shell::<NewEnvironment>(cx, |shell, window, cx| shell.prompt_new_environment(window, cx));
    on_shell::<NewFlow>(cx, |shell, window, cx| shell.prompt_new_flow(window, cx));
    on_shell::<NewHealthCheck>(cx, |shell, window, cx| shell.prompt_new_health_check(window, cx));
    on_shell::<NewWorkspace>(cx, |shell, window, cx| shell.prompt_new_workspace(window, cx));
    on_shell::<RenameWorkspace>(cx, |shell, window, cx| shell.prompt_rename_workspace(window, cx));
    on_shell::<SaveRequest>(cx, |shell, window, cx| shell.act_on_active(EntityAction::Save, window, cx));
    on_shell::<RevertRequest>(cx, |shell, window, cx| shell.act_on_active(EntityAction::Revert, window, cx));
    on_shell::<SyncEntity>(cx, |shell, window, cx| shell.act_on_active(EntityAction::Sync, window, cx));
    on_shell::<DeleteEntity>(cx, |shell, window, cx| shell.act_on_active(EntityAction::Delete, window, cx));
    on_shell::<DuplicateEntity>(cx, |shell, window, cx| shell.duplicate_active(window, cx));
    on_shell::<RenameEntity>(cx, |shell, window, cx| shell.rename_active(window, cx));
    on_shell::<FocusUrl>(cx, |shell, window, cx| shell.focus_url(window, cx));
    on_shell::<CloseRequestTab>(cx, |shell, window, cx| shell.act_on_active(EntityAction::Close, window, cx));
    on_shell::<ReopenClosedTab>(cx, |shell, window, cx| shell.reopen_closed_tab(window, cx));
    on_shell::<NextTab>(cx, |shell, window, cx| shell.step_tab(1, false, window, cx));
    on_shell::<PreviousTab>(cx, |shell, window, cx| shell.step_tab(-1, false, window, cx));
    on_shell::<NextItem>(cx, |shell, window, cx| shell.step_tab(1, true, window, cx));
    on_shell::<PreviousItem>(cx, |shell, window, cx| shell.step_tab(-1, true, window, cx));
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
    show::<ShowFlows>(cx, Screen::Flows);
    show::<ShowEnvironments>(cx, Screen::Environments);
    show::<ShowVaults>(cx, Screen::Vaults);
    show::<ShowWorkspaceConfig>(cx, Screen::WorkspaceConfig);
    show::<ShowAuditLog>(cx, Screen::AuditLog);
    show::<ShowSettings>(cx, Screen::Settings);
    on_shell::<ReloadWorkspace>(cx, |shell, _, cx| shell.reload(cx));
    cx.on_action(|_: &ZoomIn, cx: &mut App| set_zoom(zoomed(Preferences::global(cx).font_size, 1), cx));
    cx.on_action(|_: &ZoomOut, cx: &mut App| set_zoom(zoomed(Preferences::global(cx).font_size, -1), cx));
    cx.on_action(|_: &ResetZoom, cx: &mut App| set_zoom(theme::DEFAULT_FONT_SIZE, cx));
    cx.on_action(|_: &Quit, cx: &mut App| {
        cx.quit();
    });
}

/// The interface size one step from `current` in Settings' list, staying at either end.
fn zoomed(current: f32, step: isize) -> f32 {
    let sizes = theme::FONT_SIZES;
    let at = sizes.iter().position(|size| *size == current).unwrap_or(0) as isize;
    sizes[(at + step).clamp(0, sizes.len() as isize - 1) as usize]
}

fn set_zoom(size: f32, cx: &mut App) {
    theme::set_font_size(size, cx);
    Preferences::update(cx, |p| p.font_size = size);
    cx.refresh_windows();
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Action, KeyContext, Keymap, Keystroke, actions};

    use super::*;

    actions!(test, [InputEnter]);

    fn first_for(keys: &str, contexts: &[&str]) -> Option<Box<dyn Action>> {
        // gpui-kit's own binding for a text field, which is registered before the app's.
        let mut all = vec![KeyBinding::new("secondary-enter", InputEnter, Some("Input"))];
        all.extend(super::keys());
        let contexts: Vec<_> = contexts.iter().map(|c| KeyContext::parse(c).unwrap()).collect();
        let (found, _) = Keymap::new(all).bindings_for_input(&[Keystroke::parse(keys).unwrap()], &contexts);
        found.first().map(|binding| binding.action().boxed_clone())
    }

    #[test]
    fn save_and_send_reach_a_field_in_the_shell() {
        let field = ["Root", "Shell", "RestEditor", "Input"];
        assert!(first_for("ctrl-s", &field).unwrap().as_any().is::<SaveRequest>());
        assert!(first_for("ctrl-enter", &field).unwrap().as_any().is::<RunRequest>());
        assert!(first_for("ctrl-enter", &["Root", "Shell", "Input mode=editor"]).unwrap().as_any().is::<RunRequest>());
        assert!(first_for("ctrl-s", &["Root", "Shell"]).unwrap().as_any().is::<SaveRequest>());
        assert!(first_for("ctrl-shift-t", &field).unwrap().as_any().is::<ReopenClosedTab>());
    }

    #[test]
    fn editor_keys_do_not_reach_through_a_dialog() {
        let dialog = ["Root", "Dialog", "Input"];
        assert!(first_for("ctrl-s", &dialog).is_none());
        assert!(first_for("ctrl-w", &dialog).is_none());
        assert!(first_for("ctrl-shift-t", &dialog).is_none());
        assert!(first_for("ctrl-enter", &dialog).unwrap().as_any().is::<InputEnter>());
        // Palette and finder stay reachable from anywhere.
        assert!(first_for("ctrl-k", &dialog).unwrap().as_any().is::<OpenCommandPalette>());
    }

    #[test]
    fn duplicate_rename_and_focus_url_belong_to_the_shell() {
        let field = ["Root", "Shell", "RestEditor", "Input"];
        assert!(first_for("ctrl-d", &field).unwrap().as_any().is::<DuplicateEntity>());
        assert!(first_for("f2", &field).unwrap().as_any().is::<RenameEntity>());
        assert!(first_for("ctrl-l", &field).unwrap().as_any().is::<FocusUrl>());
        let dialog = ["Root", "Dialog", "Input"];
        for keys in ["ctrl-d", "f2", "ctrl-l"] {
            assert!(first_for(keys, &dialog).is_none(), "{keys}");
        }
    }

    #[test]
    fn ctrl_d_on_the_flow_canvas_duplicates_the_node() {
        let canvas = ["Root", "Shell", "FlowCanvas"];
        assert!(first_for("ctrl-d", &canvas).unwrap().as_any().is::<DuplicateNode>());
        // A field inside a node is in the canvas too.
        assert!(first_for("ctrl-d", &["Root", "Shell", "FlowCanvas", "Input"]).unwrap().as_any().is::<DuplicateNode>());
        assert!(first_for("ctrl-d", &["Root", "Shell", "FlowEditor"]).unwrap().as_any().is::<DuplicateEntity>());
    }

    #[test]
    fn alt_digits_start_requests_in_the_shell_only() {
        let shell = ["Root", "Shell"];
        let field = ["Root", "Shell", "RestEditor", "Input"];
        let dialog = ["Root", "Dialog", "Input"];
        for (ix, action) in request_actions().iter().enumerate() {
            let keys = format!("alt-{}", ix + 1);
            for contexts in [&shell[..], &field[..]] {
                assert_eq!(first_for(&keys, contexts).unwrap().name(), action.name(), "{keys}");
            }
            assert!(first_for(&keys, &dialog).is_none(), "{keys}");
        }
        assert!(first_for("ctrl-shift-g", &shell).unwrap().as_any().is::<NewFlow>());
        assert!(first_for("ctrl-shift-g", &["Root", "Dialog"]).is_none());
        // The older keys stay.
        assert!(first_for("ctrl-n", &shell).unwrap().as_any().is::<NewRestRequest>());
        assert!(first_for("ctrl-t", &shell).unwrap().as_any().is::<NewRestRequest>());
        assert!(first_for("ctrl-shift-n", &shell).unwrap().as_any().is::<NewWebSocketRequest>());
        // On the flow canvas Ctrl+N is still NewRestRequest, which the canvas answers itself with its node picker.
        assert!(first_for("ctrl-n", &["Root", "Shell", "FlowCanvas"]).unwrap().as_any().is::<NewRestRequest>());
    }

    #[test]
    fn zoom_steps_through_the_settings_sizes_and_stops_at_the_ends() {
        assert_eq!(zoomed(16., 1), 17.);
        assert_eq!(zoomed(18., 1), 20.);
        assert_eq!(zoomed(20., 1), 20.);
        assert_eq!(zoomed(16., -1), 16.);
        assert_eq!(zoomed(20., -1), 18.);
    }

    #[test]
    fn escape_cancels_from_a_field_in_a_request_editor_only() {
        assert!(first_for("escape", &["Root", "Shell", "RestEditor", "Input"]).unwrap().as_any().is::<CancelRequest>());
        assert!(first_for("escape", &["Root", "Shell"]).is_none());
    }
}
