//! An editable list of name/value pairs: query parameters, headers, form fields, variables.
//!
//! The table always ends in one empty row, so adding a pair is typing into it; there is no separate "add"
//! step. Values are variable-aware (`{{name}}` is marked). The table holds what is typed and reports that it
//! changed; the owner reads [`KeyValueTable::pairs`] when it needs the values.

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};

use super::var_editor::VarEditor;

#[derive(Debug, Clone)]
pub enum KeyValueEvent {
    Changed,
}

struct Row {
    id: u64,
    key: Entity<InputState>,
    value: VarEditor,
    _subscriptions: [Subscription; 2],
}

pub struct KeyValueTable {
    rows: Vec<Row>,
    next_id: u64,
    key_label: SharedString,
    value_label: SharedString,
    readonly: bool,
    /// Set while rows are being replaced from outside, so that filling them is not reported as an edit.
    filling: bool,
}

impl EventEmitter<KeyValueEvent> for KeyValueTable {}

impl KeyValueTable {
    pub fn new(
        key_label: impl Into<SharedString>,
        value_label: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            rows: Vec::new(),
            next_id: 0,
            key_label: key_label.into(),
            value_label: value_label.into(),
            readonly: false,
            filling: false,
        };
        this.push_row("", "", window, cx);
        this
    }

    fn push_row(&mut self, key: &str, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.next_id;
        self.next_id += 1;
        let key_state =
            cx.new(|cx| InputState::new(window, cx).placeholder(self.key_label.clone()).default_value(key.to_owned()));
        let value_editor = VarEditor::line(self.value_label.clone(), window, cx);
        value_editor.set_value(value.to_owned(), window, cx);
        if self.readonly {
            key_state.update(cx, |state, cx| state.set_readonly(true, cx));
            value_editor.set_readonly(true, cx);
        }
        let subscriptions = [
            cx.subscribe_in(&key_state, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.on_edited(window, cx);
                }
            }),
            cx.subscribe_in(value_editor.state(), window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.on_edited(window, cx);
                }
            }),
        ];
        self.rows.push(Row { id, key: key_state, value: value_editor, _subscriptions: subscriptions });
    }

    fn on_edited(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.filling {
            return;
        }
        let last_is_used =
            self.rows.last().is_none_or(|row| !row.key.read(cx).value().is_empty() || !row.value.value(cx).is_empty());
        if last_is_used && !self.readonly {
            self.push_row("", "", window, cx);
        }
        cx.emit(KeyValueEvent::Changed);
        cx.notify();
    }

    /// Replaces every row. Does not report a change: the owner already knows what it set.
    pub fn set_pairs(&mut self, pairs: &[(String, String)], window: &mut Window, cx: &mut Context<Self>) {
        self.filling = true;
        self.rows.clear();
        for (key, value) in pairs {
            self.push_row(key, value, window, cx);
        }
        if !self.readonly || self.rows.is_empty() {
            self.push_row("", "", window, cx);
        }
        self.filling = false;
        cx.notify();
    }

    /// The pairs with a name, in order. A row with only a value is not a pair yet.
    pub fn pairs(&self, cx: &gpui_kit::App) -> Vec<(String, String)> {
        self.rows
            .iter()
            .map(|row| (row.key.read(cx).value().trim().to_owned(), row.value.line_value(cx)))
            .filter(|(key, _)| !key.is_empty())
            .collect()
    }

    pub fn set_readonly(&mut self, readonly: bool, cx: &mut Context<Self>) {
        self.readonly = readonly;
        for row in &self.rows {
            row.key.update(cx, |state, cx| state.set_readonly(readonly, cx));
            row.value.set_readonly(readonly, cx);
        }
        cx.notify();
    }

    fn remove_row(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.rows.retain(|row| row.id != id);
        if self.rows.is_empty() {
            self.push_row("", "", window, cx);
        }
        cx.emit(KeyValueEvent::Changed);
        cx.notify();
    }
}

impl Render for KeyValueTable {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let last = self.rows.len().saturating_sub(1);
        let readonly = self.readonly;
        let row_height = VarEditor::row_height(cx);
        v_flex()
            .id("key-value-table")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(div().w_1_3().child(self.key_label.clone()))
                    .child(div().flex_1().child(self.value_label.clone()))
                    // The same lane the remove button occupies in the rows below.
                    .child(div().w_6().flex_none()),
            )
            .children(self.rows.iter().enumerate().map(|(ix, row)| {
                let id = row.id;
                // The trailing empty row is where a new pair is typed; there is nothing to remove yet.
                let removable = !readonly && ix != last;
                h_flex()
                    .id(("pair", id))
                    .gap_2()
                    .child(div().w_1_3().min_w_0().child(Input::new(&row.key).h(row_height)))
                    .child(div().flex_1().min_w_0().child(row.value.line_field(cx)))
                    .child(div().w_6().flex_none().children(removable.then(|| {
                        Button::new(("remove-pair", id))
                            .icon(IconName::Close)
                            .ghost()
                            .xsmall()
                            .tooltip("Remove")
                            .on_click(cx.listener(move |this, _, window, cx| this.remove_row(id, window, cx)))
                    })))
            }))
    }
}
