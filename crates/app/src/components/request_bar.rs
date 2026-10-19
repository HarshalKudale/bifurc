//! The first row of every request editor, so that the commands of a request are in the same place whatever its
//! type: the address, the primary command right after it (Send, Invoke, Connect, Disconnect), and after that the
//! commands a live request adds (Send message, End streaming).

use gpui_kit::component::{button::Button, h_flex};
use gpui_kit::{AnyElement, App, Div, IntoElement, ParentElement as _, Styled as _, div};

use super::VarEditor;

/// The row. `lead` is what stands before the address, such as a method picker; `secondary` are the commands of
/// a live request, in the order they are shown. Every button is as tall as the address.
pub fn request_bar(
    lead: Option<AnyElement>,
    address: impl IntoElement,
    primary: Button,
    secondary: impl IntoIterator<Item = Button>,
    cx: &App,
) -> Div {
    let row_height = VarEditor::row_height(cx);
    h_flex()
        .flex_none()
        .gap_2()
        .px_3()
        .py_2()
        .children(lead)
        .child(div().flex_1().min_w_0().child(address))
        .child(primary.h(row_height))
        .children(secondary.into_iter().map(|button| button.h(row_height)))
}
