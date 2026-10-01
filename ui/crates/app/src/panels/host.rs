//! A dock panel that shows whichever view its screen points it at.
//!
//! A screen with tabs keeps one dock area, not one per tab: its panels ("Request", "Response") stay where the
//! user put them and follow the active tab. Each such panel is a `HostPanel`; the screen hands it the active
//! tab's view, and with no tab open it says what would appear there.

use gpui_kit::component::{
    ActiveTheme as _,
    dock::{BasePanel, Panel, PanelEvent},
};
use gpui_kit::{
    AnyView, App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Window, div,
};

use crate::components::page;

pub struct HostPanel {
    focus_handle: FocusHandle,
    name: &'static str,
    title: SharedString,
    empty_title: SharedString,
    empty_description: SharedString,
    view: Option<AnyView>,
}

impl HostPanel {
    /// `name` identifies the panel to the dock and must be unique within its dock area.
    pub fn new(
        name: &'static str,
        title: impl Into<SharedString>,
        empty_title: impl Into<SharedString>,
        empty_description: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            name,
            title: title.into(),
            empty_title: empty_title.into(),
            empty_description: empty_description.into(),
            view: None,
        }
    }

    pub fn set_view(&mut self, view: Option<AnyView>, cx: &mut Context<Self>) {
        self.view = view;
        cx.notify();
    }
}

impl EventEmitter<PanelEvent> for HostPanel {}

impl Focusable for HostPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for HostPanel {
    fn panel_name(&self) -> &'static str {
        self.name
    }

    fn closable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for HostPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.title.clone()
    }

    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some(self.title.clone())
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for HostPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.view {
            Some(view) => view.clone().into_any_element(),
            None => page::empty(self.empty_title.clone(), self.empty_description.clone()),
        };
        div().size_full().bg(cx.theme().background).child(body)
    }
}
