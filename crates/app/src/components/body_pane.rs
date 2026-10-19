//! A body as it is read: a read-only code editor highlighted for its type, either re-indented ("Pretty") or
//! exactly as received ("Raw"), with Copy above it and the editor's own find (Ctrl+F).
//!
//! The response panel shows one for a response's body, and a streamed response shows one for each message
//! that is opened, so a single message of a stream reads exactly like a response.

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState},
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div, px,
};

use super::Syntax;
use crate::{i18n::t, model::http::pretty_json};

#[derive(Clone, Copy, PartialEq, Eq)]
enum BodyView {
    Pretty,
    Raw,
}

/// The fewest and the most rows a pane that sizes itself shows; a longer body scrolls inside it.
const MIN_ROWS: usize = 2;
const MAX_ROWS: usize = 16;
/// Roughly how many characters fit on a row, for guessing how far a long line wraps.
const WRAP_COLUMNS: usize = 100;

pub struct BodyPane {
    editor: Entity<EditorState>,
    /// The body as received, and re-indented when that is possible. Both are kept so switching views does
    /// not parse again.
    raw: String,
    pretty: Option<String>,
    view: BodyView,
    /// A pane in a list is as tall as its text, up to a limit; otherwise it fills the space it is given.
    fitted: bool,
    rows: usize,
}

/// What a message with no content type is read as: JSON when it parses, markup when it looks like it.
pub fn read_text(text: &str) -> (Syntax, Option<String>) {
    match pretty_json(text) {
        Some(pretty) => (Syntax::Json, Some(pretty)),
        None if text.trim_start().starts_with('<') => (Syntax::Markup, None),
        None => (Syntax::Plain, None),
    }
}

/// How many rows `text` takes, counting a long line as the rows it wraps to.
fn rows_of(text: &str) -> usize {
    text.lines().map(|line| line.chars().count().div_ceil(WRAP_COLUMNS).max(1)).sum()
}

impl BodyPane {
    /// An empty pane that fills the space it is given.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            let mut state =
                EditorState::new(window, cx).language(Syntax::Plain.language()).line_number(true).soft_wrap(true);
            state.set_readonly(true, cx);
            state
        });
        Self { editor, raw: String::new(), pretty: None, view: BodyView::Pretty, fitted: false, rows: MIN_ROWS }
    }

    /// A pane for one message of a stream, as tall as the message needs.
    pub fn fitted(text: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::new(window, cx);
        this.fitted = true;
        let (syntax, pretty) = read_text(text);
        this.set(text.to_owned(), pretty, syntax, window, cx);
        this
    }

    /// Shows a body. The Pretty/Raw choice is kept from the body before it.
    pub fn set(
        &mut self,
        raw: String,
        pretty: Option<String>,
        syntax: Syntax,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.raw = raw;
        self.pretty = pretty;
        self.editor.update(cx, |state, cx| state.set_highlighter(syntax.language(), cx));
        self.show(window, cx);
    }

    /// Opens the editor's own find, as Ctrl+F in it does.
    pub fn find(&mut self, cx: &mut Context<Self>) {
        self.editor.update(cx, |state, cx| state.open_search(false, cx));
    }

    /// Marks every place `query` is found in the body, or none when it is empty.
    pub fn highlight(&mut self, query: &str, cx: &mut Context<Self>) {
        self.editor.update(cx, |state, cx| {
            if query.is_empty() { state.close_search(cx) } else { state.set_search_query(query, true, cx) }
        });
    }

    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = match (self.view, &self.pretty) {
            (BodyView::Pretty, Some(pretty)) => pretty.clone(),
            _ => self.raw.clone(),
        };
        self.rows = rows_of(&text).clamp(MIN_ROWS, MAX_ROWS);
        self.editor.update(cx, |state, cx| state.set_value(text, window, cx));
        cx.notify();
    }

    fn set_view(&mut self, view: BodyView, window: &mut Window, cx: &mut Context<Self>) {
        if self.view != view {
            self.view = view;
            self.show(window, cx);
        }
    }

    /// The Pretty/Raw choice and Copy, shown above the body.
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_none()
            .px_3()
            .py_2()
            .gap_2()
            .when(self.pretty.is_some(), |this| {
                this.child(
                    TabBar::new("body-view")
                        .segmented()
                        .small()
                        .selected_index(if self.view == BodyView::Pretty { 0 } else { 1 })
                        .on_click(cx.listener(|this, ix: &usize, window, cx| {
                            this.set_view(if *ix == 0 { BodyView::Pretty } else { BodyView::Raw }, window, cx)
                        }))
                        .child(Tab::new().label(t("components.body_pane.pretty")))
                        .child(Tab::new().label(t("components.body_pane.raw"))),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("copy-body")
                    .ghost()
                    .small()
                    .icon(IconName::Copy)
                    .label(t("components.body_pane.copy"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let text = this.editor.read(cx).value().to_string();
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    })),
            )
    }
}

impl Render for BodyPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = Editor::new(&self.editor).bordered(false);
        // Several panes can be on screen at once; the id keeps their controls apart.
        let pane = v_flex().id(("body-pane", cx.entity_id())).w_full().child(self.render_toolbar(cx));
        if self.fitted {
            // The editor's rows are one and a half times its font size.
            let height = cx.theme().mono_font_size * (self.rows as f32 * 1.5) + px(12.);
            pane.child(editor.h(height))
        } else {
            pane.h_full().child(div().flex_1().min_h_0().child(editor.h_full()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_read_as_json_or_markup_when_it_is_one() {
        let (syntax, pretty) = read_text("{\"a\":1}");
        assert!(syntax == Syntax::Json);
        assert_eq!(pretty.as_deref(), Some("{\n  \"a\": 1\n}"));
        assert!(read_text("  <note>hi</note>") == (Syntax::Markup, None));
        assert!(read_text("tick 4") == (Syntax::Plain, None));
    }

    #[test]
    fn a_long_line_counts_as_the_rows_it_wraps_to() {
        assert_eq!(rows_of("a\nb\n\nc"), 4);
        assert_eq!(rows_of(&"x".repeat(WRAP_COLUMNS * 2 + 1)), 3);
        assert_eq!(rows_of(""), 0);
    }
}
