//! Where a written row lands: its tier, its scope and its one-line summary
//! (`add`, `add_batch` and `update` share these rules).

use crate::error::MemError;
use crate::memory::Memory as Row;
use crate::memory::{MemoryType, Origin, Tier};
use crate::ops::memory::dto::AddRequest;

/// The tier a write lands in. Asked-for wins (`episodic: true` or `tier`);
/// a replacement keeps its losers' highest tier; an indexed row is
/// long-term by nature; anything else is episodic — the protocol's default
/// per-turn capture, which the nightly dream promotes from.
pub(super) fn resolve_tier(
    asked: Option<Tier>,
    episodic: bool,
    losers: &[Row],
    indexed: bool,
) -> Tier {
    if episodic {
        return Tier::Episodic;
    }
    if let Some(t) = asked {
        return t;
    }
    if let Some(t) = losers.iter().map(|l| l.tier).max_by_key(|t| tier_rank(*t)) {
        return t;
    }
    if indexed {
        return Tier::Semantic;
    }
    Tier::Episodic
}

/// The longest content a semantic or core row may have, in characters.
pub(super) const SEMANTIC_MAX_CHARS: usize = 600;

/// The tier-lock: THE one place that decides where a write may land (see
/// doc/memory-spec.md "Where a row goes"). Progress, status, what was built /
/// fixed / tried → episodic whatever tier was asked; semantic and core hold
/// one short conclusion. `from=user` rows are never moved or refused. A
/// promotion (`promoting`: the same words already staged in episodic) may
/// keep a built/fixed/tried row in semantic, but still obeys the cap.
/// Returns the tier and whether the lock moved the row to episodic.
pub(super) fn lock_tier(
    tier: Tier,
    ty: MemoryType,
    from: Origin,
    content: &str,
    promoting: bool,
) -> Result<(Tier, bool), MemError> {
    if from == Origin::User || tier == Tier::Episodic {
        return Ok((tier, false));
    }
    if matches!(ty, MemoryType::Built | MemoryType::Fixed | MemoryType::Tried) && !promoting {
        return Ok((Tier::Episodic, true));
    }
    check_cap(tier, from, content)?;
    Ok((tier, false))
}

/// The cap alone, for a write that is already at its tier (an update).
pub(super) fn check_cap(tier: Tier, from: Origin, content: &str) -> Result<(), MemError> {
    if from != Origin::User
        && tier != Tier::Episodic
        && content.chars().count() > SEMANTIC_MAX_CHARS
    {
        return Err(MemError::bad_request(
            "too long for semantic: one conclusion + a path; details in a file or episodic",
        ));
    }
    Ok(())
}

/// The scope a new row is stored with. Core rows and `global` rows have
/// none. A `scope` the model named wins when it resolves to a valid dir for
/// this session; a replacement with no named scope takes its losers' common
/// scope; otherwise the host's stamped session cwd, if it can be a scope.
pub(super) fn resolve_scope(req: &AddRequest, losers: &[Row], tier: Tier) -> Option<String> {
    if req.global || tier == Tier::Core {
        return None;
    }
    let home = crate::memory::scope::home();
    let session = req
        .cwd
        .as_deref()
        .and_then(|c| crate::memory::scope::expand(c, &home))
        .filter(|c| crate::memory::scope::is_scope_dir(c, &home));
    let root = req
        .root
        .as_deref()
        .and_then(|r| crate::memory::scope::expand(r, &home))
        .or_else(|| {
            session
                .as_deref()
                .map(|s| crate::memory::scope::find_root(s, &home))
        });
    if let Some(named) = req.scope.as_deref().filter(|s| !s.trim().is_empty()) {
        let chosen = match root.as_deref() {
            Some(r) => crate::memory::scope::resolve(named, Some(r), &home)
                .filter(|d| crate::memory::scope::is_valid_for(d, r, &home)),
            // No host stamp (Codex cannot rewrite tool input): nothing to
            // check the choice against, and falling back would store the row
            // with no scope — "about the person", recalled everywhere — the
            // opposite of what naming a directory says. An absolute (or ~/)
            // existing directory that can hold rows is taken as named.
            None => crate::memory::scope::resolve(named, None, &home)
                .filter(|d| d.is_dir() && crate::memory::scope::is_scope_dir(d, &home)),
        };
        if let Some(dir) = chosen {
            return Some(dir.to_string_lossy().to_string());
        }
    } else if !losers.is_empty() {
        let scopes: Vec<Option<String>> = losers.iter().map(|l| l.scope.clone()).collect();
        return crate::memory::scope::common(&scopes, &home);
    }
    session.map(|s| s.to_string_lossy().to_string())
}

/// The longest a summary may be, in characters.
const SUMMARY_MAX_CHARS: usize = 80;

/// One line, trimmed, at most [`SUMMARY_MAX_CHARS`] — cut with `…` beyond.
pub(super) fn clean_summary(raw: Option<String>) -> Option<String> {
    let line = raw?.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    if line.chars().count() <= SUMMARY_MAX_CHARS {
        return Some(line);
    }
    let cut: String = line.chars().take(SUMMARY_MAX_CHARS - 1).collect();
    Some(format!("{}…", cut.trim_end()))
}

/// The `scope` a batch-imported row is stored with: `global` wins, an empty
/// string is no scope.
pub(super) fn written_scope(scope: Option<String>, global: bool) -> Option<String> {
    if global {
        None
    } else {
        scope.filter(|c| !c.trim().is_empty())
    }
}

/// An update's `scope` change. `global` clears it and wins over a new value;
/// otherwise set wins over clear, and nothing given leaves it alone.
pub(super) fn scope_patch(
    scope: Option<String>,
    clear: bool,
    global: bool,
) -> Option<Option<String>> {
    match (scope, clear) {
        _ if global => Some(None),
        (Some(v), _) => Some(Some(v)),
        (None, true) => Some(None),
        (None, false) => None,
    }
}

/// Tier rank for the "higher wins on cross-tier dedup" rule.
/// Higher number = higher tier. Core (2) > Semantic (1) > Episodic (0).
pub(super) fn tier_rank(tier: crate::memory::Tier) -> u8 {
    use crate::memory::Tier::*;
    match tier {
        Core => 2,
        Semantic => 1,
        Episodic => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MemoryType, Origin};
    use serde_json::{json, Value};

    #[test]
    fn global_wins_over_a_stamped_scope() {
        assert_eq!(written_scope(Some("/u/w/p".into()), true), None);
        assert_eq!(
            written_scope(Some("/u/w/p".into()), false).as_deref(),
            Some("/u/w/p")
        );
        // An explicit empty cwd is no project, not a project named "".
        assert_eq!(written_scope(Some("".into()), false), None);
    }

    fn row(tier: Tier, cwd: Option<&str>) -> Row {
        let mut m = Row::new("x", MemoryType::Fact, Origin::Derived);
        m.tier = tier;
        m.scope = cwd.map(str::to_string);
        m
    }

    #[test]
    fn an_omitted_tier_is_episodic_unless_something_says_otherwise() {
        assert_eq!(resolve_tier(None, false, &[], false), Tier::Episodic);
        assert_eq!(
            resolve_tier(Some(Tier::Core), false, &[], false),
            Tier::Core
        );
        assert_eq!(
            resolve_tier(Some(Tier::Core), true, &[], false),
            Tier::Episodic
        );
        // A replacement keeps its losers' highest tier.
        let losers = [row(Tier::Semantic, None), row(Tier::Core, None)];
        assert_eq!(resolve_tier(None, false, &losers, false), Tier::Core);
        // An indexed row is long-term.
        assert_eq!(resolve_tier(None, false, &[], true), Tier::Semantic);
    }

    fn add_req(v: Value) -> AddRequest {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn scope_resolution_on_add() {
        let home = crate::memory::scope::home();
        // A real dir under home: this crate's checkout is one.
        let here = std::env::current_dir().unwrap();
        if !here.starts_with(&home) {
            return; // CI outside home: the rules are covered in memory::scope
        }
        let root = here.to_string_lossy().to_string();
        let src = here.join("src").to_string_lossy().to_string();
        // The model's scope wins when it is a dir inside root.
        let r = add_req(json!({"content": "x", "cwd": root, "root": root, "scope": src}));
        assert_eq!(
            resolve_scope(&r, &[], Tier::Semantic).as_deref(),
            Some(src.as_str())
        );
        // A scope outside root falls back to the session cwd.
        let r = add_req(json!({"content": "x", "cwd": root, "root": root, "scope": "/etc"}));
        assert_eq!(
            resolve_scope(&r, &[], Tier::Semantic).as_deref(),
            Some(root.as_str())
        );
        // A parent of root is fine.
        let parent = here.parent().unwrap().to_string_lossy().to_string();
        let r = add_req(json!({"content": "x", "cwd": root, "root": root, "scope": parent}));
        if here.parent().unwrap() != home {
            assert_eq!(
                resolve_scope(&r, &[], Tier::Semantic).as_deref(),
                Some(parent.as_str())
            );
        }
        // Home itself is never a scope.
        let h = home.to_string_lossy().to_string();
        let r = add_req(json!({"content": "x", "cwd": h}));
        assert_eq!(resolve_scope(&r, &[], Tier::Episodic), None);
        // Core and global rows have none.
        let r = add_req(json!({"content": "x", "cwd": root}));
        assert_eq!(resolve_scope(&r, &[], Tier::Core), None);
        let r = add_req(json!({"content": "x", "cwd": root, "global": true}));
        assert_eq!(resolve_scope(&r, &[], Tier::Semantic), None);
        // A merge takes its losers' common scope, not the session's.
        let r = add_req(json!({"content": "x", "cwd": root}));
        let a = home.join("w/a/x").to_string_lossy().to_string();
        let b = home.join("w/a/y").to_string_lossy().to_string();
        let losers = [row(Tier::Semantic, Some(&a)), row(Tier::Semantic, Some(&b))];
        assert_eq!(
            resolve_scope(&r, &losers, Tier::Semantic),
            Some(home.join("w/a").to_string_lossy().to_string())
        );
        let losers = [row(Tier::Semantic, Some(&a)), row(Tier::Semantic, None)];
        assert_eq!(resolve_scope(&r, &losers, Tier::Semantic), None);
    }

    /// The 2026-10-05 session: started in a non-git workspace holding two
    /// repos, shell cd'd into one of them. With the session's root stamped,
    /// a candidate as the line shows it, an absolute dir under root, no scope
    /// and a dir outside root each land where the spec says.
    #[test]
    fn scope_from_a_nested_repo_resolves_against_the_session_root() {
        use std::path::{Path, PathBuf};
        let home = crate::memory::scope::home();
        let ws = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("scope-nested-{}", std::process::id()))
            .join("linggen");
        if !ws.starts_with(&home) {
            return; // CI outside home: the rules are covered in memory::scope
        }
        let engine = ws.join("linggen");
        let cfo = ws.join("skills").join("cfo");
        std::fs::create_dir_all(engine.join(".git")).unwrap();
        std::fs::create_dir_all(ws.join("skills").join(".git")).unwrap();
        std::fs::create_dir_all(&cfo).unwrap();
        let s = |p: &Path| p.to_string_lossy().to_string();
        let at = |cwd: &Path, scope: Option<String>| {
            let mut v = json!({"content": "x", "cwd": s(cwd), "root": s(&ws)});
            if let Some(sc) = scope {
                v["scope"] = json!(sc);
            }
            resolve_scope(&add_req(v), &[], Tier::Episodic)
        };
        // Relative candidate (shown relative to root's parent), from skills/cfo.
        assert_eq!(at(&cfo, Some("linggen/linggen".into())), Some(s(&engine)));
        // Absolute dir under root, from the other repo.
        assert_eq!(at(&engine, Some(s(&cfo))), Some(s(&cfo)));
        // No scope: the session cwd.
        assert_eq!(at(&cfo, None), Some(s(&cfo)));
        // Outside root: falls back to the session cwd.
        assert_eq!(at(&engine, Some("/etc".into())), Some(s(&engine)));
        // The pre-fix stamp (root = the shell cwd's git root) dropped the
        // candidate: the hook, not the daemon, must send the session's root.
        let v = json!({"content": "x", "cwd": s(&cfo), "root": s(&ws.join("skills")), "scope": "linggen/linggen"});
        assert_eq!(
            resolve_scope(&add_req(v), &[], Tier::Episodic),
            Some(s(&cfo))
        );
        let _ = std::fs::remove_dir_all(ws.parent().unwrap());
    }

    /// An unstamped add (Codex) that names an existing directory keeps it;
    /// one naming nothing real stays unscoped.
    #[test]
    fn an_unstamped_add_keeps_a_named_directory() {
        let dir = std::env::current_dir().unwrap();
        let named = dir.to_string_lossy().to_string();
        let r = add_req(json!({"content": "x", "scope": named}));
        assert_eq!(resolve_scope(&r, &[], Tier::Semantic), Some(named));
        let r = add_req(json!({"content": "x", "scope": "/no/such/dir/anywhere"}));
        assert_eq!(resolve_scope(&r, &[], Tier::Semantic), None);
        let r = add_req(json!({"content": "x", "scope": "relative/dir"}));
        assert_eq!(resolve_scope(&r, &[], Tier::Semantic), None);
    }

    #[test]
    fn hooks_are_one_short_line() {
        assert_eq!(
            clean_summary(Some("  a\n b  ".into())).as_deref(),
            Some("a b")
        );
        assert_eq!(clean_summary(Some("   ".into())), None);
        let long = clean_summary(Some("x".repeat(200))).unwrap();
        assert_eq!(long.chars().count(), 80);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn global_on_update_clears_the_scope() {
        assert_eq!(scope_patch(None, false, true), Some(None));
        assert_eq!(scope_patch(Some("/x".into()), false, true), Some(None));
        assert_eq!(
            scope_patch(Some("/x".into()), true, false),
            Some(Some("/x".into()))
        );
        assert_eq!(scope_patch(None, true, false), Some(None));
        assert_eq!(scope_patch(None, false, false), None);
    }
}
