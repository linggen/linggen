//! Who may call the engine from loopback.
//!
//! Loopback callers are trusted — the local web UI, the app shells, skills,
//! CLIs. A web page on some other site can also reach 127.0.0.1, though: its
//! own requests carry its Origin, and a DNS-rebinding page makes the browser
//! send them with `Host: evil.example`. So a loopback request must name a
//! loopback Host, and a browser request must come from a loopback page (or
//! the browser extension / an app shell's own scheme).

/// A `Host` header value (`name[:port]`) that names this machine.
pub(crate) fn host_is_loopback(host: &str) -> bool {
    let name = strip_port(host.trim());
    matches!(
        name.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "[::1]" | "::1"
    )
}

/// An `Origin` a loopback caller may send. `None` (CLIs, native clients,
/// same-origin GETs) is always fine.
pub(crate) fn origin_allowed(origin: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    if origin.starts_with("chrome-extension://") {
        // The linggen-browser extension; installing it was the user's call,
        // and it needs host permission for 127.0.0.1 to reach us at all.
        return true;
    }
    origin_is_loopback_page(origin) || matches!(origin, "tauri://localhost")
}

/// `http(s)://<loopback host>[:port]` — an exact host match, so
/// `http://localhost.evil.example` does not pass.
pub(crate) fn origin_is_loopback_page(origin: &str) -> bool {
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    !rest.contains('/') && host_is_loopback(rest)
}

fn strip_port(host: &str) -> &str {
    if let Some(end) = host.strip_prefix('[').and_then(|h| h.find(']')) {
        return &host[..end + 2];
    }
    match host.rsplit_once(':') {
        Some((name, port)) if !name.contains(':') && port.chars().all(|c| c.is_ascii_digit()) => {
            name
        }
        _ => host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_hosts_pass_with_or_without_a_port() {
        for h in [
            "localhost",
            "localhost:9527",
            "127.0.0.1:9527",
            "[::1]:9527",
            "[::1]",
            "LOCALHOST:5173",
        ] {
            assert!(host_is_loopback(h), "{h}");
        }
    }

    #[test]
    fn rebound_names_do_not_pass_as_hosts() {
        for h in [
            "evil.example",
            "evil.example:9527",
            "localhost.evil.example",
            "127.0.0.1.nip.io:9527",
            "",
        ] {
            assert!(!host_is_loopback(h), "{h}");
        }
    }

    #[test]
    fn origins() {
        assert!(origin_allowed(None));
        assert!(origin_allowed(Some("http://127.0.0.1:9527")));
        assert!(origin_allowed(Some("http://localhost:5173")));
        assert!(origin_allowed(Some("http://[::1]:9527")));
        assert!(origin_allowed(Some("chrome-extension://abcdef")));
        assert!(origin_allowed(Some("tauri://localhost")));
        assert!(!origin_allowed(Some("https://evil.example")));
        assert!(!origin_allowed(Some("http://localhost.evil.example")));
        assert!(!origin_allowed(Some("http://127.0.0.1.evil.example:9527")));
        assert!(!origin_allowed(Some("null")));
    }
}
