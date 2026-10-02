//! The Bifurc engine's notification-area icon.
//!
//! The icon belongs to the engine, not to an interface: it is up while the engine runs, whoever started it,
//! so an engine left behind by a closed window or a closed terminal can still be reached and stopped. The
//! engine is a Node process and cannot place an icon itself, so it runs this helper as a child
//! (`packages/engine/src/runtime/desktop.ts`) and the two talk in lines over the helper's stdio:
//!
//! - the helper prints `ready` once the icon is up, then `open`, `restart` or `quit` as the user picks them;
//! - the engine closes the helper's stdin to take the icon down, which also happens by itself if the engine
//!   dies.
//!
//! The helper decides nothing. What `open` opens, and what a restart or a quit involves, is the engine's.

// No console window when the engine starts it.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    tray::run();
}

/// No icon is placed off Windows yet. The engine reads the missing `ready` as "no tray" and stops when its
/// last interface closes instead.
#[cfg(not(windows))]
fn main() {
    eprintln!("bifurc-tray: no tray icon on this platform");
    std::process::exit(1);
}

#[cfg(windows)]
mod tray {
    use std::io::{Read as _, Write as _};

    use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tray_icon::{ClickEvent, TrayIconBuilder};
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_QUIT,
    };

    /// Tells the engine something. A line that cannot be written means the engine is gone, and so is the
    /// reason for the icon.
    fn say(line: &str) {
        let mut out = std::io::stdout().lock();
        if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
            std::process::exit(0);
        }
    }

    pub fn run() {
        let open = MenuItem::new("Open Bifurc", true, None);
        let restart = MenuItem::new("Restart engine", true, None);
        let quit = MenuItem::new("Quit Bifurc", true, None);
        let (open_id, restart_id, quit_id) = (open.id(), restart.id(), quit.id());
        // Right click shows this menu; a left click is handled as a plain click below, so the two do not
        // collide.
        let menu = Menu::new();
        menu.append_items(&[&open, &restart, &PredefinedMenuItem::separator(), &quit]);

        // Held until the message loop ends: dropping the icon takes it out of the notification area.
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Bifurc")
            .with_icon(tray_icon_from_png())
            .build();
        let Ok(_icon) = icon else {
            std::process::exit(1);
        };

        // Clicks and menu picks are published on separate channels, and a blocking receive on one would
        // stall the other, so each gets a reader.
        let click_events = tray_icon::tray_event_receiver().clone();
        std::thread::spawn(move || {
            while let Ok(event) = click_events.recv() {
                // A right click is the menu's; it is reported here as well, and ignored.
                if matches!(event.event, ClickEvent::Left | ClickEvent::Double) {
                    say("open");
                }
            }
        });
        let menu_events = tray_icon::menu::menu_event_receiver().clone();
        std::thread::spawn(move || {
            while let Ok(event) = menu_events.recv() {
                if event.id == open_id {
                    say("open");
                } else if event.id == restart_id {
                    say("restart");
                } else if event.id == quit_id {
                    say("quit");
                }
            }
        });

        // The engine closing stdin (or dying, which closes it too) ends the message loop from outside. The
        // icon's window exists by now, so this thread has a queue for the message to land in.
        let main_thread = unsafe { GetCurrentThreadId() };
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let mut buffer = [0u8; 64];
            while matches!(stdin.read(&mut buffer), Ok(read) if read > 0) {}
            unsafe {
                let _ = PostThreadMessageW(main_thread, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        });

        say("ready");

        // The icon's clicks arrive as window messages on the thread that created it.
        let mut message = MSG::default();
        unsafe {
            while GetMessageW(&mut message, HWND(0), 0, 0).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    /// The notification-area image, from the 32×32 PNG the desktop build already ships.
    ///
    /// Embedded rather than read from disk, because a path that holds while the helper runs from the
    /// repository stops holding the moment it is packaged. Both failures below are broken-build failures,
    /// not runtime conditions.
    fn tray_icon_from_png() -> tray_icon::icon::Icon {
        const PNG: &[u8] = include_bytes!("../../../../build/tray-icon.png");

        let decoded = (|| {
            let mut reader = png::Decoder::new(PNG).read_info()?;
            let mut rgba = vec![0; reader.output_buffer_size()];
            let info = reader.next_frame(&mut rgba)?;
            rgba.truncate(info.buffer_size());
            Ok::<_, png::DecodingError>((info.width, info.height, rgba))
        })();

        let (width, height, rgba) = decoded.expect("the bundled tray icon should decode");
        tray_icon::icon::Icon::from_rgba(rgba, width, height).expect("the bundled tray icon should be 32-bit RGBA")
    }
}
