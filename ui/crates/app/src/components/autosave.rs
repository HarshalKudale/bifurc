//! Writes an edit of a saved entity back a moment after the last change to it.
//!
//! A saved entity has no Save step: what is typed is written to the engine once the typing pauses. Each
//! editor keeps one `Autosave` and schedules it whenever one of its controls changes; scheduling again
//! replaces the wait that was running, so a burst of keystrokes is one write.

use std::time::Duration;

use gpui_kit::{AnyWindowHandle, Context, Task, Window};

/// How long the editor waits after the last change before writing.
const DELAY: Duration = Duration::from_millis(600);

pub struct Autosave {
    /// The window the editor lives in. A change event carries no window, and the write needs one.
    window: AnyWindowHandle,
    task: Option<Task<()>>,
}

impl Autosave {
    pub fn new(window: &Window) -> Self {
        Self { window: window.window_handle(), task: None }
    }

    /// Runs `write` on the view after the delay, unless this is scheduled again or cancelled first.
    pub fn schedule<V: 'static>(
        &mut self,
        cx: &mut Context<V>,
        write: impl FnOnce(&mut V, &mut Window, &mut Context<V>) + 'static,
    ) {
        let window = self.window;
        self.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DELAY).await;
            window
                .update(cx, |_, window, cx| {
                    this.update(cx, |this, cx| write(this, window, cx)).ok();
                })
                .ok();
        }));
    }

    /// Drops a write that is waiting: the edit it would have written is being discarded.
    pub fn cancel(&mut self) {
        self.task = None;
    }
}
