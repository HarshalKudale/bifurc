//! What commits changed, as diffs: the part of the Audit log and of the History panel that opens under a
//! change.
//!
//! The engine writes the patch (`history.patch`); this keeps one diff per commit that was opened. A commit
//! never changes, so what was loaded for it stays good until the workspace or the file it is about does.

use std::collections::HashMap;

use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    diff::{Diff, DiffFile, DiffMode, DiffState},
    h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement as _, ParentElement as _, Styled as _, div, rems,
};
use serde_json::{Value, json};

use crate::{
    app_model::AppModel,
    engine::{call, failure},
    i18n::{t, t_with},
};

enum Load {
    Loading,
    /// The diff, and how many rows it starts with: what its height is taken from.
    Ready(Entity<DiffState>, usize),
    /// The commit changed nothing here: it only moved a file, or touched other files.
    Empty,
    Failed(String),
}

pub struct CommitDiffs {
    model: Entity<AppModel>,
    /// When set, only this entity's file is shown of each commit.
    file_path: Option<String>,
    /// Side by side or unified: one choice for every diff shown.
    mode: DiffMode,
    loads: HashMap<String, Load>,
}

/// The rows a diff of `files` starts with: its changed lines, and around each file a header and some context.
fn rows(files: &[DiffFile]) -> usize {
    files.iter().map(|file| file.additions() + file.deletions() + 14).sum()
}

impl CommitDiffs {
    pub fn new(model: Entity<AppModel>) -> Self {
        Self { model, file_path: None, mode: DiffMode::default(), loads: HashMap::new() }
    }

    /// Forgets what was loaded: it belonged to another workspace.
    pub fn clear(&mut self) {
        self.loads.clear();
    }

    fn set_mode(&mut self, mode: DiffMode, cx: &mut Context<Self>) {
        self.mode = mode;
        for load in self.loads.values() {
            if let Load::Ready(state, _) = load {
                state.update(cx, |state, cx| state.set_mode(mode, cx));
            }
        }
        cx.notify();
    }

    /// Narrows every diff to the file at `path`, or shows whole commits with `None`.
    pub fn set_file(&mut self, path: Option<String>) {
        if self.file_path != path {
            self.file_path = path;
            self.loads.clear();
        }
    }

    /// Asks the engine what `commit` changed, unless that is known or on its way.
    pub fn load(&mut self, commit: &str, cx: &mut Context<Self>) {
        if matches!(self.loads.get(commit), Some(Load::Loading | Load::Ready(..) | Load::Empty)) {
            return;
        }
        self.loads.insert(commit.to_owned(), Load::Loading);
        cx.notify();
        let model = self.model.read(cx);
        let client = model.client().clone();
        let mut payload = json!({ "workspaceId": model.workspace_id(), "commitHash": commit });
        if let Some(path) = &self.file_path {
            payload["filePath"] = json!(path);
        }
        let commit = commit.to_owned();
        call(&client, cx, "history.patch", payload, move |this, outcome, cx| {
            // Cleared since: the answer is about a workspace or a file that is no longer the one shown.
            if !matches!(this.loads.get(&commit), Some(Load::Loading)) {
                return;
            }
            let load = match failure(&outcome) {
                Some(message) => Load::Failed(message),
                None => {
                    let patch = outcome.ok().and_then(|result| result.get("patch").and_then(Value::as_str).map(str::to_owned));
                    match DiffFile::parse(&patch.unwrap_or_default()) {
                        Ok(files) if files.is_empty() => Load::Empty,
                        Ok(files) => {
                            let rows = rows(&files);
                            let mode = this.mode;
                            Load::Ready(cx.new(|cx| DiffState::new(files, cx).with_mode(mode)), rows)
                        }
                        Err(error) => Load::Failed(error.to_string()),
                    }
                }
            };
            this.loads.insert(commit, load);
            cx.notify();
        });
    }

    /// What `commit` changed, or where the answer stands.
    pub fn render(this: &Entity<Self>, commit: &str, cx: &App) -> AnyElement {
        let diffs = this.read(cx);
        let theme = cx.theme();
        let note = |text: String| div().text_sm().text_color(theme.muted_foreground).child(text).into_any_element();
        match diffs.loads.get(commit) {
            None | Some(Load::Loading) => note(t("components.commit_diffs.loading").to_owned()),
            Some(Load::Failed(message)) => note(t_with("components.commit_diffs.failed", &[("message", &message)])),
            Some(Load::Empty) => note(t("components.commit_diffs.no_change").to_owned()),
            // The diff scrolls inside its own box, so the box is as tall as a short diff and no taller
            // than leaves the list around it in view.
            Some(Load::Ready(state, rows)) => {
                let option = |id: String, label: &'static str, mode: DiffMode| {
                    let this = this.clone();
                    Button::new(id)
                        .label(label)
                        .ghost()
                        .xsmall()
                        .selected(diffs.mode == mode)
                        .on_click(move |_, _, cx| this.update(cx, |diffs, cx| diffs.set_mode(mode, cx)))
                };
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(option(
                                format!("diff-unified-{commit}"),
                                t("components.commit_diffs.unified"),
                                DiffMode::Unified,
                            ))
                            .child(option(
                                format!("diff-split-{commit}"),
                                t("components.commit_diffs.side_by_side"),
                                DiffMode::Split,
                            )),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(rems((*rows as f32 * 1.4).clamp(6., 28.)))
                            .rounded_md()
                            .border_1()
                            .border_color(theme.border)
                            .overflow_hidden()
                            .child(Diff::new(state).soft_wrap(true).size_full()),
                    )
                    .into_any_element()
            }
        }
    }
}
