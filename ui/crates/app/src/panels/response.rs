//! The response panel: shows what the active request produced — an HTTP response, or a WebSocket's stream.
//!
//! It is one dockable view for every request type. The shell hands it the active session's response view
//! (see `Session::response_view`) and it hosts that view; with no active request it says so.

use gpui_kit::component::{
    ActiveTheme as _,
    dock::{BasePanel, Panel, PanelEvent},
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle},
};
use gpui_kit::{
    AnyView, App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement as _, Render, Styled as _,
    Window, div,
};

pub struct ResponsePanel {
    focus_handle: FocusHandle,
    view: Option<AnyView>,
}

impl ResponsePanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self { focus_handle: cx.focus_handle(), view: None }
    }

    pub fn set_view(&mut self, view: Option<AnyView>, cx: &mut Context<Self>) {
        self.view = view;
        cx.notify();
    }
}

impl EventEmitter<PanelEvent> for ResponsePanel {}

impl Focusable for ResponsePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ResponsePanel {
    fn panel_name(&self) -> &'static str {
        "ResponsePanel"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for ResponsePanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "Response"
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for ResponsePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.view {
            Some(view) => view.clone().into_any_element(),
            None => Empty::new()
                .header(EmptyHeader::new().title(EmptyTitle::new().child("No request open")).description(
                    EmptyDescription::new().child("Open a request from the explorer to see its response here."),
                ))
                .into_any_element(),
        };
        div().size_full().bg(cx.theme().background).child(body)
    }
}
