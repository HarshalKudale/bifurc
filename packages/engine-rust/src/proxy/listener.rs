//! The loopback TCP listener both servers are built on: bind, accept, hand each connection to a handler, and
//! report why binding failed.

use std::{
    future::Future,
    io::ErrorKind,
    net::{Ipv4Addr, SocketAddr, TcpListener as StdListener},
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use tokio::{net::TcpListener, task::JoinHandle};

use crate::kernel::traffic::emit_server_error;

/// Serves one accepted connection.
pub type ConnectionHandler =
    Arc<dyn Fn(tokio::net::TcpStream) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// How long a port just released by [`ListenerCore::stop`] is retried for before it is reported as taken.
const REBIND_WINDOW: Duration = Duration::from_secs(2);
const REBIND_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Default)]
struct State {
    /// Bumped by every start and stop, so a bind that was deferred does not outlive the call that asked.
    generation: u64,
    port: u16,
    running: bool,
    error: Option<String>,
    task: Option<JoinHandle<()>>,
    stopped_at: Option<Instant>,
}

/// One loopback listener and what is known about it.
#[derive(Clone, Default)]
pub struct ListenerCore {
    state: Arc<Mutex<State>>,
}

fn bind(port: u16) -> std::io::Result<TcpListener> {
    let listener = StdListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))?;
    listener.set_nonblocking(true)?;
    TcpListener::from_std(listener)
}

fn bind_error_message(err: &std::io::Error, port: u16) -> String {
    match err.kind() {
        ErrorKind::PermissionDenied => format!("Permission denied — run as Administrator to bind port {port}"),
        ErrorKind::AddrInUse => format!("Port {port} is already in use"),
        _ => err.to_string(),
    }
}

async fn accept_loop(listener: TcpListener, handler: ConnectionHandler) {
    loop {
        match listener.accept().await {
            Ok((socket, _)) => {
                let _ = socket.set_nodelay(true);
                tokio::spawn(handler(socket));
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
}

impl ListenerCore {
    pub fn is_running(&self) -> bool {
        self.state.lock().running
    }

    /// The port last asked for; `0` before the first start.
    pub fn port(&self) -> u16 {
        self.state.lock().port
    }

    pub fn error(&self) -> Option<String> {
        self.state.lock().error.clone()
    }

    /// Stop accepting. Connections already being served finish.
    pub fn stop(&self) {
        let mut state = self.state.lock();
        state.generation += 1;
        if let Some(task) = state.task.take() {
            task.abort();
            state.stopped_at = Some(Instant::now());
        }
        state.running = false;
    }

    /// Bind `port` on loopback and serve connections with `handler`, replacing a running listener. A bind
    /// that fails is reported by [`error`](Self::error) and as a `server.error` event.
    pub fn start(&self, port: u16, handler: ConnectionHandler) {
        self.stop();
        let (generation, just_released) = {
            let mut state = self.state.lock();
            state.port = port;
            state.error = None;
            let just_released = state.stopped_at.take().is_some_and(|at| at.elapsed() < REBIND_WINDOW);
            (state.generation, just_released)
        };
        match bind(port) {
            Ok(listener) => {
                let task = tokio::spawn(accept_loop(listener, handler));
                let mut state = self.state.lock();
                state.task = Some(task);
                state.running = true;
            }
            Err(err) if just_released && err.kind() == ErrorKind::AddrInUse => {
                // The listener this call replaces is closed when its aborted task is next polled.
                let core = self.clone();
                let task = tokio::spawn(async move { core.rebind(port, generation, handler).await });
                self.state.lock().task = Some(task);
            }
            Err(err) => self.fail(&err, port),
        }
    }

    fn fail(&self, err: &std::io::Error, port: u16) {
        let message = bind_error_message(err, port);
        self.state.lock().error = Some(message.clone());
        emit_server_error(&message);
        crate::analytics::failed("listener_bind");
    }

    async fn rebind(&self, port: u16, generation: u64, handler: ConnectionHandler) {
        let deadline = Instant::now() + REBIND_WINDOW;
        loop {
            tokio::time::sleep(REBIND_INTERVAL).await;
            if self.state.lock().generation != generation {
                return;
            }
            match bind(port) {
                Ok(listener) => {
                    {
                        let mut state = self.state.lock();
                        if state.generation != generation {
                            return;
                        }
                        state.running = true;
                    }
                    accept_loop(listener, handler).await;
                    return;
                }
                Err(err) if err.kind() == ErrorKind::AddrInUse && Instant::now() < deadline => continue,
                Err(err) => {
                    self.fail(&err, port);
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn greeter() -> ConnectionHandler {
        Arc::new(|mut socket| {
            Box::pin(async move {
                let _ = socket.write_all(b"hi").await;
            })
        })
    }

    async fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port()
    }

    async fn greeting(port: u16) -> String {
        let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut text = String::new();
        socket.read_to_string(&mut text).await.unwrap();
        text
    }

    #[tokio::test]
    async fn serves_connections_and_stops() {
        let port = free_port().await;
        let core = ListenerCore::default();
        assert!(!core.is_running());
        core.start(port, greeter());
        assert!(core.is_running());
        assert_eq!((core.port(), core.error()), (port, None));
        assert_eq!(greeting(port).await, "hi");
        core.stop();
        assert!(!core.is_running());
    }

    #[tokio::test]
    async fn a_taken_port_is_reported() {
        let taken = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = taken.local_addr().unwrap().port();
        let core = ListenerCore::default();
        core.start(port, greeter());
        assert!(!core.is_running());
        assert_eq!(core.error(), Some(format!("Port {port} is already in use")));
    }

    #[tokio::test]
    async fn restarting_on_the_same_port_succeeds() {
        let port = free_port().await;
        let core = ListenerCore::default();
        core.start(port, greeter());
        core.stop();
        core.start(port, greeter());
        for _ in 0..100 {
            if core.is_running() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(core.is_running(), "{:?}", core.error());
        assert_eq!(greeting(port).await, "hi");
        core.start(port, greeter());
        for _ in 0..100 {
            if core.is_running() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(greeting(port).await, "hi");
    }
}
