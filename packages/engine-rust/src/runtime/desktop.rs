//! The engine's presence on a desktop: its tray icon, opening the interface, and when to stop.
//!
//! The engine outlives the interfaces that use it, so it needs a handle of its own that the user can reach:
//! the tray icon. It is a small helper (`bifurc-tray`, built from `ui/crates/tray`) that the engine runs as a
//! child. The two talk in lines over the helper's stdio: the helper prints `ready` once its icon is up, then
//! `open`, `restart` or `quit` as the user picks them; the engine closes the helper's stdin to take the icon
//! down. A helper that is missing, or that cannot show an icon on this platform, leaves the engine without
//! one, and an engine nobody can reach must not linger: only then does it stop when its last interface closes.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::ChildStdin,
};

use super::notice;
use crate::{
    kernel::{SessionIdentity, bus},
    utils::process::hidden_command,
};

#[cfg(windows)]
const EXE: &str = ".exe";
#[cfg(not(windows))]
const EXE: &str = "";

fn tray_binary() -> String {
    format!("bifurc-tray{EXE}")
}

fn ui_binary() -> String {
    format!("bifurc-ui{EXE}")
}

/// Clients that are an interface a person looks at, as they name themselves in `hello`.
const INTERFACE_CLIENTS: [&str; 3] = ["desktop-ui", "electron-shell", "web-ui"];

/// How long a launched interface has to connect before another Open launches a second one.
const LAUNCH_GRACE: Duration = Duration::from_secs(10);

/// The helper programs of a desktop install.
#[derive(Debug, Clone, Default)]
pub struct DesktopCompanions {
    /// `bifurc-tray`: the tray icon.
    pub tray: Option<PathBuf>,
    /// `bifurc-ui`: the interface the tray icon opens.
    pub ui: Option<PathBuf>,
}

fn env_path(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// The tray helper found by walking up from each start: beside the directory itself, or under `ui/target`.
fn search_upward(starts: &[PathBuf], binary: &str) -> Option<PathBuf> {
    for start in starts {
        let start = std::path::absolute(start).unwrap_or_else(|_| start.clone());
        for dir in start.ancestors() {
            let candidates = [
                dir.join(binary),
                dir.join("ui").join("target").join("release").join(binary),
                dir.join("ui").join("target").join("debug").join(binary),
            ];
            if let Some(found) = candidates.into_iter().find(|candidate| candidate.exists()) {
                return Some(found);
            }
        }
    }
    None
}

fn resolve_companions(
    tray: Option<&str>,
    ui: Option<&str>,
    env_tray: Option<String>,
    env_ui: Option<String>,
    starts: &[PathBuf],
) -> DesktopCompanions {
    let tray = tray
        .map(PathBuf::from)
        .or_else(|| env_tray.map(PathBuf::from))
        .or_else(|| search_upward(starts, &tray_binary()));
    let mut ui = ui.map(PathBuf::from).or_else(|| env_ui.map(PathBuf::from));
    if ui.is_none() {
        if let Some(sibling) = tray.as_deref().and_then(Path::parent).map(|dir| dir.join(ui_binary())) {
            if sibling.exists() {
                ui = Some(sibling);
            }
        }
    }
    DesktopCompanions { tray, ui }
}

/// Locate the helpers: the explicit paths, then `BIFURC_TRAY` / `BIFURC_UI`, then a search upward from the
/// engine's executable and from the working directory (beside the engine in a packaged app, under `ui/target`
/// in a checkout). The interface defaults to the one built next to the helper.
pub fn find_desktop_companions(tray: Option<&str>, ui: Option<&str>) -> DesktopCompanions {
    let mut starts = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) {
        starts.push(dir);
    }
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    resolve_companions(tray, ui, env_path("BIFURC_TRAY"), env_path("BIFURC_UI"), &starts)
}

#[derive(Default)]
struct State {
    /// Session ids of the connected interfaces.
    interfaces: HashSet<String>,
    /// The running tray helper's number and its stdin; dropping the stdin takes the icon down.
    tray: Option<(u64, ChildStdin)>,
    next_tray: u64,
    tray_ready: bool,
    stopping: bool,
    launched_at: Option<Instant>,
}

/// The engine's presence on the desktop: its tray icon, and its lifetime relative to its interfaces.
pub struct Desktop {
    companions: DesktopCompanions,
    data_dir: PathBuf,
    shutdown: Arc<dyn Fn(bool) + Send + Sync>,
    state: Mutex<State>,
}

impl Desktop {
    /// `shutdown(restart)` asks the engine to stop.
    pub fn new(
        companions: DesktopCompanions,
        data_dir: PathBuf,
        shutdown: Arc<dyn Fn(bool) + Send + Sync>,
    ) -> Arc<Self> {
        let desktop = Arc::new(Self { companions, data_dir, shutdown, state: Mutex::new(State::default()) });
        let weak: Weak<Desktop> = Arc::downgrade(&desktop);
        bus().on("engine.stopping", move |_| {
            if let Some(desktop) = weak.upgrade() {
                desktop.engine_stopping();
            }
        });
        desktop
    }

    /// Shows the tray icon, if there is a helper to show it. Must be called inside the runtime.
    pub fn start(self: &Arc<Self>) {
        let Some(program) = self.companions.tray.clone() else { return };
        if self.state.lock().tray.is_some() {
            return;
        }
        let spawned =
            hidden_command(&program).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(err) => {
                notice(&format!("no tray icon: could not start {}: {err}", program.display()));
                return;
            }
        };
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            notice(&format!("no tray icon: could not open pipes to {}", program.display()));
            let _ = child.start_kill();
            return;
        };
        let id = {
            let mut state = self.state.lock();
            state.next_tray += 1;
            let id = state.next_tray;
            state.tray = Some((id, stdin));
            id
        };

        let reader = self.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                reader.on_tray_line(id, &line);
            }
        });
        let waiter = self.clone();
        tokio::spawn(async move {
            let _ = child.wait().await;
            waiter.tray_exited(id);
        });
    }

    fn tray_exited(&self, id: u64) {
        let stopping = {
            let mut state = self.state.lock();
            if state.tray.as_ref().map(|(current, _)| *current) != Some(id) {
                return;
            }
            state.tray = None;
            state.tray_ready = false;
            state.stopping
        };
        if !stopping {
            notice("the tray icon is gone — the engine will stop when its last interface closes");
        }
    }

    fn on_tray_line(&self, id: u64, line: &str) {
        if self.state.lock().tray.as_ref().map(|(current, _)| *current) != Some(id) {
            return;
        }
        self.handle_line(line);
    }

    fn handle_line(&self, line: &str) {
        match line.trim() {
            "ready" => self.state.lock().tray_ready = true,
            "open" => self.open_interface(),
            "restart" => (self.shutdown)(true),
            "quit" => (self.shutdown)(false),
            _ => {}
        }
    }

    fn open_interface(&self) {
        {
            let state = self.state.lock();
            if !state.interfaces.is_empty() {
                drop(state);
                bus().emit("ui.activate", json!({}));
                return;
            }
        }
        let Some(ui) = &self.companions.ui else {
            notice("the tray asked for the interface, but the engine was not told where it is (BIFURC_UI)");
            return;
        };
        {
            // A double click arrives as two requests, and the interface takes a moment to connect.
            let mut state = self.state.lock();
            if state.launched_at.is_some_and(|at| at.elapsed() < LAUNCH_GRACE) {
                return;
            }
            state.launched_at = Some(Instant::now());
        }
        let mut command = tokio::process::Command::new(ui);
        command.env("BIFURC_DATA_DIR", &self.data_dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        }
        #[cfg(unix)]
        command.process_group(0);
        match command.spawn() {
            Ok(mut child) => {
                // Reaped here so it does not linger as a zombie; it is not tied to the engine's life.
                tokio::spawn(async move {
                    let _ = child.wait().await;
                });
            }
            Err(err) => {
                self.state.lock().launched_at = None;
                notice(&format!("could not start the interface at {}: {err}", ui.display()));
            }
        }
    }

    fn engine_stopping(&self) {
        let mut state = self.state.lock();
        state.stopping = true;
        state.tray_ready = false;
        // The helper takes its icon down and exits when its stdin closes.
        state.tray = None;
    }

    pub fn on_session_start(&self, identity: &SessionIdentity) {
        let Some(name) = identity.client_name.as_deref() else { return };
        if !INTERFACE_CLIENTS.contains(&name) {
            return;
        }
        let mut state = self.state.lock();
        state.interfaces.insert(identity.session_id.clone());
        state.launched_at = None;
    }

    pub fn on_session_end(&self, identity: &SessionIdentity) {
        {
            let mut state = self.state.lock();
            if !state.interfaces.remove(&identity.session_id) {
                return;
            }
            if state.stopping || !state.interfaces.is_empty() || state.tray_ready {
                return;
            }
        }
        notice("the last interface closed and there is no tray icon to reach the engine by — stopping");
        (self.shutdown)(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI32, Ordering};

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"").unwrap();
    }

    #[test]
    fn explicit_paths_win_over_environment_and_search() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join(tray_binary()));
        let found = resolve_companions(
            Some("explicit-tray"),
            Some("explicit-ui"),
            Some("env-tray".into()),
            Some("env-ui".into()),
            &[dir.path().to_path_buf()],
        );
        assert_eq!(found.tray, Some(PathBuf::from("explicit-tray")));
        assert_eq!(found.ui, Some(PathBuf::from("explicit-ui")));

        let found =
            resolve_companions(None, None, Some("env-tray".into()), Some("env-ui".into()), &[dir.path().to_path_buf()]);
        assert_eq!(found.tray, Some(PathBuf::from("env-tray")));
        assert_eq!(found.ui, Some(PathBuf::from("env-ui")));
    }

    #[test]
    fn the_search_walks_up_and_prefers_beside_over_release_over_debug() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let nested = root.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        touch(&root.join("ui").join("target").join("debug").join(tray_binary()));
        let found = resolve_companions(None, None, None, None, &[nested.clone()]);
        assert_eq!(found.tray, Some(root.join("ui").join("target").join("debug").join(tray_binary())));

        touch(&root.join("ui").join("target").join("release").join(tray_binary()));
        let found = resolve_companions(None, None, None, None, &[nested.clone()]);
        assert_eq!(found.tray, Some(root.join("ui").join("target").join("release").join(tray_binary())));

        touch(&root.join(tray_binary()));
        let found = resolve_companions(None, None, None, None, &[nested]);
        assert_eq!(found.tray, Some(root.join(tray_binary())));
    }

    #[test]
    fn the_interface_defaults_to_the_sibling_of_the_tray() {
        let dir = tempfile::tempdir().unwrap();
        let tray = dir.path().join(tray_binary());
        touch(&tray);
        let found = resolve_companions(Some(tray.to_str().unwrap()), None, None, None, &[]);
        assert_eq!(found.ui, None);
        touch(&dir.path().join(ui_binary()));
        let found = resolve_companions(Some(tray.to_str().unwrap()), None, None, None, &[]);
        assert_eq!(found.ui, Some(dir.path().join(ui_binary())));
    }

    fn identity(id: &str, client: &str) -> SessionIdentity {
        SessionIdentity { session_id: id.into(), client_name: Some(client.into()), ..Default::default() }
    }

    /// A desktop whose shutdown records `1` for a quit and `2` for a restart.
    fn desktop() -> (Arc<Desktop>, Arc<AtomicI32>) {
        let calls = Arc::new(AtomicI32::new(0));
        let sink = calls.clone();
        let desktop = Desktop::new(
            DesktopCompanions::default(),
            PathBuf::from("data"),
            Arc::new(move |restart| {
                sink.store(if restart { 2 } else { 1 }, Ordering::SeqCst);
            }),
        );
        (desktop, calls)
    }

    #[test]
    fn the_last_interface_closing_with_no_tray_stops_the_engine() {
        let (desktop, calls) = desktop();
        desktop.on_session_start(&identity("a", "desktop-ui"));
        desktop.on_session_start(&identity("b", "web-ui"));
        desktop.on_session_start(&identity("c", "cli"));
        desktop.on_session_end(&identity("c", "cli"));
        desktop.on_session_end(&identity("a", "desktop-ui"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        desktop.on_session_end(&identity("b", "web-ui"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_ready_tray_keeps_the_engine_and_stopping_silences_it() {
        let (desktop, calls) = desktop();
        desktop.state.lock().tray_ready = true;
        desktop.on_session_start(&identity("a", "electron-shell"));
        desktop.on_session_end(&identity("a", "electron-shell"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        desktop.state.lock().tray_ready = false;
        desktop.on_session_start(&identity("b", "desktop-ui"));
        desktop.engine_stopping();
        desktop.on_session_end(&identity("b", "desktop-ui"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn tray_lines_drive_the_engine() {
        let (desktop, calls) = desktop();
        // Lines from a helper that is not the current one are ignored.
        desktop.on_tray_line(1, "quit");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        desktop.handle_line("restart");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        desktop.handle_line("ready");
        assert!(desktop.state.lock().tray_ready);
        desktop.handle_line("quit\r");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn open_with_an_interface_connected_activates_it() {
        let (desktop, _) = desktop();
        let activated = Arc::new(AtomicI32::new(0));
        let sink = activated.clone();
        let listener = bus().on("ui.activate", move |_| {
            sink.fetch_add(1, Ordering::SeqCst);
        });
        desktop.on_session_start(&identity("a", "desktop-ui"));
        desktop.open_interface();
        bus().off(listener);
        assert_eq!(activated.load(Ordering::SeqCst), 1);
    }
}
