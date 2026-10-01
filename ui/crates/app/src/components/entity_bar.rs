//! The footer of an open entity: where it stands, and the things done to the entity as a whole.
//!
//! Every editor of a saved entity — a request tab, a mock tab, a proxy rule, an environment — ends in the same
//! bar, so the same actions are in the same place on every screen:
//!
//! - **Save** creates a draft. A saved entity has no Save step; its edits are written as they are made.
//! - **Sync** commits the entity's file, and pushes it when the workspace has a remote.
//! - **Revert** puts the entity's file back to the last synced version.
//! - **Delete** removes the entity; **Close** closes its tab.
//!
//! The bar holds no state. A screen describes its open entity with an [`EntityBar`] and is told which
//! [`EntityAction`] was asked for; the engine does the rest.

use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{App, ClickEvent, Context, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div};

use crate::{app_model::AppModel, model::git::FileState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityAction {
    Save,
    Close,
    Sync,
    Revert,
    Delete,
}

/// What the footer needs to know about the open entity.
#[derive(Debug, Clone, Default)]
pub struct EntityBar {
    /// Not created yet: Save is what creates it.
    pub draft: bool,
    /// A create or an automatic write is in flight.
    pub saving: bool,
    /// There are edits the engine does not have yet.
    pub unsaved: bool,
    /// Why the edits cannot be written as they stand, if they cannot.
    pub problem: Option<SharedString>,
    /// The entity's file against the last commit; `None` when it has no file to compare.
    pub git: Option<FileState>,
    pub has_remote: bool,
    /// A sync or a revert is in flight.
    pub busy: bool,
    /// The entity is in a tab that can be closed.
    pub closable: bool,
    pub deletable: bool,
}

impl EntityBar {
    /// The bar for the entity with `saved_id`, or for a draft when it has none. The editor adds what only it
    /// knows: `saving`, `unsaved`, `problem`, and whether it can be closed or deleted.
    pub fn of(model: &AppModel, saved_id: Option<&str>) -> Self {
        let git = model.git();
        Self {
            draft: saved_id.is_none(),
            git: saved_id.and_then(|id| git.entity_state(id)),
            has_remote: git.has_remote,
            busy: model.is_publishing(),
            deletable: true,
            ..Self::default()
        }
    }

    fn status(&self) -> (SharedString, bool) {
        if self.saving {
            return ("Saving…".into(), false);
        }
        if let Some(problem) = &self.problem {
            return (problem.clone(), true);
        }
        if self.draft {
            return ("Draft. Save to create it.".into(), false);
        }
        if self.unsaved {
            return ("Saving…".into(), false);
        }
        let text = match self.git {
            Some(FileState::New) => "Saved. Not synced yet.",
            Some(FileState::Modified) | Some(FileState::Deleted) => "Saved. Changes not synced.",
            Some(FileState::Clean) => "Saved and synced.",
            None => "Saved.",
        };
        (text.into(), false)
    }
}

type OnAction<V> = Rc<dyn Fn(&mut V, EntityAction, &mut Window, &mut Context<V>)>;

/// The click handler of the button for `action`.
fn listener<V: 'static>(
    on: &OnAction<V>,
    action: EntityAction,
    cx: &mut Context<V>,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let on = on.clone();
    cx.listener(move |this, _: &ClickEvent, window, cx| on(this, action, window, cx))
}

/// Renders the footer for `bar`. `on` runs on the view with the action the user asked for.
pub fn entity_bar<V: 'static>(
    bar: EntityBar,
    cx: &mut Context<V>,
    on: impl Fn(&mut V, EntityAction, &mut Window, &mut Context<V>) + 'static,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let on: OnAction<V> = Rc::new(on);
    let (status, is_problem) = bar.status();
    let pending = matches!(bar.git, Some(FileState::New | FileState::Modified | FileState::Deleted));
    let settled = !bar.draft && !bar.busy && !bar.saving && !bar.unsaved;
    let sync_tooltip = if bar.has_remote { "Commit this version and push it" } else { "Commit this version" };

    h_flex()
        .flex_none()
        .h_10()
        .px_3()
        .gap_2()
        .border_t_1()
        .border_color(theme.border)
        .bg(theme.background)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .text_sm()
                .text_color(if is_problem { theme.warning } else { theme.muted_foreground })
                .child(status),
        )
        .child(
            Button::new("entity-delete")
                .ghost()
                .small()
                .icon(IconName::Delete)
                .label(if bar.draft { "Discard" } else { "Delete" })
                .disabled(!bar.deletable)
                .on_click(listener(&on, EntityAction::Delete, cx)),
        )
        .child(
            Button::new("entity-revert")
                .ghost()
                .small()
                .label("Revert")
                .tooltip("Go back to the last synced version")
                .disabled(!settled || bar.git != Some(FileState::Modified))
                .on_click(listener(&on, EntityAction::Revert, cx)),
        )
        .child(
            Button::new("entity-sync")
                .outline()
                .small()
                .label("Sync")
                .tooltip(sync_tooltip)
                .loading(bar.busy)
                .disabled(!settled || !pending)
                .on_click(listener(&on, EntityAction::Sync, cx)),
        )
        .when(bar.closable, |this| {
            this.child(Button::new("entity-close").ghost().small().label("Close").on_click(listener(
                &on,
                EntityAction::Close,
                cx,
            )))
        })
        .child(
            Button::new("entity-save")
                .primary()
                .small()
                .label("Save")
                .tooltip(if bar.draft { "Create it" } else { "Changes are saved as you make them" })
                .loading(bar.draft && bar.saving)
                .disabled(!bar.draft || bar.saving)
                .on_click(listener(&on, EntityAction::Save, cx)),
        )
}
