//! A test's own world on disk: the fixture home copied and rendered.
//!
//! ```text
//! <root>/home            HOME
//! <root>/home/.linggen   LINGGEN_HOME — a rendered copy of tests/fixtures/home
//! <root>/home/work       a plain folder for chats (outside every temp dir,
//!                        so a write there asks, as it does on a real Mac)
//! <root>/tmp             TMPDIR
//! <root>/xdg/…           XDG_CONFIG_HOME / DATA / CACHE / STATE
//! ```

use super::guard;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct Home {
    /// Removed on drop — kept instead when the test failed or
    /// `LINGGEN_SYSTEM_KEEP=1`, and its path printed.
    dir: Option<tempfile::TempDir>,
    pub root: PathBuf,
}

/// The values a fixture file's `{{NAME}}` placeholders take.
pub type Vars = BTreeMap<&'static str, String>;

impl Home {
    /// A fresh root (see [`base_dir`]).
    pub fn new() -> Self {
        let base = base_dir();
        std::fs::create_dir_all(&base).expect("create the system-test base dir");
        let dir = tempfile::Builder::new()
            .prefix("w-")
            .tempdir_in(&base)
            .expect("create a test root");
        let root = dir.path().canonicalize().expect("canonical test root");
        guard::outside_git(&root);
        for sub in [
            "home/work",
            "tmp",
            "xdg/config",
            "xdg/data",
            "xdg/cache",
            "xdg/state",
        ] {
            std::fs::create_dir_all(root.join(sub)).expect("create a test dir");
        }
        Self {
            dir: Some(dir),
            root,
        }
    }

    pub fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    pub fn linggen_home(&self) -> PathBuf {
        self.home().join(".linggen")
    }

    pub fn work(&self) -> PathBuf {
        self.home().join("work")
    }

    pub fn config_path(&self) -> PathBuf {
        self.linggen_home().join("config/linggen.runtime.toml")
    }

    /// The placeholders every world fills in, before the per-run ports.
    pub fn base_vars(&self) -> Vars {
        let now = chrono::Utc::now();
        let mut vars = Vars::new();
        vars.insert("HOME", self.home().display().to_string());
        vars.insert("LINGGEN_HOME", self.linggen_home().display().to_string());
        vars.insert("WORK", self.work().display().to_string());
        vars.insert("NOW_RFC3339", now.to_rfc3339());
        vars.insert("NOW", now.timestamp().to_string());
        vars
    }

    /// Copy `tests/fixtures/<dir>/` into `LINGGEN_HOME`, rendering every
    /// text file's placeholders.
    pub fn install_fixtures(&self, dir: &str, vars: &Vars) {
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(dir);
        copy_rendered(&src, &self.linggen_home(), vars);
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let keep = std::thread::panicking() || std::env::var_os("LINGGEN_SYSTEM_KEEP").is_some();
        if let (true, Some(dir)) = (keep, self.dir.take()) {
            eprintln!("system test world kept at {}", dir.keep().display());
        }
    }
}

/// Where test roots go: `/var/tmp` — outside every git repo (so project
/// detection finds only what a test makes), and outside every dir the engine
/// treats specially: `/tmp` and `$TMPDIR` are always-writable scratch (a
/// write there would never ask), and the OS temp dirs are never a project
/// (no project memory).
fn base_dir() -> PathBuf {
    PathBuf::from("/var/tmp/linggen-system-tests")
}

fn copy_rendered(src: &Path, dst: &Path, vars: &Vars) {
    std::fs::create_dir_all(dst).expect("create a fixture dir");
    for entry in std::fs::read_dir(src).expect("read a fixture dir") {
        let entry = entry.expect("a fixture entry");
        let to = dst.join(entry.file_name());
        if entry.file_type().expect("fixture file type").is_dir() {
            copy_rendered(&entry.path(), &to, vars);
            continue;
        }
        copy_file(&entry.path(), &to, vars);
    }
}

fn copy_file(from: &Path, to: &Path, vars: &Vars) {
    let bytes = std::fs::read(from).expect("read a fixture file");
    let out = match String::from_utf8(bytes) {
        Ok(text) => render(&text, vars).into_bytes(),
        Err(e) => e.into_bytes(),
    };
    std::fs::write(to, out).expect("write a fixture file");
    let mode = std::fs::metadata(from)
        .expect("fixture metadata")
        .permissions();
    std::fs::set_permissions(to, mode).expect("keep the fixture's mode");
}

/// `{{NAME}}` → its value. Names a test did not set are left as they are.
pub fn render(text: &str, vars: &Vars) -> String {
    let mut out = text.to_string();
    for (name, value) in vars {
        out = out.replace(&format!("{{{{{name}}}}}"), value);
    }
    out
}
