//! The Types panel of the right sidebar: `types.generate` infers a type for the open saved request from the
//! JSON responses it has had (its history and captured traffic), in TypeScript, Zod, Python or Go.
//!
//! It is read again when the language or the name changes, and when the request gets a new response.

use bifurc_engine_client::ClientError;
use gpui_kit::component::{
    Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState, Input, InputEvent, InputState},
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, Subscription, TestSupportExt as _, Window, div,
};
use serde_json::json;

use super::right_sidebar::RightSidebar;
use crate::{
    components::{Syntax, page},
    engine::{call_in, describe},
    i18n::t,
    model::kind::RequestKind,
};

/// The languages `types.generate` writes, as the engine names them, with their label and highlighting.
const LANGS: [(&str, &str, Syntax); 4] = [
    ("ts", "TypeScript", Syntax::JavaScript),
    ("zod", "Zod", Syntax::JavaScript),
    ("python", "Python", Syntax::Python),
    ("go", "Go", Syntax::Plain),
];

enum Generated {
    Idle,
    Loading,
    Done,
    /// The request has no JSON response yet.
    NoSamples,
    Failed(String),
}

pub struct TypesPanel {
    lang: usize,
    name: Entity<InputState>,
    code: Entity<EditorState>,
    generated: Generated,
    /// What the shown (or loading) code was asked for: request id, language, name. An answer for anything
    /// else is stale.
    asked: Option<(String, usize, String)>,
    _subscription: Subscription,
}

impl TypesPanel {
    pub fn new(window: &mut Window, cx: &mut Context<RightSidebar>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t("types.name_placeholder")));
        let code = cx.new(|cx| {
            let mut state = EditorState::new(window, cx).language(LANGS[0].2.language()).line_number(false);
            state.set_readonly(true, cx);
            state
        });
        let subscription = cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.generate_types(false, window, cx);
            }
        });
        Self { lang: 0, name, code, generated: Generated::Idle, asked: None, _subscription: subscription }
    }
}

impl RightSidebar {
    /// The open saved request whose responses can be typed.
    pub(super) fn types_request(&self) -> Option<String> {
        let target = self.target.as_ref()?;
        matches!(target.kind, Some(RequestKind::Rest | RequestKind::GraphQl)).then(|| target.entity_id.clone())?
    }

    /// Asks for the type again when what it is for changed, or always when `force` (a new response).
    pub(super) fn generate_types(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.types_request() else {
            self.types.asked = None;
            self.types.generated = Generated::Idle;
            return;
        };
        let name = self.types.name.read(cx).value().trim().to_owned();
        let key = (id.clone(), self.types.lang, name.clone());
        if !force && self.types.asked.as_ref() == Some(&key) {
            return;
        }
        self.types.asked = Some(key.clone());
        self.types.generated = Generated::Loading;
        let model = self.model.read(cx);
        let client = model.client().clone();
        let mut payload = json!({ "workspaceId": model.workspace_id(), "request": id, "lang": LANGS[key.1].0 });
        if !name.is_empty() {
            payload["name"] = json!(name);
        }
        call_in(&client, window, cx, "types.generate", payload, move |this, outcome, window, cx| {
            if this.types.asked.as_ref() != Some(&key) {
                return;
            }
            this.types.generated = match outcome {
                Ok(result) => {
                    let code = result["code"].as_str().unwrap_or_default().to_owned();
                    this.types.code.update(cx, |state, cx| {
                        state.set_highlighter(LANGS[key.1].2.language(), cx);
                        state.set_value(code, window, cx);
                    });
                    Generated::Done
                }
                Err(ClientError::Engine(error)) if error.key() == Some("types.no_samples") => Generated::NoSamples,
                Err(error) => Generated::Failed(describe(&error)),
            };
            cx.notify();
        });
        cx.notify();
    }

    pub(super) fn render_types(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        if self.types_request().is_none() {
            return match &self.target {
                None => page::empty(t("shell.right_sidebar.nothing_open"), t("types.open_request")),
                Some(_) => page::empty(t("types.unavailable"), t("types.unavailable_hint")),
            };
        }
        let body = match &self.types.generated {
            Generated::Idle | Generated::Loading => page::loading(t("types.loading")),
            Generated::NoSamples => page::empty(t("types.no_samples"), t("types.no_samples_hint")),
            Generated::Failed(message) => page::empty(t("types.failed"), message.clone()),
            Generated::Done => div()
                .id("types-code")
                .test_support()
                .size_full()
                .child(Editor::new(&self.types.code).bordered(false).h_full())
                .into_any_element(),
        };
        let done = matches!(self.types.generated, Generated::Done);
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().overflow_hidden().child(
                            TabBar::new("types-langs")
                                .segmented()
                                .small()
                                .selected_index(self.types.lang)
                                .on_click(cx.listener(|this, ix: &usize, window, cx| {
                                    this.types.lang = *ix;
                                    this.generate_types(false, window, cx);
                                }))
                                .children(LANGS.map(|(_, label, _)| Tab::new().label(label))),
                        ),
                    )
                    .child(
                        div().flex_none().child(
                            Button::new("copy-types")
                                .ghost()
                                .small()
                                .icon(IconName::Copy)
                                .tooltip(t("shell.right_sidebar.copy"))
                                .disabled(!done)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let text = this.types.code.read(cx).value().to_string();
                                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                                })),
                        ),
                    ),
            )
            .child(div().flex_none().px_3().pb_2().child(Input::new(&self.types.name).small()))
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}
