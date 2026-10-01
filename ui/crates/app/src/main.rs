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
mod system_tray;
mod theme;

use bifurc_engine_client::SidecarConfig;
use futures::StreamExt as _;
use gpui_kit::component::TitleBar;
use gpui_kit::{Bounds, Global, WindowBounds, WindowOptions, px, size};

use bootstrap::Bootstrap;
use preferences::Preferences;
use system_tray::{SystemTray, TrayEvent};

impl Global for SystemTray {}

fn main() {
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(|cx| {
        gpui_kit::init(cx);

        cx.set_quit_mode(gpui_kit::QuitMode::Explicit);

        // The look is in place before the first frame: the bundled themes, then this client's choice.
        theme::init(cx);
        let preferences = Preferences::load(SidecarConfig::default().data_dir);
        preferences.apply(cx);
        cx.set_global(preferences);

        // The icon goes up before the window does, so a click during startup already has somewhere to go.
        let tray = SystemTray::new();
        let (tray_tx, tray_rx) = futures::channel::mpsc::unbounded();
        tray.set_event_sender(tray_tx);
        tray.setup_tray_icon();
        cx.set_global(tray.clone());

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
        let (window, bootstrap) = gpui_kit::open_window(options, cx, |window, cx| {
            // Closing the window hides it instead of destroying it: `QuitMode::Explicit` already keeps the
            // app running, and the tray icon is what brings the window back. The icon is therefore the only
            // way back, so a close is only intercepted once one is actually in the notification area.
            window.on_window_should_close(cx, |window, cx| {
                let Some(tray) = cx.try_global::<SystemTray>() else { return true };
                if !tray.is_installed() {
                    return true;
                }
                let Some(handle) = system_tray::platform::window_handle(window) else { return true };
                system_tray::platform::hide_window(handle);
                false
            });
            gpui_kit::AppContext::new(cx, |cx| Bootstrap::new(window, cx))
        })
        .expect("failed to open the main window");

        // A UI that started the engine stops it when it quits; one that joined an engine leaves it running.
        // Closing the window is not quitting — it hides, and the engine keeps running — so this runs on the
        // `Quit` command and on the tray menu, both of which go through `cx.quit()`.
        cx.on_app_quit(move |cx| {
            let engine = cx.try_global::<engine::EngineSession>().cloned();
            async move {
                if let Some(engine) = engine {
                    engine.release();
                }
            }
        })
        .detach();

        // A click arrives on the tray's own thread. Showing a window and quitting the app both belong to
        // this one, so the click is carried here before it is acted on.
        cx.spawn(async move |cx| {
            // `mut` on the outer binding does not carry into the task, so it is restated.
            let mut tray_rx = tray_rx;
            while let Some(event) = tray_rx.next().await {
                match event {
                    TrayEvent::Show => {
                        window
                            .update(cx, |_, window, _| {
                                if let Some(handle) = system_tray::platform::window_handle(window) {
                                    system_tray::platform::show_window(handle);
                                }
                            })
                            .ok();
                    }
                    TrayEvent::Quit => {
                        cx.update(|cx| cx.quit());
                        return;
                    }
                }
            }
        })
        .detach();

        let _ = bootstrap;
        cx.activate(true);
    });
}
