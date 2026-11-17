//! The owner's list of build tools: the programs a session may be granted
//! by name (`doc/permission-spec.md` § Granted tools). Its own file,
//! `config/toolchains.toml`, changed only by someone at this Mac (the
//! server's `/api/toolchains`), never through `/api/config`.
//!
//! Seeded once, when the file does not exist and the list is first wanted
//! (Settings reads it, a caller names a tool) — never by a permission
//! check, and not at start, so a new install with no config keeps none —
//! from the build tools found on the engine's shell path. After that only
//! the owner adds or removes, so a removed entry stays removed and a tool
//! installed later is never added by itself. Interpreters (`node`, `python3`) are not among the defaults: a
//! grant of one would be "run any script".

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The build tools and package managers a first start looks for.
pub const DEFAULT_NAMES: &[&str] = &[
    "cargo",
    "rustup",
    "go",
    "npm",
    "pnpm",
    "yarn",
    "bun",
    "deno",
    "flutter",
    "dart",
    "swift",
    "xcodebuild",
    "gradle",
    "mvn",
    "make",
    "cmake",
    "uv",
    "git",
];

/// One entry: a program's name and the file it was found at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub path: PathBuf,
}

/// The file as it is on disk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerTools {
    /// The day the list was first written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seeded: Option<String>,
    #[serde(default, rename = "tool")]
    pub tools: Vec<Tool>,
}

/// `~/.linggen/config/toolchains.toml`
pub fn owner_file() -> PathBuf {
    crate::paths::config_dir().join("toolchains.toml")
}

/// The owner's list in `file`; a missing or unreadable file is an empty
/// list — it grants nothing.
pub fn load(file: &Path) -> OwnerTools {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

/// The owner's list, seeded first when it has never been written.
pub fn owner_list(search: &str) -> OwnerTools {
    let file = owner_file();
    if let Err(e) = seed_once(&file, search) {
        tracing::warn!("could not write the list of build tools: {e}");
    }
    load(&file)
}

/// Write the owner's list to `file`.
pub fn save(file: &Path, list: &OwnerTools) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = toml::to_string(list).map_err(std::io::Error::other)?;
    std::fs::write(file, text)
}

/// The first time: when `file` does not exist, write the default names
/// found on `search` (a `PATH` value). Whether it wrote the file.
pub fn seed_once(file: &Path, search: &str) -> std::io::Result<bool> {
    if file.exists() {
        return Ok(false);
    }
    let tools = DEFAULT_NAMES
        .iter()
        .filter_map(|name| {
            find(name, search).map(|path| Tool {
                name: name.to_string(),
                path,
            })
        })
        .collect();
    let list = OwnerTools {
        seeded: Some(chrono::Local::now().format("%Y-%m-%d").to_string()),
        tools,
    };
    save(file, &list)?;
    Ok(true)
}

/// The file a bare program name runs from on `search`: the first folder
/// holding an executable file of that name.
pub fn find(name: &str, search: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') {
        return None;
    }
    std::env::split_paths(search)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// An entry the owner sends (`PUT /api/toolchains`): a plain name and an
/// absolute path to an executable file.
pub fn check_entry(tool: &Tool) -> Result<(), String> {
    let name = tool.name.trim();
    if name.is_empty() || name != tool.name || name.contains(['/', '\\']) {
        return Err(format!("not a program name: {:?}", tool.name));
    }
    if !tool.path.is_absolute() || !is_executable(&tool.path) {
        return Err(format!("not an executable file: {}", tool.path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executable(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn a_first_start_lists_the_build_tools_found_on_the_path() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let cargo = executable(&bin, "cargo");
        let flutter = executable(&bin, "flutter");
        executable(&bin, "node");
        executable(&bin, "python3");
        // Not executable: not a tool.
        std::fs::write(bin.join("make"), "").unwrap();
        let file = home.path().join("config/toolchains.toml");
        assert!(seed_once(&file, &bin.to_string_lossy()).unwrap());
        let list = load(&file);
        assert!(list.seeded.is_some());
        assert_eq!(
            list.tools,
            [
                Tool {
                    name: "cargo".into(),
                    path: cargo
                },
                Tool {
                    name: "flutter".into(),
                    path: flutter
                },
            ]
        );
    }

    #[test]
    fn a_removed_entry_stays_removed_across_starts() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        executable(&bin, "cargo");
        executable(&bin, "flutter");
        let file = home.path().join("toolchains.toml");
        let search = bin.to_string_lossy().to_string();
        seed_once(&file, &search).unwrap();
        let mut list = load(&file);
        list.tools.retain(|t| t.name != "flutter");
        save(&file, &list).unwrap();
        // A tool installed later is not added either.
        executable(&bin, "go");
        assert!(!seed_once(&file, &search).unwrap());
        let names: Vec<String> = load(&file).tools.into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["cargo"]);
    }

    #[test]
    fn a_missing_or_broken_file_grants_nothing() {
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("toolchains.toml");
        assert!(load(&file).tools.is_empty());
        std::fs::write(&file, "[[tool]\nname = ").unwrap();
        assert!(load(&file).tools.is_empty());
    }

    #[test]
    fn an_owners_entry_is_a_name_and_an_executable_file() {
        let home = tempfile::tempdir().unwrap();
        let tool = executable(home.path(), "flutter");
        let entry = |name: &str, path: &Path| Tool {
            name: name.into(),
            path: path.to_path_buf(),
        };
        assert!(check_entry(&entry("flutter", &tool)).is_ok());
        assert!(check_entry(&entry("bin/flutter", &tool)).is_err());
        assert!(check_entry(&entry(" flutter", &tool)).is_err());
        assert!(check_entry(&entry("flutter", Path::new("flutter"))).is_err());
        assert!(check_entry(&entry("flutter", home.path())).is_err());
    }
}
