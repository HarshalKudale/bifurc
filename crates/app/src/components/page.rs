//! Furniture every screen uses the same way: the header band, empty and loading states.
//!
//! Screens share these so their titles, toolbars and placeholders sit on the same lines with the same
//! insets, instead of each inventing a slightly different one.

use gpui_kit::component::{
    ActiveTheme as _, StyledExt as _,
    empty::{Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyTitle},
    h_flex,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, Div, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _,
    TestSupportExt as _, div,
};

/// The band at the top of a screen: what the screen is, one line about it, and its actions on the trailing
/// edge.
pub fn header(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    actions: impl IntoElement,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    h_flex()
        .flex_none()
        .gap_3()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(theme.border)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().font_semibold().child(title.into()))
                .child(div().text_sm().text_color(theme.muted_foreground).child(description.into())),
        )
        .child(h_flex().flex_none().gap_2().child(actions))
}

/// A screen: header above, body filling the rest.
pub fn screen(header: impl IntoElement, body: impl IntoElement, cx: &App) -> Div {
    v_flex().size_full().bg(cx.theme().background).child(header).child(div().flex_1().min_h_0().child(body))
}

/// Centers a placeholder in whatever region it stands in for.
fn centered(content: impl IntoElement) -> AnyElement {
    // Observable by UI tests (inert in a normal build): every screen's empty state is found by this id.
    div()
        .id("empty-state")
        .test_support()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(content)
        .into_any_element()
}

pub fn empty(title: impl Into<SharedString>, description: impl Into<SharedString>) -> AnyElement {
    centered(
        Empty::new().header(
            EmptyHeader::new()
                .title(EmptyTitle::new().child(title.into()))
                .description(EmptyDescription::new().child(description.into())),
        ),
    )
}

/// An empty state that offers the next step.
pub fn empty_with_action(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    action: impl IntoElement,
) -> AnyElement {
    centered(
        Empty::new()
            .header(
                EmptyHeader::new()
                    .title(EmptyTitle::new().child(title.into()))
                    .description(EmptyDescription::new().child(description.into())),
            )
            .content(EmptyContent::new().child(action)),
    )
}

pub fn loading(text: impl Into<SharedString>) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(Spinner::new())
        .child(text.into())
        .into_any_element()
}

/// A short label above a group of related content inside a screen or an inspector.
pub fn section_label(text: impl Into<SharedString>, cx: &App) -> Div {
    div().text_xs().font_medium().text_color(cx.theme().muted_foreground).child(text.into())
}
