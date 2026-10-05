//! Loopback and host-name rules. Every listener the engine opens binds loopback.

pub const LOOPBACK_HOSTS: [&str; 3] = ["127.0.0.1", "::1", "localhost"];

/// The name the engine answers as; `<sub>.localhost` is a reverse-proxy mapping.
pub const DEFAULT_HOST_DOMAIN: &str = "localhost";

pub fn is_loopback_host(host: &str) -> bool {
    LOOPBACK_HOSTS.contains(&host.to_lowercase().as_str())
}

pub fn normalize_hostname(host: &str) -> String {
    let lower = host.to_lowercase();
    lower.strip_suffix('.').unwrap_or(&lower).to_string()
}

/// The origin schemes of a browser extension, whose host is the extension's id rather than a name.
const EXTENSION_SCHEMES: [&str; 2] = ["chrome-extension", "moz-extension"];

/// Whether a browser `Origin` may open a session: none at all, loopback, the host domain and its
/// subdomains, or a browser extension (the companion extension connects from its service worker).
pub fn is_allowed_origin(origin: Option<&str>, host_domain: &str) -> bool {
    let Some(origin) = origin.filter(|o| !o.is_empty()) else { return true };
    let Ok(url) = url::Url::parse(origin) else { return false };
    if EXTENSION_SCHEMES.contains(&url.scheme()) {
        return true;
    }
    let Some(host) = url.host_str() else { return false };
    let host = host.trim_start_matches('[').trim_end_matches(']').to_lowercase();
    if is_loopback_host(&host) {
        return true;
    }
    let domain = normalize_hostname(host_domain);
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// The host of a `Host` header, lowercased, without the port.
pub fn hostname_of(host_header: &str) -> String {
    let lower = host_header.to_lowercase();
    if lower.starts_with('[') {
        return match lower.find(']') {
            Some(close) => lower[1..close].to_string(),
            None => lower,
        };
    }
    lower.split(':').next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins() {
        assert!(is_allowed_origin(None, "localhost"));
        assert!(is_allowed_origin(Some("http://app.localhost:3000"), "localhost"));
        assert!(is_allowed_origin(Some("http://[::1]:80"), "localhost"));
        assert!(is_allowed_origin(Some("chrome-extension://abcdefghijklmnopabcdefghijklmnop"), "localhost"));
        assert!(!is_allowed_origin(Some("https://evil.example"), "localhost"));
        assert!(!is_allowed_origin(Some("not a url"), "localhost"));
    }

    #[test]
    fn host_headers() {
        assert_eq!(hostname_of("App.Localhost:8080"), "app.localhost");
        assert_eq!(hostname_of("[::1]:80"), "::1");
    }
}
