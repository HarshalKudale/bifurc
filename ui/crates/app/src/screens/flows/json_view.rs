//! A JSON value shown as a tree that starts closed: a list or an object opens on a click. It is how a node
//! shows what it was handed and what it produced, and how a part of a response is chosen: by clicking the
//! row that holds it. It is never a text to edit.

use std::{collections::HashSet, rc::Rc};

use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Div, ElementId, InteractiveElement as _, ParentElement as _, Stateful, StatefulInteractiveElement as _,
    Styled as _, Window, div, rems,
};
use serde_json::Value;

use crate::model::flow::{json_rows, path_key};

/// How many rows one tree shows. A response with more is still read whole by the engine.
const ROW_LIMIT: usize = 300;

pub type Toggle = Rc<dyn Fn(String, &mut Window, &mut App)>;
pub type Pick = Rc<dyn Fn(Vec<Value>, &mut Window, &mut App)>;

/// What names a row of the tree `scope` in the set of open rows.
pub fn open_key(scope: &str, path: &[Value]) -> String {
    format!("{scope}|{}", path_key(path))
}

/// `value` as a tree. `scope` tells this tree's rows from another's in `open`; `toggle` is told the key of a
/// row that was opened or closed. With `pick`, a click on a row chooses the part it holds and its arrow
/// opens it; without, a click anywhere on the row opens it.
/// It scrolls once it is 14 rems high; whoever draws it may give it another height.
pub fn json_tree(scope: &str, value: &Value, open: &HashSet<String>, toggle: Toggle, pick: Option<Pick>, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    let rows = json_rows(value, &|key| open.contains(&format!("{scope}|{key}")), ROW_LIMIT);
    let tree_id = ElementId::Name(format!("{scope}|tree").into());
    if rows.is_empty() {
        // A single value, or a list or an object with nothing in it.
        let shown = match value {
            Value::Null => "Nothing".to_owned(),
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        return div()
            .id(tree_id)
            .px_2()
            .py_1()
            .text_color(theme.muted_foreground)
            .overflow_hidden()
            .text_ellipsis()
            .child(shown);
    }
    let cut_short = rows.len() >= ROW_LIMIT;
    let lines = rows.into_iter().map(|row| {
        let key = open_key(scope, &row.path);
        let arrow = row.open.map(|is_open| {
            let (toggle, key) = (toggle.clone(), key.clone());
            div()
                .id(ElementId::Name(format!("{key}|arrow").into()))
                .flex_none()
                .child(
                    Icon::new(if is_open { IconName::ChevronDown } else { IconName::ChevronRight })
                        .xsmall()
                        .text_color(theme.muted_foreground),
                )
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    toggle(key.clone(), window, cx);
                })
        });
        let (row_toggle, row_pick, row_key, path) = (toggle.clone(), pick.clone(), key.clone(), row.path.clone());
        let opens = row.open.is_some();
        h_flex()
            .id(ElementId::Name(key.into()))
            .flex_none()
            .h(rems(1.375))
            .pl(rems(0.5 + row.depth as f32 * 0.75))
            .pr_2()
            .gap_1()
            .hover(|style| style.bg(theme.accent))
            .child(div().w(rems(0.75)).flex_none().children(arrow))
            .child(div().flex_none().font_family(theme.mono_font_family.clone()).child(row.key))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(theme.muted_foreground)
                    .child(row.preview),
            )
            .when(row_pick.is_some() || opens, |this| {
                this.on_click(move |_, window, cx| match &row_pick {
                    Some(pick) => pick(path.clone(), window, cx),
                    None => row_toggle(row_key.clone(), window, cx),
                })
            })
    });
    v_flex()
        .id(tree_id)
        .max_h(rems(14.0))
        .overflow_y_scroll()
        .py_1()
        .children(lines)
        .when(cut_short, |this| {
            this.child(div().px_2().text_color(theme.muted_foreground).child(format!("The first {ROW_LIMIT} rows are shown.")))
        })
}

/// Open the row named `key` in `open` if it is closed, close it if it is open.
pub fn flip(open: &mut HashSet<String>, key: String) {
    if !open.remove(&key) {
        open.insert(key);
    }
}
