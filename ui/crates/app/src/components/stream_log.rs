//! One row of a live message log, shared by every streaming response: a line that opens to the whole message.
//!
//! The gRPC call log and the SSE event log both draw their rows here, so a streamed response looks and behaves
//! the same whichever request type produced it. The owner keeps which rows are open and what a click does.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    h_flex,
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div,
};
use serde_json::Value;

use crate::model::traffic::clock_utc;

/// Which way a message went, for a log that has both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Way {
    Sent,
    Received,
}

/// What a row shows.
#[derive(Debug, Clone, PartialEq)]
pub struct LogRow {
    /// Unique in its log; the owner keeps the set of open ids.
    pub id: String,
    pub ts: u64,
    /// The label tag: `Sent` / `Received`, or the name of an event.
    pub tag: String,
    /// An outlined tag marks the quieter kind (what was sent, a plain `message`).
    pub outline: bool,
    /// An arrow before the tag, for a log that has both directions.
    pub way: Option<Way>,
    /// What is shown beside the line, such as an event's id.
    pub note: Option<String>,
    pub data: String,
}

/// The one-line form of a message: JSON on one line, anything else with its whitespace collapsed.
pub fn compact(data: &str) -> String {
    serde_json::from_str::<Value>(data)
        .ok()
        .and_then(|value| serde_json::to_string(&value).ok())
        .unwrap_or_else(|| data.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The row, opened to the whole message when `open`. `on_toggle` runs when it is clicked.
pub fn log_row(
    row: &LogRow,
    open: bool,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme().clone();
    let chevron = Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight })
        .small()
        .text_color(theme.muted_foreground);
    let line = h_flex()
        .id(ElementId::Name(format!("log-row-{}", row.id).into()))
        .w_full()
        .gap_3()
        .px_3()
        .py_2()
        .items_center()
        .text_sm()
        .cursor_pointer()
        .hover(|style| style.bg(theme.muted))
        .on_click(on_toggle)
        .child(div().size_4().flex_none().child(chevron))
        .when_some(row.way, |this, way| {
            let arrow = Icon::new(if way == Way::Sent { IconName::ArrowRight } else { IconName::ArrowLeft })
                .small()
                .text_color(theme.muted_foreground);
            this.child(div().size_4().flex_none().child(arrow))
        })
        .child(div().w_20().flex_none().child(if row.outline {
            Tag::secondary().small().outline().child(row.tag.clone())
        } else {
            Tag::secondary().small().child(row.tag.clone())
        }))
        .child(div().w_20().flex_none().text_xs().text_color(theme.muted_foreground).child(clock_utc(row.ts)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_xs()
                .font_family(theme.mono_font_family.clone())
                .text_color(theme.muted_foreground)
                .child(compact(&row.data)),
        )
        .when_some(row.note.clone(), |this, note| {
            this.child(div().flex_none().text_xs().text_color(theme.muted_foreground).child(note))
        });
    let mut block = v_flex().w_full().border_b_1().border_color(theme.border).child(line);
    if open {
        block = block.child(
            div().px_4().pb_3().child(
                div()
                    .w_full()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.muted)
                    .text_xs()
                    .font_family(theme.mono_font_family.clone())
                    .child(row.data.clone()),
            ),
        );
    }
    block.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_shown_on_one_line_and_other_text_has_its_whitespace_collapsed() {
        assert_eq!(compact("{\n  \"a\": 1,\n  \"b\": [1, 2]\n}"), "{\"a\":1,\"b\":[1,2]}");
        assert_eq!(compact("hello\n  world \n"), "hello world");
        assert_eq!(compact(""), "");
    }
}
