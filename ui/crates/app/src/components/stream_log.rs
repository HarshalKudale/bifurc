//! One row of a live message log, shared by every streaming response: a line that opens to the whole message.
//!
//! The gRPC call log and the SSE event log both draw their rows here, so a streamed response looks and behaves
//! the same whichever request type produced it. The owner keeps which rows are open and what a click does.

use std::collections::HashMap;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::Value;

use super::{BodyPane, page};
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

/// The rows of a log that are open, each with the pane its message is read in.
#[derive(Default)]
pub struct OpenRows(HashMap<String, Entity<BodyPane>>);

impl OpenRows {
    pub fn get(&self, id: &str) -> Option<&Entity<BodyPane>> {
        self.0.get(id)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Closes the row if it is open; opens it to `data` otherwise, with what the log is searched for marked.
    pub fn toggle<T: 'static>(
        &mut self,
        id: &str,
        data: &str,
        query: &str,
        window: &mut Window,
        cx: &mut Context<T>,
    ) {
        if self.0.remove(id).is_none() {
            let pane = cx.new(|cx| {
                let mut pane = BodyPane::fitted(data, window, cx);
                pane.highlight(query, cx);
                pane
            });
            self.0.insert(id.to_owned(), pane);
        }
    }

    /// Marks `query` in every open row.
    pub fn highlight(&self, query: &str, cx: &mut App) {
        for pane in self.0.values() {
            pane.update(cx, |pane, cx| pane.highlight(query, cx));
        }
    }

    /// Forgets the rows `keep` does not name: their messages have left the log.
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.0.retain(|id, _| keep(id));
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// The button that starts a search, shown at the top of every response: a log is searched through its rows,
/// a single body with its own find.
pub fn search_button(id: &'static str) -> Button {
    Button::new(id).ghost().small().icon(IconName::Search).tooltip("Search")
}

/// The search of a log: a field above the rows, which then show only the messages that have the text.
pub struct LogSearch {
    input: Entity<InputState>,
    visible: bool,
    _subscription: Subscription,
}

impl LogSearch {
    /// `on_change` runs in the owner when the text searched for changes.
    pub fn new<T: 'static>(window: &mut Window, cx: &mut Context<T>, on_change: fn(&mut T, &mut Context<T>)) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search messages"));
        let subscription = cx.subscribe(&input, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                on_change(this, cx);
            }
        });
        Self { input, visible: false, _subscription: subscription }
    }

    /// The text searched for; empty when there is no search.
    pub fn query(&self, cx: &App) -> String {
        if self.visible { self.input.read(cx).value().trim().to_owned() } else { String::new() }
    }

    /// The query in the form [`found`] compares with, or `None` when every row is shown.
    pub fn needle(&self, cx: &App) -> Option<String> {
        Some(self.query(cx).to_lowercase()).filter(|needle| !needle.is_empty())
    }

    /// Shows the field and puts the cursor in it, or hides it and ends the search.
    pub fn toggle(&mut self, window: &mut Window, cx: &mut App) {
        self.visible = !self.visible;
        self.input.update(cx, |state, cx| {
            if self.visible { state.focus(window, cx) } else { state.set_value("", window, cx) }
        });
    }

    /// The field, with how many of the rows are shown; nothing while there is no search.
    pub fn bar(&self, shown: usize, total: usize, cx: &App) -> Option<AnyElement> {
        let theme = cx.theme();
        self.visible.then(|| {
            h_flex()
                .flex_none()
                .px_3()
                .pb_2()
                .gap_3()
                .child(div().flex_1().min_w_0().child(
                    Input::new(&self.input).small().prefix(Icon::new(IconName::Search).small()).cleanable(true),
                ))
                .when(self.needle(cx).is_some(), |this| {
                    this.child(
                        div()
                            .flex_none()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(format!("{shown} of {total}")),
                    )
                })
                .into_any_element()
        })
    }
}

/// Whether a row with these texts is shown for `needle`: any of them has it, whatever the case.
pub fn found(needle: Option<&str>, texts: &[&str]) -> bool {
    needle.is_none_or(|needle| texts.iter().any(|text| text.to_lowercase().contains(needle)))
}

/// What a log shows when no row has the text searched for.
pub fn nothing_found() -> AnyElement {
    page::empty("No matches", "No message has the text you searched for.")
}

/// The row, opened to the whole message when it has a `body`. `on_toggle` runs when it is clicked.
pub fn log_row(
    row: &LogRow,
    body: Option<&Entity<BodyPane>>,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let open = body.is_some();
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
    if let Some(body) = body {
        block = block.child(
            div().px_4().pb_3().child(
                div()
                    .w_full()
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border)
                    .overflow_hidden()
                    .child(body.clone()),
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

    #[test]
    fn a_row_is_found_by_any_of_its_texts_whatever_the_case() {
        assert!(found(None, &["anything"]));
        assert!(found(Some("tick"), &["{\"n\":1}", "Tick"]));
        assert!(found(Some("\"n\""), &["{\"N\":1}", "message"]));
        assert!(!found(Some("tock"), &["{\"n\":1}", "tick"]));
    }
}
