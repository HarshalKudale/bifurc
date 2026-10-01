//! The right sidebar: reference panels about what is open, kept beside the work rather than over it.
//!
//! 1. **History** — the commits that touched the open entity's file.
//! 2. **Snippet** — the open request written out as cURL, `fetch`, axios or Python, to copy elsewhere.
//! 3. **Variables** — what `{{name}}` resolves against: the active environment over the global one.
//!
//! Nothing here is editable. The shell tells it what is open ([`ActiveTarget`]); it reads the rest from the
//! model and asks the engine for the history.

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState},
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::json;

use super::ActiveTarget;
use crate::{
    app_model::AppModel,
    components::{Syntax, page},
    engine::{call, describe},
    model::{
        git::{FileState, HistoryEntry, date_time_utc, parse_history},
        snippet::{SnippetKind, snippet},
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    History,
    Snippet,
    Variables,
}

const PANELS: [Panel; 3] = [Panel::History, Panel::Snippet, Panel::Variables];

enum History {
    /// Nothing asked for yet, or nothing to ask about.
    Idle,
    Loading,
    Loaded(Vec<HistoryEntry>),
    Failed(String),
}

pub struct RightSidebar {
    model: Entity<AppModel>,
    panel: Panel,
    target: Option<ActiveTarget>,
    snippet_kind: SnippetKind,
    snippet: Entity<EditorState>,
    history: History,
    /// The file the loaded (or loading) history belongs to; a result for any other file is stale.
    history_path: Option<String>,
    _subscription: Subscription,
}

impl RightSidebar {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let snippet = cx.new(|cx| {
            let mut state =
                EditorState::new(window, cx).language(Syntax::Shell.language()).line_number(false).soft_wrap(true);
            state.set_readonly(true, cx);
            state
        });
        let subscription = cx.observe(&model, |_, _, cx| cx.notify());
        Self {
            model,
            panel: Panel::History,
            target: None,
            snippet_kind: SnippetKind::Curl,
            snippet,
            history: History::Idle,
            history_path: None,
            _subscription: subscription,
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
        if open {
            self.refresh_snippet(window, cx);
            self.refresh_history(false, cx);
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
        self.history_path = path.clone();
        let Some(path) = path else {
            self.history = History::Idle;
            return;
        };
        self.history = History::Loading;
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

    fn render_history(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let Some(target) = &self.target else {
            return page::empty("Nothing open", "Open a request to see the commits that changed it.");
        };
        let state = target.entity_id.as_deref().and_then(|id| self.model.read(cx).git().entity_state(id));
        if target.entity_id.is_none() {
            return page::empty("Not saved yet", "History starts when the request is saved and published.");
        }
        let entries = match &self.history {
            History::Idle => return page::empty("No history", "This item is not tracked in the workspace repository."),
            History::Loading => return page::loading("Loading history…"),
            History::Failed(message) => return page::empty("Couldn't load the history", message.clone()),
            History::Loaded(entries) => entries,
        };
        if entries.is_empty() {
            return page::empty(
                "No commits yet",
                if state == Some(FileState::New) {
                    "Publish this item from the footer to record its first commit."
                } else {
                    "No commit has touched this item."
                },
            );
        }
        v_flex()
            .id("history-entries")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .gap_3()
            .children(entries.iter().map(|entry| {
                v_flex()
                    .id(gpui_kit::ElementId::Name(format!("commit-{}", entry.commit).into()))
                    .gap_1()
                    .child(h_flex().gap_2().child(Tag::secondary().small().child(entry.action.clone())).child(
                        div().flex_1().min_w_0().overflow_hidden().text_ellipsis().child(entry.entity_name.clone()),
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(
                                div()
                                    .font_family(theme.mono_font_family.clone())
                                    .child(entry.short_commit().to_owned()),
                            )
                            .child(format!("{} UTC", date_time_utc(entry.ts)))
                            .child(entry.actor.clone()),
                    )
            }))
            .into_any_element()
    }

    fn render_snippet(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let has_request = self.target.as_ref().is_some_and(|target| target.request.is_some());
        if !has_request {
            return match &self.target {
                None => page::empty("Nothing open", "Open a request to get it as code."),
                Some(_) => page::empty("No snippet", "This kind of request can't be written as an HTTP call."),
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
                    .child(div().flex_none().child(
                        Button::new("copy-snippet").ghost().small().icon(IconName::Copy).tooltip("Copy").on_click(
                            cx.listener(|this, _, _, cx| {
                                let text = this.snippet.read(cx).value().to_string();
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                            }),
                        ),
                    )),
            )
            .child(div().flex_1().min_h_0().child(Editor::new(&self.snippet).bordered(false).h_full()))
            .into_any_element()
    }

    fn render_variables(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let config = self.model.read(cx).config();
        let active = config.active_environment().filter(|env| !env.is_global());
        let global = config.global_environment();
        let overridden = |key: &str| active.is_some_and(|env| env.variables.iter().any(|v| v.key == key));

        let section = |title: String, rows: Vec<(String, String, bool)>, empty: &'static str| {
            v_flex()
                .gap_2()
                .child(page::section_label(title, cx))
                .children(rows.is_empty().then(|| div().text_sm().text_color(theme.muted_foreground).child(empty)))
                .children(rows.into_iter().map(|(key, value, shadowed)| {
                    v_flex()
                        .gap_0p5()
                        .text_sm()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div().font_family(theme.mono_font_family.clone()).child(format!("{{{{{key}}}}}")),
                                )
                                .children(shadowed.then(|| Tag::secondary().small().child("Overridden"))),
                        )
                        .child(
                            div()
                                .text_color(theme.muted_foreground)
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(if value.is_empty() { "(empty)".to_owned() } else { value }),
                        )
                }))
        };

        let active_rows = active
            .map(|env| env.variables.iter().map(|v| (v.key.clone(), v.value.clone(), false)).collect::<Vec<_>>())
            .unwrap_or_default();
        let global_rows = global
            .map(|env| {
                env.variables.iter().map(|v| (v.key.clone(), v.value.clone(), overridden(&v.key))).collect::<Vec<_>>()
            })
            .unwrap_or_default();

        v_flex()
            .id("variables")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .gap_6()
            .child(match active {
                Some(env) => {
                    section(env.name.clone(), active_rows, "This environment has no variables.").into_any_element()
                }
                None => v_flex()
                    .gap_2()
                    .child(page::section_label("Environment", cx))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("No environment is selected. Choose one in the title bar."),
                    )
                    .into_any_element(),
            })
            .child(section("Global".to_owned(), global_rows, "There are no global variables."))
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
            Panel::Variables => self.render_variables(cx),
        };
        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .child(
                h_flex().flex_none().h_10().px_2().border_b_1().border_color(theme.border).child(
                    TabBar::new("inspector-panels")
                        .underline()
                        .small()
                        .selected_index(selected)
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.panel = PANELS[*ix];
                            if this.panel == Panel::History {
                                this.refresh_history(true, cx);
                            }
                            cx.notify();
                        }))
                        .child(Tab::new().label("History"))
                        .child(Tab::new().label("Snippet"))
                        .child(Tab::new().label("Variables")),
                ),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}
