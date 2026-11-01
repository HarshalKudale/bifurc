//! What the window shows while a webhook is being captured: a blocker over everything, with a pulse, what is
//! going on, and Cancel. It opens the moment Capture is clicked and says the address is being made; when the
//! engine has the address, the same view shows it, so there is no gap with nothing on screen.

use std::{rc::Rc, time::Duration};

use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, button::Button, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, App, ClipboardItem, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Window, div, pulsating_between,
};

use crate::i18n::t;

pub struct CaptureOverlay {
    /// `None` while the engine is still opening the tunnel.
    url: Option<String>,
    #[allow(clippy::type_complexity)]
    on_cancel: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl CaptureOverlay {
    pub fn new(on_cancel: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { url: None, on_cancel: Rc::new(on_cancel) }
    }

    pub fn set_url(&mut self, url: String, cx: &mut Context<Self>) {
        self.url = Some(url);
        cx.notify();
    }
}

impl Render for CaptureOverlay {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let (title, hint) = match &self.url {
            None => (t("session.capture.generating"), t("session.capture.generating_hint")),
            Some(_) => (t("session.capture.title"), t("session.capture.instructions")),
        };
        let cancel = self.on_cancel.clone();
        v_flex()
            .id("capture-overlay")
            .size_full()
            .min_h_80()
            .items_center()
            .justify_center()
            .gap_5()
            .child(div().flex_none().size_16().rounded_full().bg(theme.primary).with_animation(
                "capture-pulse",
                Animation::new(Duration::from_millis(1400)).repeat().with_easing(pulsating_between(0.15, 1.0)),
                |dot, delta| dot.opacity(delta),
            ))
            .child(div().text_xl().font_medium().child(title))
            .child(div().max_w_96().text_center().text_sm().text_color(theme.muted_foreground).child(hint))
            .children(self.url.clone().map(|url| {
                let copy = url.clone();
                h_flex()
                    .w_full()
                    .max_w_96()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(theme.border)
                            .font_family(theme.mono_font_family.clone())
                            .text_sm()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(url),
                    )
                    .child(
                        Button::new("capture-copy")
                            .outline()
                            .small()
                            .label(t("session.capture.copy"))
                            .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))),
                    )
            }))
            .child(
                Button::new("capture-cancel")
                    .outline()
                    .label(t("components.cancel"))
                    .on_click(move |_, window, cx| cancel(window, cx)),
            )
    }
}
