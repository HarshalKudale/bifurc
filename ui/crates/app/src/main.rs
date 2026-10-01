mod app_model;
mod bootstrap;
mod commands;
mod engine;
mod model;
mod panels;
mod session;
mod shell;

use gpui_kit::component::TitleBar;
use gpui_kit::{Bounds, WindowBounds, WindowOptions, px, size};

use bootstrap::Bootstrap;

fn main() {
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(|cx| {
        gpui_kit::init(cx);

        // Bound before anything that reads the keymap, so shortcuts show beside their commands.
        commands::bind_keys(cx);
        commands::register_actions(cx);

        // Window geometry is a physical, platform-level choice, not part of the rem-scaled layout.
        let bounds = Bounds::centered(None, size(px(1360.), px(860.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(960.), px(600.))),
            ..TitleBar::window_options()
        };
        let (_, bootstrap) = gpui_kit::open_window(options, cx, |window, cx| {
            gpui_kit::AppContext::new(cx, |cx| Bootstrap::new(window, cx))
        })
        .expect("failed to open the main window");

        // A UI that started the engine stops it when it quits; one that joined an engine leaves it running.
        cx.on_app_quit(move |cx| {
            let engine = cx.try_global::<engine::EngineSession>().cloned();
            async move {
                if let Some(engine) = engine {
                    engine.release();
                }
            }
        })
        .detach();
        let _ = bootstrap;
        cx.activate(true);
    });
}
