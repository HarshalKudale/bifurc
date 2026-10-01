//! The command palette: every saved request, every mock, every command and every environment, one search.
//!
//! It is what the title bar's search opens. Commands are actions, so the palette shows the key each is bound
//! to and dispatches the same action the key does. Requests, mocks and environments are not commands;
//! confirming one runs the shell method directly.

use gpui_kit::component::IndexPath;
use gpui_kit::component::{
    IconName, WindowExt as _,
    command::{Command, CommandGroup, CommandItem},
};
use gpui_kit::{Action, Context, ParentElement as _, Styled as _, Window};

use super::{Screen, Shell};
use crate::{
    commands::{
        CheckAllHealth, ClearCapture, CloseRequestTab, DeleteEntity, NewEnvironment, NewFolder, NewGraphQlFolder,
        NewGraphQlRequest, NewGrpcFolder, NewGrpcRequest, NewHealthCheck, NewMapping, NewMock, NewMockFolder,
        NewProxyRule, NewRestRequest, NewSoapFolder, NewSoapRequest, NewWebSocketFolder, NewWebSocketRequest,
        NewWorkspace, PublishChanges, RefreshServices, ReloadWorkspace, RenameWorkspace, RestartForwardProxy,
        RevertRequest, RunRequest, SaveRequest, ShowCapture, ShowEnvironments, ShowHealthBar, ShowMappings, ShowMocks,
        ShowProxyRules, ShowRequests, ShowServices, ShowSettings, SyncEntity, ToggleCapturePaused, ToggleFolderTree,
        ToggleForwardProxy, ToggleInspector, ToggleSidebar,
    },
    model::NodeRef,
};

/// What confirming a palette row does, for the rows that are not an action.
#[derive(Clone)]
enum Choice {
    Request(NodeRef),
    Mock(String),
    Environment(Option<String>),
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
            "Requests",
            vec![
                ("New REST request", IconName::Plus, Box::new(NewRestRequest)),
                ("New GraphQL request", IconName::Plus, Box::new(NewGraphQlRequest)),
                ("New gRPC request", IconName::Plus, Box::new(NewGrpcRequest)),
                ("New SOAP request", IconName::Plus, Box::new(NewSoapRequest)),
                ("New WebSocket request", IconName::Plus, Box::new(NewWebSocketRequest)),
                ("New REST folder…", IconName::Folder, Box::new(NewFolder)),
                ("New GraphQL folder…", IconName::Folder, Box::new(NewGraphQlFolder)),
                ("New gRPC folder…", IconName::Folder, Box::new(NewGrpcFolder)),
                ("New SOAP folder…", IconName::Folder, Box::new(NewSoapFolder)),
                ("New WebSocket folder…", IconName::Folder, Box::new(NewWebSocketFolder)),
                ("Send request or connect", IconName::Play, Box::new(RunRequest)),
            ],
        ),
        (
            "Mocks",
            vec![
                ("New mock", IconName::Plus, Box::new(NewMock)),
                ("New mock folder…", IconName::Folder, Box::new(NewMockFolder)),
            ],
        ),
        (
            "Proxy",
            vec![
                ("New mapping…", IconName::Plus, Box::new(NewMapping)),
                ("New proxy rule", IconName::Plus, Box::new(NewProxyRule)),
                ("Pause or resume capture", IconName::Pause, Box::new(ToggleCapturePaused)),
                ("Clear captured traffic", IconName::Delete, Box::new(ClearCapture)),
                ("Start or stop the forward proxy", IconName::Play, Box::new(ToggleForwardProxy)),
                ("Restart the forward proxy", IconName::RefreshCw, Box::new(RestartForwardProxy)),
            ],
        ),
        (
            "Services",
            vec![
                ("Look for local services again", IconName::RefreshCw, Box::new(RefreshServices)),
                ("New health check…", IconName::Plus, Box::new(NewHealthCheck)),
                ("Check every URL now", IconName::Heart, Box::new(CheckAllHealth)),
            ],
        ),
        (
            "Open item",
            vec![
                ("Save", IconName::File, Box::new(SaveRequest)),
                ("Sync this item", IconName::Github, Box::new(SyncEntity)),
                ("Revert to the last synced version…", IconName::Undo, Box::new(RevertRequest)),
                ("Delete this item…", IconName::Delete, Box::new(DeleteEntity)),
                ("Close tab", IconName::Close, Box::new(CloseRequestTab)),
            ],
        ),
        (
            "Workspace",
            vec![
                ("New environment…", IconName::Plus, Box::new(NewEnvironment)),
                ("New workspace…", IconName::Plus, Box::new(NewWorkspace)),
                ("Rename this workspace…", IconName::File, Box::new(RenameWorkspace)),
                ("Publish this screen's changes", IconName::Github, Box::new(PublishChanges)),
                ("Reload workspace", IconName::RefreshCw, Box::new(ReloadWorkspace)),
            ],
        ),
        (
            "Go to",
            vec![
                ("Go to Services", Screen::Services.icon(), Box::new(ShowServices)),
                ("Go to Health bar", Screen::HealthBar.icon(), Box::new(ShowHealthBar)),
                ("Go to Mappings", Screen::Mappings.icon(), Box::new(ShowMappings)),
                ("Go to Proxy rules", Screen::ProxyRules.icon(), Box::new(ShowProxyRules)),
                ("Go to Capture", Screen::Capture.icon(), Box::new(ShowCapture)),
                ("Go to Requests", Screen::Requests.icon(), Box::new(ShowRequests)),
                ("Go to Mocks", Screen::Mocks.icon(), Box::new(ShowMocks)),
                ("Go to Environments", Screen::Environments.icon(), Box::new(ShowEnvironments)),
                ("Go to Settings", Screen::Settings.icon(), Box::new(ShowSettings)),
            ],
        ),
        (
            "View",
            vec![
                ("Toggle sidebar", IconName::PanelLeft, Box::new(ToggleSidebar)),
                ("Toggle inspector", IconName::PanelRight, Box::new(ToggleInspector)),
                ("Toggle folder tree", IconName::FolderOpen, Box::new(ToggleFolderTree)),
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

impl Shell {
    pub fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let state = self.palette.clone();
        state.update(cx, |state, cx| state.set_query("", window, cx));

        let config = self.model.read(cx).config().clone();
        let requests: Vec<Entry> = config
            .requests
            .iter()
            .map(|r| {
                let label = if r.name.is_empty() { r.target.clone() } else { r.name.clone() };
                Entry {
                    label: format!("{}  {label}", r.method.clone().unwrap_or_else(|| r.kind.badge().to_owned())),
                    keywords: vec![r.target.clone(), "request".to_owned()],
                    choice: Choice::Request(NodeRef::Request { kind: r.kind, id: r.id.clone() }),
                }
            })
            .collect();
        let mocks: Vec<Entry> = config
            .mocks
            .iter()
            .map(|m| {
                let label = if m.name.is_empty() { m.url_pattern.clone() } else { m.name.clone() };
                let method = if m.method == "*" { "ANY" } else { m.method.as_str() };
                Entry {
                    label: format!("{method}  {label}"),
                    keywords: vec![m.url_pattern.clone(), "mock".to_owned()],
                    choice: Choice::Mock(m.id.clone()),
                }
            })
            .collect();
        let environments: Vec<Entry> = std::iter::once(Entry {
            label: "Use no environment".to_owned(),
            keywords: vec!["environment".to_owned()],
            choice: Choice::Environment(None),
        })
        .chain(config.selectable_environments().map(|e| Entry {
            label: format!("Use environment: {}", e.name),
            keywords: vec!["environment".to_owned()],
            choice: Choice::Environment(Some(e.id.clone())),
        }))
        .collect();

        let shell = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let state = state.clone();
            let shell = shell.clone();

            // Things to open come first, then the commands, then the environments. The sections that are
            // not commands are remembered by position, which is how a confirmed row is found again.
            let mut groups = vec![choice_group("Open a request", &requests), choice_group("Open a mock", &mocks)];
            let mut choices: Vec<Vec<Choice>> = vec![
                requests.iter().map(|entry| entry.choice.clone()).collect(),
                mocks.iter().map(|entry| entry.choice.clone()).collect(),
            ];
            for (label, rows) in command_groups() {
                groups.push(rows.into_iter().fold(CommandGroup::new().label(label), |group, (label, icon, action)| {
                    group.item(CommandItem::new().label(label).icon(icon).action(action))
                }));
                // A command's action has already been dispatched when its row is confirmed.
                choices.push(Vec::new());
            }
            groups.push(choice_group("Environments", &environments));
            choices.push(environments.iter().map(|entry| entry.choice.clone()).collect());

            dialog.close_button(false).p_0().w_128().content(move |content, _, _| {
                let shell = shell.clone();
                let choices = choices.clone();
                let command = groups.iter().fold(
                    Command::new(&state).bordered(false).placeholder("Search requests, mocks and commands…"),
                    |command, group| command.group(group.clone()),
                );
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
        let state = self.palette.clone();
        window.defer(cx, move |window, cx| state.update(cx, |state, cx| state.focus(window, cx)));
    }

    fn apply_choice(&mut self, choice: Choice, window: &mut Window, cx: &mut Context<Self>) {
        match choice {
            Choice::Request(node) => self.open_request(&node, window, cx),
            Choice::Mock(id) => self.open_mock(&id, window, cx),
            Choice::Environment(id) => {
                self.model.update(cx, |model, cx| model.set_active_environment(id.as_deref(), cx))
            }
        }
    }
}
