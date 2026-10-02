use super::session::SessionOptions;

/// Serve one session on this process's stdin and stdout, until stdin closes.
pub async fn run_stdio_server(_opts: SessionOptions) {
    std::future::pending::<()>().await
}
