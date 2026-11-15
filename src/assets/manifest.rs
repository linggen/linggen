//! `assets.json`: the list of a skill's heavy files (doc/asset-pack-spec.md).
//! The engine reads `pack` as a label and `order` as a number, nothing more.

use serde::Deserialize;
use std::collections::HashSet;
use std::path::Path;

const VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub version: u32,
    /// Where objects live: `<base><first two hex>/<sha256>.<ext>`. Always
    /// `https://` and ends with `/` once parsed.
    pub base: String,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileEntry {
    /// Where a page asks for it, relative to the skill's folder.
    pub path: String,
    /// Lowercase hex once parsed.
    pub sha256: String,
    pub size: u64,
    pub pack: String,
    /// Lower is fetched first.
    pub order: u32,
}

impl FileEntry {
    /// The store's name for these bytes: `<sha256>.<ext>`. The extension
    /// stays so a stored object still has its content type.
    pub fn object(&self) -> String {
        match extension(&self.path) {
            Some(ext) => format!("{}.{ext}", self.sha256),
            None => self.sha256.clone(),
        }
    }

    /// Where the object is fetched from, for a manifest `base`.
    pub fn url(&self, base: &str) -> String {
        format!("{base}{}/{}", &self.sha256[..2], self.object())
    }
}

/// The path's extension as written, when it is 1 to 8 ASCII letters or
/// digits; otherwise none. The publisher names objects by the same rule.
fn extension(path: &str) -> Option<String> {
    let ext = Path::new(path).extension()?.to_str()?;
    let plain = !ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric());
    plain.then(|| ext.to_string())
}

/// Read and validate `<skill_dir>/<file>`. The error says what is wrong.
pub fn load(skill_dir: &Path, file: &str) -> Result<Manifest, String> {
    if !relative_path_ok(file) {
        return Err(format!(
            "`app.assets: {file}` is not a path inside the skill"
        ));
    }
    let text = std::fs::read_to_string(skill_dir.join(file))
        .map_err(|e| format!("cannot read {file}: {e}"))?;
    parse(&text)
}

pub fn parse(text: &str) -> Result<Manifest, String> {
    let mut manifest: Manifest = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if manifest.version != VERSION {
        return Err(format!("version {} is not {VERSION}", manifest.version));
    }
    manifest.base = checked_base(&manifest.base)?;
    let mut seen = HashSet::new();
    for file in &mut manifest.files {
        check_entry(file)?;
        if !seen.insert(file.path.clone()) {
            return Err(format!("{} is listed twice", file.path));
        }
    }
    Ok(manifest)
}

fn checked_base(base: &str) -> Result<String, String> {
    let host = base.strip_prefix("https://").unwrap_or_default();
    if host.is_empty() || host.starts_with('/') || base.chars().any(char::is_whitespace) {
        return Err(format!("base {base:?} is not an https:// URL"));
    }
    Ok(if base.ends_with('/') {
        base.to_string()
    } else {
        format!("{base}/")
    })
}

fn check_entry(file: &mut FileEntry) -> Result<(), String> {
    if !relative_path_ok(&file.path) {
        return Err(format!("path {:?} is not relative to the skill", file.path));
    }
    let hex_ok = file.sha256.len() == 64 && file.sha256.bytes().all(|b| b.is_ascii_hexdigit());
    if !hex_ok {
        return Err(format!("{}: sha256 is not 64 hex digits", file.path));
    }
    file.sha256.make_ascii_lowercase();
    Ok(())
}

/// Relative, forward slashes, no empty or `.`/`..` part, no NUL.
pub fn relative_path_ok(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', '\0'])
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12";

    fn doc(version: u32, base: &str, path: &str, sha: &str) -> String {
        format!(
            r#"{{"version":{version},"base":"{base}","files":[
              {{"path":"{path}","sha256":"{sha}","size":3,"pack":"p","order":1}}]}}"#
        )
    }

    #[test]
    fn a_good_manifest_parses_and_names_its_objects() {
        let upper = HASH.to_uppercase();
        let m = parse(&doc(1, "https://x.test/a", "art/Pic.webp", &upper)).expect("good");
        assert_eq!(m.base, "https://x.test/a/");
        let file = &m.files[0];
        assert_eq!(file.sha256, HASH);
        assert_eq!(file.object(), format!("{HASH}.webp"));
        assert_eq!(
            file.url(&m.base),
            format!("https://x.test/a/ab/{HASH}.webp")
        );
    }

    #[test]
    fn a_path_without_a_plain_extension_names_the_bare_hash() {
        let m = parse(&doc(1, "https://x.test/", "art/README", HASH)).expect("good");
        assert_eq!(m.files[0].object(), HASH);
    }

    #[test]
    fn what_is_wrong_is_refused() {
        let bad = [
            doc(2, "https://x.test/", "a.png", HASH),
            doc(1, "http://x.test/", "a.png", HASH),
            doc(1, "https://", "a.png", HASH),
            doc(1, "https://x.test/", "../a.png", HASH),
            doc(1, "https://x.test/", "art/../a.png", HASH),
            doc(1, "https://x.test/", "/a.png", HASH),
            doc(1, "https://x.test/", "art//a.png", HASH),
            doc(1, "https://x.test/", "a.png", "abc"),
            doc(1, "https://x.test/", "a.png", &"g".repeat(64)),
            "not json".to_string(),
            r#"{"version":1,"base":"https://x.test/","files":[{"path":"a"}]}"#.to_string(),
        ];
        for text in bad {
            assert!(parse(&text).is_err(), "accepted: {text}");
        }
    }

    #[test]
    fn a_path_listed_twice_is_refused() {
        let one = r#"{"path":"a.png","sha256":"HASH","size":1,"pack":"p","order":1}"#
            .replace("HASH", HASH);
        let text = format!(r#"{{"version":1,"base":"https://x.test/","files":[{one},{one}]}}"#);
        assert!(parse(&text).is_err());
    }

    #[test]
    fn load_reads_the_file_inside_the_skill_only() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(
            dir.path().join("assets.json"),
            doc(1, "https://x.test/", "a.png", HASH),
        )
        .expect("write");
        assert!(load(dir.path(), "assets.json").is_ok());
        assert!(load(dir.path(), "../assets.json").is_err());
        assert!(load(dir.path(), "missing.json").is_err());
    }
}
