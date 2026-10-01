//! The notification-area icon: the way back to a window the user has closed, and the way out of the app.
//!
//! Closing the window hides it rather than destroying it (see `main`), so without an icon a hidden window
//! would be unreachable. The icon is therefore the only way back, which is why `main` asks
//! [`SystemTray::is_installed`] before it agrees to hide anything.
//!
//! Nothing here touches GPUI. Windows delivers tray clicks through the same message loop GPUI runs, but the
//! icon libraries publish them on their own channels, read on threads of our own, and a window may only be
//! shown — or the app asked to quit — from the main thread. So this module only *forwards*: clicks become
//! [`TrayEvent`]s, and `main` acts on them.

use std::sync::{Arc, Mutex};

/// What a click on the tray icon asks the app to do.
pub enum TrayEvent {
    /// Show the main window, in front of whatever the user is looking at.
    Show,
    /// Quit for good, the way the `Quit` command does — not another hide.
    Quit,
}

/// The tray's line into the main thread.
///
/// A type of its own so the icon — which holds a menu and is not `Send` — stays out of the reader threads:
/// they hold only this.
#[derive(Clone)]
struct Events(Arc<Mutex<Option<Sender>>>);

/// Unbounded, so a click never blocks the thread Windows delivered it on.
type Sender = futures::channel::mpsc::UnboundedSender<TrayEvent>;

impl Events {
    fn forward(&self, event: TrayEvent) {
        if let Ok(events) = self.0.lock()
            && let Some(sender) = events.as_ref()
        {
            // A closed receiver means the app is already on its way out; there is nothing left to tell.
            let _ = sender.unbounded_send(event);
        }
    }
}

#[derive(Clone)]
pub struct SystemTray {
    events: Events,
    /// Held for as long as the app runs: dropping the icon takes it out of the notification area.
    #[cfg(windows)]
    icon: Arc<Mutex<Option<tray_icon::TrayIcon>>>,
}

impl SystemTray {
    pub fn new() -> Self {
        Self {
            events: Events(Arc::new(Mutex::new(None))),
            #[cfg(windows)]
            icon: Arc::new(Mutex::new(None)),
        }
    }

    /// `true` once an icon is actually in the notification area.
    ///
    /// Hiding the window is only safe when something can bring it back, so `main` asks this first.
    #[cfg(windows)]
    pub fn is_installed(&self) -> bool {
        self.icon.lock().map(|icon| icon.is_some()).unwrap_or(false)
    }

    #[cfg(not(windows))]
    pub fn is_installed(&self) -> bool {
        false
    }

    pub fn set_event_sender(&self, sender: Sender) {
        if let Ok(mut events) = self.events.0.lock() {
            *events = Some(sender);
        }
    }

    #[cfg(windows)]
    pub fn setup_tray_icon(&self) {
        use tray_icon::TrayIconBuilder;
        use tray_icon::menu::{Menu, MenuItem};

        // Right click shows this menu; a left click is handled as a plain click below, so the two do not
        // collide.
        let quit = MenuItem::new("Quit Bifurc", true, None);
        let quit_id = quit.id();
        let menu = Menu::new();
        menu.append(&quit);

        let Ok(icon) = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Bifurc")
            .with_icon(tray_icon_from_png())
            .build()
        else {
            return;
        };

        if let Ok(mut held) = self.icon.lock() {
            *held = Some(icon);
        }

        // Clicks and menu picks are published on separate channels, and a blocking receive on one would
        // stall the other, so each gets a reader. Both forward to the same place.
        let clicks = self.events.clone();
        let click_events = tray_icon::tray_event_receiver().clone();
        std::thread::spawn(move || {
            while let Ok(event) = click_events.recv() {
                // A right click is the menu's; it is reported here as well, and ignored.
                if matches!(event.event, tray_icon::ClickEvent::Left | tray_icon::ClickEvent::Double) {
                    clicks.forward(TrayEvent::Show);
                }
            }
        });

        let menus = self.events.clone();
        let menu_events = tray_icon::menu::menu_event_receiver().clone();
        std::thread::spawn(move || {
            while let Ok(event) = menu_events.recv() {
                if event.id == quit_id {
                    menus.forward(TrayEvent::Quit);
                }
            }
        });
    }

    #[cfg(not(windows))]
    pub fn setup_tray_icon(&self) {}
}

/// The notification-area image, from the 32×32 PNG the desktop build already ships.
///
/// Embedded rather than read from disk, because a path that holds while the app runs from the repository
/// stops holding the moment it is packaged — and an icon that fails to load is an invisible icon, which is
/// the bug this replaces. Both failures below are broken-build failures, not runtime conditions.
#[cfg(windows)]
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

/// The platform calls the tray needs, kept apart so the rest of the module stays portable.
#[cfg(windows)]
pub mod platform {
    use gpui_kit::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, SW_HIDE, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow,
    };

    /// The window's Win32 handle.
    ///
    /// Taken from the window itself rather than looked up by title: the shell draws its own title bar and
    /// never sets a native one, so there is no title to find it by.
    pub fn window_handle(window: &Window) -> Option<isize> {
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        }
    }

    /// Takes the window off the screen and out of the taskbar. The app keeps running, and the tray icon is
    /// the way back.
    pub fn hide_window(window: isize) {
        unsafe {
            let _ = ShowWindow(HWND(window), SW_HIDE);
        }
    }

    /// Puts the window back on screen, in front of whatever the user is looking at.
    ///
    /// A window the user minimized to the taskbar has to be restored; one this module hid has not, and
    /// restoring it would only fight the size and position the user left it at.
    pub fn show_window(window: isize) {
        unsafe {
            if IsIconic(HWND(window)).as_bool() {
                let _ = ShowWindow(HWND(window), SW_RESTORE);
            } else {
                let _ = ShowWindow(HWND(window), SW_SHOW);
            }
            let _ = SetForegroundWindow(HWND(window));
        }
    }
}

#[cfg(not(windows))]
pub mod platform {
    use gpui_kit::Window;

    /// No tray icon is placed off Windows, so nothing hides a window and nothing brings one back. The
    /// signatures stay so `main` reads the same on every platform.
    pub fn window_handle(_window: &Window) -> Option<isize> {
        None
    }

    pub fn hide_window(_window: isize) {}

    pub fn show_window(_window: isize) {}
}
