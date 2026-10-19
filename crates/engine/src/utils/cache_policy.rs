//! Whether the proxies strip the request headers that let an upstream answer 304. On by default, so a
//! captured response always has a body.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::kernel::traffic::Headers;

static DISABLED: AtomicBool = AtomicBool::new(true);

pub fn set_cache_disabled(value: bool) {
    DISABLED.store(value, Ordering::Relaxed);
}

pub fn is_cache_disabled() -> bool {
    DISABLED.load(Ordering::Relaxed)
}

const CACHE_REQUEST_HEADERS: [&str; 4] = ["if-none-match", "if-modified-since", "cache-control", "pragma"];

/// Remove the conditional-request headers from lowercased `headers`, when caching is disabled.
pub fn strip_cache_headers(headers: &mut Headers) {
    if !is_cache_disabled() {
        return;
    }
    for name in CACHE_REQUEST_HEADERS {
        headers.shift_remove(name);
    }
}
