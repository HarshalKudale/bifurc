//! The command palette: every command, every saved request and every environment, one search.
//!
//! It is what the title bar's search opens. Commands are actions, so the palette shows the key each is bound
//! to and dispatches the same action the key does. Requests and environments are not commands; confirming
//! one runs the shell method directly.

use gpui_kit::component::IndexPath;
use gpui_kit::component::{
    IconName, WindowExt as _,
    command::{Command, CommandGroup, CommandItem},
};
use gpui_kit::{Action, Context, ParentElement as _, Styled as _, Window};

use super::{Screen, Shell};
use crate::{
    commands::{
        NewFolder, NewGraphQlRequest, NewGrpcRequest, NewRestRequest, NewSoapRequest, NewWebSocketRequest,
        ReloadWorkspace, RevertRequest, RunRequest, SaveRequest, ShowCapture, ShowEnvironments, ShowHealthBar,
        ShowMappings, ShowMocks, ShowProxyRules, ShowRequests, ShowServices, ShowSettings, ToggleFolderTree,
        ToggleInspector, ToggleSidebar,
    },
    model::NodeRef,
};

/// What confirming a palette row does, for the rows that are not an action.
#[derive(Clone)]
enum Choice {
    Open(NodeRef),
    Environment(Option<String>),
}

struct Entry {
    label: String,
    keywords: Vec<String>,
}

fn commands() -> Vec<(&'static str, IconName, Box<dyn Action>)> {
    vec![
        ("New REST request", IconName::Plus, Box::new(NewRestRequest)),
        ("New GraphQL request", IconName::Plus, Box::new(NewGraphQlRequest)),
        ("New gRPC request", IconName::Plus, Box::new(NewGrpcRequest)),
        ("New SOAP request", IconName::Plus, Box::new(NewSoapRequest)),
        ("New WebSocket", IconName::Plus, Box::new(NewWebSocketRequest)),
        ("New folder…", IconName::Folder, Box::new(NewFolder)),
        ("Save", IconName::File, Box::new(SaveRequest)),
        ("Revert changes", IconName::Undo, Box::new(RevertRequest)),
        ("Send request or connect", IconName::Play, Box::new(RunRequest)),
        ("Go to Services", Screen::Services.icon(), Box::new(ShowServices)),
        ("Go to Health bar", Screen::HealthBar.icon(), Box::new(ShowHealthBar)),
        ("Go to Mappings", Screen::Mappings.icon(), Box::new(ShowMappings)),
        ("Go to Proxy rules", Screen::ProxyRules.icon(), Box::new(ShowProxyRules)),
        ("Go to Capture", Screen::Capture.icon(), Box::new(ShowCapture)),
        ("Go to Requests", Screen::Requests.icon(), Box::new(ShowRequests)),
        ("Go to Mocks", Screen::Mocks.icon(), Box::new(ShowMocks)),
        ("Go to Environments", Screen::Environments.icon(), Box::new(ShowEnvironments)),
        ("Go to Settings", Screen::Settings.icon(), Box::new(ShowSettings)),
        ("Toggle sidebar", IconName::PanelLeft, Box::new(ToggleSidebar)),
        ("Toggle inspector", IconName::PanelRight, Box::new(ToggleInspector)),
        ("Toggle folder tree", IconName::FolderOpen, Box::new(ToggleFolderTree)),
        ("Reload workspace", IconName::RefreshCw, Box::new(ReloadWorkspace)),
    ]
}

impl Shell {
    pub fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let state = self.palette.clone();
        state.update(cx, |state, cx| state.set_query("", window, cx));

        let config = self.model.read(cx).config().clone();
        let requests: Vec<(Entry, Choice)> = config
            .requests
            .iter()
            .map(|r| {
                let label = if r.name.is_empty() { r.target.clone() } else { r.name.clone() };
                (
                    Entry {
                        label: format!("{}  {label}", r.method.clone().unwrap_or_else(|| r.kind.badge().to_owned())),
                        keywords: vec![r.target.clone()],
                    },
                    Choice::Open(NodeRef::Request { kind: r.kind, id: r.id.clone() }),
                )
            })
            .collect();
        let environments: Vec<(Entry, Choice)> = std::iter::once((
            Entry { label: "Use no environment".to_owned(), keywords: vec!["environment".to_owned()] },
            Choice::Environment(None),
        ))
        .chain(config.selectable_environments().map(|e| {
            (
                Entry { label: format!("Use environment: {}", e.name), keywords: vec!["environment".to_owned()] },
                Choice::Environment(Some(e.id.clone())),
            )
        }))
        .collect();

        let shell = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let state = state.clone();
            let shell = shell.clone();
            let request_choices: Vec<Choice> = requests.iter().map(|(_, c)| c.clone()).collect();
            let environment_choices: Vec<Choice> = environments.iter().map(|(_, c)| c.clone()).collect();

            let command_group =
                commands().into_iter().fold(CommandGroup::new().label("Commands"), |group, (label, icon, action)| {
                    group.item(CommandItem::new().label(label).icon(icon).action(action))
                });
            let request_group = requests.iter().fold(CommandGroup::new().label("Requests"), |group, (entry, _)| {
                group.item(CommandItem::new().label(entry.label.clone()).keywords(entry.keywords.clone()))
            });
            let environment_group =
                environments.iter().fold(CommandGroup::new().label("Environments"), |group, (entry, _)| {
                    group.item(CommandItem::new().label(entry.label.clone()).keywords(entry.keywords.clone()))
                });

            dialog.close_button(false).p_0().w_128().content(move |content, _, _| {
                let shell = shell.clone();
                let request_choices = request_choices.clone();
                let environment_choices = environment_choices.clone();
                content.child(
                    Command::new(&state)
                        .bordered(false)
                        .placeholder("Search requests and commands…")
                        .group(request_group.clone())
                        .group(command_group.clone())
                        .group(environment_group.clone())
                        .on_confirm(move |ix: IndexPath, window, cx| {
                            // Section 1 is the commands, whose actions have already been dispatched.
                            let choice = match ix.section {
                                0 => request_choices.get(ix.row).cloned(),
                                2 => environment_choices.get(ix.row).cloned(),
                                _ => None,
                            };
                            window.close_dialog(cx);
                            if let Some(choice) = choice {
                                shell.update(cx, |shell, cx| shell.apply_choice(choice, window, cx)).ok();
                            }
                        }),
                )
            })
        });
        // The query field is where typing starts.
        let state = self.palette.clone();
        window.defer(cx, move |window, cx| state.update(cx, |state, cx| state.focus(window, cx)));
    }

    fn apply_choice(&mut self, choice: Choice, window: &mut Window, cx: &mut Context<Self>) {
        match choice {
            Choice::Open(node) => {
                self.show_screen(Screen::Requests, window, cx);
                self.requests.update(cx, |requests, cx| {
                    requests.open_node(&node, window, cx);
                    requests.reveal(&node, cx);
                });
            }
            Choice::Environment(id) => {
                self.model.update(cx, |model, cx| model.set_active_environment(id.as_deref(), cx))
            }
        }
    }
}
