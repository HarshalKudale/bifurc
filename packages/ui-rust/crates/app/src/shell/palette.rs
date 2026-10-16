//! The two searches the title bar opens, kept apart on purpose.
//!
//! - The **command center** (Ctrl+K) lists only actions: make a new thing, go to a screen, toggle a view. It
//!   shows the key each is bound to and dispatches the same action the key does.
//! - The **finder** (Ctrl+P) lists the things the workspace holds, to open one: requests, mocks, mappings and
//!   environments. They are not commands; confirming one runs the shell method directly.

use gpui_kit::component::IndexPath;
use gpui_kit::component::{
    IconName, WindowExt as _,
    command::{Command, CommandGroup, CommandItem},
};
use gpui_kit::{Action, Context, ParentElement as _, Styled as _, Window};

use super::{Screen, Shell};
use crate::{
    commands::{
        CheckAllHealth, ClearCapture, CloseRequestTab, DeleteEntity, ImportCurlMock, ImportCurlRequest, NewEnvironment,
        NewFolder, NewGraphQlRequest, NewGrpcRequest, NewHealthCheck, NewMapping, NewMcpRequest, NewMock, NewMockFolder,
        NewProxyRule, NewRestRequest, NewSoapRequest, NewSocketIoRequest, NewSseRequest, NewWebSocketRequest,
        NewWorkspace, PublishChanges, RefreshServices, ReloadWorkspace, ReopenClosedTab, RenameWorkspace, RestartForwardProxy,
        RevertRequest, RunRequest, SaveRequest, ShowAuditLog, ShowCapture, ShowEnvironments, ShowFlows, ShowHealthBar,
        ShowMappings, ShowMocks, ShowProxyRules, ShowRequests, ShowServices, ShowSettings, ShowVaults, ShowWorkspaceConfig,
        SyncEntity, SyncWorkspace, ToggleCapturePaused, ToggleFolderTree, ToggleForwardProxy, ToggleInspector,
        ToggleSidebar,
    },
    i18n::{t, t_with},
    model::NodeRef,
};

/// What confirming a row does, for the rows that are not an action.
#[derive(Clone)]
enum Choice {
    Request(NodeRef),
    Mock(String),
    Mapping,
    Environment(String),
    UseEnvironment(Option<String>),
}

struct Entry {
    label: String,
    keywords: Vec<String>,
    choice: Choice,
}

type CommandRow = (&'static str, IconName, Box<dyn Action>);

/// The commands, in the groups the palette shows them in. Every screen's own actions are here, so the
/// palette is the one place that lists what the app can do.
fn command_groups() -> Vec<(&'static str, Vec<CommandRow>)> {
    vec![
        (
            t("shell.screen.requests"),
            vec![
                (t("shell.palette.new_rest_request"), IconName::Plus, Box::new(NewRestRequest)),
                (t("shell.palette.new_graphql_request"), IconName::Plus, Box::new(NewGraphQlRequest)),
                (t("shell.palette.new_grpc_request"), IconName::Plus, Box::new(NewGrpcRequest)),
                (t("shell.palette.new_soap_request"), IconName::Plus, Box::new(NewSoapRequest)),
                (t("shell.palette.new_websocket_request"), IconName::Plus, Box::new(NewWebSocketRequest)),
                (t("shell.palette.new_sse_request"), IconName::Plus, Box::new(NewSseRequest)),
                (t("shell.palette.new_socketio_request"), IconName::Plus, Box::new(NewSocketIoRequest)),
                (t("shell.palette.new_mcp_request"), IconName::Plus, Box::new(NewMcpRequest)),
                (t("shell.palette.import_request_curl"), IconName::Plus, Box::new(ImportCurlRequest)),
                (t("shell.palette.new_request_folder"), IconName::Folder, Box::new(NewFolder)),
                (t("shell.palette.send_request"), IconName::Play, Box::new(RunRequest)),
            ],
        ),
        (
            t("shell.screen.mocks"),
            vec![
                (t("shell.palette.new_mock"), IconName::Plus, Box::new(NewMock)),
                (t("shell.palette.import_mock_curl"), IconName::Plus, Box::new(ImportCurlMock)),
                (t("shell.palette.new_mock_folder"), IconName::Folder, Box::new(NewMockFolder)),
            ],
        ),
        (
            t("shell.palette.group_proxy"),
            vec![
                (t("shell.palette.new_mapping"), IconName::Plus, Box::new(NewMapping)),
                (t("shell.palette.new_proxy_rule"), IconName::Plus, Box::new(NewProxyRule)),
                (t("shell.palette.pause_capture"), IconName::Pause, Box::new(ToggleCapturePaused)),
                (t("shell.palette.clear_capture"), IconName::Delete, Box::new(ClearCapture)),
                (t("shell.palette.toggle_forward_proxy"), IconName::Play, Box::new(ToggleForwardProxy)),
                (t("shell.palette.restart_forward_proxy"), IconName::RefreshCw, Box::new(RestartForwardProxy)),
            ],
        ),
        (
            t("shell.screen.services"),
            vec![
                (t("shell.palette.refresh_services"), IconName::RefreshCw, Box::new(RefreshServices)),
                (t("shell.palette.new_health_check"), IconName::Plus, Box::new(NewHealthCheck)),
                (t("shell.palette.check_all_health"), IconName::Heart, Box::new(CheckAllHealth)),
            ],
        ),
        (
            t("shell.palette.group_open_item"),
            vec![
                (t("shell.palette.save"), IconName::File, Box::new(SaveRequest)),
                (t("shell.palette.sync_item"), IconName::Github, Box::new(SyncEntity)),
                (t("shell.palette.revert_item"), IconName::Undo, Box::new(RevertRequest)),
                (t("shell.palette.delete_item"), IconName::Delete, Box::new(DeleteEntity)),
                (t("shell.palette.close_tab"), IconName::Close, Box::new(CloseRequestTab)),
                (t("shell.palette.reopen_tab"), IconName::Undo, Box::new(ReopenClosedTab)),
            ],
        ),
        (
            t("shell.group.workspace"),
            vec![
                (t("shell.palette.new_environment"), IconName::Plus, Box::new(NewEnvironment)),
                (t("shell.palette.add_workspace"), IconName::Plus, Box::new(NewWorkspace)),
                (t("shell.palette.rename_workspace"), IconName::File, Box::new(RenameWorkspace)),
                (t("shell.palette.publish_screen"), IconName::Github, Box::new(PublishChanges)),
                (t("shell.palette.sync_workspace"), IconName::Github, Box::new(SyncWorkspace)),
                (t("shell.palette.reload_workspace"), IconName::RefreshCw, Box::new(ReloadWorkspace)),
            ],
        ),
        (
            t("shell.palette.group_go_to"),
            vec![
                (t("shell.palette.go_services"), Screen::Services.icon(), Box::new(ShowServices)),
                (t("shell.palette.go_health_bar"), Screen::HealthBar.icon(), Box::new(ShowHealthBar)),
                (t("shell.palette.go_mappings"), Screen::Mappings.icon(), Box::new(ShowMappings)),
                (t("shell.palette.go_proxy_rules"), Screen::ProxyRules.icon(), Box::new(ShowProxyRules)),
                (t("shell.palette.go_capture"), Screen::Capture.icon(), Box::new(ShowCapture)),
                (t("shell.palette.go_requests"), Screen::Requests.icon(), Box::new(ShowRequests)),
                (t("shell.palette.go_mocks"), Screen::Mocks.icon(), Box::new(ShowMocks)),
                (t("shell.palette.go_flows"), Screen::Flows.icon(), Box::new(ShowFlows)),
                (t("shell.palette.go_environments"), Screen::Environments.icon(), Box::new(ShowEnvironments)),
                (t("shell.palette.go_vaults"), Screen::Vaults.icon(), Box::new(ShowVaults)),
                (t("shell.palette.go_workspace_config"), Screen::WorkspaceConfig.icon(), Box::new(ShowWorkspaceConfig)),
                (t("shell.palette.go_audit_log"), Screen::AuditLog.icon(), Box::new(ShowAuditLog)),
                (t("shell.palette.go_settings"), Screen::Settings.icon(), Box::new(ShowSettings)),
            ],
        ),
        (
            t("shell.palette.group_view"),
            vec![
                (t("shell.palette.toggle_sidebar"), IconName::PanelLeft, Box::new(ToggleSidebar)),
                (t("shell.palette.toggle_inspector"), IconName::PanelRight, Box::new(ToggleInspector)),
                (t("shell.palette.toggle_folder_tree"), IconName::FolderOpen, Box::new(ToggleFolderTree)),
            ],
        ),
    ]
}

/// A group of rows that are things to open or choose, not actions.
fn choice_group(label: &'static str, entries: &[Entry]) -> CommandGroup {
    entries.iter().fold(CommandGroup::new().label(label), |group, entry| {
        group.item(CommandItem::new().label(entry.label.clone()).keywords(entry.keywords.clone()))
    })
}

/// The dialog both searches share: `groups` as rows over `state`, and what to do with a confirmed one.
/// `choices` is parallel to `groups`, section by section; a command's action has already been dispatched
/// by the time its row is confirmed, so its section holds nothing.
fn open_search(
    shell: gpui_kit::WeakEntity<Shell>,
    state: gpui_kit::Entity<gpui_kit::component::command::CommandState>,
    placeholder: &'static str,
    groups: Vec<CommandGroup>,
    choices: Vec<Vec<Choice>>,
    window: &mut Window,
    cx: &mut Context<Shell>,
) {
    state.update(cx, |state, cx| state.set_query("", window, cx));
    let for_focus = state.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let state = state.clone();
        let shell = shell.clone();
        let groups = groups.clone();
        let choices = choices.clone();
        dialog.close_button(false).p_0().w_128().content(move |content, _, _| {
            let shell = shell.clone();
            let choices = choices.clone();
            let command =
                groups.iter().fold(Command::new(&state).bordered(false).placeholder(placeholder), |command, group| {
                    command.group(group.clone())
                });
            content.child(command.on_confirm(move |ix: IndexPath, window, cx| {
                let choice = choices.get(ix.section).and_then(|section| section.get(ix.row)).cloned();
                window.close_dialog(cx);
                if let Some(choice) = choice {
                    shell.update(cx, |shell, cx| shell.apply_choice(choice, window, cx)).ok();
                }
            }))
        })
    });
    // The query field is where typing starts.
    window.defer(cx, move |window, cx| for_focus.update(cx, |state, cx| state.focus(window, cx)));
}

impl Shell {
    /// Ctrl+K: only actions, and the environment to use.
    pub fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let config = self.model.read(cx).config().clone();
        let environments: Vec<Entry> = std::iter::once(Entry {
            label: t("shell.palette.use_no_environment").to_owned(),
            keywords: vec![t("shell.palette.keyword_environment").to_owned()],
            choice: Choice::UseEnvironment(None),
        })
        .chain(config.selectable_environments().map(|e| Entry {
            label: t_with("shell.palette.use_environment", &[("name", &e.name)]),
            keywords: vec![t("shell.palette.keyword_environment").to_owned()],
            choice: Choice::UseEnvironment(Some(e.id.clone())),
        }))
        .collect();

        let mut groups = Vec::new();
        let mut choices: Vec<Vec<Choice>> = Vec::new();
        for (label, rows) in command_groups() {
            groups.push(rows.into_iter().fold(CommandGroup::new().label(label), |group, (label, icon, action)| {
                group.item(CommandItem::new().label(label).icon(icon).action(action))
            }));
            choices.push(Vec::new());
        }
        groups.push(choice_group(t("shell.palette.group_environment"), &environments));
        choices.push(environments.iter().map(|entry| entry.choice.clone()).collect());

        let (state, shell) = (self.palette.clone(), cx.weak_entity());
        open_search(shell, state, t("shell.palette.search_commands"), groups, choices, window, cx);
    }

    /// Ctrl+P: the things in the workspace, to open one.
    pub fn open_finder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let config = self.model.read(cx).config().clone();
        let requests: Vec<Entry> = config
            .requests
            .iter()
            .filter(|r| !r.deleted && r.locked.is_none())
            .map(|r| {
                let label = if r.name.is_empty() { r.target.clone() } else { r.name.clone() };
                Entry {
                    label: format!("{}  {label}", r.method.clone().unwrap_or_else(|| r.kind.badge().to_owned())),
                    keywords: vec![r.target.clone(), t("shell.palette.keyword_request").to_owned()],
                    choice: Choice::Request(NodeRef::Request { kind: r.kind, id: r.id.clone() }),
                }
            })
            .collect();
        let mocks: Vec<Entry> = config
            .mocks
            .iter()
            .filter(|m| !m.deleted && m.locked.is_none())
            .map(|m| {
                let label = if m.name.is_empty() { m.url_pattern.clone() } else { m.name.clone() };
                let method = if m.method == "*" { t("shell.palette.method_any") } else { m.method.as_str() };
                Entry {
                    label: format!("{method}  {label}"),
                    keywords: vec![m.url_pattern.clone(), t("shell.palette.keyword_mock").to_owned()],
                    choice: Choice::Mock(m.id.clone()),
                }
            })
            .collect();
        let mappings: Vec<Entry> = config
            .mappings
            .iter()
            .filter(|m| !m.deleted && m.locked.is_none())
            .map(|m| Entry {
                label: format!("{}  →  {}", m.domain, m.target),
                keywords: vec![m.label.clone().unwrap_or_default(), t("shell.palette.keyword_mapping").to_owned()],
                choice: Choice::Mapping,
            })
            .collect();
        let environments: Vec<Entry> = config
            .selectable_environments()
            .map(|e| Entry {
                label: e.name.clone(),
                keywords: vec![t("shell.palette.keyword_environment").to_owned()],
                choice: Choice::Environment(e.id.clone()),
            })
            .collect();

        let sections = [
            (t("shell.screen.requests"), requests),
            (t("shell.screen.mocks"), mocks),
            (t("shell.screen.mappings"), mappings),
            (t("shell.screen.environments"), environments),
        ];
        let groups = sections.iter().map(|(label, entries)| choice_group(*label, entries)).collect();
        let choices = sections.iter().map(|(_, entries)| entries.iter().map(|e| e.choice.clone()).collect()).collect();

        let (state, shell) = (self.finder.clone(), cx.weak_entity());
        open_search(shell, state, t("shell.palette.search_finder"), groups, choices, window, cx);
    }

    fn apply_choice(&mut self, choice: Choice, window: &mut Window, cx: &mut Context<Self>) {
        match choice {
            Choice::Request(node) => self.open_request(&node, window, cx),
            Choice::Mock(id) => self.open_mock(&id, window, cx),
            Choice::Mapping => self.show_screen(Screen::Mappings, window, cx),
            Choice::Environment(id) => {
                self.show_screen(Screen::Environments, window, cx);
                self.environments.update(cx, |screen, cx| screen.select(id, window, cx));
            }
            Choice::UseEnvironment(id) => {
                self.model.update(cx, |model, cx| model.set_active_environment(id.as_deref(), cx))
            }
        }
    }
}
