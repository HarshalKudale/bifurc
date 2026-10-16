//! The two list-and-detail layouts the screens share, so pane widths, borders and backgrounds agree.
//!
//! `master_detail` is a list pane beside a detail pane (environments, proxy rules, vaults, flows).
//! `list_over_detail` is a list with a resizable detail panel under it that appears on selection (capture,
//! health bar). Rows, detail content and empty states stay with the screen.

use gpui_kit::component::{
    ActiveTheme as _, h_flex,
    resizable::{resizable_panel, v_resizable},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, Stateful,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

/// A scrolling column for `ListItem` rows, to be passed as the `list` of [`master_detail`].
pub fn list_rows(id: impl Into<ElementId>) -> Stateful<Div> {
    v_flex().id(id).size_full().overflow_y_scroll().p_2().gap_1()
}

/// A fixed-width list pane on the left, the detail filling the rest. `list` is `None` to hide the pane. The
/// detail owns its own empty state. A list that scrolls itself (a tree) is passed as is.
pub fn master_detail(list: Option<impl IntoElement>, detail: impl IntoElement, cx: &App) -> Div {
    let theme = cx.theme();
    h_flex()
        .size_full()
        .items_stretch()
        .bg(theme.background)
        .children(list.map(|list| {
            div().w_64().flex_none().h_full().border_r_1().border_color(theme.border).bg(theme.sidebar).child(list)
        }))
        .child(div().flex_1().min_w_0().child(detail))
}

/// A list filling the region, with a resizable detail panel below it while `detail` is `Some`.
pub fn list_over_detail(
    id: impl Into<ElementId>,
    list: impl IntoElement,
    detail: Option<AnyElement>,
) -> impl IntoElement {
    let visible = detail.is_some();
    // The panel's size is a physical starting point the user resizes, not part of the rem scale.
    v_resizable(id)
        .child(resizable_panel().child(list))
        .child(resizable_panel().visible(visible).size(px(360.)).size_range(px(200.)..px(900.)).children(detail))
}
