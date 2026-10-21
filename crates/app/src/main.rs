// An installed app opens no console window; a debug build keeps one for its output.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app_model;
mod bootstrap;
mod commands;
mod components;
mod engine;
mod i18n;
mod model;
mod panels;
mod preferences;
mod screens;
mod session;
mod shell;
#[cfg(test)]
mod test_support;
mod theme;
#[cfg(test)]
mod ui_tests;

use bifurc_engine_client::SidecarConfig;
use gpui_kit::component::TitleBar;
use gpui_kit::{Bounds, WindowBounds, WindowOptions, px, size};

use bootstrap::Bootstrap;
use preferences::{Preferences, WindowState};

const MIN_SIZE: (f32, f32) = (1080., 640.);

// Icons the app uses that are not in gpui-kit's default bundle (they render blank without this).
gpui_kit::assets::icon_assets!(ExtraIcons, [Lock, Mail, Webhook]);

struct AppAssets;

impl gpui_kit::AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        match ExtraIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<gpui_kit::SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

fn main() {
    bifurc_engine_client::crash::install("desktop", env!("CARGO_PKG_VERSION"), SidecarConfig::default().data_dir);
    gpui_kit::application().with_assets(AppAssets).run(|cx| {
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
        // Where it was left is used only while it still shows on a connected display; monitors come and go.
        let displays: Vec<_> = cx.displays().iter().map(|d| d.bounds()).collect();
        let window_bounds = Preferences::global(cx)
            .window
            .and_then(|saved| saved.restore(&displays, MIN_SIZE))
            .unwrap_or_else(|| WindowBounds::Windowed(Bounds::centered(None, size(px(1440.), px(900.)), cx)));
        let options = WindowOptions {
            window_bounds: Some(window_bounds),
            window_min_size: Some(size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
            // What a Linux desktop matches against `bifurc.desktop` to find the window's icon and name.
            app_id: Some("bifurc".to_owned()),
            ..TitleBar::window_options()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            // Written once, as the window closes, so dragging or resizing it never touches the file.
            window.on_window_should_close(cx, |window, cx| {
                let state = WindowState::of(window.window_bounds());
                Preferences::update(cx, |p| p.window = Some(state));
                true
            });
            gpui_kit::AppContext::new(cx, |cx| Bootstrap::new(window, cx))
        })
        .expect("failed to open the main window");

        cx.activate(true);
    });
}
