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
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div,
};
use serde_json::{Map, Value, json};

use super::{
    audit::{now_ms, relative_time},
    workspace_setup::{Purpose, open_setup_dialog},
};
use crate::{
    app_model::{AppModel, ModelEvent},
    components::{
        binary::{pick_file, save_to_file},
        dialogs::confirm_danger,
    },
    engine::{call, describe, failure, fetch_artifact, upload_and_call},
    i18n::{t, t_with},
    model::git::FileState,
};

/// The kinds of thing the engine can export and import, in the order they are offered.
const KINDS: [(&str, &str); 6] = [
    ("workspace", "workspace.kind.workspace"),
    ("requests", "workspace.kind.requests"),
    ("mocks", "workspace.kind.mocks"),
    ("environments", "workspace.kind.environments"),
    ("mappings", "workspace.kind.mappings"),
    ("proxyRules", "workspace.kind.proxy_rules"),
];

/// What an import does with something that is already in the workspace.
const COLLISIONS: [(&str, &str); 3] = [
    ("keep", "workspace.collision.keep"),
    ("override", "workspace.collision.replace"),
    ("new", "workspace.collision.keep_both"),
];

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
    /// The login of the account that owns the workspace: its repository's owner once it has one.
    owner: Option<String>,
    /// What the workspace's account may do with the repository: `owner`, `write` or `read`.
    access: Option<String>,
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
            owner: value.pointer("/owner/login").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned),
            access: text("access"),
        }
    }

    fn is_working(&self) -> bool {
        matches!(self.status.as_str(), "pushing" | "pulling" | "cloning" | "syncing")
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
    /// The workspace the inputs were filled for, and the name they were filled with.
    shown: (String, String),
    /// The account and remote the sync state was read for, with the access to the remote; a publish, a
    /// sign-in or a change of access on GitHub changes them.
    link: (Option<String>, Option<String>),
    sync: SyncInfo,
    formats: Vec<(String, Vec<Format>)>,
    export: (String, String),
    import: (String, String),
    collision: &'static str,
    /// An export or an import is in flight.
    transferring: Option<&'static str>,
    /// The workspace's cookie jar, as `cookies.list` answers.
    cookies: Value,
    _subscriptions: Vec<Subscription>,
}

impl WorkspaceScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t("workspace.general.name_placeholder")));
        let subscriptions = vec![
            cx.observe_in(&model, window, |this, _, window, cx| {
                this.follow_workspace(window, cx);
                // The count of what is not synced is the model's, and changes without the workspace changing.
                cx.notify();
            }),
            cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| match event {
                ModelEvent::SyncStatus | ModelEvent::AccountChanged(_) => this.refresh_sync(cx),
                ModelEvent::CookiesChanged => this.refresh_cookies(cx),
                ModelEvent::EngineRestarted => {
                    this.load_formats(cx);
                    this.refresh_sync(cx);
                    this.refresh_cookies(cx);
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
            shown: (String::new(), String::new()),
            link: (None, None),
            sync: SyncInfo::default(),
            formats: Vec::new(),
            export: ("workspace".to_owned(), String::new()),
            import: ("workspace".to_owned(), String::new()),
            collision: "keep",
            transferring: None,
            cookies: Value::Null,
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
        let link = (
            workspace.account.as_ref().map(|a| format!("{}:{}", a.id, a.signed_in)),
            workspace.sync.as_ref().map(|s| format!("{}:{}", s.remote, s.read_only)),
        );
        if self.link != link {
            self.link = link;
            self.refresh_sync(cx);
        }
        if self.shown == current {
            return;
        }
        let switched = self.shown.0 != current.0;
        self.name.update(cx, |input, cx| input.set_value(workspace.name.clone(), window, cx));
        if switched {
            self.sync = SyncInfo::default();
        }
        self.shown = current;
        if switched {
            self.refresh_sync(cx);
            self.cookies = Value::Null;
            self.refresh_cookies(cx);
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

    fn refresh_cookies(&mut self, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let workspace_id = model.workspace_id().to_owned();
        if workspace_id.is_empty() {
            return;
        }
        let client = model.client().clone();
        let asked = workspace_id.clone();
        call(&client, cx, "cookies.list", json!({ "workspaceId": workspace_id }), move |this, outcome, cx| {
            if let Ok(listed) = outcome
                && this.model.read(cx).workspace_id() == asked
            {
                this.cookies = listed;
                cx.notify();
            }
        });
    }

    /// `cookies.clear` or `cookies.configure` on the workspace in front. The list is re-read on the engine's
    /// `cookies.changed`.
    fn cookie_command(&mut self, action: &'static str, mut payload: Value, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        payload["workspaceId"] = json!(model.workspace_id());
        let client = model.client().clone();
        call(&client, cx, action, payload, |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.report(message, cx);
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

    /// A sync command that takes only the workspace. Progress arrives as `event.sync.status`.
    fn run_sync(&mut self, action: &'static str, what: &'static str, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id() });
        call(&client, cx, action, payload, move |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.report(t_with(what, &[("message", &message)]), cx);
            }
            this.model.update(cx, |model, cx| model.refresh(cx));
            this.refresh_sync(cx);
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
                Ok(path) => this.report(t_with("workspace.transfer.exported", &[("path", &path.display())]), cx),
                Err(message) => this.report(t_with("workspace.transfer.save_failed", &[("message", &message)]), cx),
            }),
            Err(message) => self.report(t_with("workspace.transfer.export_failed", &[("message", &message)]), cx),
        }
    }

    fn import_now(&mut self, cx: &mut Context<Self>) {
        let (kind, format) = self.import.clone();
        if self.transferring.is_some() || format.is_empty() {
            return;
        }
        let collision = self.collision;
        pick_file(t("workspace.transfer.choose_file"), cx, move |this, picked, cx| {
            let file = match picked {
                Ok(file) => file,
                Err(error) => return this.report(t_with("workspace.transfer.read_failed", &[("error", &error)]), cx),
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
                            let message = if skipped > 0 {
                                t_with(
                                    "workspace.transfer.imported_skipped",
                                    &[("imported", &imported), ("skipped", &skipped)],
                                )
                            } else {
                                t_with("workspace.transfer.imported", &[("imported", &imported)])
                            };
                            this.report(message, cx);
                            this.model.update(cx, |model, cx| model.refresh(cx));
                        }
                        Err(message) => {
                            this.report(t_with("workspace.transfer.import_failed", &[("message", &message)]), cx)
                        }
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
            t_with("workspace.delete.title", &[("name", &workspace.name)]),
            t("workspace.delete.consequence"),
            t("workspace.delete.confirm"),
            move |this, _, cx| this.model.update(cx, |model, cx| model.delete_workspace(&workspace.id, cx)),
        );
    }

    fn general_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let this = cx.weak_entity();
        let name = self.name.clone();
        let model = self.model.clone();
        SettingPage::new(t("workspace.page.general")).icon(IconName::Building2).default_open(true).groups(vec![
            SettingGroup::new().title(t("workspace.general.group")).item(
                SettingItem::render(move |options, _, cx| {
                    let id = model.read(cx).workspace_id().to_owned();
                    let theme = cx.theme().clone();
                    let rename = this.clone();
                    v_flex()
                        .w_full()
                        .gap_1p5()
                        .child(
                            h_flex().w_full().gap_2().child(div().flex_1().child(Input::new(&name))).child(
                                Button::new("rename-workspace")
                                    .with_size(options.size())
                                    .label(t("workspace.general.rename"))
                                    .on_click(move |_, _, cx| {
                                        rename.update(cx, |this, cx| this.rename(cx)).ok();
                                    }),
                            ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(t_with("workspace.general.id", &[("id", &id)])),
                        )
                })
                .keywords(t("workspace.general.keywords").split(',')),
            ),
        ])
    }

    /// A value shown, not edited: who the workspace syncs as, where to, who owns it.
    fn fact(title: &'static str, value: String, mono: bool, description: Option<&'static str>) -> SettingItem {
        let item = SettingItem::new(
            title,
            SettingField::render(move |_, _, cx| {
                div()
                    .text_color(cx.theme().muted_foreground)
                    .when(mono, |this| this.font_family(cx.theme().mono_font_family.clone()))
                    .child(value.clone())
            }),
        );
        match description {
            Some(description) => item.description(description),
            None => item,
        }
    }

    fn sync_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let this = cx.weak_entity();
        let sync = self.sync.clone();
        let model = self.model.clone();
        let setup = self.model.read(cx).setup().clone();
        let page = SettingPage::new(t("workspace.page.sync")).icon(IconName::Github);
        let Some(workspace) = self.model.read(cx).config().active_workspace().cloned() else {
            return page.groups(Vec::new());
        };

        if setup.known && !setup.git_installed {
            let check_model = model.clone();
            return page.groups(vec![
                SettingGroup::new()
                    .title(t("workspace.sync.git_group"))
                    .item(SettingItem::render(move |options, _, cx| {
                        let model = check_model.clone();
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t("workspace.sync.git_missing")),
                            )
                            .child(
                                Button::new("check-git")
                                    .with_size(options.size())
                                    .label(t("workspace.sync.git_check_again"))
                                    .on_click(move |_, _, cx| model.update(cx, |model, cx| model.refresh_setup(cx))),
                            )
                    }))
                    .footer(|_, _| t("workspace.sync.git_footer")),
            ]);
        }

        let changes_group = self.changes_group(workspace.sync.is_some(), cx);

        let account_label = match &workspace.account {
            Some(account) if account.signed_in => format!("@{}", account.login),
            Some(account) => t_with("workspace.account.signed_out", &[("login", &account.login)]),
            None => t("workspace.account.none").to_owned(),
        };
        let signed_out = workspace.account.clone().filter(|account| !account.signed_in);
        let mut account_group = SettingGroup::new().title(t("workspace.account.group")).item(Self::fact(
            t("workspace.account.title"),
            account_label,
            false,
            Some(if workspace.account.is_some() {
                t("workspace.account.bound_description")
            } else {
                t("workspace.account.unbound_description")
            }),
        ));
        if let Some(owner) = sync.owner.clone() {
            account_group = account_group.item(Self::fact(
                t("workspace.owner.title"),
                format!("@{owner}"),
                false,
                Some(if setup.team_sync {
                    t("workspace.owner.team_description")
                } else {
                    t("workspace.owner.description")
                }),
            ));
        }
        // Said only where it is not the owner's own: what someone else gave this account.
        match sync.access.as_deref().filter(|_| workspace.sync.is_some()) {
            Some("read") => {
                account_group = account_group.item(Self::fact(
                    t("workspace.access.title"),
                    t("workspace.access.read_only").to_owned(),
                    false,
                    Some(t("workspace.access.read_only_description")),
                ));
            }
            Some("write") => {
                account_group = account_group.item(Self::fact(
                    t("workspace.access.title"),
                    t("workspace.access.read_write").to_owned(),
                    false,
                    Some(t("workspace.access.read_write_description")),
                ));
            }
            _ => {}
        }
        if let Some(account) = signed_out {
            let model = model.clone();
            account_group = account_group.item(SettingItem::render(move |options, _, cx| {
                let (model, account) = (model.clone(), account.clone());
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_color(cx.theme().muted_foreground)
                            .child(t_with("workspace.account.paused", &[("login", &account.login)])),
                    )
                    .child(
                        Button::new("sign-in-again")
                            .with_size(options.size())
                            .primary()
                            .label(t("workspace.account.sign_in"))
                            .on_click(move |_, window, cx| {
                                let purpose = Purpose::SignIn { account_id: account.id.clone() };
                                open_setup_dialog(model.clone(), purpose, window, cx);
                            }),
                    )
            }));
        }

        let Some(config) = workspace.sync.clone() else {
            let publish_model = model.clone();
            let (id, name) = (workspace.id.clone(), workspace.name.clone());
            let remote_group = SettingGroup::new()
                .title(t("workspace.remote.title"))
                .item(SettingItem::render(move |options, _, cx| {
                    let (model, id, name) = (publish_model.clone(), id.clone(), name.clone());
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t("workspace.remote.not_published")),
                        )
                        .child(
                            Button::new("publish-workspace")
                                .with_size(options.size())
                                .primary()
                                .icon(IconName::Github)
                                .label(t("workspace.remote.publish"))
                                .on_click(move |_, window, cx| {
                                    let purpose =
                                        Purpose::Publish { workspace_id: id.clone(), workspace_name: name.clone() };
                                    open_setup_dialog(model.clone(), purpose, window, cx);
                                }),
                        )
                }))
                .footer(|_, _| t("workspace.remote.publish_footer"));
            return page.groups(vec![account_group, changes_group, remote_group]);
        };

        let remote_group = SettingGroup::new()
            .title(t("workspace.remote.title"))
            .item(Self::fact(
                t("workspace.remote.repository_title"),
                config.repo.clone().unwrap_or_else(|| config.remote.clone()),
                true,
                Some(t("workspace.remote.repository_description")),
            ))
            .footer(|_, _| t("workspace.remote.footer"));

        let git = self.model.read(cx).git();
        let not_synced = git.files.values().filter(|s| !matches!(s, FileState::Clean)).count();
        let synced = git.files.len() - not_synced;
        let status_label = t(match sync.status.as_str() {
            "pushing" => "workspace.status.pushing",
            "syncing" => "workspace.status.syncing",
            "pulling" => "workspace.status.pulling",
            "cloning" => "workspace.status.cloning",
            "error" => "workspace.status.error",
            _ => "workspace.status.idle",
        });
        let working = sync.is_working();
        let read_only = config.read_only;
        let (pull, push) = (this.clone(), this.clone());
        let status_group =
            SettingGroup::new().title(t("workspace.status.title")).item(SettingItem::render(move |options, _, cx| {
                let theme = cx.theme().clone();
                let (pull, push) = (pull.clone(), push.clone());
                let now = now_ms();
                let when = |ts: Option<i64>| {
                    ts.map_or_else(|| t("workspace.status.never").to_owned(), |ts| relative_time(now, ts))
                };
                let state = match sync.status.as_str() {
                    "error" => Tag::danger().child(status_label),
                    _ if working => {
                        Tag::warning().child(sync.progress.clone().unwrap_or_else(|| status_label.to_owned()))
                    }
                    _ => Tag::secondary().child(status_label),
                };
                let counts = if not_synced == 0 { Tag::success() } else { Tag::warning() }
                    .child(t_with("workspace.status.counts", &[("synced", &synced), ("not_synced", &not_synced)]));
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(div().flex_1().child(h_flex().gap_2().child(state).child(counts)))
                            .child(
                                Button::new("pull-remote")
                                    .with_size(options.size())
                                    .icon(IconName::ArrowDown)
                                    .label(t("workspace.status.pull"))
                                    .disabled(working)
                                    .on_click(move |_, _, cx| {
                                        pull.update(cx, |this, cx| {
                                            this.run_sync("sync.pull", "workspace.sync.pull_failed", cx)
                                        })
                                        .ok();
                                    }),
                            )
                            .child(
                                Button::new("push-remote")
                                    .with_size(options.size())
                                    .icon(IconName::ArrowUp)
                                    .label(t("workspace.status.push"))
                                    .disabled(working || read_only)
                                    .on_click(move |_, _, cx| {
                                        push.update(cx, |this, cx| {
                                            this.run_sync("sync.push", "workspace.sync.push_failed", cx)
                                        })
                                        .ok();
                                    }),
                            ),
                    )
                    .child(div().text_xs().text_color(theme.muted_foreground).child(t_with(
                        "workspace.status.last_synced",
                        &[("pushed", &when(sync.last_pushed)), ("pulled", &when(sync.last_pulled))],
                    )))
                    .children(sync.error.clone().map(|error| div().text_xs().text_color(theme.danger).child(error)))
            }));
        page.groups(vec![account_group, changes_group, remote_group, status_group])
    }

    /// What the workspace holds that was never synced, and the one action that syncs all of it.
    fn changes_group(&self, has_remote: bool, cx: &mut Context<Self>) -> SettingGroup {
        let model = self.model.clone();
        let git = self.model.read(cx).git();
        let waiting = git.files.values().filter(|s| matches!(s, FileState::New | FileState::Modified)).count();
        let deleted = git.files.values().filter(|s| **s == FileState::Deleted).count();
        let unpushed = git.has_remote && git.ahead;
        let busy = self.model.read(cx).is_publishing() || self.sync.is_working();
        let read_only = self.model.read(cx).is_read_only();
        let progress = self.sync.progress.clone().filter(|_| self.sync.is_working());
        let plural = |n: usize, one: &str, other: &str| t_with(if n == 1 { one } else { other }, &[("count", &n)]);
        let mut summary = match (waiting, unpushed) {
            (0, _) if read_only => t("workspace.changes.nothing_changed").to_owned(),
            (n, _) if read_only => plural(n, "workspace.changes.saved_here_one", "workspace.changes.saved_here_other"),
            (0, true) => t("workspace.changes.committed").to_owned(),
            (0, false) => t("workspace.changes.synced").to_owned(),
            (n, _) => plural(n, "workspace.changes.not_synced_one", "workspace.changes.not_synced_other"),
        };
        if deleted > 0 && read_only {
            summary.push(' ');
            summary.push_str(&plural(
                deleted,
                "workspace.changes.deleted_read_only_one",
                "workspace.changes.deleted_read_only_other",
            ));
        } else if deleted > 0 {
            summary.push(' ');
            summary.push_str(&plural(
                deleted,
                "workspace.changes.deleted_waiting_one",
                "workspace.changes.deleted_waiting_other",
            ));
        }
        SettingGroup::new()
            .title(t("workspace.changes.title"))
            .item(SettingItem::render(move |options, _, cx| {
                let model = model.clone();
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(progress.clone().unwrap_or_else(|| summary.clone())),
                    )
                    .child(
                        Button::new("sync-all")
                            .with_size(options.size())
                            .primary()
                            .icon(IconName::Github)
                            .label(t("workspace.changes.sync_all"))
                            .loading(busy)
                            .disabled(busy || read_only || (waiting == 0 && !unpushed))
                            .on_click(move |_, _, cx| model.update(cx, |model, cx| model.sync_all(cx))),
                    )
            }))
            .footer(move |_, _| {
                if read_only {
                    t("workspace.changes.footer_read_only")
                } else if has_remote {
                    t("workspace.changes.footer_remote")
                } else {
                    t("workspace.changes.footer_local")
                }
            })
    }

    /// A dropdown of kinds, and one of the formats the chosen kind has.
    fn transfer_fields(&self, exporting: bool, cx: &mut Context<Self>) -> Vec<SettingItem> {
        let (kind, format) = if exporting { self.export.clone() } else { self.import.clone() };
        let this = cx.weak_entity();
        let kinds: Vec<(SharedString, SharedString)> = KINDS
            .iter()
            .filter(|(k, _)| !self.formats_of(k, exporting).is_empty())
            .map(|(k, label)| (SharedString::from(*k), SharedString::from(t(label))))
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
                t("workspace.transfer.what"),
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
                t("workspace.transfer.format"),
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
            COLLISIONS.iter().map(|(id, label)| (SharedString::from(*id), SharedString::from(t(*label)))).collect();

        let export_group = SettingGroup::new()
            .title(t("workspace.transfer.export_title"))
            .items(self.transfer_fields(true, cx))
            .item(SettingItem::render(move |options, _, _| {
                let export = export.clone();
                h_flex().w_full().justify_end().child(
                    Button::new("export-workspace")
                        .with_size(options.size())
                        .primary()
                        .icon(IconName::ArrowUp)
                        .label(t("workspace.transfer.export_button"))
                        .loading(transferring == Some("Exporting"))
                        .disabled(transferring.is_some())
                        .on_click(move |_, _, cx| {
                            export.update(cx, |this, cx| this.export_now(cx)).ok();
                        }),
                )
            }))
            .footer(|_, _| t("workspace.transfer.export_footer"));
        let import_group = SettingGroup::new()
            .title(t("workspace.transfer.import_title"))
            .items(self.transfer_fields(false, cx))
            .item(
                SettingItem::new(
                    t("workspace.transfer.when_exists"),
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
                .description(t("workspace.transfer.when_exists_description")),
            )
            .item(SettingItem::render(move |options, _, _| {
                let import = import.clone();
                h_flex().w_full().justify_end().child(
                    Button::new("import-workspace")
                        .with_size(options.size())
                        .primary()
                        .icon(IconName::ArrowDown)
                        .label(t("workspace.transfer.import_button"))
                        .loading(transferring == Some("Importing"))
                        .disabled(transferring.is_some())
                        .on_click(move |_, _, cx| {
                            import.update(cx, |this, cx| this.import_now(cx)).ok();
                        }),
                )
            }))
            .footer(|_, _| t("workspace.transfer.import_footer"));
        SettingPage::new(t("workspace.page.transfer")).icon(IconName::Inbox).groups(vec![export_group, import_group])
    }

    fn cookies_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let this = cx.weak_entity();
        let clear_on_restart = self.cookies.get("clearOnRestart").and_then(Value::as_bool).unwrap_or(true);
        let domains = self.cookies.get("domains").and_then(Value::as_array).cloned().unwrap_or_default();
        let cookies_of = |entry: &Value| entry.get("cookies").and_then(Value::as_array).cloned().unwrap_or_default();
        let total: usize = domains.iter().map(|entry| cookies_of(entry).len()).sum();
        let summary = match (total, domains.len()) {
            (0, _) => t("workspace.cookies.none").to_owned(),
            (1, _) => t("workspace.cookies.one").to_owned(),
            (n, 1) => t_with("workspace.cookies.one_site", &[("count", &n)]),
            (n, sites) => t_with("workspace.cookies.many_sites", &[("count", &n), ("sites", &sites)]),
        };

        let configure = this.clone();
        let behaviour = SettingGroup::new().title(t("workspace.cookies.restart_group")).item(
            SettingItem::new(
                t("workspace.cookies.clear_on_restart"),
                SettingField::switch(
                    move |_: &App| clear_on_restart,
                    move |value: bool, cx: &mut App| {
                        configure
                            .update(cx, |this, cx| {
                                this.cookie_command("cookies.configure", json!({ "clearOnRestart": value }), cx)
                            })
                            .ok();
                    },
                ),
            )
            .description(t("workspace.cookies.clear_on_restart_description"))
            .keywords(t("workspace.cookies.keywords").split(',')),
        );

        let clear_all = this.clone();
        let mut stored = SettingGroup::new().title(t("workspace.cookies.stored_group")).item(SettingItem::render(
            move |options, _, cx| {
                let clear_all = clear_all.clone();
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(div().flex_1().text_sm().text_color(cx.theme().muted_foreground).child(summary.clone()))
                    .child(
                        Button::new("clear-all-cookies")
                            .with_size(options.size())
                            .label(t("workspace.cookies.clear_all"))
                            .disabled(total == 0)
                            .on_click(move |_, _, cx| {
                                clear_all
                                    .update(cx, |this, cx| this.cookie_command("cookies.clear", json!({}), cx))
                                    .ok();
                            }),
                    )
            },
        ));
        for (index, entry) in domains.into_iter().enumerate() {
            let this = this.clone();
            let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
            let domain = text(&entry, "domain");
            let cookies: Vec<(String, String, String)> = cookies_of(&entry)
                .iter()
                .map(|cookie| (text(cookie, "name"), text(cookie, "value"), text(cookie, "path")))
                .collect();
            stored = stored.item(SettingItem::render(move |options, _, cx| {
                let theme = cx.theme().clone();
                let clear = this.clone();
                let cleared = domain.clone();
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(div().flex_1().font_family(theme.mono_font_family.clone()).child(domain.clone()))
                            .child(
                                Button::new(("clear-cookies", index))
                                    .with_size(options.size())
                                    .label(t("workspace.cookies.clear"))
                                    .on_click(move |_, _, cx| {
                                        let payload = json!({ "domain": cleared });
                                        clear
                                            .update(cx, |this, cx| this.cookie_command("cookies.clear", payload, cx))
                                            .ok();
                                    }),
                            ),
                    )
                    .children(cookies.iter().map(|(name, value, path)| {
                        h_flex()
                            .w_full()
                            .gap_2()
                            .text_xs()
                            .font_family(theme.mono_font_family.clone())
                            .child(div().child(name.clone()))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme.muted_foreground)
                                    .child(value.clone()),
                            )
                            .child(div().text_color(theme.muted_foreground).child(path.clone()))
                    }))
            }));
        }
        SettingPage::new(t("workspace.page.cookies")).icon(IconName::Globe).groups(vec![behaviour, stored])
    }

    fn danger_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let this = cx.weak_entity();
        let can_delete = self.model.read(cx).config().workspaces.len() > 1;
        SettingPage::new(t("workspace.page.danger")).icon(IconName::TriangleAlert).groups(vec![
            SettingGroup::new()
                .title(t("workspace.danger.group"))
                .item(SettingItem::render(move |options, _, cx| {
                    let theme = cx.theme().clone();
                    let delete = this.clone();
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div().flex_1().text_color(theme.muted_foreground).child(t("workspace.danger.description")),
                        )
                        .child(
                            Button::new("delete-workspace")
                                .with_size(options.size())
                                .danger()
                                .label(t("workspace.danger.delete"))
                                .disabled(!can_delete)
                                .on_click(move |_, window, cx| {
                                    delete.update(cx, |this, cx| this.ask_delete(window, cx)).ok();
                                }),
                        )
                }))
                .footer(|_, _| t("workspace.danger.footer")),
        ])
    }
}

impl Render for WorkspaceScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pages = vec![
            self.general_page(cx),
            self.sync_page(cx),
            self.cookies_page(cx),
            self.transfer_page(cx),
            self.danger_page(cx),
        ];
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
        assert_eq!(info.access, None);
        assert_eq!(SyncInfo::from_value(&json!({ "access": "read" })).access.as_deref(), Some("read"));
    }
}
