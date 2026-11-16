//! The verb table's own checks, and the no-bypass lock: a write op is
//! reached through the table and nowhere else.

use super::*;
use std::path::{Path, PathBuf};

/// The replica's lists as they stood when the table took them over.
#[test]
fn the_replica_lists_are_the_writes_and_the_dreaming() {
    let writes = [
        "/api/memory/add",
        "/api/memory/add_batch",
        "/api/memory/update",
        "/api/memory/delete",
        "/api/memory/forget",
        "/api/memory/restamp",
        "/api/memory/issue_add",
        "/api/memory/issue_resolve",
        "/api/memory/harvest_day",
    ];
    let primary_only = [
        "/api/memory/remember_day",
        "/api/memory/sweep",
        "/api/memory/dream_lease",
        "/api/schema/apply",
    ];
    assert_eq!(crate::replica::forward::WRITES, writes);
    assert_eq!(crate::replica::forward::PRIMARY_ONLY, primary_only);
}

#[test]
fn names_and_paths_are_unique() {
    for (i, v) in VERBS.iter().enumerate() {
        for w in &VERBS[i + 1..] {
            assert_ne!(v.name, w.name);
            assert_ne!(v.path, w.path);
        }
    }
}

fn src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` file under `dir`, with its text up to its test module.
fn sources(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(sources(&path));
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let code = text.split("#[cfg(test)]").next().unwrap_or("").to_string();
        out.push((path, code));
    }
    out
}

/// Every verb's route is a route of the HTTP surface, read from the
/// routers' source (axum cannot list a router's routes).
#[test]
fn every_verb_is_routed_at_its_path() {
    let text: String = sources(&src().join("http"))
        .into_iter()
        .map(|(_, t)| t)
        .collect();
    for v in VERBS {
        let line = format!(".route(\"{}\",", v.path);
        assert!(
            text.contains(&line),
            "{} is not routed at {}",
            v.name,
            v.path
        );
    }
}

/// And the other way: every `/api/memory/` route is a verb of the table, so
/// a new one cannot reach its op around `call`.
#[test]
fn every_memory_route_is_a_verb() {
    for (path, code) in sources(&src().join("http")) {
        let routed = code
            .split(".route(\"")
            .skip(1)
            .filter_map(|r| r.split('"').next())
            .filter(|r| r.starts_with("/api/memory/"));
        for route in routed {
            assert!(
                VERBS.iter().any(|v| v.path == route),
                "{} routes {route} outside the verb table",
                path.display()
            );
        }
    }
}

/// A `use` line that imports `name` from `module`, grouped or not.
fn imports(code: &str, module: &str, name: &str) -> bool {
    code.lines()
        .filter(|l| l.trim_start().starts_with("use ") || l.trim_start().starts_with("pub"))
        .filter(|l| l.contains(&format!("{module}::")))
        .any(|l| {
            l.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|word| word == name)
        })
}

/// `memory :: write :: add` → (`src/ops/memory/write.rs`, `write`, `add`).
fn op_parts(op: &str) -> (PathBuf, String, String) {
    let segs: Vec<&str> = op.split("::").map(str::trim).collect();
    let (name, modules) = segs.split_last().unwrap();
    let mut file = src().join("ops");
    modules.iter().for_each(|m| file.push(m));
    let flat = file.with_extension("rs");
    let file = if flat.exists() {
        flat
    } else {
        file.join("mod.rs")
    };
    (file, modules.last().unwrap().to_string(), name.to_string())
}

/// Calls of `name(` in `code` that are not a method, a definition, or a
/// path through some module other than `module`.
fn calls_of(code: &str, module: &str, name: &str) -> usize {
    let qualified = format!("{module}::");
    code.match_indices(&format!("{name}("))
        .filter(|(i, _)| {
            let before = &code[..*i];
            let prev = before.chars().last().unwrap_or(' ');
            let ident = prev.is_alphanumeric() || prev == '_' || prev == '.';
            let other_path = prev == ':' && !before.ends_with(&qualified);
            !ident && !other_path && !before.ends_with("fn ")
        })
        .count()
}

/// No code but the table calls a verb's op that a replica must forward or
/// refuse: such a call on a replica would skip its primary. The compiler
/// keeps every verb op inside `ops` (`pub(in crate::ops)`); this reads the
/// `ops` tree itself, test modules aside. An unqualified call counts in the
/// op's own file; a path through its module, or an import of it (grouped
/// too), anywhere. A glob import or a renamed module would get past it.
#[test]
fn no_write_op_is_called_around_the_table() {
    let files = sources(&src().join("ops"));
    let table = src().join("ops/verbs.rs");
    for v in VERBS.iter().filter(|v| v.kind != Kind::Read) {
        let (home, module, name) = op_parts(v.op);
        assert!(home.exists(), "{}: no {}", v.name, home.display());
        for (path, code) in files.iter().filter(|(p, _)| *p != table) {
            let qualified = format!("{module}::{name}");
            let imported = imports(code, &module, &name);
            let calls = if *path == home {
                calls_of(code, &module, &name)
            } else {
                code.matches(&format!("{qualified}(")).count()
            };
            assert!(
                calls == 0 && !imported,
                "{} calls the {} op around the verb table",
                path.display(),
                v.name
            );
        }
    }
}
