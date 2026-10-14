//! The engine's composition: one data directory, every workspace in it, the forward proxy, the edge, and one
//! RPC endpoint the clients share.

use std::{path::PathBuf, sync::Arc, time::Duration};

use parking_lot::RwLock;
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use super::{
    args::TransportKind,
    desktop::{Desktop, DesktopCompanions},
    discovery::{EngineRuntimeInfo, engine_socket_path, find_running_engine, remove_runtime_info, write_runtime_info},
    instances::{Listener, bind_edge, bind_forward_proxy},
    notice,
    ports::{SCAN_LIMIT, find_free_port_from, next_free_port_above},
};
use crate::{
    kernel::{CommandRegistry, bus},
    protocol::{EngineError, PROTOCOL_VERSION, Result},
    register_all::register_all_engine_commands,
    transport::{
        companions::CompanionRegistry,
        event_log::EventLog,
        scopes::companion_scopes,
        session::SessionOptions,
        socket::{SocketServer, create_socket_server},
        stdio::run_stdio_server,
        ws::{WsServer, create_ws_server},
    },
    utils::{
        id::now_ms,
        json::str_of,
        net::DEFAULT_HOST_DOMAIN,
    },
    workspace::{
        fs::{init_workspace_dir, ws_dir},
        git_store::{check_git_installed, init_workspace_repo},
        paths::{data_dir, set_data_root},
        settings::{AppSettings, load_settings, save_settings},
    },
};

/// How long `engine.stopping` gets to reach the clients before the endpoint closes.
const SHUTDOWN_FLUSH: Duration = Duration::from_millis(100);

pub const EDGE_BIND_HOST: &str = "127.0.0.1";

#[derive(Debug, Clone)]
pub enum HostTransport {
    Socket { path: Option<String> },
    Ws { port: Option<u16> },
    Stdio,
}

impl HostTransport {
    fn kind(&self) -> TransportKind {
        match self {
            HostTransport::Socket { .. } => TransportKind::Socket,
            HostTransport::Ws { .. } => TransportKind::Ws,
            HostTransport::Stdio => TransportKind::Stdio,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HostOptions {
    pub data_dir: PathBuf,
    /// Serve only these workspaces; `None` serves every workspace in `app.json`.
    pub workspace_ids: Option<Vec<String>>,
    pub transport: HostTransport,
    pub edge_port: Option<u16>,
    pub engine_version: String,
    /// The tray helper and the interface it opens; `None` runs without a tray icon.
    pub desktop: Option<DesktopCompanions>,
}

#[derive(Debug, Clone)]
pub struct ServedWorkspace {
    pub workspace_id: String,
    pub name: String,
    pub active: bool,
}

enum Rpc {
    Socket(Arc<SocketServer>),
    Ws(Arc<WsServer>),
    Stdio,
}

/// A running engine.
pub struct WorkspaceHost {
    opts: HostOptions,
    log: Arc<EventLog>,
    forward_proxy: Arc<dyn Listener>,
    edge: Arc<dyn Listener>,
    edge_url: String,
    rpc: Rpc,
    rpc_address: String,
    /// The WebSocket endpoint the browser extension connects to, when it is not the RPC endpoint itself.
    companion: Option<Arc<WsServer>>,
    companion_port: Option<u16>,
    served: Arc<RwLock<Vec<ServedWorkspace>>>,
    sweeper: tokio::task::JoinHandle<()>,
}

/// The name the command line gives in its handshake.
const CLI_CLIENT: &str = "cli";

/// What the command line may ask before the first-run setup is finished: whether it is, and for the engine
/// to stop (an installer or an uninstaller stops the engine whatever state the setup is in).
const BEFORE_SETUP: [&str; 2] = ["setup.getState", "engine.shutdown"];

/// `details.reason` of the refusal, which is how a client tells it from any other `FORBIDDEN`.
pub const SETUP_REQUIRED: &str = "setupRequired";

/// Why `action` from `client` is refused, if it is: the first-run setup is not finished and the client is the
/// command line. `onboarded` is only asked when the answer depends on it.
fn setup_refusal(client: Option<&str>, action: &str, onboarded: impl FnOnce() -> bool) -> Option<EngineError> {
    if client != Some(CLI_CLIENT) || BEFORE_SETUP.contains(&action) || onboarded() {
        return None;
    }
    Some(EngineError::with_details(
        crate::protocol::ErrorCode::Forbidden,
        "Bifurc has not been set up yet. Finish the first-run setup in the Bifurc app, then run this command again.",
        json!({ "reason": SETUP_REQUIRED }),
    ))
}

fn workspace_dir_exists(ws_id: &str) -> bool {
    !ws_id.is_empty() && ws_dir(ws_id).is_dir()
}

/// Make every workspace's directory a usable repository, resume auto-sync, and make sure the active
/// workspace exists (creating "Workspace 1" in an empty data directory).
async fn bootstrap_workspaces(mut settings: AppSettings) -> Result<AppSettings> {
    notice("starting: looking for git");
    let git_installed = check_git_installed().await;
    notice("starting: git looked for");
    if !git_installed {
        notice("git is not installed: workspaces are plain files, with no history, audit log or sync");
    }
    for ws in settings.workspaces().to_vec() {
        let id = str_of(&ws, "id");
        init_workspace_dir(id, str_of(&ws, "name"))?;
        if !git_installed {
            continue;
        }
        if let Err(err) = init_workspace_repo(id).await {
            notice(&format!("workspace \"{id}\": could not prepare its git repository: {err}"));
            crate::analytics::failed("workspace_repo_init");
        }
        crate::gitops::start_auto_sync(id);
    }
    if !workspace_dir_exists(&settings.active_workspace_id()) {
        let valid =
            settings.workspaces().iter().map(|w| str_of(w, "id").to_string()).find(|id| workspace_dir_exists(id));
        // With no workspace at all there is no active one: the first is created or imported by a client
        // (`workspace.add`, `workspace.import`), and becomes active then.
        let active = valid.unwrap_or_default();
        if settings.active_workspace_id() != active {
            settings.set("activeWorkspaceId", json!(active));
            save_settings(&settings)?;
        }
    }
    Ok(settings)
}

impl WorkspaceHost {
    /// Start everything. `shutdown` is how the engine is asked to stop from inside (the `engine.shutdown`
    /// command, the tray icon, a closed stdio stream): it carries whether to restart.
    async fn start(opts: HostOptions, shutdown: UnboundedSender<bool>) -> Result<Self> {
        if let Some(running) = find_running_engine(&opts.data_dir).filter(|info| info.pid != std::process::id()) {
            return Err(EngineError::engine(format!(
                "an engine (pid {}) is already serving {} over {} at {} — connect to it, or stop it first",
                running.pid,
                opts.data_dir.display(),
                running.transport,
                running.address
            )));
        }

        notice("starting: no other engine is running here");
        let mut registry = CommandRegistry::new();
        register_all_engine_commands(&mut registry);
        let stop = shutdown.clone();
        let companions = CompanionRegistry::new();
        let listed = companions.clone();
        registry.register_sync("engine.companions", move |_, _| Ok(listed.snapshot()));
        registry.register_sync("engine.shutdown", move |params, _| {
            let restart = params.get("restart").and_then(Value::as_bool).unwrap_or(false);
            stop.send(restart).map_err(|_| EngineError::engine("no engine is running in this process"))?;
            Ok(json!({ "ok": true }))
        });
        let registry = Arc::new(registry);

        set_data_root(&opts.data_dir)?;
        // Before anything that can fail, so that what fails while starting is reported too.
        notice("starting: data directory set");
        crate::analytics::start(opts.transport.kind().as_str());
        notice("starting: analytics started");
        let log = EventLog::new(bus());
        crate::capture::start_capture_recording();
        let sweeper = crate::workspace::blob::start_blob_sweeper();

        notice("starting: capture and blob sweeper started");
        let mut settings = load_settings();
        notice("starting: settings loaded");
        if let Some(wanted) = &opts.workspace_ids {
            for id in wanted {
                if settings.workspace(id).is_none() {
                    notice(&format!(
                        "requested workspace \"{id}\" is not in the settings (looked in {}) — it will not be served",
                        data_dir().display()
                    ));
                }
            }
            settings.workspaces_mut().retain(|w| wanted.iter().any(|id| id == str_of(w, "id")));
        }
        let settings = bootstrap_workspaces(settings).await?;
        notice("starting: workspaces prepared");

        let forward_proxy = crate::proxy::create_proxy_server();
        let forward_port = settings.forward_proxy_port();
        // A data directory whose first run has not reached its servers step starts neither proxy: the ports
        // are still the defaults, and 80 is one a user may not be allowed to bind. `setup.startServers` starts
        // them, and from then on every run does.
        let start_listeners = settings.servers_confirmed();
        if start_listeners {
            forward_proxy.start(forward_port);
        }
        bind_forward_proxy(Some(forward_proxy.clone()));

        let served: Arc<RwLock<Vec<ServedWorkspace>>> = Arc::new(RwLock::new(Vec::new()));
        let edge_port = opts.edge_port.unwrap_or_else(|| settings.port());
        let (edge_wanted, edge_proxy) = (opts.workspace_ids.clone(), forward_proxy.clone());
        let edge = crate::reverseproxy::create_edge_router(Arc::new(move || {
            // The edge serves the active workspace, and only if this host serves it. A host that was not
            // limited to some workspaces serves every one in the settings, including ones added since it started.
            let settings = load_settings();
            let active = settings.active_workspace_id();
            let served = match &edge_wanted {
                Some(wanted) => wanted.iter().any(|id| *id == active),
                None => settings.workspace(&active).is_some(),
            };
            if !served {
                return None;
            }
            edge_proxy.dispatch_config()
        }));
        let edge_url = format!("http://{EDGE_BIND_HOST}:{edge_port}");
        bind_edge(Some(edge.clone()));
        if start_listeners {
            edge.start(edge_port);
        }

        let active = settings.active_workspace_id();
        *served.write() = settings
            .workspaces()
            .iter()
            .map(|ws| ServedWorkspace {
                workspace_id: str_of(ws, "id").to_string(),
                name: str_of(ws, "name").to_string(),
                active: str_of(ws, "id") == active,
            })
            .collect();

        if !start_listeners {
            notice(
                "the reverse and forward proxies are not started: the first-run setup has not confirmed their \
                 ports yet (`setup.startServers` does, and starts them)",
            );
        }
        match edge.error() {
            Some(err) => {
                notice(&format!("edge on {edge_url} FAILED to bind: {err} — no *.localhost mapping will resolve"))
            }
            None if !start_listeners => {}
            None => notice(&format!("edge: {edge_url} — reverse proxy and home page for the active workspace")),
        }
        match forward_proxy.error() {
            None if !start_listeners => {}
            Some(err) => {
                let advice = match next_free_port_above(u32::from(forward_port) + 1).await {
                    Some(free) => format!("Port {free} is free: set it in Settings → Ports, or free {forward_port}."),
                    None => {
                        format!("No free port was found above it — free {forward_port}, or choose another in Settings.")
                    }
                };
                notice(&format!(
                    "forward proxy FAILED to bind {forward_port}: {err} — your browser's proxy setting points at \
                     this port, so it has to be one that works. {advice}"
                ));
            }
            None => notice(&format!("forward proxy: {forward_port} — point your browser's proxy setting at this port")),
        }

        let desktop = match (&opts.desktop, opts.transport.kind()) {
            (Some(companions), kind) if kind != TransportKind::Stdio => {
                let stop = shutdown.clone();
                Some(Desktop::new(
                    companions.clone(),
                    opts.data_dir.clone(),
                    Arc::new(move |restart| {
                        let _ = stop.send(restart);
                    }),
                ))
            }
            _ => None,
        };

        let (started, ended, setup) = (desktop.clone(), desktop.clone(), desktop.clone());
        let (companions_in, companions_out) = (companions.clone(), companions.clone());
        let session = SessionOptions {
            registry: registry.clone(),
            log: log.clone(),
            engine_version: opts.engine_version.clone(),
            host_domain: Some(DEFAULT_HOST_DOMAIN.to_string()),
            capabilities: Vec::new(),
            scopes_for: None,
            on_auth_failure: Some(Arc::new(|error, client| {
                notice(&format!(
                    "rpc session refused ({}) from \"{}\": {}",
                    error.code.as_str(),
                    client.unwrap_or("unnamed"),
                    error.message
                ));
            })),
            on_session_start: Some(Arc::new(move |identity| {
                if companions_in.connected(identity) {
                    bus().emit("companions.changed", companions_in.snapshot());
                }
                if let Some(desktop) = &started {
                    desktop.on_session_start(identity);
                }
            })),
            on_session_end: Some(Arc::new(move |identity| {
                if companions_out.disconnected(identity) {
                    bus().emit("companions.changed", companions_out.snapshot());
                }
                if let Some(desktop) = &ended {
                    desktop.on_session_end(identity);
                }
            })),
            // The command line is for a Bifurc that has been set up, and the setup is the interface's to walk
            // the user through: until it is done, a command from the CLI is refused and the interface opened.
            refuse_command: Some(Arc::new(move |identity, action| {
                let refusal = setup_refusal(identity.client_name.as_deref(), action, || load_settings().onboarded())?;
                if let Some(desktop) = &setup {
                    desktop.open_interface();
                }
                Some(refusal)
            })),
            peer: None,
        };

        let mut companion_port = None;
        let mut companion = None;
        let (rpc, rpc_address) = match &opts.transport {
            HostTransport::Socket { path } => {
                // A browser extension can only speak WebSocket, so the companion endpoint is served beside the
                // socket. The engine is whole without it, so one that cannot be served is only reported.
                let companion_session =
                    SessionOptions { scopes_for: Some(Arc::new(|_: &Value| companion_scopes())), ..session.clone() };
                let base = settings.companion_port();
                match find_free_port_from(base, &[edge_port, forward_port]).await {
                    Some(port) => match create_ws_server(port, companion_session).await {
                        Ok(server) => {
                            companion_port = Some(port);
                            companion = Some(server);
                            notice(&format!("companion: {port} — the browser extension connects here"));
                        }
                        Err(err) => {
                            notice(&format!("companion on {port} FAILED to start: {}", err.message));
                            crate::analytics::failed("companion_start");
                        }
                    },
                    None => {
                        notice(&format!(
                            "companion FAILED to start: no free port within {SCAN_LIMIT} of {base} — set a different                          base in Settings → Ports"
                        ));
                        crate::analytics::failed("companion_no_port");
                    }
                }
                let path = path.clone().unwrap_or_else(|| engine_socket_path(&opts.data_dir));
                let server = create_socket_server(&path, session).await?;
                (Rpc::Socket(server), path)
            }
            HostTransport::Stdio => {
                let stop = shutdown.clone();
                tokio::spawn(async move {
                    run_stdio_server(session).await;
                    // The parent closed the stream: there is no one left to serve.
                    let _ = stop.send(false);
                });
                (Rpc::Stdio, "stdio".to_string())
            }
            HostTransport::Ws { port } => {
                let port = match port.filter(|p| *p != 0) {
                    Some(port) => port,
                    None => find_free_port_from(settings.companion_port(), &[edge_port, forward_port]).await.ok_or_else(
                        || {
                            EngineError::engine(format!(
                                "No free companion port within {SCAN_LIMIT} of {} — set a different base in Settings → Ports.",
                                settings.companion_port()
                            ))
                        },
                    )?,
                };
                companion_port = Some(port);
                notice(&format!("companion: {port}"));
                let server = create_ws_server(port, session).await?;
                let url = server.url.clone();
                (Rpc::Ws(server), url)
            }
        };
        notice(&format!("rpc endpoint: {rpc_address}"));

        write_runtime_info(&EngineRuntimeInfo {
            pid: std::process::id(),
            transport: opts.transport.kind().as_str().to_string(),
            address: rpc_address.clone(),
            data_dir: opts.data_dir.to_string_lossy().into_owned(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            engine_version: opts.engine_version.clone(),
            started_at: now_ms(),
        })?;

        if let Some(desktop) = &desktop {
            desktop.start();
        }
        crate::updates::start();
        crate::gitops::start_history_watch();

        Ok(Self {
            opts,
            log,
            forward_proxy,
            edge,
            edge_url,
            rpc,
            rpc_address,
            companion,
            companion_port,
            served,
            sweeper,
        })
    }

    /// Close the endpoint and the listeners, and flush what is pending to disk.
    async fn stop(&self) {
        remove_runtime_info(&self.opts.data_dir);
        match &self.rpc {
            Rpc::Socket(server) => server.close().await,
            Rpc::Ws(server) => server.close().await,
            Rpc::Stdio => {}
        }
        if let Some(companion) = &self.companion {
            companion.close().await;
        }
        self.edge.stop();
        self.forward_proxy.stop();
        crate::applications::stop_all();
        crate::gitops::stop_all_auto_sync();
        crate::gitops::stop_history_watch();
        crate::capture::flush_capture();
        crate::analytics::stop().await;
        crate::updates::stop();
        self.sweeper.abort();
        bind_forward_proxy(None);
        bind_edge(None);
        self.served.write().clear();
        self.log.close();
    }

    fn print_status(&self) {
        notice(&format!(
            "domain {DEFAULT_HOST_DOMAIN} — the name this engine answers as (mappings: <sub>.{DEFAULT_HOST_DOMAIN})"
        ));
        if let Some(port) = self.companion_port {
            notice(&format!("companion {port}"));
        }
        let served = self.served.read();
        let ids: Vec<&str> = served.iter().map(|s| s.workspace_id.as_str()).collect();
        notice(&format!(
            "serving {} workspace(s): {}; rpc {}; edge {}",
            served.len(),
            if ids.is_empty() { "(none)".to_string() } else { ids.join(", ") },
            self.rpc_address,
            self.edge_url
        ));
    }
}

/// Start a successor with this process's arguments, detached, so a restart survives this process exiting.
fn start_successor() {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut command = std::process::Command::new(exe);
    command.args(std::env::args_os().skip(1)).stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    if let Err(err) = command.spawn() {
        notice(&format!("could not start the engine again: {err}"));
    }
}

#[cfg(unix)]
async fn terminated() {
    use tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut stream) => {
            stream.recv().await;
        }
        Err(_) => std::future::pending().await,
    }
}

#[cfg(not(unix))]
async fn terminated() {
    std::future::pending::<()>().await
}

/// Run the engine until it is asked to stop. Returns the process exit code.
pub async fn run_workspace_host(opts: HostOptions) -> Result<i32> {
    let (shutdown, mut shutdown_requested) = unbounded_channel::<bool>();
    let host = WorkspaceHost::start(opts, shutdown).await?;
    host.print_status();

    tokio::select! {
        request = shutdown_requested.recv() => {
            let restart = request.unwrap_or(false);
            bus().emit("engine.stopping", json!({ "restart": restart }));
            tokio::time::sleep(SHUTDOWN_FLUSH).await;
            host.stop().await;
            if restart {
                start_successor();
            }
            Ok(0)
        }
        _ = tokio::signal::ctrl_c() => {
            host.stop().await;
            Ok(130)
        }
        _ = terminated() => {
            host.stop().await;
            Ok(143)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_is_refused_until_the_first_run_setup_is_finished() {
        let refused = setup_refusal(Some("cli"), "config.get", || false).expect("refused");
        assert_eq!(refused.code.as_str(), "FORBIDDEN");
        assert_eq!(refused.details, Some(json!({ "reason": SETUP_REQUIRED })));
        assert!(setup_refusal(Some("cli"), "request.send", || false).is_some());
        assert!(setup_refusal(Some("cli"), "config.get", || true).is_none());
    }

    #[test]
    fn the_command_line_may_always_ask_about_the_setup_and_stop_the_engine() {
        for action in BEFORE_SETUP {
            assert!(setup_refusal(Some("cli"), action, || false).is_none(), "{action}");
        }
    }

    #[test]
    fn an_interface_is_never_refused_and_the_setup_is_not_even_read_for_it() {
        for client in [Some("desktop-ui"), Some("bifurc-extension"), None] {
            assert!(setup_refusal(client, "config.get", || panic!("not asked")).is_none(), "{client:?}");
        }
    }
}
