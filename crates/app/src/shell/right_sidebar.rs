//! The right sidebar: reference panels about what is open, kept beside the work rather than over it.
//!
//! 1. **History** — the commits that touched the open entity's file; each opens to the diff it made there.
//! 2. **Snippet** — the open request written out as cURL, `fetch`, axios or Python, to copy elsewhere.
//! 3. **Types** — a type inferred from the open request's JSON responses ([`super::types_tab`]).
//! 4. **Variables** — what `{{name}}` resolves against: the active environment over the global one.
//!
//! Nothing here is editable. The shell tells it what is open ([`ActiveTarget`]); it reads the rest from the
//! model and asks the engine for the history, again whenever the engine says the history moved.

use std::collections::HashSet;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState},
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Div, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window, div, px,
};
use serde_json::json;

use super::{ActiveTarget, types_tab::TypesPanel};
use crate::{
    app_model::{AppModel, ModelEvent},
    components::{Syntax, commit_diffs::CommitDiffs, page},
    engine::{call, describe},
    i18n::{t, t_with},
    model::{
        git::{FileState, HistoryEntry, date_time_utc, parse_history},
        snippet::{SnippetKind, snippet},
        vault::VaultState,
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    History,
    Snippet,
    Types,
    Variables,
    Vault,
}

const PANELS: [Panel; 5] = [Panel::History, Panel::Snippet, Panel::Types, Panel::Variables, Panel::Vault];

impl Panel {
    fn label(self) -> &'static str {
        match self {
            Panel::History => t("shell.right_sidebar.panel_history"),
            Panel::Snippet => t("shell.right_sidebar.panel_snippet"),
            Panel::Types => t("types.panel"),
            Panel::Variables => t("shell.right_sidebar.panel_variables"),
            Panel::Vault => t("shell.right_sidebar.panel_vault"),
        }
    }
}

enum History {
    /// Nothing asked for yet, or nothing to ask about.
    Idle,
    Loading,
    Loaded(Vec<HistoryEntry>),
    Failed(String),
}

pub struct RightSidebar {
    pub(super) model: Entity<AppModel>,
    panel: Panel,
    pub(super) target: Option<ActiveTarget>,
    /// Whether the sidebar is shown; work is only done while it is.
    open: bool,
    pub(super) types: TypesPanel,
    snippet_kind: SnippetKind,
    snippet: Entity<EditorState>,
    history: History,
    /// The file the loaded (or loading) history belongs to; a result for any other file is stale.
    history_path: Option<String>,
    /// The commits whose diff is open.
    expanded: HashSet<String>,
    diffs: Entity<CommitDiffs>,
    _subscriptions: [Subscription; 3],
}

impl RightSidebar {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let snippet = cx.new(|cx| {
            let mut state =
                EditorState::new(window, cx).language(Syntax::Shell.language()).line_number(false).soft_wrap(true);
            state.set_readonly(true, cx);
            state
        });
        let diffs = cx.new(|_| CommitDiffs::new(model.clone()));
        let subscriptions = [
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.observe(&diffs, |_, _, cx| cx.notify()),
            cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| match event {
                ModelEvent::HistoryChanged => this.refresh_history(true, cx),
                ModelEvent::ResponseReceived(id)
                    if this.open && this.panel == Panel::Types && this.types_request().as_ref() == Some(id) =>
                {
                    this.generate_types(true, window, cx)
                }
                _ => {}
            }),
        ];
        Self {
            model,
            panel: Panel::History,
            target: None,
            open: false,
            types: TypesPanel::new(window, cx),
            snippet_kind: SnippetKind::Curl,
            snippet,
            history: History::Idle,
            history_path: None,
            expanded: HashSet::new(),
            diffs,
            _subscriptions: subscriptions,
        }
    }

    /// Points the sidebar at what is open now. Work is only done while the sidebar is `open`.
    pub fn set_target(
        &mut self,
        target: Option<ActiveTarget>,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.target = target;
        self.open = open;
        if open {
            self.refresh_snippet(window, cx);
            self.refresh_history(false, cx);
            if self.panel == Panel::Types {
                self.generate_types(false, window, cx);
            }
        }
        cx.notify();
    }

    fn refresh_snippet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .target
            .as_ref()
            .and_then(|target| target.request.as_ref())
            .map(|request| snippet(self.snippet_kind, request))
            .unwrap_or_default();
        if self.snippet.read(cx).value() != text {
            self.snippet.update(cx, |state, cx| state.set_value(text, window, cx));
        }
    }

    fn set_snippet_kind(&mut self, kind: SnippetKind, window: &mut Window, cx: &mut Context<Self>) {
        self.snippet_kind = kind;
        let syntax = match kind {
            SnippetKind::Curl => Syntax::Shell,
            SnippetKind::Fetch | SnippetKind::Axios => Syntax::JavaScript,
            SnippetKind::Python => Syntax::Python,
        };
        self.snippet.update(cx, |state, cx| state.set_highlighter(syntax.language(), cx));
        self.refresh_snippet(window, cx);
        cx.notify();
    }

    /// The open entity's file in the workspace repository, once the entity is saved.
    fn target_path(&self, cx: &gpui_kit::App) -> Option<String> {
        let id = self.target.as_ref()?.entity_id.as_deref()?;
        self.model.read(cx).git().entity_path(id).map(str::to_owned)
    }

    fn refresh_history(&mut self, force: bool, cx: &mut Context<Self>) {
        let path = self.target_path(cx);
        if path == self.history_path && !force {
            return;
        }
        // The same file read again keeps its list on screen until the new one arrives, and what is open
        // in it stays open.
        let same_file = path == self.history_path;
        if !same_file {
            self.expanded.clear();
            self.diffs.update(cx, |diffs, _| diffs.set_file(path.clone()));
        }
        self.history_path = path.clone();
        let Some(path) = path else {
            self.history = History::Idle;
            return;
        };
        if !(same_file && matches!(self.history, History::Loaded(_))) {
            self.history = History::Loading;
        }
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "relPath": path, "limit": 50 });
        call(&client, cx, "git.history", payload, move |this, outcome, cx| {
            // The answer is for the file that was open when it was asked; another may be open now.
            if this.history_path.as_deref() != Some(path.as_str()) {
                return;
            }
            this.history = match outcome {
                Ok(result) => History::Loaded(parse_history(&result)),
                Err(error) => History::Failed(describe(&error)),
            };
            cx.notify();
        });
    }

    /// Opens or closes a commit; opening one loads what it changed in the file the first time.
    fn toggle_commit(&mut self, commit: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(commit) {
            self.expanded.insert(commit.to_owned());
            self.diffs.update(cx, |diffs, cx| diffs.load(commit, cx));
        }
        cx.notify();
    }

    fn render_history(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let Some(target) = &self.target else {
            return page::empty(t("shell.right_sidebar.nothing_open"), t("shell.right_sidebar.open_for_commits"));
        };
        let state = target.entity_id.as_deref().and_then(|id| self.model.read(cx).git().entity_state(id));
        if target.entity_id.is_none() {
            return page::empty(t("shell.right_sidebar.not_saved_yet"), t("shell.right_sidebar.history_starts"));
        }
        let entries = match &self.history {
            History::Idle => {
                return page::empty(t("shell.right_sidebar.no_history"), t("shell.right_sidebar.not_tracked"));
            }
            History::Loading => return page::loading(t("shell.right_sidebar.loading_history")),
            History::Failed(message) => {
                return page::empty(t("shell.right_sidebar.history_failed"), message.clone());
            }
            History::Loaded(entries) => entries,
        };
        if entries.is_empty() {
            return page::empty(
                t("shell.right_sidebar.no_commits"),
                if state == Some(FileState::New) {
                    t("shell.right_sidebar.publish_first_commit")
                } else {
                    t("shell.right_sidebar.no_commit_touched")
                },
            );
        }
        v_flex()
            .id("history-entries")
            .size_full()
            .overflow_y_scroll()
            .children(entries.iter().map(|entry| {
                let open = self.expanded.contains(&entry.commit);
                let icon = if open { IconName::ChevronDown } else { IconName::ChevronRight };
                let title = if entry.message.is_empty() { &entry.entity_name } else { &entry.message };
                let commit = entry.commit.clone();
                let summary = v_flex()
                    .id(gpui_kit::ElementId::Name(format!("commit-{}", entry.commit).into()))
                    .w_full()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.muted))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_commit(&commit, cx)))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Icon::new(icon).small().text_color(theme.muted_foreground))
                            .child(Tag::secondary().small().child(entry.action.clone()))
                            .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().child(title.clone())),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .pl_6()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(
                                div()
                                    .font_family(theme.mono_font_family.clone())
                                    .child(entry.short_commit().to_owned()),
                            )
                            .child(t_with("shell.right_sidebar.utc_time", &[("time", &date_time_utc(entry.ts))]))
                            .child(entry.actor.clone()),
                    );
                v_flex().w_full().border_b_1().border_color(theme.border).child(summary).when(open, |this| {
                    this.child(div().px_3().pb_3().child(CommitDiffs::render(&self.diffs, &entry.commit, cx)))
                })
            }))
            .into_any_element()
    }

    fn render_snippet(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let has_request = self.target.as_ref().is_some_and(|target| target.request.is_some());
        if !has_request {
            return match &self.target {
                None => page::empty(t("shell.right_sidebar.nothing_open"), t("shell.right_sidebar.open_for_code")),
                Some(_) => page::empty(t("shell.right_sidebar.no_snippet"), t("shell.right_sidebar.no_http_call")),
            };
        }
        let selected = SnippetKind::ALL.iter().position(|kind| *kind == self.snippet_kind).unwrap_or(0);
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .px_3()
                    .py_2()
                    .gap_2()
                    // The language choice gives way before the copy button does: the sidebar can be narrow.
                    .child(
                        div().flex_1().min_w_0().overflow_hidden().child(
                            TabBar::new("snippet-kinds")
                                .segmented()
                                .small()
                                .selected_index(selected)
                                .on_click(cx.listener(|this, ix: &usize, window, cx| {
                                    this.set_snippet_kind(SnippetKind::ALL[*ix], window, cx)
                                }))
                                .children(SnippetKind::ALL.map(|kind| Tab::new().label(kind.label()))),
                        ),
                    )
                    .child(
                        div().flex_none().child(
                            Button::new("copy-snippet")
                                .ghost()
                                .small()
                                .icon(IconName::Copy)
                                .tooltip(t("shell.right_sidebar.copy"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let text = this.snippet.read(cx).value().to_string();
                                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                                })),
                        ),
                    ),
            )
            .child(div().flex_1().min_h_0().child(Editor::new(&self.snippet).bordered(false).h_full()))
            .into_any_element()
    }

    fn render_variables(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let config = self.model.read(cx).config();
        let rows = config.resolved_variables();
        if rows.is_empty() {
            return page::empty(t("shell.right_sidebar.no_variables"), t("shell.right_sidebar.add_variables"));
        }
        v_flex()
            .size_full()
            .child(table_header(
                [
                    t("shell.right_sidebar.col_variable"),
                    t("shell.right_sidebar.col_value"),
                    t("shell.right_sidebar.col_from"),
                ],
                cx,
            ))
            .child(v_flex().id("variables").flex_1().min_h_0().overflow_y_scroll().children(
                rows.into_iter().enumerate().map(|(ix, (variable, env))| {
                    table_row(cx)
                        .child(cell(variable.key.clone()).font_family(theme.mono_font_family.clone()))
                        .child(cell(variable.value.clone()))
                        .child(cell(env.name.clone()).text_color(theme.muted_foreground))
                        .child(copy_button(("copy-variable", ix), format!("{{{{{}}}}}", variable.key)))
                }),
            ))
            .into_any_element()
    }
}

/// The header of the Variables and Vault tables; its cells are as wide as the rows' ([`cell`]).
fn table_header(labels: [&'static str; 3], cx: &gpui_kit::App) -> Div {
    let theme = cx.theme();
    h_flex()
        .flex_none()
        .gap_3()
        .px_3()
        .py_2()
        .text_xs()
        .text_color(theme.muted_foreground)
        .border_b_1()
        .border_color(theme.border)
        .children(labels.map(cell))
        // As wide as the rows' copy button, so the columns line up.
        .child(div().flex_none().w_6())
}

/// Copies a row's reference, `{{name}}`, ready to paste into a request.
fn copy_button(id: impl Into<gpui_kit::ElementId>, reference: String) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .icon(IconName::Copy)
        .tooltip(t("shell.right_sidebar.copy_reference"))
        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(reference.clone())))
}

fn table_row(cx: &gpui_kit::App) -> Div {
    h_flex().w_full().gap_3().px_3().py_1p5().text_sm().border_b_1().border_color(cx.theme().border)
}

/// One column's share of a table row: the columns are equally wide, and a long value is cut short.
fn cell(content: impl IntoElement) -> Div {
    div().flex_1().min_w_0().overflow_hidden().text_ellipsis().child(content)
}

impl RightSidebar {
    /// The secrets `{{vault.NAME}}` can name: every vault's secret names, never values. A sealed vault's
    /// names are shown too, marked, because they resolve once it is unsealed.
    fn render_vault(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let vaults = self.model.read(cx).vaults().to_vec();
        if vaults.iter().all(|vault| vault.secrets.is_empty()) {
            return page::empty(t("shell.right_sidebar.no_secrets"), t("shell.right_sidebar.add_secrets"));
        }
        let mut rows = Vec::new();
        for vault in &vaults {
            let sealed = vault.state == VaultState::Sealed;
            for secret in &vault.secrets {
                let copy = vault.reference(secret);
                rows.push(
                    table_row(cx)
                        .id(gpui_kit::ElementId::Name(format!("vault-ref-{}-{secret}", vault.id).into()))
                        .when(sealed, |this| this.text_color(theme.muted_foreground))
                        .child(cell(secret.clone()).font_family(theme.mono_font_family.clone()))
                        .child(cell(vault.name.clone()))
                        .child(cell(vault.state.label()).text_color(theme.muted_foreground))
                        .child(copy_button(rows.len(), copy.clone()))
                        // A click copies the reference, `{{vault.SECRET}}`.
                        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))),
                );
            }
        }
        v_flex()
            .size_full()
            .child(table_header(
                [
                    t("shell.right_sidebar.col_secret"),
                    t("shell.right_sidebar.panel_vault"),
                    t("shell.right_sidebar.col_state"),
                ],
                cx,
            ))
            .child(v_flex().id("vault-secrets").flex_1().min_h_0().overflow_y_scroll().children(rows))
            .into_any_element()
    }
}

impl Render for RightSidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let selected = PANELS.iter().position(|panel| *panel == self.panel).unwrap_or(0);
        let body = match self.panel {
            Panel::History => self.render_history(cx),
            Panel::Snippet => self.render_snippet(cx),
            Panel::Types => self.render_types(cx),
            Panel::Variables => self.render_variables(cx),
            Panel::Vault => self.render_vault(cx),
        };
        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            // The strip is as tall as the top bar beside it, and its tabs fill it: the kit's tabs come in
            // fixed heights, none of them the bar's, so these are drawn here in the same colors.
            .child(h_flex().flex_none().h_10().bg(theme.tab_bar).border_b_1().border_color(theme.border).children(
                PANELS.iter().enumerate().map(|(ix, panel)| {
                    let active = ix == selected;
                    h_flex()
                        .id(("inspector-panel", ix))
                        .test_support()
                        .h_full()
                        .px_3()
                        .text_sm()
                        .border_r_1()
                        .border_color(theme.border)
                        .map(|this| {
                            if active {
                                // The active tab reaches over the strip's bottom border into its panel.
                                this.bg(theme.tab_active).text_color(theme.tab_active_foreground).mb(px(-1.))
                            } else {
                                this.text_color(theme.tab_foreground)
                                    .hover(|this| this.text_color(theme.tab_active_foreground))
                            }
                        })
                        .child(panel.label())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.panel = PANELS[ix];
                            match this.panel {
                                Panel::History => this.refresh_history(true, cx),
                                Panel::Types => this.generate_types(false, window, cx),
                                _ => {}
                            }
                            cx.notify();
                        }))
                }),
            ))
            .child(div().flex_1().min_h_0().child(body))
    }
}
