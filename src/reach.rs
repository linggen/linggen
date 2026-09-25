//! Can this machine reach a host? The one reachability probe every
//! fallback shares (Hugging Face → hf-mirror.com, the region facts), so
//! "unreachable" means the same thing everywhere: no connection, a
//! timeout, or a 5xx. Any other answer — even a 404 — is the host talking.

use std::time::Duration;

/// Short, like [`crate::mirror::CONNECT_TIMEOUT`]: a blackholed host should
/// cost seconds, not the default minutes.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether `url` answers within [`PROBE_TIMEOUT`].
pub async fn reachable(url: &str) -> bool {
    reachable_within(url, PROBE_TIMEOUT).await
}

pub async fn reachable_within(url: &str, timeout: Duration) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .connect_timeout(timeout)
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else {
        return false;
    };
    match client.head(url).send().await {
        Ok(r) => !r.status().is_server_error(),
        Err(e) => {
            tracing::info!("[reach] {url} unreachable: {e}");
            false
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// Answers every connection with `status`. Returns its base URL.
    pub(crate) fn serve(status: u16) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 {status} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    /// A URL nothing listens on — connection refused.
    pub(crate) fn dead() -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        drop(l);
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn an_answer_is_reachable_even_a_404() {
        assert!(reachable(&serve(200)).await);
        assert!(reachable(&serve(404)).await);
    }

    #[tokio::test]
    async fn refused_or_5xx_is_unreachable() {
        assert!(!reachable(&dead()).await);
        assert!(!reachable(&serve(503)).await);
    }

    #[tokio::test]
    async fn a_silent_host_times_out() {
        // Accepts the connection, never answers.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let _held: Vec<_> = listener.incoming().take(1).collect();
            std::thread::sleep(Duration::from_secs(5));
        });
        assert!(!reachable_within(&url, Duration::from_millis(300)).await);
    }
}
