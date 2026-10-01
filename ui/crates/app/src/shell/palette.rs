//! The command palette: every command, every saved request and every environment, one search.
//!
//! Commands are actions, so the palette shows the key each is bound to and dispatches the same action the
//! key does. Requests and environments are not commands; confirming one runs the shell method directly.

use gpui_kit::component::IndexPath;
use gpui_kit::component::{
    IconName, WindowExt as _,
    command::{Command, CommandGroup, CommandItem},
};
use gpui_kit::{Action, Context, ParentElement as _, Styled as _, Window};

use super::{Module, Shell};
use crate::{
    commands::{
        NewFolder, NewRestRequest, NewWebSocketRequest, ReloadWorkspace, RevertRequest, RunRequest, SaveRequest,
        ShowEnvironments, ShowMappings, ShowRequests, ToggleActivity, ToggleDetails, ToggleExplorer,
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

fn commands() -> Vec<(&'static str, Option<IconName>, Box<dyn Action>)> {
    vec![
        ("New request", Some(IconName::Plus), Box::new(NewRestRequest)),
        ("New WebSocket", Some(IconName::Plus), Box::new(NewWebSocketRequest)),
        ("New folder…", Some(IconName::Folder), Box::new(NewFolder)),
        ("Save request", Some(IconName::File), Box::new(SaveRequest)),
        ("Revert request", Some(IconName::Undo), Box::new(RevertRequest)),
        ("Send request or connect", Some(IconName::Play), Box::new(RunRequest)),
        ("Show requests", Some(IconName::Inbox), Box::new(ShowRequests)),
        ("Show environments", Some(IconName::Globe), Box::new(ShowEnvironments)),
        ("Show mappings", Some(IconName::Replace), Box::new(ShowMappings)),
        ("Toggle explorer", Some(IconName::PanelLeft), Box::new(ToggleExplorer)),
        ("Toggle details", Some(IconName::PanelRight), Box::new(ToggleDetails)),
        ("Toggle activity", Some(IconName::PanelBottom), Box::new(ToggleActivity)),
        ("Reload workspace", Some(IconName::RefreshCw), Box::new(ReloadWorkspace)),
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
            Entry { label: "No environment".to_owned(), keywords: vec!["environment".to_owned()] },
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
                    let item = CommandItem::new().label(label).action(action);
                    group.item(match icon {
                        Some(icon) => item.icon(icon),
                        None => item,
                    })
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
                        .placeholder("Type a command, or search requests…")
                        .group(command_group.clone())
                        .group(request_group.clone())
                        .group(environment_group.clone())
                        .on_confirm(move |ix: IndexPath, window, cx| {
                            // Section 0 is the commands, whose actions have already been dispatched.
                            let choice = match ix.section {
                                1 => request_choices.get(ix.row).cloned(),
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
                self.show_module(Module::Requests, window, cx);
                self.open_node(&node, window, cx);
                self.explorer.update(cx, |explorer, cx| explorer.reveal(&node, cx));
            }
            Choice::Environment(id) => {
                self.model.update(cx, |model, cx| model.set_active_environment(id.as_deref(), cx))
            }
        }
    }
}
