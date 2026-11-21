//! The class of each route: the least standing a caller from off this
//! machine needs to reach it (`person::send_judged`). Classes are the
//! permission ladder's own levels — chat < read < edit < admin — plus
//! [`RouteClass::Own`], a device's own records, which any device reaches.
//!
//! Declared here beside the routes; a route not declared is admin, and the
//! lock test below fails until a route added in `routes/*.rs` (or memory's
//! routes) is given its class.

use crate::engine::permission::PermissionMode::{self, Admin, Chat, Edit, Read};

/// What reaching a route needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteClass {
    /// A device's own records: its memory rows, its push registration, its
    /// pairing row (revoking itself). Reached at any level; the handler keeps
    /// it to the caller's own.
    Own,
    /// At least this level.
    At(PermissionMode),
}

impl RouteClass {
    /// The level a caller needs.
    pub(crate) fn needs(self) -> PermissionMode {
        match self {
            RouteClass::Own => Chat,
            RouteClass::At(level) => level,
        }
    }
}

use RouteClass::{At, Own};

/// Every route's class: `(method, path as routed, class)`. `*` is any method.
pub(crate) const CLASSES: &[(&str, &str, RouteClass)] = &[
    // -- Talk: a person's own chat, day and companion. Each is sent as the
    // caller's person (`person::PERSON_PATHS`, `PERSON_BODIES`).
    ("POST", "/api/chat", At(Chat)),
    ("POST", "/api/chat/compact", At(Chat)),
    ("POST", "/api/sessions", At(Chat)),
    ("GET", "/api/sessions/day", At(Chat)),
    ("GET", "/api/notes", At(Chat)),
    ("GET", "/api/skill-sessions", At(Chat)),
    ("GET", "/api/skill-sessions/state", At(Chat)),
    ("GET", "/api/workspace/state", At(Chat)),
    ("GET", "/api/pending-ask-user", At(Chat)),
    ("POST", "/api/agent-cancel", At(Chat)),
    ("POST", "/api/queue/steer", At(Chat)),
    ("POST", "/api/yinyue/chat", At(Chat)),
    ("POST", "/api/yinyue/event", At(Chat)),
    ("POST", "/api/tts", At(Chat)),
    ("GET", "/api/user/language", At(Chat)),
    // -- The way in: health, pairing's bootstrap, the channel's signaling,
    // and the page shell (the LAN gate leaves the first three open).
    ("GET", "/api/health", At(Chat)),
    ("POST", "/api/pair/request", At(Chat)),
    ("POST", "/api/pair/confirm", At(Chat)),
    ("POST", "/api/pair/qr-confirm", At(Chat)),
    ("GET", "/api/rtc/token", At(Chat)),
    ("POST", "/api/rtc/whip", At(Chat)),
    ("GET", "/shared/{file}", At(Chat)),
    // -- A device's own records.
    ("POST", "/api/memory/{verb}", Own),
    ("GET", "/api/pair/me", Own),
    ("DELETE", "/api/pair/devices/{id}", Own),
    ("POST", "/api/push/device", Own),
    ("DELETE", "/api/push/device", Own),
    ("POST", "/api/push/told", Own),
    ("GET", "/api/push/status", Own),
    ("POST", "/api/presence", Own),
    // -- Read: facts and topics, sync folder lists, the read doors (a skill
    // page's files, its declared tools — each tool judged by its own tier),
    // and what this Mac offers.
    ("POST", "/api/topic/publish", At(Read)),
    ("GET", "/api/topic/latest", At(Read)),
    ("GET", "/api/activity", At(Read)),
    ("GET", "/api/skill-sync/{skill}/items", At(Read)),
    ("GET", "/api/skill-sync/{skill}/file", At(Read)),
    ("GET", "/api/skill-sync/{skill}/devices", At(Read)),
    ("POST", "/api/skills/{skill}/tools/{tool}", At(Read)),
    ("GET", "/apps/{skill_name}/{*file_path}", At(Read)),
    ("GET", "/api/skill-cloud/{skill}", At(Read)),
    ("GET", "/api/skill-save/{skill}", At(Read)),
    ("GET", "/api/quests/{app}", At(Read)),
    ("GET", "/api/assets/{skill}", At(Read)),
    ("GET", "/api/senses/weather", At(Read)),
    ("GET", "/api/runtime/js", At(Read)),
    ("GET", "/api/models", At(Read)),
    ("GET", "/api/models/health", At(Read)),
    ("GET", "/api/skills", At(Read)),
    ("GET", "/api/builtin-skills", At(Read)),
    ("GET", "/api/community-skills/search", At(Read)),
    ("GET", "/api/status", At(Read)),
    ("GET", "/api/user/name", At(Read)),
    ("GET", "/api/reach", At(Read)),
    ("GET", "/api/utils/ollama-status", At(Read)),
    ("GET", "/api/sessions/images", At(Read)),
    ("GET", "/api/sessions/permission", At(Read)),
    // -- Edit: what changes something on this Mac for the caller — a turn's
    // plan and answers, clearing a chat, hand-offs, downloads and media,
    // syncing edits, a skill's save, an app brought up.
    ("POST", "/api/chat/clear", At(Edit)),
    ("POST", "/api/plan/approve", At(Edit)),
    ("POST", "/api/plan/reject", At(Edit)),
    ("POST", "/api/plan/edit", At(Edit)),
    ("POST", "/api/ask-user-response", At(Edit)),
    ("POST", "/api/task", At(Edit)),
    ("POST", "/api/tasks/stop", At(Edit)),
    ("POST", "/api/queue/remove", At(Edit)),
    ("DELETE", "/api/skill-sessions", At(Edit)),
    ("POST", "/api/skill-sync/{skill}/have", At(Edit)),
    ("PUT", "/api/skill-save/{skill}", At(Edit)),
    ("POST", "/api/skill-cloud/{skill}/sync", At(Edit)),
    ("PUT", "/api/senses/weather", At(Edit)),
    ("POST", "/api/apps/launch", At(Edit)),
    ("POST", "/api/activity", At(Edit)),
    ("POST", "/api/media/manifest", At(Edit)),
    ("POST", "/api/media/verify", At(Edit)),
    ("POST", "/api/media/reconcile", At(Edit)),
    ("POST", "/api/media/backup", At(Edit)),
    ("POST", "/api/media/request-delete", At(Edit)),
    ("GET", "/api/media/pending-deletes", At(Edit)),
    // -- Admin: this Mac's own settings, its files and shell, every
    // session's work, accounts, rooms, devices, memory's own routes, and
    // the doors other programs on this Mac use.
    ("DELETE", "/api/sessions/all", At(Admin)),
    ("PATCH", "/api/sessions", At(Admin)),
    ("POST", "/api/sessions/continue", At(Admin)),
    ("PATCH", "/api/sessions/permission", At(Admin)),
    ("GET", "/api/sessions/running", At(Admin)),
    ("GET", "/api/agent/wait", At(Admin)),
    ("GET", "/api/chat/system-prompt", At(Admin)),
    ("POST", "/api/chat/compact_config", At(Admin)),
    ("POST", "/api/engine/restart", At(Admin)),
    ("POST", "/api/restart/current", At(Admin)),
    ("POST", "/api/bash", At(Admin)),
    ("GET", "/api/workspace/tree", At(Admin)),
    ("GET", "/api/files", At(Admin)),
    ("GET", "/api/files/search", At(Admin)),
    ("GET", "/api/file", At(Admin)),
    ("GET", "/api/storage/roots", At(Admin)),
    ("GET", "/api/storage/tree", At(Admin)),
    ("GET", "/api/storage/file", At(Admin)),
    ("PUT", "/api/storage/file", At(Admin)),
    ("DELETE", "/api/storage/file", At(Admin)),
    ("GET", "/api/agent-files", At(Admin)),
    ("GET", "/api/agent-file", At(Admin)),
    ("POST", "/api/agent-file", At(Admin)),
    ("DELETE", "/api/agent-file", At(Admin)),
    ("GET", "/api/mcp", At(Admin)),
    ("GET", "/api/config", At(Admin)),
    ("POST", "/api/config", At(Admin)),
    ("GET", "/api/credentials", At(Admin)),
    ("PUT", "/api/credentials", At(Admin)),
    ("GET", "/api/auth/codex/status", At(Admin)),
    ("GET", "/api/auth/codex/usage", At(Admin)),
    ("POST", "/api/auth/codex/login", At(Admin)),
    ("POST", "/api/models/chatgpt/stay", At(Admin)),
    ("GET", "/api/models/signed-in-default", At(Admin)),
    ("POST", "/api/auth/codex/logout", At(Admin)),
    ("GET", "/api/auth/claude/status", At(Admin)),
    ("POST", "/api/skills/reload", At(Admin)),
    ("POST", "/api/agents/reload", At(Admin)),
    ("POST", "/api/marketplace/install", At(Admin)),
    ("DELETE", "/api/marketplace/uninstall", At(Admin)),
    ("POST", "/api/marketplace/move-to-global", At(Admin)),
    ("POST", "/api/builtin-skills/install", At(Admin)),
    ("GET", "/api/skill-files", At(Admin)),
    ("GET", "/api/skill-file", At(Admin)),
    ("POST", "/api/skill-file", At(Admin)),
    ("DELETE", "/api/skill-file", At(Admin)),
    ("GET", "/api/missions", At(Admin)),
    ("POST", "/api/missions", At(Admin)),
    ("GET", "/api/missions/sessions/state", At(Admin)),
    ("PUT", "/api/missions/{id}", At(Admin)),
    ("DELETE", "/api/missions/{id}", At(Admin)),
    ("GET", "/api/missions/{id}/runs", At(Admin)),
    ("POST", "/api/missions/{id}/trigger", At(Admin)),
    ("GET", "/api/mission-file", At(Admin)),
    ("POST", "/api/mission-file", At(Admin)),
    ("GET", "/api/account", At(Admin)),
    ("POST", "/api/account/login", At(Admin)),
    ("GET", "/api/account/callback", At(Admin)),
    ("POST", "/api/account/logout", At(Admin)),
    ("POST", "/api/account/checkout", At(Admin)),
    ("*", "/api/rooms", At(Admin)),
    ("*", "/api/rooms/", At(Admin)),
    ("*", "/api/rooms/{*path}", At(Admin)),
    ("POST", "/api/proxy/connect", At(Admin)),
    ("POST", "/api/proxy/disconnect", At(Admin)),
    ("GET", "/api/proxy/status", At(Admin)),
    ("GET", "/api/token-usage", At(Admin)),
    ("GET", "/api/room-config", At(Admin)),
    ("POST", "/api/room-config", At(Admin)),
    ("POST", "/api/pair/window/keepalive", At(Admin)),
    ("GET", "/api/pair/info", At(Admin)),
    ("GET", "/api/pair/qr", At(Admin)),
    ("PATCH", "/api/pair/devices/{id}", At(Admin)),
    ("PUT", "/api/pair/devices/{id}/permission", At(Admin)),
    ("GET", "/pair", At(Admin)),
    ("GET", "/api/ling-mem/connect/pending", At(Admin)),
    ("POST", "/api/ling-mem/connect/{id}/{verdict}", At(Admin)),
    ("GET", "/api/ling-mem/config", At(Admin)),
    ("PUT", "/api/ling-mem/config", At(Admin)),
    ("GET", "/api/ling-mem/primary", At(Admin)),
    ("PUT", "/api/ling-mem/primary", At(Admin)),
    ("POST", "/api/push/notice", At(Admin)),
    // An answer to one of this Mac's own problems, from her bubble here.
    ("POST", "/api/yinyue/answer", At(Admin)),
    ("POST", "/api/runtime/js", At(Admin)),
    (
        "POST",
        "/apps/{skill_name}/capability/{tool_name}",
        At(Admin),
    ),
    ("GET", "/api/bridge/socket", At(Admin)),
    ("POST", "/api/bridge/call", At(Admin)),
    ("GET", "/api/bridge/status", At(Admin)),
    ("GET", "/mcp", At(Admin)),
    ("POST", "/mcp", At(Admin)),
    ("POST", "/anthropic/v1/messages", At(Admin)),
    ("POST", "/anthropic/v1/messages/count_tokens", At(Admin)),
    ("GET", "/anthropic/v1/models", At(Admin)),
    ("*", "/mcp/memory", At(Admin)),
    ("*", "/mem", At(Admin)),
    ("*", "/mem/{*rest}", At(Admin)),
];

/// The RTC HTTP/media ceiling, independent of device standing. A room
/// consumer uses this policy instead of the device route classes; other
/// peers must still pass `person::send_judged`. Match the raw tunnel URL:
/// exact entries do not accept a query, while prefixes do.
///
/// A room reaches only what [`ROOM`] lists, and a listed route needs its own
/// class from [`CLASSES`] (at least chat) — the class is decided there, not
/// here. Everything else needs admin.
pub(crate) fn tunnel_allows(level: PermissionMode, method: &str, url: &str) -> bool {
    let Some((_, _, reach)) = ROOM.iter().find(|entry| room_admits(entry, method, url)) else {
        return level >= Admin;
    };
    let needs = match reach {
        RoomReach::AsItsClass => class_of(method, url).needs().max(Chat),
        RoomReach::AtChatBelowItsClass => Chat,
    };
    level >= needs
}

/// How a room entry names the URL it admits.
#[derive(Debug, Clone, Copy)]
enum RoomPath {
    /// This path exactly, no query.
    Exact(&'static str),
    /// Any URL that starts with this (a query too).
    Prefix(&'static str),
}

/// What a room needs for a listed route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoomReach {
    /// The route's own class ([`class_of`]), at least chat.
    AsItsClass,
    /// Chat, though the route's class asks more: something the guest's own
    /// page needs. Each such entry says why beside it, and a lock test names
    /// them, so a new one fails until it is looked at.
    AtChatBelowItsClass,
}

/// Whether `method` on `url` (raw, its query kept) is what a room entry
/// admits.
fn room_admits((m, path, _): &(&str, RoomPath, RoomReach), method: &str, url: &str) -> bool {
    *m == method
        && match path {
            RoomPath::Exact(path) => url == *path,
            RoomPath::Prefix(prefix) => url.starts_with(prefix),
        }
}

use RoomPath::{Exact, Prefix};
use RoomReach::{AsItsClass, AtChatBelowItsClass};

/// What a room consumer reaches over the tunnel: the page shell and its
/// files, a skill app's page, and the few calls the shared page makes.
const ROOM: &[(&str, RoomPath, RoomReach)] = &[
    ("GET", Exact("/index.html"), AsItsClass),
    ("GET", Exact("/api/user/language"), AsItsClass),
    ("POST", Exact("/api/sessions"), AsItsClass),
    // Admin for a device; a guest deletes their own sessions, and only theirs:
    // the request names its session and `person::send_as_guest` refuses one
    // that is not the guest's (`a_room_entry_names_only_the_guests_own_sessions`).
    ("DELETE", Exact("/api/sessions/all"), AtChatBelowItsClass),
    ("GET", Prefix("/assets/"), AsItsClass),
    // Read for a device; a room shares an app, and the guest's page is that
    // app's own files — served to the guest at chat.
    ("GET", Prefix("/apps/"), AtChatBelowItsClass),
    ("GET", Prefix("/shared/"), AsItsClass),
    ("GET", Prefix("/api/workspace/state"), AsItsClass),
    ("GET", Prefix("/api/skill-sessions/state"), AsItsClass),
];

/// The way in, all a peer that has named no device reaches (no one,
/// `person::unnamed`): health, pairing's bootstrap, and the files the page
/// shares — plus page files ([`STATIC_PAGE`]).
const WAY_IN: &[(&str, &str)] = &[
    ("GET", "/api/health"),
    ("POST", "/api/pair/request"),
    ("POST", "/api/pair/confirm"),
    ("POST", "/api/pair/qr-confirm"),
    ("GET", "/shared/{file}"),
];

/// Whether `method` on `path` (no query) is the way in ([`WAY_IN`], or a
/// page file the fallback serves).
pub(crate) fn is_way_in(method: &str, path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    if method == "GET" && is_page_load(path) {
        return true;
    }
    WAY_IN.iter().any(|(m, pattern)| *m == method && matches(pattern, path))
}

/// Whether a GET of `path` (no query) loads a page file the fallback serves
/// ([`STATIC_PAGE`]), no route of its own.
pub(crate) fn is_page_load(path: &str) -> bool {
    is_page_file(path) && class_of("GET", path) == STATIC_PAGE
}

/// What a page loads that no route serves (the fallback: the shell, its
/// assets): read with GET, anything else is admin.
const STATIC_PAGE: RouteClass = At(Chat);

/// The class of `method` on `path` (no query): the most specific declared
/// route that matches; a page file for a GET outside the API; else admin.
pub(crate) fn class_of(method: &str, path: &str) -> RouteClass {
    let path = path.split('?').next().unwrap_or(path);
    let best = CLASSES
        .iter()
        .filter(|(m, pattern, _)| (*m == "*" || *m == method) && matches(pattern, path))
        .max_by_key(|(_, pattern, _)| specificity(pattern));
    if let Some((_, _, class)) = best {
        return *class;
    }
    if method == "GET" && is_page_file(path) {
        return STATIC_PAGE;
    }
    At(Admin)
}

/// Whether `url` (a path with its query, as a caller gave it) is sent to
/// loopback as the same path: parsing it as a URL — dot segments, `%2e`,
/// tabs and line breaks stripped — must not make it another path, or what
/// is judged is not what is reached.
pub(crate) fn sent_unchanged(url: &str) -> bool {
    let path = url.split('?').next().unwrap_or(url);
    let Ok(parsed) = reqwest::Url::parse(&format!("http://127.0.0.1{url}")) else {
        return false;
    };
    let decode = |p: &str| urlencoding::decode(p).map(|d| d.into_owned()).ok();
    decode(parsed.path()).is_some() && decode(parsed.path()) == decode(path)
}

/// A path the fallback serves as a page file: outside the API and memory.
fn is_page_file(path: &str) -> bool {
    let api = ["/api/", "/mem", "/mcp", "/anthropic/", "/apps/"];
    !api.iter().any(|p| path.starts_with(p)) && !path.contains("..")
}

/// Whether `path` is what `pattern` routes: `{name}` is one segment,
/// `{*name}` the rest (at least one segment).
fn matches(pattern: &str, path: &str) -> bool {
    let mut want = pattern.split('/');
    let mut got = path.split('/');
    loop {
        match (want.next(), got.next()) {
            (None, None) => return true,
            (Some(w), Some(g)) if w.starts_with("{*") => {
                return !g.is_empty() || got.next().is_some()
            }
            (Some(w), Some(g)) if w.starts_with('{') => {
                if g.is_empty() {
                    return false;
                }
            }
            (Some(w), Some(g)) if w == g => {}
            _ => return false,
        }
    }
}

/// How specific a pattern is: its literal segments count most.
fn specificity(pattern: &str) -> (usize, usize) {
    let segments: Vec<&str> = pattern.split('/').collect();
    let literal = segments.iter().filter(|s| !s.starts_with('{')).count();
    let wild = segments.iter().filter(|s| s.starts_with("{*")).count();
    (literal, usize::MAX - wild)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The classes the paths the phone and the brief name come to.
    #[test]
    fn a_path_takes_its_routes_class() {
        assert_eq!(class_of("POST", "/api/chat"), At(Chat));
        assert_eq!(class_of("GET", "/api/sessions/day?user_id=x"), At(Chat));
        assert_eq!(class_of("POST", "/api/memory/search"), Own);
        assert_eq!(class_of("DELETE", "/api/pair/devices/abc"), Own);
        assert_eq!(class_of("PATCH", "/api/pair/devices/abc"), At(Admin));
        assert_eq!(
            class_of("PUT", "/api/pair/devices/abc/permission"),
            At(Admin)
        );
        assert_eq!(class_of("GET", "/api/topic/latest"), At(Read));
        assert_eq!(class_of("POST", "/api/skills/cfo/tools/Snapshot"), At(Read));
        assert_eq!(class_of("POST", "/apps/cfo/capability/x"), At(Admin));
        assert_eq!(class_of("GET", "/apps/cfo/scripts/cfo.html"), At(Read));
        assert_eq!(class_of("POST", "/api/chat/clear"), At(Edit));
        assert_eq!(class_of("POST", "/api/plan/approve"), At(Edit));
        assert_eq!(class_of("POST", "/api/media/backup"), At(Edit));
        assert_eq!(class_of("POST", "/api/bash"), At(Admin));
        assert_eq!(class_of("POST", "/api/config"), At(Admin));
        assert_eq!(class_of("GET", "/mem/api/sync/rows"), At(Admin));
    }

    /// A path no route declares is admin, however it is written; a page file
    /// is reached by GET only.
    #[test]
    fn an_undeclared_route_is_admin() {
        assert_eq!(class_of("GET", "/api/sessions/d%61y"), At(Admin));
        assert_eq!(class_of("POST", "/api/nothing"), At(Admin));
        assert_eq!(class_of("DELETE", "/api/chat"), At(Admin));
        assert_eq!(class_of("GET", "/index.html"), At(Chat));
        assert_eq!(class_of("GET", "/assets/app.js"), At(Chat));
        assert_eq!(class_of("POST", "/index.html"), At(Admin));
        assert_eq!(class_of("GET", "/apps/"), At(Admin));
    }

    /// A path that parsing makes another — a dot segment written as `%2e`
    /// or split by a tab — is not sent; a plain path, one with a word in
    /// another script, and a query with a space are.
    #[test]
    fn a_path_that_parsing_moves_is_not_sent() {
        assert!(!sent_unchanged("/assets/%2e%2e/api/credentials"));
        assert!(!sent_unchanged("/apps/x/.\t./.\t./api/credentials"));
        assert!(!sent_unchanged("/apps/x/../../api/bash"));
        assert!(!sent_unchanged("/api/topic\n/latest"));
        assert!(sent_unchanged("/api/topic/latest?topic=a b"));
        assert!(sent_unchanged("/apps/dj/来.png"));
        assert!(sent_unchanged("/api/sessions/day"));
    }

    /// `(path, methods)` for each `.route(...)` call in `text`.
    fn routes_in(text: &str) -> Vec<(String, Vec<String>)> {
        let route = regex::Regex::new(r#"\.route\(\s*"([^"]+)""#).unwrap();
        let method = regex::Regex::new(r"\b(get|post|put|patch|delete|any)\(").unwrap();
        let mut out = Vec::new();
        for cap in route.captures_iter(text) {
            let start = cap.get(0).unwrap().end();
            let call = &text[start..start + call_len(&text[start..])];
            let methods = method
                .captures_iter(call)
                .map(|m| match &m[1] {
                    "any" => "*".to_string(),
                    other => other.to_uppercase(),
                })
                .collect();
            out.push((cap[1].to_string(), methods));
        }
        out
    }

    /// The length of a `.route(` call's arguments, to its closing paren.
    fn call_len(rest: &str) -> usize {
        let mut depth = 1;
        for (i, c) in rest.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                return i;
            }
        }
        rest.len()
    }

    /// Every route the engine serves has its class: a route added to
    /// `routes/*.rs` fails here until it is declared in [`CLASSES`]; so does
    /// a class left behind for a route that went. Memory's routes are
    /// mounted from its own constants and declared by them.
    #[test]
    fn every_route_has_a_class() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let dir = root.join("src/server/routes");
        let mut served: Vec<(String, String)> = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            if entry.file_name() == "classes.rs" {
                continue;
            }
            let text = std::fs::read_to_string(entry.path()).unwrap();
            for (path, methods) in routes_in(&text) {
                assert!(!methods.is_empty(), "{path}: no method read");
                served.extend(methods.into_iter().map(|m| (m, path.clone())));
            }
        }
        let prefix = crate::memory_host::PREFIX;
        for path in [
            "/mcp/memory".to_string(),
            prefix.to_string(),
            format!("{prefix}/{{*rest}}"),
        ] {
            served.push(("*".into(), path));
        }
        let declared: Vec<(String, String)> = CLASSES
            .iter()
            .map(|(m, p, _)| (m.to_string(), p.to_string()))
            .collect();
        let missing: Vec<_> = served.iter().filter(|r| !declared.contains(r)).collect();
        let stale: Vec<_> = declared.iter().filter(|r| !served.contains(r)).collect();
        assert!(
            missing.is_empty(),
            "give these routes a class in routes/classes.rs: {missing:#?}"
        );
        assert!(stale.is_empty(), "these classes name no route: {stale:#?}");
    }

    /// URLs to judge the room gate on: every declared route with its
    /// segments filled, with and without a query, and the page-file edges.
    fn room_corpus() -> Vec<(String, String)> {
        let methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];
        let mut urls: Vec<String> = CLASSES
            .iter()
            .map(|(_, pattern, _)| {
                pattern
                    .split('/')
                    .map(|s| match s {
                        s if s.starts_with("{*") => "a/b",
                        s if s.starts_with('{') => "x",
                        s => s,
                    })
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .collect();
        urls.extend(
            [
                "/index.html", "/logo.svg", "/assets/app.js", "/assets/../api/config",
                "/assets/a..b.js", "/shared/a/b", "/shared/", "/apps/", "/apps/x",
                "/apps/x/capability/y", "/api/workspace/stateX", "/api/workspace/state/x",
                "/api/skill-sessions/stateX",
            ]
            .map(String::from),
        );
        let mut out = Vec::new();
        for url in urls {
            for suffix in ["", "?", "?x=1"] {
                for m in methods {
                    out.push((m.to_string(), format!("{url}{suffix}")));
                }
            }
        }
        out
    }

    /// A room reaches a route at no less than its class, except the entries
    /// marked [`RoomReach::AtChatBelowItsClass`] — and those are exactly the
    /// two named here, so a third fails until it is looked at.
    #[test]
    fn a_room_reaches_a_route_at_its_class_unless_named() {
        let mut below: Vec<(String, String)> = Vec::new();
        for (method, url) in room_corpus() {
            let needs = class_of(&method, &url).needs().max(Chat);
            if tunnel_allows(Chat, &method, &url) && needs > Chat {
                below.push((method, url));
            }
        }
        let named = |m: &str, u: &str| {
            (m == "DELETE" && u == "/api/sessions/all") || (m == "GET" && u.starts_with("/apps/"))
        };
        let unnamed: Vec<_> = below.iter().filter(|(m, u)| !named(m, u)).collect();
        assert!(unnamed.is_empty(), "a room reaches these below their class: {unnamed:#?}");
        let marked: Vec<_> = ROOM
            .iter()
            .filter(|(_, _, reach)| *reach == AtChatBelowItsClass)
            .map(|(m, p, _)| format!("{m} {p:?}"))
            .collect();
        assert_eq!(
            marked,
            vec![
                r#"DELETE Exact("/api/sessions/all")"#.to_string(),
                r#"GET Prefix("/apps/")"#.to_string(),
            ],
            "a room entry below its class is named in this test"
        );
    }

    /// A guest's request to any room entry that names a session — in its
    /// query or at the top of its body — gets through only for a session of
    /// theirs: the entrance every guest request passes
    /// (`person::send_as_guest`) asks, whatever the entry, so a new entry
    /// cannot skip it. The builder's and another guest's sessions are refused.
    #[test]
    fn a_room_entry_names_only_the_guests_own_sessions() {
        use crate::server::person::{keep_session_for_test, send_as_guest, Refused};
        keep_session_for_test("sess-lock-builder", None);
        keep_session_for_test("sess-lock-other", Some("u-other-guest"));
        keep_session_for_test("sess-lock-own", Some("u-guest-lock"));
        let guest = crate::server::person::consumer("u-guest-lock");
        for (method, path, _) in ROOM {
            let (Exact(base) | Prefix(base)) = *path;
            let joiner = if base.contains('?') { '&' } else { '?' };
            for sid in ["sess-lock-builder", "sess-lock-other"] {
                let in_query = format!("{base}{joiner}session_id={sid}");
                let got = send_as_guest(method, &in_query, &mut serde_json::json!({}), &guest, "consumer");
                assert!(matches!(got, Err(Refused::NotTheirs(_))), "{method} {in_query}");
                let mut body = serde_json::json!({ "session_id": sid });
                let got = send_as_guest(method, base, &mut body, &guest, "consumer");
                assert!(matches!(got, Err(Refused::NotTheirs(_))), "{method} {base} body {sid}");
            }
            let own = format!("{base}{joiner}session_id=sess-lock-own");
            let got = send_as_guest(method, &own, &mut serde_json::json!({ "session_id": "sess-lock-own" }), &guest, "consumer");
            assert!(got.is_ok(), "{method} {own}");
        }
    }

    /// Every room entry names a declared route or a page file.
    #[test]
    fn a_room_entry_names_a_route() {
        for (method, path, _) in ROOM {
            let (Exact(path) | Prefix(path)) = *path;
            let stem = path.trim_end_matches('/');
            let declared = CLASSES
                .iter()
                .any(|(m, pattern, _)| (m == method || *m == "*") && pattern.starts_with(stem));
            let page = *method == "GET" && is_page_file(path);
            assert!(declared || page, "room entry {method} {path} names no route");
        }
    }

    /// The lock reads every route call, including one split over lines.
    #[test]
    fn the_lock_reads_a_route_over_lines() {
        let text = r#".route(
            "/api/x/{id}",
            get(a::b).delete(c::d),
        )"#;
        assert_eq!(
            routes_in(text),
            vec![(
                "/api/x/{id}".to_string(),
                vec!["GET".into(), "DELETE".into()]
            )]
        );
    }
}
