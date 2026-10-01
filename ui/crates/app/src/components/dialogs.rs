//! The short decisions every screen asks for the same way: naming something, and confirming a deletion.
//!
//! Each is a dialog over the window. It collects text or a yes and hands it to the view that asked; that
//! view issues the engine command. Nothing is edited locally and written back.

use std::rc::Rc;

use gpui_kit::component::{
    WindowExt as _,
    button::ButtonVariant,
    dialog::DialogButtonProps,
    form::{Field, Form},
    input::{Input, InputState},
    notification::Notification,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, ParentElement as _, SharedString, Styled as _, WeakEntity, Window,
};

/// One text field of a [`prompt`]: its label, starting text and placeholder.
pub struct PromptField {
    pub label: &'static str,
    pub initial: String,
    pub placeholder: &'static str,
    /// An optional field may be left empty; the others must be filled before the dialog confirms.
    pub optional: bool,
}

impl PromptField {
    pub fn new(label: &'static str, placeholder: &'static str) -> Self {
        Self { label, initial: String::new(), placeholder, optional: false }
    }

    pub fn initial(mut self, initial: impl Into<String>) -> Self {
        self.initial = initial.into();
        self
    }

    pub fn optional(mut self) -> Self {
        self.optional = true;
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
    let inputs: Rc<Vec<(&'static str, bool, Entity<InputState>)>> = Rc::new(
        fields
            .into_iter()
            .map(|field| {
                let input = cx
                    .new(|cx| InputState::new(window, cx).placeholder(field.placeholder).default_value(field.initial));
                (field.label, field.optional, input)
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
                inputs.iter().map(|(_, _, input)| input.read(cx).value().trim().to_owned()).collect();
            let missing = inputs.iter().zip(&values).any(|((_, optional, _), value)| !optional && value.is_empty());
            if missing {
                window.push_notification(
                    Notification::warning("Fill in the required fields.").title("Can't continue yet"),
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
        let first = inputs.first().map(|(_, _, input)| input.clone());
        window.open_dialog(cx, {
            let inputs = inputs.clone();
            let confirm_now = confirm_now.clone();
            move |dialog, _, _| {
                let confirm_now = confirm_now.clone();
                dialog
                    .title(title.clone())
                    .w_96()
                    .child(inputs.iter().fold(Form::new(), |form, (label, _, input)| {
                        // The kit's medium input leaves less room than a line of text needs; a little less
                        // vertical padding keeps descenders from being clipped.
                        form.child(Field::new().label(*label).child(Input::new(input).py_1p5()))
                    }))
                    .button_props(
                        DialogButtonProps::default()
                            .ok_text(confirm)
                            .show_cancel(true)
                            .on_ok(move |_, window, cx| confirm_now(window, cx)),
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
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    view.update(cx, |view, cx| on_ok(view, window, cx)).ok();
                    true
                })
        });
    });
}
