//! The response half of an HTTP-like request: what the engine sent back.
//!
//! One `Exchange` belongs to one open request. The response panel shows whichever exchange belongs to the
//! active tab, so switching tabs switches responses without any copying.
//!
//! The body is shown in a read-only code editor highlighted for its content type, either re-indented
//! ("Pretty") or exactly as received ("Raw").

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState},
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
};
use serde_json::Value;

use crate::{
    components::{Syntax, page},
    model::http::{BodyEncoding, HttpResponse, pretty_json, status_text},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Body,
    Headers,
    Tests,
}

const SECTIONS: [Section; 3] = [Section::Body, Section::Headers, Section::Tests];

#[derive(Clone, Copy, PartialEq, Eq)]
enum BodyView {
    Pretty,
    Raw,
}

enum Phase {
    Idle,
    Sending,
    Failed(String),
    Http(HttpResponse),
    /// A result that is not HTTP-shaped (gRPC, SOAP), shown as the JSON the engine returned.
    Raw,
}

pub struct Exchange {
    phase: Phase,
    section: Section,
    body_view: BodyView,
    body: Entity<EditorState>,
    /// The body as received, and re-indented when that is possible. Both are kept so switching views does
    /// not parse again.
    raw_text: String,
    pretty_text: Option<String>,
}

impl Exchange {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let body = cx.new(|cx| {
            let mut state =
                EditorState::new(window, cx).language(Syntax::Plain.language()).line_number(true).soft_wrap(true);
            state.set_readonly(true, cx);
            state
        });
        Self {
            phase: Phase::Idle,
            section: Section::Body,
            body_view: BodyView::Pretty,
            body,
            raw_text: String::new(),
            pretty_text: None,
        }
    }

    pub fn begin(&mut self, cx: &mut Context<Self>) {
        self.phase = Phase::Sending;
        cx.notify();
    }

    pub fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        self.phase = Phase::Failed(message);
        cx.notify();
    }

    /// Takes the engine's send result. A result the HTTP reader does not recognise is another protocol's
    /// shape; it is shown as formatted JSON rather than refused. Returns the HTTP status, if there is one.
    pub fn finish(
        &mut self,
        result: &Value,
        encoding: BodyEncoding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u16> {
        let (syntax, status) = match HttpResponse::from_value(result, encoding) {
            Some(response) => {
                let syntax = if response.is_binary {
                    Syntax::Plain
                } else {
                    match Syntax::for_content_type(response.content_type().unwrap_or_default()) {
                        // A JSON body is recognised by its content too: servers often mislabel it.
                        Syntax::Plain if pretty_json(&response.body).is_some() => Syntax::Json,
                        other => other,
                    }
                };
                self.raw_text = response.display_body_raw();
                self.pretty_text = (syntax == Syntax::Json).then(|| pretty_json(&response.body)).flatten();
                let status = response.status;
                self.phase = Phase::Http(response);
                (syntax, Some(status))
            }
            None => {
                self.raw_text = serde_json::to_string(result).unwrap_or_default();
                self.pretty_text = serde_json::to_string_pretty(result).ok();
                self.phase = Phase::Raw;
                (Syntax::Json, None)
            }
        };
        self.section = Section::Body;
        self.body.update(cx, |state, cx| state.set_highlighter(syntax.language(), cx));
        self.show_body(window, cx);
        cx.notify();
        status
    }

    fn show_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = match (self.body_view, &self.pretty_text) {
            (BodyView::Pretty, Some(pretty)) => pretty.clone(),
            _ => self.raw_text.clone(),
        };
        self.body.update(cx, |state, cx| state.set_value(text, window, cx));
    }

    fn set_body_view(&mut self, view: BodyView, window: &mut Window, cx: &mut Context<Self>) {
        if self.body_view != view {
            self.body_view = view;
            self.show_body(window, cx);
            cx.notify();
        }
    }

    fn tabs(&self, response: &HttpResponse, cx: &mut Context<Self>) -> TabBar {
        let selected = SECTIONS.iter().position(|s| *s == self.section).unwrap_or(0);
        TabBar::new("response-sections")
            .underline()
            .small()
            .selected_index(selected)
            .on_click(cx.listener(move |this, ix: &usize, _, cx| {
                this.section = SECTIONS[*ix];
                cx.notify();
            }))
            .child(Tab::new().label("Body"))
            .child(Tab::new().label(format!("Headers ({})", response.headers.len())))
            .child(Tab::new().label(if response.tests.is_empty() {
                "Tests".to_owned()
            } else {
                let passed = response.tests.iter().filter(|t| t.passed).count();
                format!("Tests ({passed}/{})", response.tests.len())
            }))
    }

    fn status_tag(response: &HttpResponse) -> Tag {
        let tag = if response.is_success() { Tag::success() } else { Tag::danger() };
        let reason = status_text(response.status);
        tag.child(if reason.is_empty() { response.status.to_string() } else { format!("{} {reason}", response.status) })
    }

    /// The Pretty/Raw choice and Copy, shown above a body.
    fn render_body_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_pretty = self.pretty_text.is_some();
        h_flex()
            .flex_none()
            .px_3()
            .py_2()
            .gap_2()
            .when(has_pretty, |this| {
                this.child(
                    TabBar::new("body-view")
                        .segmented()
                        .small()
                        .selected_index(if self.body_view == BodyView::Pretty { 0 } else { 1 })
                        .on_click(cx.listener(|this, ix: &usize, window, cx| {
                            this.set_body_view(if *ix == 0 { BodyView::Pretty } else { BodyView::Raw }, window, cx)
                        }))
                        .child(Tab::new().label("Pretty"))
                        .child(Tab::new().label("Raw")),
                )
            })
            .child(div().flex_1())
            .child(Button::new("copy-body").ghost().small().icon(IconName::Copy).label("Copy").on_click(cx.listener(
                |this, _, _, cx| {
                    let text = this.body.read(cx).value().to_string();
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                },
            )))
    }

    fn render_body(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        v_flex()
            .size_full()
            .child(self.render_body_toolbar(cx))
            .child(div().flex_1().min_h_0().child(Editor::new(&self.body).bordered(false).h_full()))
            .into_any_element()
    }

    fn render_http(&self, response: &HttpResponse, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let content =
            match self.section {
                Section::Body if response.is_binary => page::empty(
                    "Binary body",
                    format!("{} of data that can't be shown as text.", response.body_size_label()),
                ),
                Section::Body if response.size_bytes == 0 => page::empty("Empty body", "The response has no body."),
                Section::Body => self.render_body(cx),
                Section::Headers => v_flex()
                    .id("response-headers")
                    .overflow_y_scroll()
                    .size_full()
                    .p_3()
                    .gap_2()
                    .children(response.headers.iter().map(|(name, value)| {
                        h_flex()
                            .gap_3()
                            .items_start()
                            .text_sm()
                            .child(div().w_48().flex_none().text_color(theme.muted_foreground).child(name.clone()))
                            .child(div().flex_1().min_w_0().child(value.clone()))
                    }))
                    .into_any_element(),
                Section::Tests if response.tests.is_empty() => {
                    page::empty("No tests ran", "Add a test script to the request to check its response.")
                }
                Section::Tests => v_flex()
                    .id("response-tests")
                    .overflow_y_scroll()
                    .size_full()
                    .p_3()
                    .gap_2()
                    .children(response.tests.iter().map(|test| {
                        h_flex()
                            .gap_2()
                            .items_start()
                            .child(div().w_16().flex_none().child(if test.passed {
                                Tag::success().child("Passed")
                            } else {
                                Tag::danger().child("Failed")
                            }))
                            .child(v_flex().flex_1().min_w_0().child(test.name.clone()).when_some(
                                test.error.clone(),
                                |this, error| {
                                    this.child(div().text_sm().text_color(theme.muted_foreground).child(error))
                                },
                            ))
                    }))
                    .into_any_element(),
            };

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .px_3()
                    .pt_2()
                    .gap_3()
                    .child(Self::status_tag(response))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(format!("{} ms", response.duration_ms)),
                    )
                    .child(div().text_sm().text_color(theme.muted_foreground).child(response.body_size_label())),
            )
            .children(response.script_errors.iter().enumerate().map(|(ix, error)| {
                div().px_3().pt_2().child(
                    Alert::warning(("script-error", ix), SharedString::from(error.clone())).title("A script failed"),
                )
            }))
            .child(div().flex_none().px_3().child(self.tabs(response, cx)))
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}

impl Render for Exchange {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.phase {
            Phase::Idle => page::empty("No response yet", "Send the request to see what comes back."),
            Phase::Sending => page::loading("Waiting for the response…"),
            Phase::Failed(message) => div()
                .p_3()
                .child(
                    Alert::error("send-failed", SharedString::from(message.clone())).title("Couldn't send the request"),
                )
                .into_any_element(),
            Phase::Http(_) => {
                // The renderer borrows `self` for its handlers while reading the response, so the response
                // is moved out for the call and put back.
                let Phase::Http(response) = std::mem::replace(&mut self.phase, Phase::Idle) else { unreachable!() };
                let element = self.render_http(&response, cx);
                self.phase = Phase::Http(response);
                element
            }
            Phase::Raw => self.render_body(cx),
        };
        div().size_full().bg(cx.theme().background).child(body)
    }
}
