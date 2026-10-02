mod app_model;
mod bootstrap;
mod commands;
mod components;
mod engine;
mod model;
mod panels;
mod preferences;
mod screens;
mod session;
mod shell;
mod theme;

use bifurc_engine_client::SidecarConfig;
use gpui_kit::component::TitleBar;
use gpui_kit::{Bounds, WindowBounds, WindowOptions, px, size};

use bootstrap::Bootstrap;
use preferences::Preferences;

fn main() {
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(|cx| {
        gpui_kit::init(cx);

        // Closing the window closes this app, and only this app. The engine is a process of its own with its
        // own tray icon: it keeps running, and its icon opens this app again.
        cx.set_quit_mode(gpui_kit::QuitMode::LastWindowClosed);

        // The look is in place before the first frame: the bundled themes, then this client's choice.
        theme::init(cx);
        let preferences = Preferences::load(SidecarConfig::default().data_dir);
        preferences.apply(cx);
        cx.set_global(preferences);

        // Bound before anything that reads the keymap, so shortcuts show beside their commands.
        commands::bind_keys(cx);
        commands::register_actions(cx);

        // Window geometry is a physical, platform-level choice, not part of the rem-scaled layout.
        let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(1080.), px(640.))),
            ..TitleBar::window_options()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            gpui_kit::AppContext::new(cx, |cx| Bootstrap::new(window, cx))
        })
        .expect("failed to open the main window");

        cx.activate(true);
    });
}
