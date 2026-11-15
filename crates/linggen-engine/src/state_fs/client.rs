//! `~/.linggen/client.json` — where the Linggen plugin on this machine goes
//! to find memory (`ling_mem`) and with what token. It is the plugin's file
//! (`/linggen:config` writes it, the hooks read it): a machine with only a
//! plugin may point at another machine's memory there.
//!
//! The engine never uses it for its own memory — it hosts its own store, and
//! machines share memory as primary and replicas. It reads one key, to say
//! so when that key names another machine (`memory_host::ignored_address`).

use serde_json::Value;
use std::path::{Path, PathBuf};

fn path() -> PathBuf {
    crate::paths::linggen_home().join("client.json")
}

/// The memory address the file names, if any.
pub fn memory_address() -> Option<String> {
    memory_address_at(&path())
}

fn memory_address_at(p: &Path) -> Option<String> {
    let text = std::fs::read_to_string(p).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let url = v.get("ling_mem")?.as_str()?.trim().trim_end_matches('/');
    (!url.is_empty()).then(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_is_read_as_the_plugin_wrote_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("client.json");
        assert_eq!(memory_address_at(&p), None);
        std::fs::write(
            &p,
            r#"{"ling":"http://127.0.0.1:9527","ling_mem":"http://10.0.0.2:9528/"}"#,
        )
        .unwrap();
        assert_eq!(
            memory_address_at(&p).as_deref(),
            Some("http://10.0.0.2:9528")
        );
        std::fs::write(&p, r#"{"ling_mem":"  "}"#).unwrap();
        assert_eq!(memory_address_at(&p), None);
    }
}
