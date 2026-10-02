use std::{path::PathBuf, sync::Arc};

use crate::kernel::SessionIdentity;

/// The helper programs of a desktop install.
#[derive(Debug, Clone, Default)]
pub struct DesktopCompanions {
    /// `bifurc-tray`: the tray icon.
    pub tray: Option<PathBuf>,
    /// `bifurc-ui`: the interface the tray icon opens.
    pub ui: Option<PathBuf>,
}

/// Locate the helpers: the explicit paths, then `BIFURC_TRAY` / `BIFURC_UI`, then beside the engine.
pub fn find_desktop_companions(_tray: Option<&str>, _ui: Option<&str>) -> DesktopCompanions {
    DesktopCompanions::default()
}

/// The engine's presence on the desktop: its tray icon, and its lifetime relative to its interfaces.
pub struct Desktop;

impl Desktop {
    /// `shutdown(restart)` asks the engine to stop.
    pub fn new(
        _companions: DesktopCompanions,
        _data_dir: PathBuf,
        _shutdown: Arc<dyn Fn(bool) + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self)
    }

    pub fn start(&self) {}

    pub fn on_session_start(&self, _identity: &SessionIdentity) {}

    pub fn on_session_end(&self, _identity: &SessionIdentity) {}
}
