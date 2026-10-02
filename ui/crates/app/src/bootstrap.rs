//! The window's root view: it is on screen before the engine is, then becomes the shell.
//!
//! Starting the sidecar can take seconds (a Node process boots, binds its ports, writes `runtime.json`), so
//! it runs on a background thread while this view shows what is happening. If it fails, the view says why
//! and offers a retry instead of leaving an empty window.

use bifurc_engine_client::{SidecarConfig, attach_or_start};
use gpui_kit::component::{ActiveTheme as _, TitleBar, alert::Alert, button::Button, h_flex, spinner::Spinner, v_flex};
use gpui_kit::{AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Window, div};

use crate::{
    app_model::AppModel,
    engine::{EngineSession, sidecar_config},
    shell::Shell,
};

enum Phase {
    Starting,
    Failed(String),
    Ready(Entity<Shell>),
}

pub struct Bootstrap {
    phase: Phase,
    data_dir: String,
}

impl Bootstrap {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = sidecar_config();
        let mut this = Self { phase: Phase::Starting, data_dir: config.data_dir.display().to_string() };
        this.start(config, window, cx);
        this
    }

    fn start(&mut self, config: SidecarConfig, window: &mut Window, cx: &mut Context<Self>) {
        self.phase = Phase::Starting;
        let starting = cx.background_spawn(async move {
            attach_or_start(&config, env!("CARGO_PKG_VERSION")).map_err(|e| e.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let outcome = starting.await;
            this.update_in(cx, |this, window, cx| match outcome {
                Ok((client, hello, sidecar)) => {
                    let engine = EngineSession::new(client, hello, sidecar);
                    cx.set_global(engine.clone());
                    let model = cx.new(|cx| AppModel::new(engine, cx));
                    let shell = cx.new(|cx| Shell::new(model, window, cx));
                    this.phase = Phase::Ready(shell);
                    cx.notify();
                }
                Err(message) => {
                    this.phase = Phase::Failed(message);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start(sidecar_config(), window, cx);
        cx.notify();
    }

    fn render_message(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        match &self.phase {
            Phase::Starting => v_flex()
                .gap_3()
                .items_center()
                .child(Spinner::new())
                .child("Starting the Bifurc engine…")
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child(self.data_dir.clone()))
                .into_any_element(),
            Phase::Failed(message) => v_flex()
                .gap_3()
                .max_w_128()
                .child(Alert::error("engine-failed", message.clone()).title("Couldn't start the engine"))
                .child(
                    h_flex().justify_end().child(
                        Button::new("retry-engine")
                            .label("Try again")
                            .on_click(cx.listener(|this, _, window, cx| this.retry(window, cx))),
                    ),
                )
                .into_any_element(),
            Phase::Ready(_) => div().into_any_element(),
        }
    }
}

impl Render for Bootstrap {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Phase::Ready(shell) = &self.phase {
            return shell.clone().into_any_element();
        }
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(TitleBar::new().child("Bifurc"))
            .child(div().flex_1().flex().items_center().justify_center().child(self.render_message(cx)))
            .into_any_element()
    }
}
