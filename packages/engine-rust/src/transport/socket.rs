//! The local socket transport: a named pipe on Windows, a 0600 unix socket elsewhere. This is what the
//! desktop UI and the CLI connect to.

use std::sync::Arc;

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{mpsc::unbounded_channel, watch},
    task::JoinHandle,
};

use super::{
    framing::{FrameDecoder, encode_frame},
    session::{Outbound, Session, SessionOptions},
};
use crate::protocol::{EngineError, Result, capability};

/// Serve one session over a byte stream of length-prefixed frames, until either side closes it or `closed`
/// fires.
pub async fn serve_framed_stream<S>(stream: S, opts: SessionOptions, mut closed: watch::Receiver<bool>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    let (tx, mut rx) = unbounded_channel();
    let session = Session::new(opts, tx);
    let mut decoder = FrameDecoder::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        tokio::select! {
            read = reader.read(&mut buffer) => {
                let Ok(n) = read else { break };
                if n == 0 {
                    break;
                }
                match decoder.push(&buffer[..n]) {
                    Ok(frames) => frames.into_iter().for_each(|frame| session.handle_frame(frame)),
                    Err(_) => break,
                }
            }
            outbound = rx.recv() => {
                let (frame, then_close) = match outbound {
                    Some(Outbound::Frame(frame)) => (frame, false),
                    Some(Outbound::FrameThenClose(frame)) => (frame, true),
                    Some(Outbound::Close) | None => break,
                };
                let Ok(bytes) = encode_frame(&frame) else { break };
                if writer.write_all(&bytes).await.is_err() || writer.flush().await.is_err() || then_close {
                    break;
                }
            }
            _ = closed.changed() => break,
        }
    }
    session.close();
    let _ = writer.shutdown().await;
}

pub struct SocketServer {
    pub path: String,
    closed: watch::Sender<bool>,
    accept: JoinHandle<()>,
}

impl SocketServer {
    /// Stop accepting, end every session and remove the socket file.
    pub async fn close(&self) {
        let _ = self.closed.send(true);
        self.accept.abort();
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.path);
    }
}

fn session_options(mut opts: SessionOptions) -> SessionOptions {
    if !opts.capabilities.contains(&capability::SOCKET) {
        opts.capabilities.push(capability::SOCKET);
    }
    opts.peer = Some("local".to_string());
    opts
}

#[cfg(windows)]
pub async fn create_socket_server(path: &str, opts: SessionOptions) -> Result<Arc<SocketServer>> {
    use tokio::net::windows::named_pipe::ServerOptions;

    if path.is_empty() {
        return Err(EngineError::bad_request("createSocketServer needs a non-empty socket path."));
    }
    let opts = session_options(opts);
    let mut listener = ServerOptions::new()
        .first_pipe_instance(true)
        .create(path)
        .map_err(|e| EngineError::engine(format!("Could not bind {path}: {e}")))?;
    let (closed, closed_rx) = watch::channel(false);
    let pipe = path.to_string();
    let accept = tokio::spawn(async move {
        loop {
            if listener.connect().await.is_err() {
                break;
            }
            let connected = listener;
            listener = match ServerOptions::new().create(&pipe) {
                Ok(next) => next,
                Err(e) => {
                    eprintln!("[socket] could not keep listening on {pipe}: {e}");
                    tokio::spawn(serve_framed_stream(connected, opts.clone(), closed_rx.clone()));
                    break;
                }
            };
            tokio::spawn(serve_framed_stream(connected, opts.clone(), closed_rx.clone()));
        }
    });
    Ok(Arc::new(SocketServer { path: path.to_string(), closed, accept }))
}

#[cfg(unix)]
pub async fn create_socket_server(path: &str, opts: SessionOptions) -> Result<Arc<SocketServer>> {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use tokio::net::{UnixListener, UnixStream};

    if path.is_empty() {
        return Err(EngineError::bad_request("createSocketServer needs a non-empty socket path."));
    }
    let opts = session_options(opts);
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if !meta.file_type().is_socket() {
            return Err(EngineError::engine(format!(
                "Refusing to bind {path}: something already exists there and it is not a socket. Remove it \
                 yourself if that is what you intended — the engine will not delete a file it did not create."
            )));
        }
        if UnixStream::connect(path).await.is_ok() {
            return Err(EngineError::engine(format!(
                "Refusing to bind {path}: another engine is already listening there. Attach to it, or stop it first."
            )));
        }
        std::fs::remove_file(path)
            .map_err(|e| EngineError::engine(format!("Could not remove the stale socket at {path}: {e}")))?;
    }
    let listener = UnixListener::bind(path).map_err(|e| EngineError::engine(format!("Could not bind {path}: {e}")))?;
    // The socket transport relies on filesystem permissions for access control.
    let secured = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .and_then(|_| std::fs::metadata(path))
        .map(|meta| meta.permissions().mode() & 0o077 == 0);
    if !matches!(secured, Ok(true)) {
        let _ = std::fs::remove_file(path);
        return Err(EngineError::engine(format!(
            "Could not make {path} private (0600). The socket transport relies on filesystem permissions for \
             access control, so it refuses to serve one it cannot secure."
        )));
    }
    let (closed, closed_rx) = watch::channel(false);
    let accept = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve_framed_stream(stream, opts.clone(), closed_rx.clone()));
        }
    });
    Ok(Arc::new(SocketServer { path: path.to_string(), closed, accept }))
}
