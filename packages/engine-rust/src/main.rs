//! `bifurc-engine`: the engine as a standalone process.

use bifurc_engine::{
    ENGINE_VERSION,
    runtime::{
        args::{HOST_USAGE, HostArgsResult, TransportKind, parse_host_args},
        desktop::find_desktop_companions,
        host::{HostOptions, HostTransport, run_workspace_host},
        notice, route_notices_to_stderr,
    },
};

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_host_args(&argv) {
        HostArgsResult::Help => {
            println!("{HOST_USAGE}");
            return;
        }
        HostArgsResult::Error(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
        HostArgsResult::Args(args) => args,
    };
    bifurc_protocol::crash::install("engine", ENGINE_VERSION, args.data_dir.clone());

    // A DMG install has no installer to put `bifurc` on PATH, so the engine does it.
    #[cfg(target_os = "macos")]
    if bifurc_engine::updates::channel::detect() == bifurc_engine::updates::channel::Channel::Dmg {
        if let Some(dir) = std::env::current_exe().ok().as_deref().and_then(std::path::Path::parent) {
            bifurc_engine::runtime::shell_path::ensure(dir);
        }
    }

    let transport = match args.transport {
        TransportKind::Stdio => HostTransport::Stdio,
        TransportKind::Ws => HostTransport::Ws { port: args.ws_port },
        TransportKind::Socket => HostTransport::Socket { path: args.rpc_socket.clone() },
    };
    if args.transport == TransportKind::Stdio {
        // stdout carries frames.
        route_notices_to_stderr();
    }
    let desktop = if args.no_tray || args.transport == TransportKind::Stdio {
        None
    } else {
        Some(find_desktop_companions(args.tray.as_deref(), args.ui.as_deref()))
    };
    let opts = HostOptions {
        data_dir: args.data_dir,
        workspace_ids: args.workspace_ids,
        transport,
        edge_port: args.edge_port,
        engine_version: ENGINE_VERSION.to_string(),
        desktop,
    };

    notice(&format!("starting: engine {ENGINE_VERSION}, pid {}", std::process::id()));

    // Outbound TLS and the interception proxy share one crypto provider.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("the async runtime starts");
    let code = runtime.block_on(async {
        match run_workspace_host(opts).await {
            Ok(code) => code,
            Err(err) => {
                eprintln!("[host] failed to start: {err}");
                1
            }
        }
    });
    // Background tasks (open sockets, timers) must not keep a stopped engine alive.
    runtime.shutdown_background();
    std::process::exit(code);
}
