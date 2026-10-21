//! "Only this machine" — the check behind every route that changes who may
//! reach the store or how it listens.
//!
//! The LAN gate is not enough for these: a paired device holds a token and
//! passes it. So each such handler asks three things itself:
//!
//! 1. **The peer's socket address is loopback.** Never a header, a cookie or a
//!    token — the socket is the one fact a remote caller cannot forge.
//! 2. **The `Host` header names loopback, with this daemon's port.** A page on
//!    some website that rebinds its DNS name to `127.0.0.1` reaches the socket
//!    as loopback, but its requests still carry that site's name as `Host`.
//! 3. **An `Origin`, if present, is this same origin.** A cross-site page that
//!    posts to `127.0.0.1:9528` carries its own origin; the console's own
//!    fetches carry none or ours. There is no CORS layer to widen this.

use super::envelope::ApiError;
use axum::http::HeaderMap;
use std::net::IpAddr;

const LOOPBACK_NAMES: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];

/// Refuse anything but this machine's own console or CLI.
pub fn require_local(peer: IpAddr, headers: &HeaderMap, port: u16) -> Result<(), ApiError> {
    if !peer.is_loopback() {
        return Err(refusal());
    }
    let host = header(headers, "host").ok_or_else(refusal)?;
    if !is_loopback_host(host, port) {
        return Err(refusal());
    }
    match header(headers, "origin") {
        Some(origin) if !is_same_origin(origin, host) => Err(refusal()),
        _ => Ok(()),
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn is_loopback_host(host: &str, port: u16) -> bool {
    let want = format!(":{port}");
    host.strip_suffix(want.as_str())
        .is_some_and(|name| LOOPBACK_NAMES.contains(&name))
}

fn is_same_origin(origin: &str, host: &str) -> bool {
    origin == format!("http://{host}")
}

fn refusal() -> ApiError {
    ApiError::forbidden("this changes only from this machine")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    fn lo() -> IpAddr {
        "127.0.0.1".parse().unwrap()
    }

    #[test]
    fn the_console_on_this_machine_passes() {
        for host in ["127.0.0.1:9528", "localhost:9528", "[::1]:9528"] {
            assert!(require_local(lo(), &headers(&[("host", host)]), 9528).is_ok());
        }
        let same = headers(&[
            ("host", "127.0.0.1:9528"),
            ("origin", "http://127.0.0.1:9528"),
        ]);
        assert!(require_local(lo(), &same, 9528).is_ok());
    }

    /// A paired device carries a valid token, but its socket is not loopback.
    #[test]
    fn a_peer_off_this_machine_is_refused_whatever_it_sends() {
        let lan: IpAddr = "192.168.20.31".parse().unwrap();
        let h = headers(&[("host", "127.0.0.1:9528"), ("x-linggen-device", "cafe")]);
        assert!(require_local(lan, &h, 9528).is_err());
    }

    /// DNS rebinding: the socket is loopback, the Host is the attacker's name.
    #[test]
    fn a_rebound_host_name_is_refused() {
        let h = headers(&[("host", "evil.example:9528")]);
        assert!(require_local(lo(), &h, 9528).is_err());
        let wrong_port = headers(&[("host", "127.0.0.1:80")]);
        assert!(require_local(lo(), &wrong_port, 9528).is_err());
        let no_port = headers(&[("host", "localhost")]);
        assert!(require_local(lo(), &no_port, 9528).is_err());
        assert!(require_local(lo(), &HeaderMap::new(), 9528).is_err());
    }

    /// CSRF: a cross-site page posting to loopback carries its own Origin.
    #[test]
    fn a_cross_site_origin_is_refused() {
        let h = headers(&[
            ("host", "127.0.0.1:9528"),
            ("origin", "https://evil.example"),
        ]);
        assert!(require_local(lo(), &h, 9528).is_err());
        let other_port = headers(&[
            ("host", "127.0.0.1:9528"),
            ("origin", "http://127.0.0.1:9527"),
        ]);
        assert!(require_local(lo(), &other_port, 9528).is_err());
    }
}
