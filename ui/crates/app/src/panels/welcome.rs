//! What the editor area shows before any request is open.

use gpui_kit::component::{
    ActiveTheme as _, StyledExt as _,
    button::Button,
    dock::{BasePanel, Panel, PanelEvent},
    h_flex, v_flex,
};
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div,
};

use crate::commands::{NewRestRequest, NewWebSocketRequest, OpenCommandPalette};

pub struct WelcomePanel {
    focus_handle: FocusHandle,
}

impl WelcomePanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self { focus_handle: cx.focus_handle() }
    }
}

impl EventEmitter<PanelEvent> for WelcomePanel {}

impl Focusable for WelcomePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for WelcomePanel {
    fn panel_name(&self) -> &'static str {
        "WelcomePanel"
    }
}

impl Panel for WelcomePanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "Welcome"
    }
}

impl Render for WelcomePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        div().size_full().flex().items_center().justify_center().bg(theme.background).child(
            v_flex()
                .gap_4()
                .items_center()
                .child(div().text_xl().font_semibold().child("Bifurc"))
                .child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child("Open a request from the explorer, or start a new one."),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("welcome-new-rest")
                                .label("New request")
                                .on_click(|_, window, cx| window.dispatch_action(Box::new(NewRestRequest), cx)),
                        )
                        .child(
                            Button::new("welcome-new-ws")
                                .label("New WebSocket")
                                .on_click(|_, window, cx| window.dispatch_action(Box::new(NewWebSocketRequest), cx)),
                        )
                        .child(
                            Button::new("welcome-palette")
                                .label("Commands…")
                                .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenCommandPalette), cx)),
                        ),
                ),
        )
    }
}
