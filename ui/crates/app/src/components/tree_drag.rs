//! Dragging a row of a folder tree onto a folder, to move it there.
//!
//! The tree decides what may go where and what a drop does; this is the gesture: the row that is picked up,
//! the chip that follows the pointer, and the look of a place that takes the drop. What is dragged is a
//! [`DraggedRow`] of the tree's own key type, so a row of one tree cannot be dropped in another.

use gpui_kit::component::{ActiveTheme as _, list::ListItem};
use gpui_kit::{
    App, AppContext as _, Context, InteractiveElement, IntoElement, ParentElement as _, Pixels, Point, Render,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Window, div,
};

/// A tree row on its way to another folder.
pub struct DraggedRow<K> {
    pub key: K,
    label: SharedString,
}

/// What follows the pointer during the drag: the name of the row.
struct DragChip {
    label: SharedString,
    /// Where in the row the pointer took hold. The chip is drawn from the row's corner, so it is pushed
    /// back by this much to sit at the pointer.
    offset: Point<Pixels>,
}

impl Render for DragChip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div().pl(self.offset.x).pt(self.offset.y).child(
            div()
                .px_2()
                .py_1()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .bg(theme.popover)
                .text_color(theme.popover_foreground)
                .text_sm()
                .shadow_md()
                .child(self.label.clone()),
        )
    }
}

/// Lets the row be picked up. `on_start` hears that the drag began.
pub fn draggable<K: 'static>(
    row: ListItem,
    key: K,
    label: SharedString,
    on_start: impl Fn(&K, &mut App) + 'static,
) -> ListItem {
    row.on_drag(DraggedRow { key, label }, move |dragged, offset, _, cx| {
        on_start(&dragged.key, cx);
        let label = dragged.label.clone();
        cx.new(|_| DragChip { label, offset })
    })
}

/// Makes the element a place a dragged row can be dropped, when `accepts` says so for that row. It is
/// tinted while such a row is over it.
pub fn drop_target<E: InteractiveElement, K: 'static>(
    element: E,
    accepts: impl Fn(&K, &App) -> bool + 'static,
    on_drop: impl Fn(&K, &mut Window, &mut App) + 'static,
) -> E {
    element
        .can_drop(move |dragged, _, cx| {
            dragged.downcast_ref::<DraggedRow<K>>().is_some_and(|dragged| accepts(&dragged.key, cx))
        })
        .drag_over::<DraggedRow<K>>(|style, _, _, cx| style.bg(cx.theme().drop_target))
        .on_drop(move |dragged: &DraggedRow<K>, window, cx| on_drop(&dragged.key, window, cx))
}

/// The strip under a tree that takes a folder dragged out to the top level. A tree shows it while such a
/// folder is being dragged: a full tree has no empty space to drop on.
pub fn top_level_target<K: 'static>(
    id: &'static str,
    accepts: impl Fn(&K, &App) -> bool + 'static,
    on_drop: impl Fn(&K, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Stateful<gpui_kit::Div> {
    let theme = cx.theme();
    let strip = div()
        .id(id)
        .flex_none()
        .mx_2()
        .mb_2()
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .border_1()
        .border_dashed()
        .border_color(theme.border)
        .text_xs()
        .text_center()
        .text_color(theme.muted_foreground)
        .child("Move to top level");
    drop_target(strip, accepts, on_drop)
}
