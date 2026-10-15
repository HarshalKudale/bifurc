//! What a GraphQL request knows about its endpoint's schema: the explorer tab and the query editor's completion.
//!
//! The schema is fetched with `graphql.introspect` and lives only in this session's memory; the engine stores
//! none for it. The panel owns the schema and its view; the session (`protocol`) owns the fetch, because it is
//! the one that can read the endpoint and headers out of the editor.

use std::{cell::RefCell, rc::Rc};

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{CompletionProvider, Input, InputState, Rope},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Result, SharedString, StatefulInteractiveElement as _, Styled as _, Task, Window, div,
};
use lsp_types::{CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, Documentation};

use super::protocol::ProtocolSession;
use crate::{
    components::page,
    i18n::{t, t_with},
    model::graphql::{Field, Schema, TypeInfo},
};

/// Rows drawn per list. A large schema has thousands of types; the filter narrows them.
const MAX_ROWS: usize = 200;

type Shared = Rc<RefCell<Option<Rc<Schema>>>>;

pub struct SchemaPanel {
    /// What the completion provider reads, so a fetch reaches the editor without touching it.
    shared: Shared,
    pub fetching: bool,
    pub filter: Entity<InputState>,
    /// The type whose fields are shown instead of the root operations.
    selected: Option<String>,
}

impl SchemaPanel {
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(t("session.schema.filter_placeholder")));
        Self { shared: Shared::default(), fetching: false, filter, selected: None }
    }

    /// The completion provider for the query editor, fed by this panel's schema.
    pub fn completions(&self) -> Rc<dyn CompletionProvider> {
        Rc::new(SchemaCompletions(self.shared.clone()))
    }

    pub fn set_schema(&mut self, schema: Schema) {
        *self.shared.borrow_mut() = Some(Rc::new(schema));
        self.selected = None;
    }

    fn schema(&self) -> Option<Rc<Schema>> {
        self.shared.borrow().clone()
    }

    pub fn open_type(&mut self, name: Option<String>) {
        self.selected = name;
    }

    pub fn view(&self, cx: &mut Context<ProtocolSession>) -> AnyElement {
        let fetch = Button::new("fetch-schema")
            .small()
            .when(self.schema().is_none(), |this| this.primary())
            .when(self.schema().is_some(), |this| this.outline())
            .label(t(if self.schema().is_some() { "session.schema.refresh" } else { "session.schema.fetch" }))
            .loading(self.fetching)
            .on_click(cx.listener(|this, _, window, cx| this.fetch_schema(window, cx)));
        let Some(schema) = self.schema() else {
            return page::empty_with_action(t("session.schema.empty_title"), t("session.schema.empty_description"), fetch);
        };
        let filter = self.filter.read(cx).value().trim().to_lowercase();
        let theme = cx.theme().clone();
        let body = match self.selected.as_deref().and_then(|name| schema.types.get(name)) {
            Some(info) => type_detail(info, &schema, &filter, cx),
            None => overview(&schema, &filter, cx),
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .items_center()
                    .child(fetch)
                    .when(self.selected.is_some(), |this| {
                        this.child(
                            Button::new("schema-back")
                                .small()
                                .ghost()
                                .icon(IconName::ArrowLeft)
                                .label(t("session.schema.back"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(panel) = this.schema_panel_mut() {
                                        panel.open_type(None);
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .child(div().w_64().child(Input::new(&self.filter).small().cleanable(true))),
            )
            .child(
                v_flex()
                    .id("schema-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_3()
                    .pb_3()
                    .gap_0p5()
                    .text_color(theme.foreground)
                    .child(body),
            )
            .into_any_element()
    }
}

fn matches(name: &str, filter: &str) -> bool {
    filter.is_empty() || name.to_lowercase().contains(filter)
}

fn note_more(shown: usize, total: usize, cx: &App) -> Option<AnyElement> {
    (total > shown).then(|| {
        div()
            .py_1()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(t_with("session.schema.more_rows", &[("shown", &shown), ("total", &total)]))
            .into_any_element()
    })
}

/// The three root operations' fields, then every type.
fn overview(schema: &Schema, filter: &str, cx: &mut Context<ProtocolSession>) -> AnyElement {
    let mut sections = Vec::new();
    for (keyword, root) in schema.roots() {
        let Some(root) = root else { continue };
        let fields: Vec<&Field> = root.fields.iter().filter(|field| matches(&field.name, filter)).collect();
        if fields.is_empty() {
            continue;
        }
        sections.push(
            v_flex()
                .gap_0p5()
                .child(page::section_label(t(&format!("session.schema.{keyword}")), cx).pt_2())
                .children(fields.iter().take(MAX_ROWS).map(|field| field_row(&root.name, field, schema, cx)))
                .children(note_more(MAX_ROWS, fields.len(), cx))
                .into_any_element(),
        );
    }
    let types: Vec<&TypeInfo> = schema.types.values().filter(|info| matches(&info.name, filter)).collect();
    sections.push(
        v_flex()
            .gap_0p5()
            .child(page::section_label(t("session.schema.types"), cx).pt_2())
            .children(types.iter().take(MAX_ROWS).map(|info| type_row(info, cx)))
            .children(note_more(MAX_ROWS, types.len(), cx))
            .into_any_element(),
    );
    v_flex().children(sections).into_any_element()
}

fn type_detail(info: &TypeInfo, schema: &Schema, filter: &str, cx: &mut Context<ProtocolSession>) -> AnyElement {
    let theme = cx.theme().clone();
    let members: Vec<AnyElement> = info
        .fields
        .iter()
        .filter(|field| matches(&field.name, filter))
        .take(MAX_ROWS)
        .map(|field| field_row(&info.name, field, schema, cx))
        .collect();
    let values = info.enum_values.iter().filter(|value| matches(value, filter)).map(|value| {
        div().py_0p5().text_sm().font_family(theme.mono_font_family.clone()).child(value.clone()).into_any_element()
    });
    v_flex()
        .gap_0p5()
        .child(
            h_flex()
                .gap_2()
                .pt_2()
                .items_baseline()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(info.name.clone()))
                .child(div().text_xs().text_color(theme.muted_foreground).child(kind_label(&info.kind))),
        )
        .when(!info.description.is_empty(), |this| {
            this.child(div().text_xs().text_color(theme.muted_foreground).child(info.description.clone()))
        })
        .children(members)
        .children(values)
        .into_any_element()
}

fn kind_label(kind: &str) -> SharedString {
    t(match kind {
        "OBJECT" => "session.schema.kind_object",
        "INTERFACE" => "session.schema.kind_interface",
        "ENUM" => "session.schema.kind_enum",
        "INPUT_OBJECT" => "session.schema.kind_input",
        "UNION" => "session.schema.kind_union",
        _ => "session.schema.kind_scalar",
    })
    .into()
}

fn type_row(info: &TypeInfo, cx: &mut Context<ProtocolSession>) -> AnyElement {
    let theme = cx.theme().clone();
    let name = info.name.clone();
    h_flex()
        .id(SharedString::from(format!("schema-type-{}", info.name)))
        .gap_2()
        .px_1()
        .py_0p5()
        .items_baseline()
        .cursor_pointer()
        .hover(|style| style.bg(theme.muted))
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(panel) = this.schema_panel_mut() {
                panel.open_type(Some(name.clone()));
            }
            cx.notify();
        }))
        .child(div().text_sm().child(info.name.clone()))
        .child(div().text_xs().text_color(theme.muted_foreground).child(kind_label(&info.kind)))
        .into_any_element()
}

/// `name(arg: Type): Return` with its description under it. A field whose type is in the schema opens that type.
fn field_row(owner: &str, field: &Field, schema: &Schema, cx: &mut Context<ProtocolSession>) -> AnyElement {
    let theme = cx.theme().clone();
    let args = if field.args.is_empty() {
        String::new()
    } else {
        format!("({})", field.args.iter().map(|arg| format!("{}: {}", arg.name, arg.ty)).collect::<Vec<_>>().join(", "))
    };
    let target = schema.types.contains_key(&field.target).then(|| field.target.clone());
    v_flex()
        .id(SharedString::from(format!("schema-field-{owner}-{}", field.name)))
        .px_1()
        .py_0p5()
        .when_some(target.clone(), |this, _| this.cursor_pointer().hover(|style| style.bg(theme.muted)))
        .when_some(target, |this, target| {
            this.on_click(cx.listener(move |this, _, _, cx| {
                if let Some(panel) = this.schema_panel_mut() {
                    panel.open_type(Some(target.clone()));
                }
                cx.notify();
            }))
        })
        .child(
            h_flex()
                .flex_wrap()
                .font_family(theme.mono_font_family.clone())
                .text_sm()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(field.name.clone()))
                .child(div().text_color(theme.muted_foreground).child(format!("{args}: ")))
                .child(div().text_color(theme.primary).child(field.ty.clone())),
        )
        .when(!field.description.is_empty(), |this| {
            this.child(div().text_xs().text_color(theme.muted_foreground).child(field.description.clone()))
        })
        .into_any_element()
}

/// Field names for the query editor, from whatever schema the panel holds.
struct SchemaCompletions(Shared);

impl CompletionProvider for SchemaCompletions {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        _: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let items = match self.0.borrow().as_ref() {
            Some(schema) => schema
                .completions(&text.to_string(), offset)
                .into_iter()
                .map(|candidate| CompletionItem {
                    label: candidate.label,
                    detail: Some(candidate.detail),
                    kind: Some(CompletionItemKind::FIELD),
                    documentation: (!candidate.description.is_empty())
                        .then_some(Documentation::String(candidate.description)),
                    ..Default::default()
                })
                .collect(),
            None => Vec::new(),
        };
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    /// Typing a name asks. A line break or a brace does not: Enter would then pick a suggestion.
    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        !new_text.is_empty() && new_text.chars().all(|c| c.is_alphanumeric() || c == '_')
    }
}
