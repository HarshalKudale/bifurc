//! Finding free loopback ports.

use tokio::net::TcpListener;

/// How far above a base port to look.
pub const SCAN_LIMIT: u32 = 1000;

/// Whether `port` can be bound on loopback right now.
pub async fn try_bind(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).await.is_ok()
}

/// The first free port at or above `base` that is not in `reserved`.
pub async fn find_free_port_from(base: u16, reserved: &[u16]) -> Option<u16> {
    for offset in 0..SCAN_LIMIT {
        let candidate = u16::try_from(u32::from(base) + offset).ok()?;
        if reserved.contains(&candidate) {
            continue;
        }
        if try_bind(candidate).await {
            return Some(candidate);
        }
    }
    None
}

pub async fn next_free_port_above(from: u32) -> Option<u16> {
    for candidate in from..from.saturating_add(SCAN_LIMIT) {
        let port = u16::try_from(candidate).ok()?;
        if try_bind(port).await {
            return Some(port);
        }
    }
    None
}
