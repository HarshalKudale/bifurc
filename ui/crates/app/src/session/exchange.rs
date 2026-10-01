//! The response half of an HTTP-like request: what the engine sent back, as a dockable view.
//!
//! One `Exchange` belongs to one open request. The response panel shows whichever exchange belongs to the
//! active tab, so switching tabs switches responses without any copying.

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    alert::Alert,
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle},
    h_flex,
    input::{Textarea, TextareaState},
    spinner::Spinner,
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div,
};
use serde_json::Value;

use crate::model::http::{BodyEncoding, HttpResponse, status_text};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Body,
    Headers,
    Tests,
}

enum Phase {
    Idle,
    Sending,
    Failed(String),
    Http(HttpResponse),
    /// A result that is not HTTP-shaped (gRPC, SOAP), shown as the text the engine returned.
    Raw,
}

pub struct Exchange {
    phase: Phase,
    section: Section,
    body: Entity<TextareaState>,
}

impl Exchange {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let body = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx);
            state.set_readonly(true, cx);
            state
        });
        Self { phase: Phase::Idle, section: Section::Body, body }
    }

    pub fn begin(&mut self, cx: &mut Context<Self>) {
        self.phase = Phase::Sending;
        cx.notify();
    }

    pub fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        self.phase = Phase::Failed(message);
        cx.notify();
    }

    /// Takes the engine's send result. `None` from the HTTP reader means the result is another protocol's
    /// shape; it is shown as formatted JSON rather than refused.
    pub fn finish(
        &mut self,
        result: &Value,
        encoding: BodyEncoding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u16> {
        let (text, status) = match HttpResponse::from_value(result, encoding) {
            Some(response) => {
                let text = response.display_body();
                let status = response.status;
                self.phase = Phase::Http(response);
                (text, Some(status))
            }
            None => {
                let text = serde_json::to_string_pretty(result).unwrap_or_default();
                self.phase = Phase::Raw;
                (text, None)
            }
        };
        self.section = Section::Body;
        self.body.update(cx, |state, cx| state.set_value(text, window, cx));
        cx.notify();
        status
    }

    fn tabs(&self, response: &HttpResponse, cx: &mut Context<Self>) -> TabBar {
        let sections = [Section::Body, Section::Headers, Section::Tests];
        let selected = sections.iter().position(|s| *s == self.section).unwrap_or(0);
        TabBar::new("response-sections")
            .underline()
            .small()
            .selected_index(selected)
            .on_click(cx.listener(move |this, ix: &usize, _, cx| {
                this.section = sections[*ix];
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

    fn render_http(&mut self, response: &HttpResponse, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let content = match self.section {
            Section::Body => Textarea::new(&self.body).h_full().bordered(false).into_any_element(),
            Section::Headers => v_flex()
                .id("response-headers")
                .overflow_y_scroll()
                .size_full()
                .p_3()
                .gap_1()
                .children(response.headers.iter().map(|(name, value)| {
                    h_flex()
                        .gap_3()
                        .items_start()
                        .child(div().w_48().flex_none().text_color(theme.muted_foreground).child(name.clone()))
                        .child(div().flex_1().min_w_0().child(value.clone()))
                }))
                .into_any_element(),
            Section::Tests => v_flex()
                .id("response-tests")
                .overflow_y_scroll()
                .size_full()
                .p_3()
                .gap_2()
                .when(response.tests.is_empty(), |this| {
                    this.child(div().text_color(theme.muted_foreground).child("This request has no tests."))
                })
                .children(response.tests.iter().map(|test| {
                    h_flex()
                        .gap_2()
                        .items_start()
                        .child(if test.passed { Tag::success().child("Passed") } else { Tag::danger().child("Failed") })
                        .child(v_flex().child(test.name.clone()).when_some(test.error.clone(), |this, error| {
                            this.child(div().text_sm().text_color(theme.muted_foreground).child(error))
                        }))
                }))
                .into_any_element(),
        };

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .child(Self::status_tag(response))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(format!("{} ms", response.duration_ms)),
                    )
                    .child(div().text_sm().text_color(theme.muted_foreground).child(response.body_size_label())),
            )
            .children(response.script_errors.iter().map(|error| {
                div().px_3().pb_2().child(Alert::warning("script-error", SharedString::from(error.clone())))
            }))
            .child(self.tabs(response, cx))
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}

use gpui_kit::prelude::FluentBuilder as _;

impl Render for Exchange {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.phase {
            Phase::Idle => Empty::new()
                .header(
                    EmptyHeader::new()
                        .title(EmptyTitle::new().child("No response yet"))
                        .description(EmptyDescription::new().child("Send the request to see what comes back.")),
                )
                .into_any_element(),
            Phase::Sending => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .child(Spinner::new())
                .child("Waiting for the response…")
                .into_any_element(),
            Phase::Failed(message) => div()
                .p_3()
                .child(
                    Alert::error("send-failed", SharedString::from(message.clone())).title("Couldn't send the request"),
                )
                .into_any_element(),
            Phase::Http(_) => {
                // `render_http` needs `&mut self` for the tab handlers while borrowing the response, so the
                // response is moved out for the call and put back.
                let Phase::Http(response) = std::mem::replace(&mut self.phase, Phase::Idle) else { unreachable!() };
                let element = self.render_http(&response, cx);
                self.phase = Phase::Http(response);
                element
            }
            Phase::Raw => Textarea::new(&self.body).h_full().bordered(false).into_any_element(),
        };
        div().size_full().bg(cx.theme().background).child(body)
    }
}
