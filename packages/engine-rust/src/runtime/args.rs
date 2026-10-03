//! `bifurc-engine`'s command line.

use std::path::PathBuf;

use crate::workspace::paths::platform_default_data_dir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    Socket,
    Ws,
    Stdio,
}

impl TransportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TransportKind::Socket => "socket",
            TransportKind::Ws => "ws",
            TransportKind::Stdio => "stdio",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostArgs {
    pub data_dir: PathBuf,
    pub workspace_ids: Option<Vec<String>>,
    pub rpc_socket: Option<String>,
    pub ws_port: Option<u16>,
    pub transport: TransportKind,
    pub edge_port: Option<u16>,
    pub tray: Option<String>,
    pub ui: Option<String>,
    pub no_tray: bool,
}

#[derive(Debug, PartialEq)]
pub enum HostArgsResult {
    Args(HostArgs),
    Help,
    Error(String),
}

pub const HOST_USAGE: &str = "Usage: bifurc-engine [--data-dir <path>] [--transport socket|ws|stdio] [options]

The Bifurc engine, standalone. One engine serves one data directory; it records where it listens in
<data-dir>/runtime.json so the desktop app, the CLI and other clients can find it.

  --data-dir <path>       The data root. Defaults to the desktop app's data directory.
  --transport <kind>      socket (default): a named pipe on Windows, a unix socket elsewhere, at a path
                          derived from the data dir. ws: WebSocket on loopback. stdio: frames on
                          stdin/stdout, for a parent process that spawned the engine.

Every listener binds loopback and there is no way to change that: every client is on this machine. The
engine answers as \"localhost\" — a subdomain of it (<sub>.localhost) is a reverse-proxy mapping, and the
base domain itself serves the mapping directory.

Options
  --rpc-socket <path>     The RPC endpoint's path. Defaults to the platform's standard location
                          (a named pipe on Windows, a 0600 unix socket elsewhere).
  --ws-port <port>        Serve the RPC endpoint over WebSocket on this port, on loopback, instead of
                          over a socket. Use 0 for an ephemeral port. This flag is what selects
                          WebSocket mode.
  --workspace <id>        Serve only this workspace. Repeatable, and the order is preserved.
                          Omitted means every workspace in app.json.
  --edge-port <port>      The edge router's port — the reverse proxy and the home page. Defaults to the
                          port in app.json (80), which is the port existing *.localhost mappings
                          already point at.
  --tray <path>           The tray helper (bifurc-tray). Defaults to BIFURC_TRAY, then the one found
                          beside the engine. While its icon is up the engine keeps running after its
                          interfaces close; the icon opens the interface, restarts and quits.
  --ui <path>             The interface the tray icon opens. Defaults to BIFURC_UI, then the one
                          beside the tray helper.
  --no-tray               No tray icon, and the engine does not stop when its interfaces close.
  -h, --help              Print this.

Environment
  BIFURC_PROXY_BASE_PORT   Sets the forward proxy's port.
  BIFURC_DATA_DIR          The data root, if not passed via --data-dir.";

const VALUE_FLAGS: [&str; 8] =
    ["--data-dir", "--rpc-socket", "--ws-port", "--workspace", "--edge-port", "--transport", "--tray", "--ui"];

const REMOVED_FLAGS: [(&str, &str); 9] = [
    ("--ws-host", "every listener binds loopback, so the bind host is not a choice"),
    ("--ws-allow-non-loopback", "the engine is not deployable remotely, so there is nothing to publish"),
    ("--ws-tls-cert", "TLS was only required to satisfy the non-loopback bind, which no longer exists"),
    ("--ws-tls-key", "TLS was only required to satisfy the non-loopback bind, which no longer exists"),
    ("--token-file", "the engine token is gone — the RPC surface binds loopback, so there is no boundary to guard"),
    ("--proxy-bind-host", "the forward proxy binds loopback, always"),
    ("--proxy-allow-non-loopback", "the engine is not deployable remotely, so there is nothing to publish"),
    ("--edge-bind-host", "the edge binds loopback, always"),
    ("--edge-allow-non-loopback", "the engine is not deployable remotely, so there is nothing to publish"),
];

fn error(message: impl AsRef<str>) -> HostArgsResult {
    HostArgsResult::Error(format!("{}\n\n{HOST_USAGE}", message.as_ref()))
}

/// Parse the arguments after the program name. `BIFURC_DATA_DIR` supplies the data dir when the flag is
/// absent.
pub fn parse_host_args(argv: &[String]) -> HostArgsResult {
    let mut data_dir = std::env::var("BIFURC_DATA_DIR").ok();
    let (mut rpc_socket, mut ws_port, mut edge_port, mut transport) = (None, None, None, None);
    let (mut tray, mut ui, mut no_tray) = (None, None, false);
    let mut workspace_ids = Vec::new();

    let mut i = 0;
    while i < argv.len() {
        let raw = &argv[i];
        i += 1;
        if raw == "-h" || raw == "--help" {
            return HostArgsResult::Help;
        }
        if raw == "--no-tray" {
            no_tray = true;
            continue;
        }
        let (flag, inline_value) = match raw.split_once('=') {
            Some((flag, value)) => (flag, Some(value.to_string())),
            None => (raw.as_str(), None),
        };
        if !flag.starts_with("--") {
            return error(format!("unexpected argument {raw:?}."));
        }
        if let Some((_, reason)) = REMOVED_FLAGS.iter().find(|(name, _)| *name == flag) {
            return error(format!("{flag} was removed on 2026-09-25: {reason}."));
        }
        if !VALUE_FLAGS.contains(&flag) {
            return error(format!("unrecognised flag {flag:?}."));
        }
        let value = match inline_value {
            Some(value) => value,
            None => match argv.get(i).filter(|next| !next.starts_with("--")) {
                Some(next) => {
                    i += 1;
                    next.clone()
                }
                None => return error(format!("{flag} needs a value.")),
            },
        };
        if value.is_empty() {
            return error(format!("{flag} needs a value."));
        }
        match flag {
            "--data-dir" => data_dir = Some(value),
            "--rpc-socket" => rpc_socket = Some(value),
            "--ws-port" => match value.parse::<u16>() {
                Ok(port) => ws_port = Some(port),
                Err(_) => return error("--ws-port must be an integer from 0 to 65535."),
            },
            "--workspace" => workspace_ids.push(value),
            "--transport" => {
                transport = Some(match value.as_str() {
                    "socket" => TransportKind::Socket,
                    "ws" => TransportKind::Ws,
                    "stdio" => TransportKind::Stdio,
                    _ => return error("--transport must be socket, ws or stdio."),
                })
            }
            "--tray" => tray = Some(value),
            "--ui" => ui = Some(value),
            "--edge-port" => match value.parse::<u16>() {
                Ok(port) => edge_port = Some(port),
                Err(_) => return error("--edge-port must be an integer from 0 to 65535."),
            },
            _ => unreachable!("every value flag is handled"),
        }
    }

    if transport == Some(TransportKind::Stdio) && (rpc_socket.is_some() || ws_port.is_some()) {
        return error("--transport stdio takes no --rpc-socket or --ws-port.");
    }
    if no_tray && tray.is_some() {
        return error("--no-tray cannot be combined with --tray.");
    }
    if transport == Some(TransportKind::Socket) && ws_port.is_some() {
        return error("--transport socket cannot be combined with --ws-port.");
    }
    if transport == Some(TransportKind::Ws) && rpc_socket.is_some() {
        return error("--transport ws cannot be combined with --rpc-socket.");
    }
    if rpc_socket.is_some() && ws_port.is_some() {
        return error(
            "--rpc-socket cannot be combined with --ws-port — a host serves one RPC endpoint, and these select \
             different kinds of endpoint.",
        );
    }

    HostArgsResult::Args(HostArgs {
        data_dir: data_dir.filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(platform_default_data_dir),
        transport: transport.unwrap_or(if ws_port.is_some() { TransportKind::Ws } else { TransportKind::Socket }),
        rpc_socket,
        ws_port,
        workspace_ids: if workspace_ids.is_empty() { None } else { Some(workspace_ids) },
        edge_port,
        tray,
        ui,
        no_tray,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> HostArgsResult {
        parse_host_args(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    fn args(result: HostArgsResult) -> HostArgs {
        match result {
            HostArgsResult::Args(args) => args,
            other => panic!("expected args, got {other:?}"),
        }
    }

    #[test]
    fn flags_take_values_in_either_form() {
        let parsed = args(parse(&["--data-dir", "d", "--edge-port=8081", "--workspace", "a", "--workspace", "b"]));
        assert_eq!(parsed.data_dir, PathBuf::from("d"));
        assert_eq!(parsed.edge_port, Some(8081));
        assert_eq!(parsed.workspace_ids, Some(vec!["a".to_string(), "b".to_string()]));
        assert_eq!(parsed.transport, TransportKind::Socket);
    }

    #[test]
    fn ws_port_selects_websocket() {
        assert_eq!(args(parse(&["--data-dir", "d", "--ws-port", "0"])).transport, TransportKind::Ws);
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(matches!(parse(&["--nope"]), HostArgsResult::Error(_)));
        assert!(matches!(parse(&["--data-dir"]), HostArgsResult::Error(_)));
        assert!(matches!(parse(&["--ws-port", "70000"]), HostArgsResult::Error(_)));
        assert!(matches!(parse(&["--token-file", "x"]), HostArgsResult::Error(_)));
        assert!(matches!(parse(&["--transport", "stdio", "--ws-port", "1"]), HostArgsResult::Error(_)));
        assert!(matches!(parse(&["--no-tray", "--tray", "x"]), HostArgsResult::Error(_)));
        assert_eq!(parse(&["--help"]), HostArgsResult::Help);
    }
}
