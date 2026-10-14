//! An editable list of multipart fields: each has a name and either a text value or an attached file.
//!
//! Built like [`KeyValueTable`](super::KeyValueTable): the table always ends in one empty row, so adding a
//! field is typing into it (or attaching a file to it). The table holds what is chosen and reports that it
//! changed; the owner reads [`PartsTable::parts`] when it needs the values. The bytes of a file are held
//! base64-encoded, which is how the engine takes them.

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::{Value, json};

use super::{binary::PickedFile, var_editor::VarEditor};
use crate::{
    i18n::{t, t_with},
    model::media::size_label,
};

/// A file attached to a field, or chosen as a whole body.
#[derive(Debug, Clone, PartialEq)]
pub struct FileData {
    pub name: String,
    pub content_type: String,
    /// The file's bytes, base64. Shared, so comparing and cloning an unchanged file is cheap.
    pub data: Arc<str>,
    /// Bytes, before encoding.
    pub size: usize,
}

impl FileData {
    pub fn from_picked(file: PickedFile) -> Self {
        Self {
            size: file.bytes.len(),
            data: STANDARD.encode(&file.bytes).into(),
            content_type: file.content_type.to_owned(),
            name: file.name,
        }
    }

    /// A file already held as base64; its size is read from the length of the text.
    pub fn from_base64(name: String, content_type: String, data: String) -> Self {
        let padding = data.bytes().rev().take_while(|b| *b == b'=').count();
        Self { size: (data.len() * 3 / 4).saturating_sub(padding), name, content_type, data: data.into() }
    }

    /// One line about the file: its name and its size.
    pub fn summary(&self) -> String {
        format!("{} · {}", self.name, size_label(self.size))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PartValue {
    Text(String),
    File(FileData),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub name: String,
    pub value: PartValue,
}

/// The `bodyParts` array of a request.
pub fn parts_to_json(parts: &[Part]) -> Value {
    Value::Array(
        parts
            .iter()
            .map(|part| match &part.value {
                PartValue::Text(text) => json!({ "name": part.name, "kind": "text", "value": text }),
                PartValue::File(file) => json!({
                    "name": part.name,
                    "kind": "file",
                    "value": &*file.data,
                    "fileName": file.name,
                    "contentType": file.content_type,
                }),
            })
            .collect(),
    )
}

/// The parts of a `bodyParts` array. A missing field reads as empty.
pub fn parts_from_json(value: &Value) -> Vec<Part> {
    let Some(items) = value.as_array() else { return Vec::new() };
    items
        .iter()
        .map(|item| {
            let text = |key: &str| item.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
            let value = if item.get("kind").and_then(Value::as_str) == Some("file") {
                PartValue::File(FileData::from_base64(text("fileName"), text("contentType"), text("value")))
            } else {
                PartValue::Text(text("value"))
            };
            Part { name: text("name"), value }
        })
        .collect()
}

#[derive(Debug, Clone)]
pub enum PartsEvent {
    Changed,
}

struct Row {
    id: u64,
    name: Entity<InputState>,
    value: VarEditor,
    file: Option<FileData>,
    _subscriptions: [Subscription; 2],
}

pub struct PartsTable {
    rows: Vec<Row>,
    next_id: u64,
    /// Set while rows are being replaced from outside, so that filling them is not reported as an edit.
    filling: bool,
    /// Why the last file could not be read, until the next edit.
    last_error: Option<SharedString>,
}

impl EventEmitter<PartsEvent> for PartsTable {}

impl PartsTable {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self { rows: Vec::new(), next_id: 0, filling: false, last_error: None };
        this.push_row("", "", None, window, cx);
        this
    }

    fn push_row(
        &mut self,
        name: &str,
        text: &str,
        file: Option<FileData>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_id;
        self.next_id += 1;
        let name_state = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t("components.parts_table.name")).default_value(name.to_owned())
        });
        let value_editor = VarEditor::line(t("components.parts_table.value"), window, cx);
        value_editor.set_value(text.to_owned(), window, cx);
        let subscriptions = [
            cx.subscribe_in(&name_state, window, |this, _, event: &InputEvent, window, cx| {
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
        self.rows.push(Row { id, name: name_state, value: value_editor, file, _subscriptions: subscriptions });
    }

    fn last_is_used(&self, cx: &gpui_kit::App) -> bool {
        self.rows.last().is_none_or(|row| {
            row.file.is_some() || !row.name.read(cx).value().is_empty() || !row.value.value(cx).is_empty()
        })
    }

    fn on_edited(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.filling {
            return;
        }
        self.last_error = None;
        if self.last_is_used(cx) {
            self.push_row("", "", None, window, cx);
        }
        cx.emit(PartsEvent::Changed);
        cx.notify();
    }

    /// The parts with a name or a value, in order. The trailing empty row is not one.
    pub fn parts(&self, cx: &gpui_kit::App) -> Vec<Part> {
        self.rows
            .iter()
            .filter_map(|row| {
                let name = row.name.read(cx).value().trim().to_owned();
                let value = match &row.file {
                    Some(file) => PartValue::File(file.clone()),
                    None => PartValue::Text(row.value.line_value(cx)),
                };
                let empty = name.is_empty() && matches!(&value, PartValue::Text(text) if text.is_empty());
                (!empty).then_some(Part { name, value })
            })
            .collect()
    }

    /// Replaces every row. Does not report a change: the owner already knows what it set.
    pub fn set_parts(&mut self, parts: &[Part], window: &mut Window, cx: &mut Context<Self>) {
        self.filling = true;
        self.rows.clear();
        self.last_error = None;
        for part in parts {
            match &part.value {
                PartValue::Text(text) => self.push_row(&part.name, text, None, window, cx),
                PartValue::File(file) => self.push_row(&part.name, "", Some(file.clone()), window, cx),
            }
        }
        self.push_row("", "", None, window, cx);
        self.filling = false;
        cx.notify();
    }

    fn remove_row(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.rows.retain(|row| row.id != id);
        if self.rows.is_empty() {
            self.push_row("", "", None, window, cx);
        }
        self.last_error = None;
        cx.emit(PartsEvent::Changed);
        cx.notify();
    }

    fn remove_file(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.file = None;
        }
        cx.emit(PartsEvent::Changed);
        cx.notify();
    }

    /// Asks for a file and attaches it to the row. The row is found again by its id: rows may come and go
    /// while the dialog is open.
    fn choose_file(&mut self, id: u64, cx: &mut Context<Self>) {
        super::binary::pick_file(t("components.parts_table.pick_file"), cx, move |this, outcome, cx| {
            match outcome {
                Ok(file) => {
                    this.last_error = None;
                    if let Some(row) = this.rows.iter_mut().find(|row| row.id == id) {
                        row.file = Some(FileData::from_picked(file));
                        cx.emit(PartsEvent::Changed);
                    }
                }
                Err(message) => {
                    this.last_error = Some(t_with("components.parts_table.read_failed", &[("message", &message)]).into())
                }
            }
            cx.notify();
        });
    }

    fn render_value(&self, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let id = row.id;
        match &row.file {
            Some(file) => h_flex()
                .gap_2()
                .items_center()
                .h(VarEditor::row_height(cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child(file.summary()),
                )
                .child(
                    Button::new(("remove-file", id))
                        .ghost()
                        .xsmall()
                        .label(t("components.parts_table.remove_file"))
                        .on_click(cx.listener(move |this, _, _, cx| this.remove_file(id, cx))),
                )
                .into_any_element(),
            None => row.value.line_field(cx).into_any_element(),
        }
    }
}

impl Render for PartsTable {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Attaching a file to the trailing row leaves it used; the row after it is made here, where a
        // window is at hand.
        if self.last_is_used(cx) {
            self.push_row("", "", None, window, cx);
        }
        let theme = cx.theme().clone();
        let last = self.rows.len().saturating_sub(1);
        v_flex()
            .id("parts-table")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(div().w_1_4().child(t("components.parts_table.name")))
                    .child(div().flex_1().child(t("components.parts_table.value")))
                    // The lanes the file and remove buttons occupy in the rows below.
                    .child(div().w_32().flex_none())
                    .child(div().w_6().flex_none()),
            )
            .children(self.rows.iter().enumerate().map(|(ix, row)| {
                let id = row.id;
                // The trailing empty row is where a new field is typed; there is nothing to remove yet.
                let removable = ix != last;
                h_flex()
                    .id(("part", id))
                    .gap_2()
                    .child(div().w_1_4().min_w_0().child(VarEditor::row_input(&row.name, cx)))
                    .child(div().flex_1().min_w_0().child(self.render_value(row, cx)))
                    .child(
                        div().w_32().flex_none().child(
                            Button::new(("choose-file", id))
                                .ghost()
                                .small()
                                .label(t("components.parts_table.choose_file"))
                                .on_click(cx.listener(move |this, _, _, cx| this.choose_file(id, cx))),
                        ),
                    )
                    .child(div().w_6().flex_none().children(removable.then(|| {
                        Button::new(("remove-part", id))
                            .icon(IconName::Close)
                            .ghost()
                            .xsmall()
                            .tooltip(t("components.remove"))
                            .on_click(cx.listener(move |this, _, window, cx| this.remove_row(id, window, cx)))
                    })))
            }))
            .children(self.last_error.clone().map(|message| div().text_xs().text_color(theme.warning).child(message)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> FileData {
        FileData { name: "a.png".into(), content_type: "image/png".into(), data: "aGVsbG8=".into(), size: 5 }
    }

    #[test]
    fn parts_become_the_body_parts_array() {
        let parts = vec![
            Part { name: "title".into(), value: PartValue::Text("hi".into()) },
            Part { name: "avatar".into(), value: PartValue::File(file()) },
        ];
        assert_eq!(
            parts_to_json(&parts),
            json!([
                { "name": "title", "kind": "text", "value": "hi" },
                { "name": "avatar", "kind": "file", "value": "aGVsbG8=", "fileName": "a.png", "contentType": "image/png" },
            ])
        );
    }

    #[test]
    fn parts_round_trip_through_json() {
        let parts = vec![
            Part { name: "title".into(), value: PartValue::Text("hi".into()) },
            Part { name: "avatar".into(), value: PartValue::File(file()) },
        ];
        assert_eq!(parts_from_json(&parts_to_json(&parts)), parts);
    }

    #[test]
    fn a_files_size_is_read_from_its_base64() {
        let parts = parts_from_json(&json!([
            { "name": "a", "kind": "file", "value": "aGVsbG8=" },
            { "name": "b", "kind": "file", "value": "aGVsbG8h" },
            { "name": "c", "kind": "file", "value": "aGk=" },
        ]));
        let sizes: Vec<usize> = parts
            .iter()
            .map(|p| match &p.value {
                PartValue::File(f) => f.size,
                PartValue::Text(_) => 0,
            })
            .collect();
        assert_eq!(sizes, [5, 6, 2]);
    }

    #[test]
    fn missing_fields_read_as_empty() {
        assert_eq!(parts_from_json(&json!({})), Vec::new());
        assert_eq!(
            parts_from_json(&json!([{}])),
            vec![Part { name: String::new(), value: PartValue::Text(String::new()) }]
        );
        let parts = parts_from_json(&json!([{ "kind": "file" }]));
        assert_eq!(
            parts[0].value,
            PartValue::File(FileData { name: String::new(), content_type: String::new(), data: "".into(), size: 0 })
        );
    }

    #[test]
    fn a_picked_file_is_encoded() {
        let data = FileData::from_picked(PickedFile {
            name: "a.txt".into(),
            bytes: b"hello".to_vec(),
            content_type: "text/plain",
        });
        assert_eq!((&*data.data, data.size), ("aGVsbG8=", 5));
        assert_eq!(data.summary(), "a.txt · 5 B");
    }
}
