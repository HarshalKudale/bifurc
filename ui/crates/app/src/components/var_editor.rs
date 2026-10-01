//! A code editor that marks `{{variable}}` references as the text is typed.
//!
//! The editor is the kit's; this adds one behavior. After every change the text is scanned for references
//! (`model::variables`) and two decoration collections are replaced: one colors the reference with the
//! theme's syntax color for a special variable, one fills the range behind it. Nothing is resolved here —
//! the mark only says "the engine will substitute this".

use gpui_kit::component::{
    ActiveTheme as _, Theme,
    input::{
        Editor, EditorState, InputEvent, RangeDecoration, RangeDecorationCollection, RangeDecorationStyle,
        TextDecoration, TextDecorationCollection,
    },
};
use gpui_kit::{
    App, AppContext as _, Entity, FontWeight, HighlightStyle, Pixels, SharedString, Subscription, Window, px,
};

use crate::model::variables::variable_ranges;

/// The languages the editors are opened with. The names are the highlighter's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Syntax {
    Plain,
    Json,
    /// HTML, and XML too: the HTML grammar reads XML's tags and attributes well enough to color them.
    Markup,
    JavaScript,
    GraphQl,
    Shell,
    Python,
}

impl Syntax {
    pub fn language(self) -> &'static str {
        match self {
            Syntax::Plain => "text",
            Syntax::Json => "json",
            Syntax::Markup => "html",
            Syntax::JavaScript => "javascript",
            Syntax::GraphQl => "graphql",
            Syntax::Shell => "bash",
            Syntax::Python => "python",
        }
    }

    /// The syntax for a body with this `Content-Type`.
    pub fn for_content_type(content_type: &str) -> Self {
        let content_type = content_type.to_ascii_lowercase();
        if content_type.contains("json") {
            Syntax::Json
        } else if content_type.contains("html") || content_type.contains("xml") {
            Syntax::Markup
        } else if content_type.contains("javascript") {
            Syntax::JavaScript
        } else if content_type.contains("graphql") {
            Syntax::GraphQl
        } else {
            Syntax::Plain
        }
    }
}

pub struct VarEditor {
    state: Entity<EditorState>,
    text_marks: TextDecorationCollection,
    fill_marks: RangeDecorationCollection,
    _subscriptions: Vec<Subscription>,
}

impl VarEditor {
    /// A multi-line editor for `syntax`, with line numbers.
    pub fn code(syntax: Syntax, placeholder: impl Into<SharedString>, window: &mut Window, cx: &mut App) -> Self {
        let placeholder = placeholder.into();
        Self::build(window, cx, |state| {
            state.language(syntax.language()).line_number(true).soft_wrap(false).placeholder(placeholder)
        })
    }

    /// A one-row field, such as a URL: Enter submits instead of adding a line.
    pub fn line(placeholder: impl Into<SharedString>, window: &mut Window, cx: &mut App) -> Self {
        let placeholder = placeholder.into();
        Self::build(window, cx, |state| {
            state
                .language(Syntax::Plain.language())
                .line_number(false)
                .folding(false)
                .soft_wrap(false)
                .submit_on_enter(true)
                .placeholder(placeholder)
        })
    }

    /// The height of a one-row field: one line of the editor's font inside the kit's fixed editor paddings
    /// (8 px above and below) and border (1 px each side). Controls that share a row with such a field take
    /// this height so the row has one top and one bottom edge.
    pub fn row_height(cx: &App) -> Pixels {
        cx.theme().mono_font_size * 1.5 + px(18.)
    }

    /// This editor as a one-row field.
    pub fn line_field(&self, cx: &App) -> Editor {
        Editor::new(&self.state).h(Self::row_height(cx))
    }

    fn build(window: &mut Window, cx: &mut App, configure: impl FnOnce(EditorState) -> EditorState) -> Self {
        let state = cx.new(|cx| configure(EditorState::new(window, cx)));
        let (text_marks, fill_marks) = state.update(cx, |state, cx| {
            (
                state.create_decorations_collection(Vec::new(), cx),
                state.create_range_decorations_collection(Vec::new(), cx),
            )
        });

        let on_change = {
            let (text_marks, fill_marks) = (text_marks.clone(), fill_marks.clone());
            cx.subscribe(&state, move |state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    mark(&state, &text_marks, &fill_marks, cx);
                }
            })
        };
        // The marks carry colors, so a theme change has to repaint them.
        let on_theme = {
            let (state, text_marks, fill_marks) = (state.downgrade(), text_marks.clone(), fill_marks.clone());
            cx.observe_global::<Theme>(move |cx| {
                if let Some(state) = state.upgrade() {
                    mark(&state, &text_marks, &fill_marks, cx);
                }
            })
        };
        Self { state, text_marks, fill_marks, _subscriptions: vec![on_change, on_theme] }
    }

    pub fn state(&self) -> &Entity<EditorState> {
        &self.state
    }

    pub fn value(&self, cx: &App) -> String {
        self.state.read(cx).value().to_string()
    }

    /// The value of a one-row field: a pasted line break is not part of it.
    pub fn line_value(&self, cx: &App) -> String {
        self.value(cx).replace(['\r', '\n'], "").trim().to_owned()
    }

    pub fn set_value(&self, value: impl Into<SharedString>, window: &mut Window, cx: &mut App) {
        let value = value.into();
        self.state.update(cx, |state, cx| state.set_value(value, window, cx));
        mark(&self.state, &self.text_marks, &self.fill_marks, cx);
    }

    pub fn set_syntax(&self, syntax: Syntax, cx: &mut App) {
        self.state.update(cx, |state, cx| state.set_highlighter(syntax.language(), cx));
    }

    pub fn set_readonly(&self, readonly: bool, cx: &mut App) {
        self.state.update(cx, |state, cx| state.set_readonly(readonly, cx));
    }
}

fn mark(
    state: &Entity<EditorState>,
    text_marks: &TextDecorationCollection,
    fill_marks: &RangeDecorationCollection,
    cx: &mut App,
) {
    let ranges = variable_ranges(&state.read(cx).value());
    let theme = cx.theme();
    let style = theme
        .highlight_theme
        .style
        .syntax
        .style("variable.special")
        .or_else(|| theme.highlight_theme.style.syntax.style("constant"))
        .unwrap_or(HighlightStyle { color: Some(theme.primary), ..Default::default() });
    let style = HighlightStyle { font_weight: Some(FontWeight::SEMIBOLD), ..style };
    let fill = theme.accent;
    text_marks.set(ranges.iter().cloned().map(|range| TextDecoration::new(range, style)).collect(), cx);
    fill_marks.set(
        ranges
            .into_iter()
            .map(|range| RangeDecoration::new(range).with_style(RangeDecorationStyle::Fill).with_color(fill))
            .collect(),
        cx,
    );
}
