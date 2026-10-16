//! The short decisions every screen asks for the same way: naming something, and confirming a deletion.
//!
//! Each is a dialog over the window. It collects text or a yes and hands it to the view that asked; that
//! view issues the engine command. Nothing is edited locally and written back.

use std::rc::Rc;

use gpui_kit::component::{
    WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    dialog::{DialogButtonProps, DialogFooter},
    form::{Field, Form},
    input::{Input, InputState},
    notification::Notification,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, ParentElement as _, SharedString, Styled as _, WeakEntity, Window,
};

use crate::{
    app_model::{AppModel, Deleted},
    commands::ShowSettings,
    i18n::{t, t_with},
    model::git::FileState,
};

/// One text field of a [`prompt`]: its label, starting text and placeholder.
pub struct PromptField {
    pub label: &'static str,
    pub initial: String,
    pub placeholder: &'static str,
    /// An optional field may be left empty; the others must be filled before the dialog confirms.
    pub optional: bool,
    /// Typed text is hidden (a passphrase, a secret's value).
    pub secret: bool,
}

impl PromptField {
    pub fn new(label: &'static str, placeholder: &'static str) -> Self {
        Self { label, initial: String::new(), placeholder, optional: false, secret: false }
    }

    pub fn initial(mut self, initial: impl Into<String>) -> Self {
        self.initial = initial.into();
        self
    }

    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    pub fn secret(mut self) -> Self {
        self.secret = true;
        self
    }
}

/// Opens a dialog with `fields`. `submit` runs on `view` with their trimmed values, in order, when the user
/// confirms by the button or by Enter in a field.
pub fn prompt<V: 'static>(
    view: WeakEntity<V>,
    window: &mut Window,
    cx: &mut App,
    title: impl Into<SharedString>,
    confirm: &'static str,
    fields: Vec<PromptField>,
    submit: impl Fn(&mut V, Vec<String>, &mut Window, &mut Context<V>) + 'static,
) {
    let title = title.into();
    type PromptInput = (&'static str, bool, bool, Entity<InputState>);
    let inputs: Rc<Vec<PromptInput>> = Rc::new(
        fields
            .into_iter()
            .map(|field| {
                let secret = field.secret;
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(field.placeholder)
                        .default_value(field.initial)
                        .masked(secret)
                });
                (field.label, field.optional, field.secret, input)
            })
            .collect(),
    );
    let submit = Rc::new(submit);

    // The dialog confirms on its button and on Enter in a field; both arrive here, outside any update of
    // the asking view, so the view is reached through its handle.
    let confirm_now = {
        let inputs = inputs.clone();
        Rc::new(move |window: &mut Window, cx: &mut App| {
            let values: Vec<String> =
                inputs.iter().map(|(_, _, _, input)| input.read(cx).value().trim().to_owned()).collect();
            let missing = inputs.iter().zip(&values).any(|((_, optional, _, _), value)| !optional && value.is_empty());
            if missing {
                window.push_notification(
                    Notification::warning(t("components.dialogs.fill_required"))
                        .title(t("components.dialogs.cant_continue")),
                    cx,
                );
                return false;
            }
            view.update(cx, |view, cx| submit(view, values, window, cx)).ok();
            true
        })
    };

    // Deferred so a dialog opened by a command chosen from the palette opens after the palette closes.
    window.defer(cx, move |window, cx| {
        let first = inputs.first().map(|(_, _, _, input)| input.clone());
        window.open_dialog(cx, {
            let inputs = inputs.clone();
            let confirm_now = confirm_now.clone();
            move |dialog, _, _| {
                let confirm_now = confirm_now.clone();
                dialog
                    .title(title.clone())
                    .w_96()
                    .child(inputs.iter().fold(Form::new(), |form, (label, _, secret, input)| {
                        // The kit's medium input leaves less room than a line of text needs; a little less
                        // vertical padding keeps descenders from being clipped.
                        form.child(Field::new().label(*label).child(if *secret {
                            Input::new(input).py_1p5().mask_toggle()
                        } else {
                            Input::new(input).py_1p5()
                        }))
                    }))
                    // Enter in a field confirms through the dialog; the footer gives the same two choices to
                    // the pointer. A plain dialog draws no buttons of its own.
                    .button_props(DialogButtonProps::default().on_ok({
                        let confirm_now = confirm_now.clone();
                        move |_, window, cx| confirm_now(window, cx)
                    }))
                    .footer(
                        DialogFooter::new()
                            .child(
                                Button::new("prompt-cancel")
                                    .outline()
                                    .label(t("components.cancel"))
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                            .child(Button::new("prompt-confirm").primary().label(confirm).on_click(
                                move |_, window, cx| {
                                    if confirm_now(window, cx) {
                                        window.close_dialog(cx);
                                    }
                                },
                            )),
                    )
            }
        });
        if let Some(input) = first {
            window.defer(cx, move |window, cx| input.update(cx, |state, cx| state.focus(window, cx)));
        }
    });
}

/// Asks before something is deleted or discarded. `title` names the object; `consequence` says only what the
/// title does not.
pub fn confirm_danger<V: 'static>(
    view: WeakEntity<V>,
    window: &mut Window,
    cx: &mut App,
    title: impl Into<SharedString>,
    consequence: impl Into<SharedString>,
    ok_text: &'static str,
    on_ok: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
) {
    confirm_with(ButtonVariant::Danger, view, window, cx, title, consequence, ok_text, on_ok);
}

/// What deleting a saved entity does, for the dialog that asks first. `state` is its file against the last
/// sync, as the model's git status has it.
pub fn delete_consequence(state: Option<FileState>) -> &'static str {
    match state {
        // Never synced: nothing has a copy of it.
        Some(FileState::New) | None => t("components.dialogs.never_synced"),
        Some(_) => t("components.dialogs.stays_struck"),
    }
}

/// What deleting a folder does, for the dialog that asks first.
pub const FOLDER_DELETE_CONSEQUENCE: &str = "components.dialogs.folder_delete_consequence";

/// Opened in place of something that is deleted: it can't be opened, only brought back or deleted for good.
pub fn resolve_deleted(
    model: Entity<AppModel>,
    target: Deleted,
    name: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    let name = name.into();
    let folder = matches!(target, Deleted::Folder { .. });
    window.defer(cx, move |window, cx| {
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (revert_model, revert_target) = (model.clone(), target.clone());
            let (confirm_model, confirm_target) = (model.clone(), target.clone());
            alert
                .title(t_with("components.dialogs.deleted_title", &[("name", &name)]))
                .description(if folder {
                    t("components.dialogs.deleted_folder")
                } else {
                    t("components.dialogs.deleted_item")
                })
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("deleted-cancel")
                                .outline()
                                .label(t("components.cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("deleted-revert")
                                .outline()
                                .label(t("components.dialogs.revert_delete"))
                                .on_click(move |_, window, cx| {
                                    revert_model.update(cx, |model, cx| model.revert_delete(revert_target.clone(), cx));
                                    window.close_dialog(cx);
                                }),
                        )
                        .child(
                            Button::new("deleted-confirm")
                                .danger()
                                .label(t("components.dialogs.confirm_delete"))
                                .on_click(move |_, window, cx| {
                                    confirm_model
                                        .update(cx, |model, cx| model.confirm_delete(confirm_target.clone(), cx));
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
    });
}

/// Opened in place of something a newer Bifurc than this one last saved: it can't be opened, changed or
/// deleted here until this install is `version` or later. Settings is where an update is found.
pub fn needs_update(
    name: impl Into<SharedString>,
    version: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    let (name, version) = (name.into(), version.into());
    window.defer(cx, move |window, cx| {
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .title(t_with("components.dialogs.needs_update_title", &[("name", &name), ("version", &version)]))
                .description(t("components.dialogs.needs_update"))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("needs-update-cancel")
                                .outline()
                                .label(t("components.cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("needs-update-settings")
                                .primary()
                                .label(t("components.dialogs.open_settings"))
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                    window.dispatch_action(Box::new(ShowSettings), cx);
                                }),
                        ),
                )
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn confirm_with<V: 'static>(
    variant: ButtonVariant,
    view: WeakEntity<V>,
    window: &mut Window,
    cx: &mut App,
    title: impl Into<SharedString>,
    consequence: impl Into<SharedString>,
    ok_text: &'static str,
    on_ok: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
) {
    let (title, consequence) = (title.into(), consequence.into());
    let on_ok = Rc::new(on_ok);
    window.defer(cx, move |window, cx| {
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (view, on_ok) = (view.clone(), on_ok.clone());
            alert
                .title(title.clone())
                .description(consequence.clone())
                .confirm()
                .ok_text(ok_text)
                .ok_variant(variant)
                .on_ok(move |_, window, cx| {
                    view.update(cx, |view, cx| on_ok(view, window, cx)).ok();
                    true
                })
        });
    });
}
