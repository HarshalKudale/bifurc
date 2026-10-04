//! The box a cURL command is pasted into, to start a request or a mock from it.
//!
//! It fills a new tab. The command goes to the engine as it was pasted; the engine reads it and answers with
//! the request (and its type) or the mock it describes, and the screen that owns the tab turns the tab into
//! that. Nothing about the command is decided here: text the engine cannot use comes back as its reason.

use gpui_kit::component::{
    ActiveTheme as _, StyledExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, InputEvent},
    v_flex,
};
use gpui_kit::{
    Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription,
    Window, div, rems,
};
use serde_json::{Value, json};

use super::{Syntax, VarEditor};
use crate::{
    app_model::AppModel,
    engine::{call, describe},
};

/// What the tab asking for a command is called until it becomes what the command describes.
pub const CURL_TAB_TITLE: &str = "Import cURL";
pub const CURL_TAB_BADGE: &str = "cURL";

/// What the pasted command becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurlTarget {
    Request,
    Mock,
}

impl CurlTarget {
    fn command(self) -> &'static str {
        match self {
            CurlTarget::Request => "curl.buildRequest",
            CurlTarget::Mock => "curl.buildMock",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            CurlTarget::Request => {
                "Paste a cURL command. This tab becomes the request it describes: REST, GraphQL, SOAP or WebSocket."
            }
            CurlTarget::Mock => "Paste a cURL command. This tab becomes a mock of the call it makes.",
        }
    }
}

#[derive(Debug, Clone)]
pub enum CurlImportEvent {
    /// The engine's answer for the command: `{ kind, request }` for a request, `{ mock }` for a mock.
    Built(Value),
}

pub struct CurlImport {
    model: Entity<AppModel>,
    target: CurlTarget,
    input: VarEditor,
    /// Whether the box held nothing before its last change: text arriving all at once in an empty box was
    /// pasted, and is sent without a further step.
    was_empty: bool,
    busy: bool,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CurlImportEvent> for CurlImport {}

impl CurlImport {
    pub fn new(model: Entity<AppModel>, target: CurlTarget, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = VarEditor::pasted(Syntax::Shell, "curl https://api.example.test/resource -H 'Accept: application/json'", window, cx);
        let on_change = cx.subscribe(input.state(), |this, _, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            let text = this.input.value(cx);
            let pasted = this.was_empty && text.trim().chars().count() > 1;
            this.was_empty = text.trim().is_empty();
            this.error = None;
            if pasted {
                this.submit(cx);
            }
            cx.notify();
        });
        Self { model, target, input, was_empty: true, busy: false, error: None, _subscriptions: vec![on_change] }
    }

    /// Puts the cursor in the box, so a command can be pasted straight away.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.state().update(cx, |state, cx| state.focus(window, cx));
    }

    /// Sends the command to the engine. Its answer leaves as [`CurlImportEvent::Built`]; its refusal is shown
    /// under the box.
    pub fn submit(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "curl": self.input.value(cx) });
        call(&client, cx, self.target.command(), payload, |this, outcome, cx| {
            this.busy = false;
            match outcome {
                Ok(built) => cx.emit(CurlImportEvent::Built(built)),
                Err(error) => this.error = Some(describe(&error).into()),
            }
            cx.notify();
        });
    }
}

impl Render for CurlImport {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let error = self.error.clone().map(|message| Alert::error("curl-refused", message).title("Can't import this"));
        div().size_full().flex().justify_center().bg(theme.background).child(
            v_flex()
                .w(rems(52.))
                .max_w_full()
                .h_full()
                .p_6()
                .gap_3()
                .child(
                    v_flex()
                        .flex_none()
                        .gap_1()
                        .child(div().font_semibold().child(CURL_TAB_TITLE))
                        .child(div().text_sm().text_color(theme.muted_foreground).child(self.target.hint())),
                )
                .child(div().flex_1().min_h(rems(8.)).max_h(rems(24.)).child(Editor::new(self.input.state()).h_full()))
                .children(error)
                .child(
                    h_flex().flex_none().child(
                        Button::new("curl-import")
                            .primary()
                            .label("Import")
                            .loading(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    ),
                ),
        )
    }
}
