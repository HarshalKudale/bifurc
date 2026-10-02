//! The workspace's own settings: its name, where it syncs, what moves in and out, and its deletion.
//!
//! Like Settings, the screen is the kit's `Settings` component with a page per concern. Everything shown is
//! what the engine reports (`config.get` for the name and the remote, `sync.getState` for how a sync is
//! going, `export.formats` for what can be moved), and every change is an engine command.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    group_box::GroupBoxVariant,
    h_flex,
    input::{Input, InputEvent, InputState},
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div,
};
use serde_json::{Map, Value, json};

use super::audit::{now_ms, relative_time};
use crate::{
    app_model::{AppModel, ModelEvent},
    components::{
        binary::{pick_file, save_to_file},
        dialogs::{confirm, confirm_danger},
    },
    engine::{call, describe, failure, fetch_artifact, upload_and_call},
};

/// The kinds of thing the engine can export and import, in the order they are offered.
const KINDS: [(&str, &str); 7] = [
    ("workspace", "Whole workspace"),
    ("requests", "Requests"),
    ("mocks", "Mocks"),
    ("environments", "Environments"),
    ("mappings", "Mappings"),
    ("proxyRules", "Proxy rules"),
    ("websockets", "WebSockets"),
];

/// What an import does with something that is already in the workspace.
const COLLISIONS: [(&str, &str); 3] = [("keep", "Keep existing"), ("override", "Replace"), ("new", "Keep both")];

#[derive(Debug, Clone, PartialEq)]
struct Format {
    id: String,
    label: String,
    can_export: bool,
    can_import: bool,
}

/// How the workspace's remote sync is going, as `sync.getState` reports it.
#[derive(Debug, Clone, Default, PartialEq)]
struct SyncInfo {
    status: String,
    error: Option<String>,
    last_pushed: Option<i64>,
    last_pulled: Option<i64>,
    progress: Option<String>,
}

impl SyncInfo {
    fn from_value(value: &Value) -> Self {
        let text = |key: &str| value.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
        let time = |key: &str| value.get(key).and_then(Value::as_i64);
        Self {
            status: text("status").unwrap_or_else(|| "idle".to_owned()),
            error: text("error"),
            last_pushed: time("lastPushedAt"),
            last_pulled: time("lastPulledAt"),
            progress: text("progressMessage"),
        }
    }

    fn is_working(&self) -> bool {
        matches!(self.status.as_str(), "pushing" | "pulling" | "cloning")
    }
}

fn formats_from(value: &Value) -> Vec<(String, Vec<Format>)> {
    KINDS
        .iter()
        .filter_map(|(kind, _)| {
            let formats = value.get(*kind)?.as_array()?;
            let formats = formats
                .iter()
                .filter_map(|f| {
                    Some(Format {
                        id: f.get("id")?.as_str()?.to_owned(),
                        label: f.get("label").and_then(Value::as_str).unwrap_or_default().to_owned(),
                        can_export: f.get("supportsExport").and_then(Value::as_bool).unwrap_or(true),
                        can_import: f.get("supportsImport").and_then(Value::as_bool).unwrap_or(true),
                    })
                })
                .collect();
            Some(((*kind).to_owned(), formats))
        })
        .collect()
}

pub struct WorkspaceScreen {
    model: Entity<AppModel>,
    name: Entity<InputState>,
    remote: Entity<InputState>,
    branch: Entity<InputState>,
    /// The workspace the inputs were filled for, and the name they were filled with.
    shown: (String, String),
    sync: SyncInfo,
    /// A connect or disconnect is in flight.
    connecting: bool,
    formats: Vec<(String, Vec<Format>)>,
    export: (String, String),
    import: (String, String),
    collision: &'static str,
    /// An export or an import is in flight.
    transferring: Option<&'static str>,
    _subscriptions: Vec<Subscription>,
}

impl WorkspaceScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Workspace name"));
        let remote = cx.new(|cx| InputState::new(window, cx).placeholder("https://github.com/you/workspace.git"));
        let branch = cx.new(|cx| InputState::new(window, cx).placeholder("main").default_value("main"));
        let subscriptions = vec![
            cx.observe_in(&model, window, |this, _, window, cx| this.follow_workspace(window, cx)),
            cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| match event {
                ModelEvent::SyncStatus => this.refresh_sync(cx),
                ModelEvent::EngineRestarted => {
                    this.load_formats(cx);
                    this.refresh_sync(cx);
                }
                ModelEvent::WorkspaceSwitched => this.follow_workspace(window, cx),
                _ => {}
            }),
            cx.subscribe_in(&name, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.rename(cx);
                }
            }),
        ];
        let mut this = Self {
            model,
            name,
            remote,
            branch,
            shown: (String::new(), String::new()),
            sync: SyncInfo::default(),
            connecting: false,
            formats: Vec::new(),
            export: ("workspace".to_owned(), String::new()),
            import: ("workspace".to_owned(), String::new()),
            collision: "keep",
            transferring: None,
            _subscriptions: subscriptions,
        };
        this.follow_workspace(window, cx);
        this.load_formats(cx);
        this
    }

    fn report(&mut self, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(message, cx));
    }

    /// Fills the inputs when the workspace in front is not the one they were filled for, or was renamed
    /// elsewhere. What the user is typing is never replaced by a re-read of the same thing.
    fn follow_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.model.read(cx).config().active_workspace().cloned() else { return };
        let current = (workspace.id.clone(), workspace.name.clone());
        if self.shown == current {
            return;
        }
        let switched = self.shown.0 != current.0;
        self.name.update(cx, |input, cx| input.set_value(workspace.name.clone(), window, cx));
        if switched {
            self.remote.update(cx, |input, cx| input.set_value("", window, cx));
            self.branch.update(cx, |input, cx| input.set_value("main", window, cx));
            self.sync = SyncInfo::default();
        }
        self.shown = current;
        if switched {
            self.refresh_sync(cx);
        }
        cx.notify();
    }

    fn refresh_sync(&mut self, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let workspace_id = model.workspace_id().to_owned();
        if workspace_id.is_empty() {
            return;
        }
        let client = model.client().clone();
        let asked = workspace_id.clone();
        call(&client, cx, "sync.getState", json!({ "workspaceId": workspace_id }), move |this, outcome, cx| {
            // An answer for a workspace that is no longer in front describes the wrong repository.
            if let Ok(state) = outcome
                && this.model.read(cx).workspace_id() == asked
            {
                this.sync = SyncInfo::from_value(&state);
                cx.notify();
            }
        });
    }

    fn load_formats(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "export.formats", json!({}), |this, outcome, cx| {
            if let Ok(value) = outcome {
                this.formats = formats_from(&value);
                this.export.1 = this.first_format(&this.export.0, true);
                this.import.1 = this.first_format(&this.import.0, false);
                cx.notify();
            }
        });
    }

    /// The first format of `kind` that can be exported (or imported, when `exporting` is not set).
    fn first_format(&self, kind: &str, exporting: bool) -> String {
        self.formats_of(kind, exporting).first().map(|f| f.id.clone()).unwrap_or_default()
    }

    fn formats_of(&self, kind: &str, exporting: bool) -> Vec<&Format> {
        self.formats
            .iter()
            .find(|(k, _)| k == kind)
            .map(|(_, formats)| {
                formats.iter().filter(|f| if exporting { f.can_export } else { f.can_import }).collect()
            })
            .unwrap_or_default()
    }

    fn rename(&mut self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_owned();
        let model = self.model.read(cx);
        let Some(workspace) = model.config().active_workspace() else { return };
        if name.is_empty() || name == workspace.name {
            return;
        }
        let id = workspace.id.clone();
        self.model.update(cx, |model, cx| model.rename_workspace(&id, &name, cx));
    }

    fn ask_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let remote = self.remote.read(cx).value().trim().to_owned();
        if remote.is_empty() {
            return self.report("Enter the remote repository's URL first.".to_owned(), cx);
        }
        let has_data = {
            let config = self.model.read(cx).config();
            !(config.requests.is_empty()
                && config.mocks.is_empty()
                && config.mappings.is_empty()
                && config.proxy_rule_count == 0
                && config.selectable_environments().next().is_none())
        };
        let consequence = if has_data {
            "Your local data will be pushed to the remote. The remote must be empty."
        } else {
            "The remote repository will be cloned into this workspace."
        };
        confirm(cx.weak_entity(), window, cx, "Connect to this remote?", consequence, "Connect", |this, _, cx| {
            this.connect(cx)
        });
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        if self.connecting {
            return;
        }
        let remote = self.remote.read(cx).value().trim().to_owned();
        let branch = match self.branch.read(cx).value().trim() {
            "" => "main".to_owned(),
            branch => branch.to_owned(),
        };
        self.connecting = true;
        cx.notify();
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "remote": remote, "branch": branch });
        let current = model.workspace_id().to_owned();
        call(&client, cx, "sync.setRemote", payload, move |this, outcome, cx| {
            this.connecting = false;
            match failure(&outcome) {
                Some(message) => this.report(format!("Couldn't connect. {message}"), cx),
                None => {
                    // Cloning a remote that already holds a workspace adopts that workspace's identity.
                    let adopted = outcome
                        .ok()
                        .and_then(|result| result.get("adoptedId").and_then(Value::as_str).map(str::to_owned))
                        .filter(|id| *id != current);
                    match adopted {
                        Some(id) => this.model.update(cx, |model, cx| model.set_active_workspace(&id, cx)),
                        None => this.model.update(cx, |model, cx| model.refresh(cx)),
                    }
                }
            }
            this.refresh_sync(cx);
            cx.notify();
        });
    }

    fn ask_disconnect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            "Disconnect from the remote?",
            "Your data on this computer is kept. It just stops syncing.",
            "Disconnect",
            |this, _, cx| this.disconnect(cx),
        );
    }

    fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.run_sync("sync.disconnect", "Couldn't disconnect.", cx);
    }

    /// A sync command that takes only the workspace. Progress arrives as `event.sync.status`.
    fn run_sync(&mut self, action: &'static str, what: &'static str, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id() });
        call(&client, cx, action, payload, move |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.report(format!("{what} {message}"), cx);
            }
            this.model.update(cx, |model, cx| model.refresh(cx));
            this.refresh_sync(cx);
        });
    }

    fn set_auto_sync(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "enabled": enabled });
        call(&client, cx, "sync.setAutoSync", payload, |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.report(format!("Couldn't change auto sync. {message}"), cx);
            }
            this.model.update(cx, |model, cx| model.refresh(cx));
        });
    }

    fn export_now(&mut self, cx: &mut Context<Self>) {
        let (kind, format) = self.export.clone();
        if self.transferring.is_some() || format.is_empty() {
            return;
        }
        self.transferring = Some("Exporting");
        cx.notify();
        let model = self.model.read(cx);
        let client = model.client().clone();
        let workspace = model.workspace_id().to_owned();
        let payload = json!({ "kind": kind, "format": format, "workspaceId": workspace });
        call(&client.clone(), cx, "export.create", payload, move |this, outcome, cx| match outcome {
            Ok(artifact) => fetch_artifact(this, &client, cx, &workspace, &artifact, |this, fetched, cx| {
                this.finish_export(fetched, cx)
            }),
            Err(error) => this.finish_export(Err(describe(&error)), cx),
        });
    }

    fn finish_export(&mut self, fetched: Result<(Vec<u8>, String), String>, cx: &mut Context<Self>) {
        self.transferring = None;
        cx.notify();
        match fetched {
            Ok((bytes, name)) => save_to_file(bytes, name, cx, |this, outcome, cx| match outcome {
                Ok(path) => this.report(format!("Exported to {}.", path.display()), cx),
                Err(message) => this.report(format!("Couldn't save the export. {message}"), cx),
            }),
            Err(message) => self.report(format!("Couldn't export. {message}"), cx),
        }
    }

    fn import_now(&mut self, cx: &mut Context<Self>) {
        let (kind, format) = self.import.clone();
        if self.transferring.is_some() || format.is_empty() {
            return;
        }
        let collision = self.collision;
        pick_file("Choose the file to import", cx, move |this, picked, cx| {
            let file = match picked {
                Ok(file) => file,
                Err(error) => return this.report(format!("Couldn't read the file. {error}"), cx),
            };
            this.transferring = Some("Importing");
            cx.notify();
            let model = this.model.read(cx);
            let client = model.client().clone();
            let workspace = model.workspace_id().to_owned();
            let mut extra = Map::new();
            extra.insert("kind".to_owned(), json!(kind));
            extra.insert("format".to_owned(), json!(format));
            extra.insert("workspaceId".to_owned(), json!(workspace));
            extra.insert("collisionStrategy".to_owned(), json!(collision));
            upload_and_call(
                &client,
                cx,
                Some(&workspace),
                &file.name,
                &file.bytes,
                "import.commit",
                extra,
                |this, outcome, cx| {
                    this.transferring = None;
                    match outcome {
                        Ok(result) => {
                            let imported = result.get("imported").and_then(Value::as_u64).unwrap_or(0);
                            let skipped = result.get("skipped").and_then(Value::as_u64).unwrap_or(0);
                            let skipped = if skipped > 0 { format!(", {skipped} skipped") } else { String::new() };
                            this.report(format!("Imported {imported} item(s){skipped}."), cx);
                            this.model.update(cx, |model, cx| model.refresh(cx));
                        }
                        Err(message) => this.report(format!("Couldn't import. {message}"), cx),
                    }
                    cx.notify();
                },
            );
        });
    }

    fn ask_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn general_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let this = cx.weak_entity();
        let name = self.name.clone();
        let model = self.model.clone();
        SettingPage::new("General").icon(IconName::Building2).default_open(true).groups(vec![
            SettingGroup::new().title("Workspace").item(
                SettingItem::render(move |options, _, cx| {
                    let id = model.read(cx).workspace_id().to_owned();
                    let theme = cx.theme().clone();
                    let rename = this.clone();
                    v_flex()
                        .w_full()
                        .gap_1p5()
                        .child(h_flex().w_full().gap_2().child(div().flex_1().child(Input::new(&name))).child(
                            Button::new("rename-workspace").with_size(options.size()).label("Rename").on_click(
                                move |_, _, cx| {
                                    rename.update(cx, |this, cx| this.rename(cx)).ok();
                                },
                            ),
                        ))
                        .child(div().text_xs().text_color(theme.muted_foreground).child(format!("Id: {id}")))
                })
                .keywords(["name", "rename", "title"]),
            ),
        ])
    }

    fn sync_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let this = cx.weak_entity();
        let sync = self.sync.clone();
        let connecting = self.connecting;
        let (remote_input, branch_input) = (self.remote.clone(), self.branch.clone());
        let model = self.model.clone();
        let auto_read = self.model.clone();
        let auto_write = this.clone();

        let connected = self.model.read(cx).config().active_workspace().and_then(|w| w.sync.clone());
        let mut groups = Vec::new();

        let Some(config) = connected else {
            groups.push(
                SettingGroup::new()
                    .title("Remote")
                    .item(SettingItem::render(move |options, _, cx| {
                        let theme = cx.theme().clone();
                        let connect = this.clone();
                        v_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().text_sm().text_color(theme.muted_foreground).child("Repository URL"))
                                    .child(Input::new(&remote_input)),
                            )
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().text_sm().text_color(theme.muted_foreground).child("Branch"))
                                    .child(Input::new(&branch_input)),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .justify_between()
                                    .items_center()
                                    .child(div().text_sm().text_color(theme.muted_foreground).child("Not connected"))
                                    .child(
                                        Button::new("connect-remote")
                                            .with_size(options.size())
                                            .primary()
                                            .label("Connect")
                                            .loading(connecting)
                                            .disabled(connecting)
                                            .on_click(move |_, window, cx| {
                                                connect.update(cx, |this, cx| this.ask_connect(window, cx)).ok();
                                            }),
                                    ),
                            )
                    }))
                    .footer(|_, _| "A git repository the workspace's files are pushed to and pulled from."),
            );
            return SettingPage::new("Git sync").icon(IconName::Github).groups(groups);
        };

        let status_label = match sync.status.as_str() {
            "pushing" => "Pushing…",
            "pulling" => "Pulling…",
            "cloning" => "Cloning…",
            "error" => "Error",
            _ => "Synced",
        };
        let working = sync.is_working();
        let (pull, push, disconnect) = (this.clone(), this.clone(), this.clone());
        groups.push(
            SettingGroup::new()
                .title("Remote")
                .item(
                    SettingItem::new(
                        "Repository",
                        SettingField::render({
                            let remote = config.remote.clone();
                            move |_, _, cx| {
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .child(remote.clone())
                            }
                        }),
                    )
                    .description("It can't be changed while connected."),
                )
                .item(SettingItem::new(
                    "Branch",
                    SettingField::render({
                        let branch = config.branch.clone();
                        move |_, _, cx| div().text_color(cx.theme().muted_foreground).child(branch.clone())
                    }),
                ))
                .item(
                    SettingItem::new(
                        "Auto sync",
                        SettingField::switch(
                            move |cx: &App| {
                                auto_read
                                    .read(cx)
                                    .config()
                                    .active_workspace()
                                    .and_then(|w| w.sync.as_ref())
                                    .is_some_and(|s| s.auto_sync)
                            },
                            move |value: bool, cx: &mut App| {
                                auto_write.update(cx, |this, cx| this.set_auto_sync(value, cx)).ok();
                            },
                        ),
                    )
                    .description("Push after changes and pull every 30 seconds."),
                ),
        );
        let _ = model;
        groups.push(
            SettingGroup::new()
                .title("Status")
                .item(SettingItem::render(move |options, _, cx| {
                    let theme = cx.theme().clone();
                    let (pull, push) = (pull.clone(), push.clone());
                    let now = now_ms();
                    let when = |ts: Option<i64>| ts.map_or_else(|| "never".to_owned(), |ts| relative_time(now, ts));
                    let state = match sync.status.as_str() {
                        "error" => Tag::danger().child(status_label),
                        _ if working => {
                            Tag::warning().child(sync.progress.clone().unwrap_or_else(|| status_label.to_owned()))
                        }
                        _ => Tag::success().child(status_label),
                    };
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .child(div().flex_1().child(state))
                                .child(
                                    Button::new("pull-remote")
                                        .with_size(options.size())
                                        .icon(IconName::ArrowDown)
                                        .label("Pull")
                                        .disabled(working)
                                        .on_click(move |_, _, cx| {
                                            pull.update(cx, |this, cx| {
                                                this.run_sync("sync.pull", "Couldn't pull.", cx)
                                            })
                                            .ok();
                                        }),
                                )
                                .child(
                                    Button::new("push-remote")
                                        .with_size(options.size())
                                        .icon(IconName::ArrowUp)
                                        .label("Push")
                                        .disabled(working)
                                        .on_click(move |_, _, cx| {
                                            push.update(cx, |this, cx| {
                                                this.run_sync("sync.push", "Couldn't push.", cx)
                                            })
                                            .ok();
                                        }),
                                ),
                        )
                        .child(div().text_xs().text_color(theme.muted_foreground).child(format!(
                            "Last pushed {} · last pulled {}",
                            when(sync.last_pushed),
                            when(sync.last_pulled)
                        )))
                        .children(sync.error.clone().map(|error| div().text_xs().text_color(theme.danger).child(error)))
                }))
                .item(SettingItem::render(move |options, _, _| {
                    let disconnect = disconnect.clone();
                    h_flex().w_full().justify_end().child(
                        Button::new("disconnect-remote")
                            .with_size(options.size())
                            .outline()
                            .label("Disconnect")
                            .loading(connecting)
                            .on_click(move |_, window, cx| {
                                disconnect.update(cx, |this, cx| this.ask_disconnect(window, cx)).ok();
                            }),
                    )
                })),
        );
        SettingPage::new("Git sync").icon(IconName::Github).groups(groups)
    }

    /// A dropdown of kinds, and one of the formats the chosen kind has.
    fn transfer_fields(&self, exporting: bool, cx: &mut Context<Self>) -> Vec<SettingItem> {
        let (kind, format) = if exporting { self.export.clone() } else { self.import.clone() };
        let this = cx.weak_entity();
        let kinds: Vec<(SharedString, SharedString)> = KINDS
            .iter()
            .filter(|(k, _)| !self.formats_of(k, exporting).is_empty())
            .map(|(k, label)| (SharedString::from(*k), SharedString::from(*label)))
            .collect();
        let formats: Vec<(SharedString, SharedString)> = self
            .formats_of(&kind, exporting)
            .into_iter()
            .map(|f| (SharedString::from(f.id.clone()), SharedString::from(f.label.clone())))
            .collect();

        let (kind_for_read, kind_for_write) = (kind.clone(), this.clone());
        let (format_for_read, format_for_write) = (format.clone(), this);
        vec![
            SettingItem::new(
                "What",
                SettingField::dropdown(
                    kinds,
                    move |_: &App| SharedString::from(kind_for_read.clone()),
                    move |value: SharedString, cx: &mut App| {
                        kind_for_write
                            .update(cx, |this, cx| {
                                let pair = (value.to_string(), this.first_format(&value, exporting));
                                if exporting {
                                    this.export = pair;
                                } else {
                                    this.import = pair;
                                }
                                cx.notify();
                            })
                            .ok();
                    },
                ),
            ),
            SettingItem::new(
                "Format",
                SettingField::dropdown(
                    formats,
                    move |_: &App| SharedString::from(format_for_read.clone()),
                    move |value: SharedString, cx: &mut App| {
                        format_for_write
                            .update(cx, |this, cx| {
                                if exporting {
                                    this.export.1 = value.to_string();
                                } else {
                                    this.import.1 = value.to_string();
                                }
                                cx.notify();
                            })
                            .ok();
                    },
                ),
            ),
        ]
    }

    fn transfer_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let transferring = self.transferring;
        let (export, import) = (cx.weak_entity(), cx.weak_entity());
        let collision_write = cx.weak_entity();
        let collision_now = self.collision;
        let collisions: Vec<(SharedString, SharedString)> =
            COLLISIONS.iter().map(|(id, label)| (SharedString::from(*id), SharedString::from(*label))).collect();

        let export_group = SettingGroup::new()
            .title("Export")
            .items(self.transfer_fields(true, cx))
            .item(SettingItem::render(move |options, _, _| {
                let export = export.clone();
                h_flex().w_full().justify_end().child(
                    Button::new("export-workspace")
                        .with_size(options.size())
                        .primary()
                        .icon(IconName::ArrowUp)
                        .label("Export…")
                        .loading(transferring == Some("Exporting"))
                        .disabled(transferring.is_some())
                        .on_click(move |_, _, cx| {
                            export.update(cx, |this, cx| this.export_now(cx)).ok();
                        }),
                )
            }))
            .footer(|_, _| "Writes a file you choose the place of. Nothing in the workspace changes.");
        let import_group = SettingGroup::new()
            .title("Import")
            .items(self.transfer_fields(false, cx))
            .item(
                SettingItem::new(
                    "When it already exists",
                    SettingField::dropdown(
                        collisions,
                        move |_: &App| SharedString::from(collision_now),
                        move |value: SharedString, cx: &mut App| {
                            collision_write
                                .update(cx, |this, cx| {
                                    this.collision =
                                        COLLISIONS.iter().map(|(id, _)| *id).find(|id| value == *id).unwrap_or("keep");
                                    cx.notify();
                                })
                                .ok();
                        },
                    ),
                )
                .description("What to do with an item whose id is already in the workspace."),
            )
            .item(SettingItem::render(move |options, _, _| {
                let import = import.clone();
                h_flex().w_full().justify_end().child(
                    Button::new("import-workspace")
                        .with_size(options.size())
                        .primary()
                        .icon(IconName::ArrowDown)
                        .label("Import…")
                        .loading(transferring == Some("Importing"))
                        .disabled(transferring.is_some())
                        .on_click(move |_, _, cx| {
                            import.update(cx, |this, cx| this.import_now(cx)).ok();
                        }),
                )
            }))
            .footer(|_, _| "Imported items are added to this workspace.");
        SettingPage::new("Import & export").icon(IconName::Inbox).groups(vec![export_group, import_group])
    }

    fn danger_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let this = cx.weak_entity();
        let can_delete = self.model.read(cx).config().workspaces.len() > 1;
        SettingPage::new("Danger zone").icon(IconName::TriangleAlert).groups(vec![
            SettingGroup::new()
                .title("Delete workspace")
                .item(SettingItem::render(move |options, _, cx| {
                    let theme = cx.theme().clone();
                    let delete = this.clone();
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .text_color(theme.muted_foreground)
                                .child("Removes the workspace and everything in it from this computer."),
                        )
                        .child(
                            Button::new("delete-workspace")
                                .with_size(options.size())
                                .danger()
                                .label("Delete…")
                                .disabled(!can_delete)
                                .on_click(move |_, window, cx| {
                                    delete.update(cx, |this, cx| this.ask_delete(window, cx)).ok();
                                }),
                        )
                }))
                .footer(|_, _| "The last workspace can't be deleted. Remote data, if connected, is not touched."),
        ])
    }
}

impl Render for WorkspaceScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pages = vec![self.general_page(cx), self.sync_page(cx), self.transfer_page(cx), self.danger_page(cx)];
        div()
            .size_full()
            .bg(cx.theme().background)
            .child(Settings::new("workspace-config").with_group_variant(GroupBoxVariant::Outline).pages(pages))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_listed_in_the_order_of_the_kinds() {
        let value = json!({
            "mocks": [{ "id": "json", "label": "JSON", "supportsExport": true, "supportsImport": false }],
            "workspace": [{ "id": "zip", "label": "Zip" }],
            "unknown": [{ "id": "x" }],
        });
        let formats = formats_from(&value);
        assert_eq!(formats.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["workspace", "mocks"]);
        assert!(formats[0].1[0].can_export && formats[0].1[0].can_import);
        assert!(formats[1].1[0].can_export && !formats[1].1[0].can_import);
    }

    #[test]
    fn a_sync_state_reads_what_the_engine_sends() {
        let info = SyncInfo::from_value(&json!({
            "status": "pulling", "error": null, "lastPushedAt": 5, "lastPulledAt": null, "progressMessage": "Fetching",
        }));
        assert!(info.is_working());
        assert_eq!((info.last_pushed, info.last_pulled), (Some(5), None));
        assert_eq!(info.progress.as_deref(), Some("Fetching"));
        assert!(!SyncInfo::from_value(&json!({})).is_working());
    }
}
