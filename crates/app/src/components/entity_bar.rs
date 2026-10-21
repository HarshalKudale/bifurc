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
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{App, ClickEvent, Context, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div};

use crate::{
    app_model::AppModel,
    components::lock_banner::lock_banner,
    i18n::t,
    model::{
        claims::Claim,
        git::FileState,
        tree::{FolderRow, folder_choices},
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityAction {
    Save,
    Close,
    Sync,
    Revert,
    Delete,
    /// Put it in the folder with this id, or at the top level.
    Move(Option<String>),
}

/// The folder an entity is in and the ones it could be in, for the entities that live in folders.
#[derive(Debug, Clone, Default)]
pub struct FolderPicker {
    pub current: Option<String>,
    /// Every folder: its id and its full path, in the order they are offered.
    pub choices: Vec<(String, String)>,
}

impl FolderPicker {
    pub fn new(current: Option<String>, folders: &[FolderRow]) -> Self {
        Self { current, choices: folder_choices(folders) }
    }

    fn label(&self) -> SharedString {
        self.current
            .as_deref()
            .and_then(|id| self.choices.iter().find(|(choice, _)| choice == id))
            .map_or(NO_FOLDER.into(), |(_, path)| path.clone().into())
    }
}

/// What a draft with no folder shows: the engine puts it in Default when it is created.
const NO_FOLDER: &str = "Default";

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
    /// The workspace is read-only: what is saved stays on this device, and nothing is synced.
    pub read_only: bool,
    /// A sync or a revert is in flight.
    pub busy: bool,
    /// The entity is in a tab that can be closed.
    pub closable: bool,
    pub deletable: bool,
    /// Where it is filed, for an entity kept in folders.
    pub folder: Option<FolderPicker>,
    /// Another session's claim on it: the entity is read-only here (its writes are refused) and says so.
    pub locked: Option<Claim>,
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
            read_only: model.is_read_only(),
            busy: model.is_publishing(),
            locked: saved_id.and_then(|id| model.claims().held_by_other(id).cloned()),
            deletable: true,
            ..Self::default()
        }
    }

    fn status(&self) -> (SharedString, bool) {
        if self.saving {
            return (t("components.entity_bar.status.saving").into(), false);
        }
        if let Some(problem) = &self.problem {
            return (problem.clone(), true);
        }
        if self.draft {
            return (t("components.entity_bar.status.draft").into(), false);
        }
        if self.unsaved {
            return (t("components.entity_bar.status.saving").into(), false);
        }
        let text = t(match self.git {
            Some(FileState::New | FileState::Modified | FileState::Deleted) if self.read_only => {
                "components.entity_bar.status.saved_read_only"
            }
            Some(FileState::New) => "components.entity_bar.status.saved_not_synced",
            Some(FileState::Modified) | Some(FileState::Deleted) => {
                "components.entity_bar.status.saved_changes_not_synced"
            }
            Some(FileState::Clean) => "components.entity_bar.status.saved_and_synced",
            None => "components.entity_bar.status.saved",
        });
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
    cx.listener(move |this, _: &ClickEvent, window, cx| on(this, action.clone(), window, cx))
}

/// The folder the entity is in, as a button that offers the others. Choosing one moves the entity: a saved
/// one with its next write, a draft when it is created.
fn folder_button<V: 'static>(picker: FolderPicker, on: &OnAction<V>, cx: &mut Context<V>) -> impl IntoElement {
    let view = cx.weak_entity();
    let on = on.clone();
    let label = picker.label();
    Button::new("entity-folder")
        .ghost()
        .small()
        .icon(IconName::Folder)
        .label(label)
        .tooltip(t("components.entity_bar.folder.tooltip"))
        .dropdown_menu(move |menu, _, _| {
            let item = |label: SharedString, target: Option<String>| {
                let (view, on) = (view.clone(), on.clone());
                PopupMenuItem::new(label).checked(picker.current == target).on_click(move |_, window, cx| {
                    let action = EntityAction::Move(target.clone());
                    view.update(cx, |this, cx| on(this, action, window, cx)).ok();
                })
            };
            // Default is always offered. Before the folder exists, choosing it asks the engine for it with no folder.
            let has_default = picker.choices.iter().any(|(_, path)| path == NO_FOLDER);
            let menu = if has_default { menu } else { menu.item(item(NO_FOLDER.into(), None)) };
            picker.choices.iter().fold(menu, |menu, (id, path)| menu.item(item(path.clone().into(), Some(id.clone()))))
        })
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
    let sync_tooltip = t(match (bar.read_only, bar.has_remote) {
        (true, _) => "components.entity_bar.sync.tooltip_read_only",
        (false, true) => "components.entity_bar.sync.tooltip_push",
        (false, false) => "components.entity_bar.sync.tooltip_commit",
    });

    let banner = bar.locked.clone().map(|claim| lock_banner(claim, cx));
    let footer = h_flex()
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
        .children(bar.folder.clone().map(|picker| folder_button(picker, &on, cx)))
        .child(
            Button::new("entity-delete")
                .ghost()
                .small()
                .icon(IconName::Delete)
                .label(t(if bar.draft {
                    "components.entity_bar.delete.discard"
                } else {
                    "components.entity_bar.delete.delete"
                }))
                .disabled(!bar.deletable || bar.locked.is_some())
                .on_click(listener(&on, EntityAction::Delete, cx)),
        )
        .child(
            Button::new("entity-revert")
                .ghost()
                .small()
                .label(t("components.entity_bar.revert.label"))
                .tooltip(t("components.entity_bar.revert.tooltip"))
                .disabled(!settled || bar.git != Some(FileState::Modified))
                .on_click(listener(&on, EntityAction::Revert, cx)),
        )
        .child(
            Button::new("entity-sync")
                .outline()
                .small()
                .label(t("components.entity_bar.sync.label"))
                .tooltip(sync_tooltip)
                .loading(bar.busy)
                .disabled(!settled || !pending || bar.read_only)
                .on_click(listener(&on, EntityAction::Sync, cx)),
        )
        .when(bar.closable, |this| {
            this.child(
                Button::new("entity-close").ghost().small().label(t("components.entity_bar.close")).on_click(listener(
                    &on,
                    EntityAction::Close,
                    cx,
                )),
            )
        })
        .child(
            Button::new("entity-save")
                .primary()
                .small()
                .label(t("components.entity_bar.save.label"))
                .tooltip(t(if bar.draft {
                    "components.entity_bar.save.tooltip_draft"
                } else {
                    "components.entity_bar.save.tooltip_autosave"
                }))
                .loading(bar.draft && bar.saving)
                .disabled(!bar.draft || bar.saving)
                .on_click(listener(&on, EntityAction::Save, cx)),
        );
    v_flex().flex_none().children(banner).child(footer)
}
