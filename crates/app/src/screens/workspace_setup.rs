//! Getting a workspace, and giving one a remote: the first-run screen, the "Add workspace" dialog and the
//! "Publish to GitHub" dialog are this one view with a different purpose.
//!
//! The steps are the same wherever it is shown. Git is checked first, because without it a workspace is plain
//! files. Signing in to GitHub is optional and belongs to the workspace being made: it is only needed to sync
//! with a remote. Then the workspace is either created here or imported from a repository the account owns
//! or, with team sync, one it was given access to.
//! Every rule (who may import what, what may be pushed where) is the engine's; this view sends commands and
//! shows what came back.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, StyledExt as _, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    menu::{DropdownMenu as _, PopupMenuItem},
    spinner::Spinner,
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    rems,
};
use serde_json::{Value, json};
use std::rc::Rc;

use crate::{
    app_model::{AppModel, ModelEvent},
    engine::{call, call_in, describe, error_text, failure},
    i18n::{t, t_with},
    model::config::AccountInfo,
};

/// The view's width in rems, in a dialog and on the first-run screen.
pub const DIALOG_WIDTH: f32 = 40.;

/// Why the view is on screen.
#[derive(Debug, Clone, PartialEq)]
pub enum Purpose {
    /// There is no workspace yet: the window shows nothing else.
    First,
    /// One more workspace, from the workspace menu.
    Add,
    /// Push the workspace with this id and name to a new repository.
    Publish { workspace_id: String, workspace_name: String },
    /// Sign this account back in, for a workspace that syncs as it.
    SignIn { account_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    New,
    Import,
}

/// A sign-in the engine is waiting on: the user types `user_code` on the page at `uri`.
#[derive(Debug, Clone, PartialEq)]
struct Login {
    id: String,
    user_code: String,
    uri: String,
}

#[derive(Debug, Clone, PartialEq)]
struct Repo {
    full_name: String,
    private: bool,
    /// Someone else's, which the account was given access to.
    shared: bool,
    /// Shared for reading only: the workspace imported from it is read-only.
    read_only: bool,
    /// The workspace on this device that is already connected to it.
    workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum Repos {
    Idle,
    Loading,
    Ready(Vec<Repo>),
    Failed(String),
}

fn repos_from(value: &Value) -> Vec<Repo> {
    value
        .get("repos")
        .and_then(Value::as_array)
        .map(|repos| {
            repos
                .iter()
                .filter_map(|repo| {
                    // An engine that says nothing about access lists only what the account owns.
                    let access = repo.get("access").and_then(Value::as_str).unwrap_or("owner");
                    Some(Repo {
                        full_name: repo.get("fullName")?.as_str()?.to_owned(),
                        private: repo.get("private").and_then(Value::as_bool).unwrap_or(false),
                        shared: access != "owner",
                        read_only: access == "read",
                        workspace_id: repo.get("workspaceId").and_then(Value::as_str).map(str::to_owned),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A repository name made from a workspace's name: what GitHub would accept without changing it.
fn repo_name_for(workspace_name: &str) -> String {
    let mut name = String::new();
    for c in workspace_name.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
            name.push(c.to_ascii_lowercase());
        } else if !name.ends_with('-') {
            name.push('-');
        }
    }
    let name = name.trim_matches('-').to_owned();
    if name.is_empty() { "workspace".to_owned() } else { name }
}

type BackHandler = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct WorkspaceSetup {
    model: Entity<AppModel>,
    purpose: Purpose,
    mode: Mode,
    /// The new workspace's name, or the new repository's when publishing.
    name: Entity<InputState>,
    filter: Entity<InputState>,
    /// The account the workspace is made, imported or published as.
    account: Option<String>,
    /// The account cannot be changed: the workspace being published already has one.
    account_fixed: bool,
    login: Option<Login>,
    /// A sign-in was asked for and the engine has not answered with a code yet.
    starting_login: bool,
    repos: Repos,
    selected_repo: Option<String>,
    private: bool,
    /// Publishing also syncs what the workspace holds that was never synced.
    sync_all: bool,
    /// The create, import or publish command is in flight.
    busy: bool,
    error: Option<String>,
    /// What "Back" does, where the view is one step of a longer setup.
    on_back: Option<BackHandler>,
    _subscriptions: Vec<Subscription>,
}

impl WorkspaceSetup {
    pub fn new(model: Entity<AppModel>, purpose: Purpose, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (placeholder, initial) = match &purpose {
            Purpose::Publish { workspace_name, .. } => {
                (t("workspace_setup.placeholder.repository_name"), repo_name_for(workspace_name))
            }
            _ => (t("workspace_setup.placeholder.workspace_name"), String::new()),
        };
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(initial));
        let filter =
            cx.new(|cx| InputState::new(window, cx).placeholder(t("workspace_setup.placeholder.filter_repos")));
        // A workspace that already commits as an account is published as that account and no other.
        let bound = match &purpose {
            Purpose::Publish { workspace_id, .. } => model
                .read(cx)
                .config()
                .workspaces
                .iter()
                .find(|w| w.id == *workspace_id)
                .and_then(|w| w.account.clone()),
            _ => None,
        };
        let (account, account_fixed) = match &purpose {
            Purpose::SignIn { account_id } => (Some(account_id.clone()), true),
            _ => (bound.as_ref().map(|account| account.id.clone()), bound.is_some()),
        };
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| {
                if let ModelEvent::AccountChanged(payload) = event {
                    this.on_account_changed(payload, window, cx);
                }
            }),
            cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.submit(window, cx);
                }
            }),
            cx.observe(&filter, |_, _, cx| cx.notify()),
        ];
        Self {
            model,
            purpose,
            mode: Mode::New,
            name,
            filter,
            account,
            account_fixed,
            login: None,
            starting_login: false,
            repos: Repos::Idle,
            selected_repo: None,
            private: true,
            sync_all: true,
            busy: false,
            error: None,
            on_back: None,
            _subscriptions: subscriptions,
        }
    }

    /// Puts a "Back" button beside the main action, for when the view is one step of a longer setup.
    pub fn on_back(mut self, on_back: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_back = Some(Rc::new(on_back));
        self
    }

    fn accounts(&self, cx: &Context<Self>) -> Vec<AccountInfo> {
        self.model.read(cx).setup().accounts.clone()
    }

    fn selected_account(&self, cx: &Context<Self>) -> Option<AccountInfo> {
        let id = self.account.as_deref()?;
        self.accounts(cx).into_iter().find(|account| account.id == id)
    }

    /// The selected account, if it can be used right now.
    fn usable_account(&self, cx: &Context<Self>) -> Option<AccountInfo> {
        self.selected_account(cx).filter(|account| account.signed_in)
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.error = None;
        if mode == Mode::Import {
            // Importing needs an account; with exactly one signed in there is nothing to choose.
            if self.usable_account(cx).is_none() {
                let signed_in: Vec<AccountInfo> = self.accounts(cx).into_iter().filter(|a| a.signed_in).collect();
                if let [only] = signed_in.as_slice() {
                    self.account = Some(only.id.clone());
                }
            }
            self.load_repos(cx);
        }
        cx.notify();
    }

    fn select_account(&mut self, account: Option<String>, cx: &mut Context<Self>) {
        if self.account_fixed || self.account == account {
            return;
        }
        self.account = account;
        self.selected_repo = None;
        self.error = None;
        if self.mode == Mode::Import {
            self.load_repos(cx);
        }
        cx.notify();
    }

    fn sign_in(&mut self, cx: &mut Context<Self>) {
        if self.starting_login || self.login.is_some() {
            return;
        }
        self.starting_login = true;
        self.error = None;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "account.login.start", json!({}), |this, outcome, cx| {
            this.starting_login = false;
            let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
            match outcome {
                Ok(started) if started.get("status").and_then(Value::as_str) == Some("pending") => {
                    this.login = Some(Login {
                        id: text(&started, "loginId"),
                        user_code: text(&started, "userCode"),
                        uri: text(&started, "verificationUri"),
                    });
                }
                Ok(started) => {
                    this.error =
                        Some(t_with("workspace_setup.error.start_sign_in", &[("reason", &text(&started, "error"))]))
                }
                Err(error) => {
                    this.error = Some(t_with("workspace_setup.error.start_sign_in", &[("reason", &describe(&error))]))
                }
            }
            cx.notify();
        });
    }

    fn cancel_login(&mut self, cx: &mut Context<Self>) {
        let Some(login) = self.login.take() else { return };
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "account.login.cancel", json!({ "loginId": login.id }), |_, _, _| {});
        cx.notify();
    }

    /// The engine finished a sign-in. Only the one this view started changes what it shows.
    fn on_account_changed(&mut self, payload: &Value, window: &mut Window, cx: &mut Context<Self>) {
        let ours =
            self.login.as_ref().is_some_and(|login| payload.get("loginId").and_then(Value::as_str) == Some(&login.id));
        if !ours {
            return;
        }
        self.login = None;
        match payload.get("status").and_then(Value::as_str) {
            Some("done") => {
                let signed_in = payload.pointer("/account/id").and_then(Value::as_str).map(str::to_owned);
                if self.account_fixed && signed_in != self.account {
                    let login = payload.pointer("/account/login").and_then(Value::as_str).unwrap_or_default();
                    self.error = Some(t_with("workspace_setup.error.wrong_account", &[("login", &login)]));
                } else if matches!(self.purpose, Purpose::SignIn { .. }) {
                    // The account is back, which is all this dialog was for.
                    window.close_dialog(cx);
                } else {
                    self.account = signed_in;
                    self.selected_repo = None;
                    if self.mode == Mode::Import {
                        self.load_repos(cx);
                    }
                }
            }
            Some("cancelled") => {}
            _ => {
                let reason =
                    error_text(payload).unwrap_or_else(|| t("workspace_setup.error.sign_in_not_completed").to_owned());
                self.error = Some(t_with("workspace_setup.error.sign_in_failed", &[("reason", &reason)]));
            }
        }
        cx.notify();
    }

    fn load_repos(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self.account.clone() else {
            self.repos = Repos::Idle;
            return;
        };
        self.repos = Repos::Loading;
        let client = self.model.read(cx).client().clone();
        let asked = account.clone();
        call(&client, cx, "account.repos", json!({ "accountId": account }), move |this, outcome, cx| {
            // An answer for an account that is no longer the chosen one lists the wrong repositories.
            if this.account.as_deref() != Some(asked.as_str()) {
                return;
            }
            this.repos = match failure(&outcome) {
                Some(message) => Repos::Failed(message),
                None => Repos::Ready(repos_from(&outcome.unwrap_or(Value::Null))),
            };
            cx.notify();
        });
    }

    /// Why the main action cannot run yet, if it cannot.
    fn blocked(&self, cx: &Context<Self>) -> bool {
        if self.busy || self.login.is_some() {
            return true;
        }
        let named = !self.name.read(cx).value().trim().is_empty();
        match (&self.purpose, self.mode) {
            (Purpose::SignIn { .. }, _) => true,
            (Purpose::Publish { .. }, _) => !named || self.usable_account(cx).is_none(),
            (_, Mode::New) => !named || self.selected_account(cx).is_some_and(|account| !account.signed_in),
            (_, Mode::Import) => self.selected_repo.is_none() || self.usable_account(cx).is_none(),
        }
    }

    /// Creates, imports or publishes. In a dialog, success closes it; a failure is shown and it stays.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked(cx) {
            return;
        }
        let name = self.name.read(cx).value().trim().to_owned();
        let (action, payload) = match (&self.purpose, self.mode) {
            (Purpose::SignIn { .. }, _) => return,
            (Purpose::Publish { workspace_id, .. }, _) => (
                "sync.publish",
                json!({
                    "workspaceId": workspace_id, "accountId": self.account, "repoName": name,
                    "private": self.private, "syncAll": self.sync_all,
                }),
            ),
            (_, Mode::New) => {
                let mut payload = json!({ "name": name });
                if let Some(account) = &self.account {
                    payload["accountId"] = json!(account);
                }
                ("workspace.add", payload)
            }
            (_, Mode::Import) => ("workspace.import", json!({ "accountId": self.account, "repo": self.selected_repo })),
        };
        self.busy = true;
        self.error = None;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        let in_dialog = self.purpose != Purpose::First;
        call_in(&client, window, cx, action, payload, move |this, outcome, window, cx| {
            this.busy = false;
            if let Some(message) = failure(&outcome) {
                this.error = Some(message);
                // A publish may have created the repository before it failed; the workspace shows what is true.
                this.model.update(cx, |model, cx| model.refresh(cx));
                cx.notify();
                return;
            }
            let result = outcome.unwrap_or(Value::Null);
            // A new workspace is put in front; an imported one already is, and a published one stays there.
            match (action, result.get("id").and_then(Value::as_str)) {
                ("workspace.add", Some(id)) => {
                    let id = id.to_owned();
                    this.model.update(cx, |model, cx| model.set_active_workspace(&id, cx));
                }
                _ => this.model.update(cx, |model, cx| model.refresh(cx)),
            }
            if in_dialog {
                window.close_dialog(cx);
            }
            cx.notify();
        });
    }

    fn check_git_again(&mut self, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.refresh_setup(cx));
    }

    fn render_git_notice(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_2()
            .child(
                Alert::warning("git-missing", t("workspace_setup.git.missing.description"))
                    .title(t("workspace_setup.git.missing.title")),
            )
            .child(
                h_flex().justify_end().child(
                    Button::new("check-git")
                        .outline()
                        .small()
                        .label(t("workspace_setup.button.check_again"))
                        .on_click(cx.listener(|this, _, _, cx| this.check_git_again(cx))),
                ),
            )
            .into_any_element()
    }

    /// The code to type on GitHub, while the engine waits for it to be used.
    fn render_login(&self, login: &Login, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (code, uri) = (login.user_code.clone(), login.uri.clone());
        v_flex()
            .gap_3()
            .p_4()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .child(div().text_sm().child(t("workspace_setup.login.enter_code")))
            .child(
                div()
                    .text_2xl()
                    .font_semibold()
                    .font_family(theme.mono_font_family.clone())
                    .child(SharedString::from(login.user_code.clone())),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("copy-code")
                            .outline()
                            .small()
                            .icon(IconName::Copy)
                            .label(t("workspace_setup.button.copy_code"))
                            .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(code.clone()))),
                    )
                    .child(
                        Button::new("open-github")
                            .primary()
                            .small()
                            .icon(IconName::ExternalLink)
                            .label(t("workspace_setup.button.open_github"))
                            .on_click(move |_, _, cx| cx.open_url(&uri)),
                    )
                    .child(div().flex_1())
                    .child(Spinner::new().small())
                    .child(div().text_sm().text_color(theme.muted_foreground).child(t("workspace_setup.login.waiting")))
                    .child(
                        Button::new("cancel-login")
                            .ghost()
                            .small()
                            .label(t("workspace_setup.button.cancel"))
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_login(cx))),
                    ),
            )
            .into_any_element()
    }

    /// Who the workspace is made as. Optional, except to import or publish.
    fn render_account(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let setup = self.model.read(cx).setup().clone();
        let needs_account = self.mode == Mode::Import || !matches!(self.purpose, Purpose::First | Purpose::Add);
        let help = if needs_account && setup.team_sync {
            t("workspace_setup.account.help_team")
        } else if needs_account {
            t("workspace_setup.account.help_owner")
        } else {
            t("workspace_setup.account.help_optional")
        };
        let heading = v_flex()
            .gap_1()
            .child(div().text_sm().font_medium().child(t("workspace_setup.account.heading")))
            .child(div().text_sm().text_color(theme.muted_foreground).child(help));

        if let Some(login) = &self.login {
            return v_flex().gap_2().child(heading).child(self.render_login(login, cx)).into_any_element();
        }
        if !setup.sign_in_available && setup.accounts.is_empty() {
            return v_flex()
                .gap_2()
                .child(heading)
                .child(
                    div().text_sm().text_color(theme.muted_foreground).child(t("workspace_setup.account.not_set_up")),
                )
                .into_any_element();
        }

        let selected = self.selected_account(cx);
        let label = match &selected {
            Some(account) if account.signed_in => t_with("workspace_setup.account.login", &[("login", &account.login)]),
            Some(account) => t_with("workspace_setup.account.login_signed_out", &[("login", &account.login)]),
            None => t("workspace_setup.account.none").to_owned(),
        };
        // Short enough to sit beside the account chooser at any width the view is shown at.
        let sign_in_label = match (&selected, setup.accounts.is_empty()) {
            (Some(account), _) if !account.signed_in => t("workspace_setup.account.sign_in_again"),
            (_, true) => t("workspace_setup.sign_in.title"),
            (_, false) => t("workspace_setup.account.add"),
        };
        // A workspace's own account is not swapped for another: it only ever signs back in.
        let offer_sign_in = !(self.account_fixed && selected.as_ref().is_some_and(|account| account.signed_in));
        let sign_in = Button::new("sign-in")
            .outline()
            .flex_shrink_0()
            .icon(IconName::Github)
            .label(sign_in_label)
            .loading(self.starting_login)
            .disabled(self.starting_login || !setup.sign_in_available)
            .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx)));

        // The chooser takes the row's width; the sign-in button keeps its own beside it.
        let chooser = if self.account_fixed {
            div().flex_1().min_w_0().child(label).into_any_element()
        } else {
            let this = cx.weak_entity();
            let accounts = setup.accounts.clone();
            let current = self.account.clone();
            let menu = Button::new("account-menu").outline().w_full().label(label).dropdown_caret(true).dropdown_menu(
                move |menu, _, _| {
                    let menu = accounts.iter().fold(menu, |menu, account| {
                        let (this, id) = (this.clone(), account.id.clone());
                        let name = if account.signed_in {
                            t_with("workspace_setup.account.login", &[("login", &account.login)])
                        } else {
                            t_with("workspace_setup.account.login_signed_out", &[("login", &account.login)])
                        };
                        menu.item(
                            PopupMenuItem::new(name).checked(current.as_deref() == Some(account.id.as_str())).on_click(
                                move |_, _, cx| {
                                    this.update(cx, |this, cx| this.select_account(Some(id.clone()), cx)).ok();
                                },
                            ),
                        )
                    });
                    let this = this.clone();
                    menu.separator().item(
                        PopupMenuItem::new(t("workspace_setup.account.none")).checked(current.is_none()).on_click(
                            move |_, _, cx| {
                                this.update(cx, |this, cx| this.select_account(None, cx)).ok();
                            },
                        ),
                    )
                },
            );
            v_flex().flex_1().min_w_0().child(menu).into_any_element()
        };

        v_flex()
            .gap_2()
            .child(heading)
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .when(!setup.accounts.is_empty() || self.account_fixed, |this| this.child(chooser))
                    .when(offer_sign_in, |this| this.child(sign_in)),
            )
            .into_any_element()
    }

    fn render_mode(&self, cx: &mut Context<Self>) -> AnyElement {
        let mode = self.mode;
        // Two equal halves: the choice reads as one control across the view.
        h_flex()
            .w_full()
            .gap_2()
            .child(
                Button::new("mode-new")
                    .outline()
                    .flex_1()
                    .label(t("workspace_setup.mode.new"))
                    .selected(mode == Mode::New)
                    .on_click(cx.listener(|this, _, _, cx| this.set_mode(Mode::New, cx))),
            )
            .child(
                Button::new("mode-import")
                    .outline()
                    .flex_1()
                    .icon(IconName::Github)
                    .label(t("workspace_setup.mode.import"))
                    .selected(mode == Mode::Import)
                    .on_click(cx.listener(|this, _, _, cx| this.set_mode(Mode::Import, cx))),
            )
            .into_any_element()
    }

    fn render_name(&self, label: &'static str, help: Option<&'static str>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        v_flex()
            .gap_2()
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().font_medium().child(label))
                    .children(help.map(|help| div().text_sm().text_color(theme.muted_foreground).child(help))),
            )
            .child(Input::new(&self.name))
            .into_any_element()
    }

    /// The repositories the account can import from: its own and, with team sync, the ones shared with it.
    /// One that is already a workspace here cannot be imported twice.
    fn render_repos(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let muted = |text: &'static str| div().text_sm().text_color(theme.muted_foreground).child(text);
        if self.usable_account(cx).is_none() {
            return muted(t("workspace_setup.repos.sign_in_first")).into_any_element();
        }
        let repos = match &self.repos {
            Repos::Idle | Repos::Loading => {
                return h_flex()
                    .gap_2()
                    .child(Spinner::new().small())
                    .child(muted(t("workspace_setup.repos.loading")))
                    .into_any_element();
            }
            Repos::Failed(message) => {
                return v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.danger)
                            .child(t_with("workspace_setup.error.list_repos", &[("message", &message)])),
                    )
                    .child(
                        h_flex().child(
                            Button::new("retry-repos")
                                .outline()
                                .small()
                                .label(t("workspace_setup.button.try_again"))
                                .on_click(cx.listener(|this, _, _, cx| this.load_repos(cx))),
                        ),
                    )
                    .into_any_element();
            }
            Repos::Ready(repos) => repos,
        };
        if repos.is_empty() {
            let team_sync = self.model.read(cx).setup().team_sync;
            return muted(if team_sync {
                t("workspace_setup.repos.empty_team")
            } else {
                t("workspace_setup.repos.empty_owner")
            })
            .into_any_element();
        }
        let filter = self.filter.read(cx).value().trim().to_lowercase();
        let rows: Vec<AnyElement> = repos
            .iter()
            .filter(|repo| filter.is_empty() || repo.full_name.to_lowercase().contains(&filter))
            .map(|repo| {
                let name = repo.full_name.clone();
                let here = repo.workspace_id.is_some();
                ListItem::new(gpui_kit::ElementId::Name(format!("repo-{}", repo.full_name).into()))
                    .selected(self.selected_repo.as_deref() == Some(repo.full_name.as_str()))
                    .disabled(here)
                    .px_2()
                    .py_1()
                    .text_sm()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(repo.full_name.clone()),
                            )
                            .when(repo.private, |this| {
                                this.child(Tag::secondary().small().child(t("workspace_setup.tag.private")))
                            })
                            .when(repo.shared && !repo.read_only, |this| {
                                this.child(Tag::secondary().small().child(t("workspace_setup.tag.shared")))
                            })
                            .when(repo.read_only, |this| {
                                this.child(Tag::warning().small().child(t("workspace_setup.tag.read_only")))
                            })
                            .when(here, |this| {
                                this.child(Tag::secondary().small().child(t("workspace_setup.tag.on_device")))
                            }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected_repo = Some(name.clone());
                        this.error = None;
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        // Said before the import, not found out after it.
        let selected_read_only =
            repos.iter().any(|repo| repo.read_only && self.selected_repo.as_deref() == Some(repo.full_name.as_str()));
        v_flex()
            .gap_2()
            .child(div().text_sm().font_medium().child(t("workspace_setup.repos.heading")))
            .child(Input::new(&self.filter))
            .child(
                v_flex()
                    .id("setup-repos")
                    .max_h(rems(14.))
                    .overflow_y_scroll()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .p_1()
                    .gap_1()
                    .children(rows),
            )
            .when(selected_read_only, |this| this.child(muted(t("workspace_setup.repos.read_only_note"))))
            .into_any_element()
    }

    fn render_actions(&self, cx: &mut Context<Self>) -> AnyElement {
        if matches!(self.purpose, Purpose::SignIn { .. }) {
            return h_flex()
                .justify_end()
                .child(
                    Button::new("setup-close")
                        .outline()
                        .label(t("workspace_setup.button.close"))
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                )
                .into_any_element();
        }
        let label = match (&self.purpose, self.mode) {
            (Purpose::Publish { .. }, _) => t("workspace_setup.submit.publish"),
            (_, Mode::New) => t("workspace_setup.submit.create"),
            (_, Mode::Import) => t("workspace_setup.submit.import"),
        };
        let in_dialog = self.purpose != Purpose::First;
        h_flex()
            .w_full()
            .gap_2()
            .justify_end()
            .when_some(self.on_back.clone(), |this, on_back| {
                this.child(
                    Button::new("setup-back")
                        .outline()
                        .label(t("workspace_setup.button.back"))
                        .disabled(self.busy)
                        .on_click(move |_, window, cx| on_back(window, cx)),
                )
                .child(div().flex_1())
            })
            .when(in_dialog, |this| {
                this.child(
                    Button::new("setup-cancel")
                        .outline()
                        .label(t("workspace_setup.button.cancel"))
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                )
            })
            .child(
                Button::new("setup-submit")
                    .primary()
                    .label(label)
                    .loading(self.busy)
                    .disabled(self.blocked(cx))
                    .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
            )
            .into_any_element()
    }

    /// Roughly how tall the view's dialog is, in rems, for placing it: the title, the sections shown and
    /// the actions.
    fn estimated_height(&self) -> f32 {
        let body = match (&self.purpose, self.mode) {
            (Purpose::SignIn { .. }, _) => 0.,
            (Purpose::Publish { .. }, _) => 9.,
            (_, Mode::New) => 8.,
            // The label, the filter and the list, which grows with its rows up to where it scrolls.
            (_, Mode::Import) => match &self.repos {
                Repos::Ready(repos) if !repos.is_empty() => {
                    let note = if repos.iter().any(|repo| repo.read_only) { 3. } else { 0. };
                    7. + (repos.len() as f32 * 2.25).min(14.) + note
                }
                _ => 3.,
            },
        };
        let login = if self.login.is_some() { 7. } else { 0. };
        16. + body + login
    }

    /// Puts the cursor where typing starts.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |input, cx| input.focus(window, cx));
    }
}

/// Opens the view in a dialog: one more workspace, or a workspace's first publish.
pub fn open_setup_dialog(model: Entity<AppModel>, purpose: Purpose, window: &mut Window, cx: &mut App) {
    let title = match purpose {
        Purpose::Publish { .. } => t("workspace_setup.dialog.publish"),
        Purpose::SignIn { .. } => t("workspace_setup.sign_in.title"),
        Purpose::First | Purpose::Add => t("workspace_setup.dialog.add"),
    };
    // Deferred so a dialog opened from a menu or the palette opens after that closes.
    window.defer(cx, move |window, cx| {
        let view = cx.new(|cx| WorkspaceSetup::new(model, purpose, window, cx));
        let focus = view.clone();
        window.open_dialog(cx, move |dialog, window, cx| {
            let submit = view.clone();
            // Wide enough for the account row and a repository's full name, and set in the middle of the
            // window: the height is the view's own estimate, since the dialog is placed before it is measured.
            let rem = window.rem_size();
            let height = rems(view.read(cx).estimated_height()).to_pixels(rem);
            let top = ((window.viewport_size().height - height) / 2.).max(rems(2.).to_pixels(rem));
            // The view draws its own actions; Enter goes to its main one, which closes the dialog when it worked.
            dialog.title(title).w(rems(DIALOG_WIDTH).to_pixels(rem)).margin_top(top).child(view.clone()).button_props(
                DialogButtonProps::default().on_ok(move |_, window, cx| {
                    submit.update(cx, |view, cx| view.submit(window, cx));
                    false
                }),
            )
        });
        window.defer(cx, move |window, cx| focus.update(cx, |view, cx| view.focus(window, cx)));
    });
}

impl Render for WorkspaceSetup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let setup = self.model.read(cx).setup().clone();
        let publishing = matches!(self.purpose, Purpose::Publish { .. });
        let signing_in = matches!(self.purpose, Purpose::SignIn { .. });
        let body = match (publishing, self.mode) {
            _ if signing_in => div().into_any_element(),
            (true, _) => v_flex()
                .gap_4()
                .child(self.render_name(
                    t("workspace_setup.publish.name_label"),
                    Some(t("workspace_setup.publish.name_help")),
                    cx,
                ))
                .child(
                    Checkbox::new("publish-private")
                        .label(t("workspace_setup.publish.private"))
                        .checked(self.private)
                        .on_change(cx.listener(|this, checked: &bool, _, cx| {
                            this.private = *checked;
                            cx.notify();
                        })),
                )
                .child(
                    Checkbox::new("publish-sync-all")
                        .label(t("workspace_setup.publish.sync_all"))
                        .checked(self.sync_all)
                        .on_change(cx.listener(|this, checked: &bool, _, cx| {
                            this.sync_all = *checked;
                            cx.notify();
                        })),
                )
                .into_any_element(),
            (false, Mode::New) => self.render_name(t("workspace_setup.name.label"), None, cx),
            (false, Mode::Import) => self.render_repos(cx),
        };

        v_flex()
            .w_full()
            .gap_6()
            .when(self.purpose != Purpose::First, |this| this.pt_2())
            .when(setup.known && !setup.git_installed, |this| this.child(self.render_git_notice(cx)))
            // Without git there is nothing to sign in for, and nothing to import.
            .when(setup.git_installed && !publishing && !signing_in, |this| this.child(self.render_mode(cx)))
            .when(setup.git_installed, |this| this.child(self.render_account(cx)))
            .child(body)
            .children(self.error.clone().map(|error| div().text_sm().text_color(theme.danger).child(error)))
            .child(self.render_actions(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repository_name_is_made_from_the_workspace_name() {
        assert_eq!(repo_name_for("Payments API (staging)"), "payments-api-staging");
        assert_eq!(repo_name_for("  my_ws.v2 "), "my_ws.v2");
        assert_eq!(repo_name_for("***"), "workspace");
    }

    #[test]
    fn repositories_are_read_with_the_workspace_they_already_are() {
        let repos = repos_from(&json!({ "ok": true, "repos": [
            { "fullName": "octo/a", "private": true, "workspaceId": "w1" },
            { "fullName": "octo/b", "private": false, "workspaceId": null },
            { "name": "no full name" },
        ]}));
        assert_eq!(repos.len(), 2);
        assert_eq!((repos[0].private, repos[0].workspace_id.as_deref()), (true, Some("w1")));
        assert_eq!(repos[1].workspace_id, None);
        assert!(!repos[0].shared && !repos[0].read_only);
    }

    #[test]
    fn repositories_shared_with_the_account_say_what_it_may_do_with_them() {
        let repos = repos_from(&json!({ "ok": true, "repos": [
            { "fullName": "eve/own", "access": "owner" },
            { "fullName": "octo/team", "access": "write" },
            { "fullName": "octo/docs", "access": "read" },
        ]}));
        let flags: Vec<(bool, bool)> = repos.iter().map(|repo| (repo.shared, repo.read_only)).collect();
        assert_eq!(flags, [(false, false), (true, false), (true, true)]);
    }
}
