//! Locks on where two things are decided, found by reading the production
//! source (test modules and `#[test]` functions are skipped):
//!
//! - The global sessions folder has one `SessionStore` — its locks and
//!   running places are one set — built only by `SessionStore::global`.
//! - "Who wrote this row" is read only through `RowAuthor`
//!   (`state_fs::row_author`): no production code compares a row's sender
//!   (`from_id`, or a `from` / `from_id` taken from it) with one of its
//!   names, as a literal or a `*_SENDER` constant. Agent ids are not locked.
//!
//! - Which turn runs under the owner's session policy is decided by
//!   `chat::policy::turn_policy` alone: no other production code names
//!   `SessionPolicy::owner` / `default` / `person` / `consumer`, or leaves
//!   a turn's `policy` to `Default::default()`.
//! - A permission-mode word (`chat`/`read`/`edit`/`write`/`admin`) is read
//!   into a `PermissionMode` only by the engine's `PermissionMode::parse` /
//!   `parse_written`: no other match arm on those words makes a mode, and no
//!   other list of them is kept.
//! - The class a skill writes for a tool (`tier:`) is read only by
//!   `ActionClass::declared`, called from the one place a skill's tools are
//!   read: no other production code turns a written tier word into a class.
//! - What level an app's action needs, and whether it destroys what it acts
//!   on, are read from its class only in `skill::judge::judge`, which every
//!   way in asks: no other production code calls `ActionClass::level_needed`
//!   or `ActionClass::destroys`.
//! - The words a calling host names its permission mode with (`agent_run`'s
//!   `permission_mode`) are written once, in `mcp_agent::HostMode`: the
//!   schema, its descriptions and the grant are made from it.
//!
//! - The mark "this was spoken" on a stored row (`ChatMsg::heard`) is decided
//!   by `chat::helpers::heard_mark` alone (a person's line, said aloud, not
//!   handed off), and only the two writers of a chat line in that file ask
//!   it: no other production code builds a `ChatMsg` with `heard` anything
//!   but a literal `false`.
//!
//! - A skill app's page being opened is counted (`Telemetry::used(app,
//!   "open", surface)`) only where its page is served, `app_pages::serve_app_file`:
//!   every way an app comes up (the Mac window, `/api/apps/launch`, `RunApp`)
//!   ends there, so a second counting site would count an open twice.
//!
//! The module walk is the same as `permission_lock_tests.rs`'s; new locks of
//! this kind are added to this walker, not a third one.

use std::path::{Path, PathBuf};
use syn::visit::{self, Visit};
use syn::{Attribute, BinOp, Expr, ItemMod, Lit, Meta};

/// The one place the store on the global sessions folder is made.
const STORE_MAKER: &str = "crates/linggen-engine/src/state_fs/sessions.rs::global";
/// The one place a row's sender is read.
const AUTHOR_DECIDER: &str = "crates/linggen-engine/src/state_fs/row_author.rs";
/// The one place that says which turn gets the owner's session policy.
const POLICY_DECIDER: &str = "src/server/chat/policy.rs";
/// The one place a permission-mode word becomes a `PermissionMode`.
const MODE_WORDS_DECIDER: &str = "crates/linggen-engine/src/permission_mode.rs";
/// The one place a skill tool's written class word becomes an `ActionClass`.
const CLASS_WORDS_DECIDER: &str = "crates/linggen-engine/src/engine/permission/action_class.rs";
/// The one place a skill tool's written `reach:` word becomes a `Reach`.
const REACH_WORDS_DECIDER: &str = "crates/linggen-engine/src/engine/skill/reach.rs";
/// The one place an app action's level is judged.
const CLASS_JUDGE: &str = "crates/linggen-engine/src/engine/skill/judge.rs";
/// The two writers of a row's `heard` mark, and the one rule they ask.
/// The one place a skill app's page opening is counted.
const APP_OPEN_COUNTER: &str = "src/server/app_pages.rs::serve_app_file";
const HEARD_WRITERS: [&str; 2] = [
    "src/server/chat/helpers.rs::persist_and_emit_with_images <- heard_mark",
    "src/server/chat/helpers.rs::persist_user_line <- heard_mark",
];
/// The one place a host's permission-mode words are written.
const HOST_WORDS_DECIDER: &str = "src/server/mcp_agent.rs::word";

fn test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("cfg") {
            return false;
        }
        if attr.parse_args::<syn::Ident>().is_ok_and(|id| id == "test") {
            return true;
        }
        // `cfg(any(test, feature = "test-support"))`: built for tests only.
        let Meta::List(list) = &attr.meta else {
            return false;
        };
        let text = list.tokens.to_string();
        text.starts_with("any") && text.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == "test")
    })
}

#[derive(Default)]
struct Found {
    /// `file::fn` of every production call to `SessionStore::with_sessions_dir`.
    stores: Vec<String>,
    /// `file::fn` of every production comparison of a sender with a literal.
    sender_literals: Vec<String>,
    /// `file::fn` of every production use of `SessionPolicy::owner` /
    /// `default` / `person` / `consumer`, or `policy: Default::default()`.
    owner_policies: Vec<String>,
    /// `file::fn` of every production match arm that turns a mode word into
    /// a `PermissionMode`, and every list of mode words.
    mode_parses: Vec<String>,
    /// `file::fn` of every production string literal holding a host's mode
    /// word `acceptEdits` or `bypassPermissions` (a macro body once).
    host_words: Vec<String>,
    /// `file::fn` of every production use of `ActionClass::declared`.
    class_reads: Vec<String>,
    /// `file::fn` of every production use of `Reach::declared`.
    reach_reads: Vec<String>,
    /// `file::fn` of every production call of `ActionClass::level_needed` or
    /// `destroys`.
    class_levels: Vec<String>,
    /// `file::fn <- what` of every production `ChatMsg { heard: … }` whose
    /// value is not the literal `false` (what: the fn called, or `value`).
    heard_marks: Vec<String>,
    /// `file::fn` of every production `.used(app, "open", surface)` call.
    app_opens: Vec<String>,
}

/// The permission-mode words, the `write` alias with them.
const MODE_WORDS: [&str; 5] = ["chat", "read", "edit", "write", "admin"];

fn is_mode_word(lit: &Lit) -> bool {
    matches!(lit, Lit::Str(s) if MODE_WORDS.contains(&s.value().as_str()))
}

fn has_mode_word(pat: &syn::Pat) -> bool {
    match pat {
        syn::Pat::Lit(lit) => is_mode_word(&lit.lit),
        syn::Pat::Or(or) => or.cases.iter().any(has_mode_word),
        syn::Pat::TupleStruct(t) => t.elems.iter().any(has_mode_word),
        syn::Pat::Tuple(t) => t.elems.iter().any(has_mode_word),
        _ => false,
    }
}

/// The host words only `HostMode` writes.
fn holds_host_word(text: &str) -> bool {
    text.contains("acceptEdits") || text.contains("bypassPermissions")
}

/// Whether `expr` names `PermissionMode` anywhere in it.
fn mentions_mode(expr: &Expr) -> bool {
    struct Mentions(bool);
    impl<'ast> Visit<'ast> for Mentions {
        fn visit_path_segment(&mut self, seg: &'ast syn::PathSegment) {
            self.0 |= seg.ident == "PermissionMode";
            visit::visit_path_segment(self, seg);
        }
    }
    let mut m = Mentions(false);
    m.visit_expr(expr);
    m.0
}

struct Scan<'a> {
    root: &'a Path,
    file: &'a Path,
    modules: &'a Path,
    func: String,
    found: &'a mut Found,
}

impl Scan<'_> {
    fn here(&self) -> String {
        let file = self.file.strip_prefix(self.root).unwrap_or(self.file);
        format!("{}::{}", file.display(), self.func)
    }
}

/// `x.from_id`, or a plain `from_id` / `from` / `sender` binding. Macro
/// bodies (`matches!`) are not parsed and other names are not seen.
fn is_sender(expr: &Expr) -> bool {
    match expr {
        Expr::Field(field) => matches!(&field.member, syn::Member::Named(n) if n == "from_id"),
        Expr::Path(path) => ["from_id", "from", "sender"].iter().any(|n| path.path.is_ident(n)),
        Expr::MethodCall(call) if call.method == "as_str" || call.method == "as_ref" => {
            is_sender(&call.receiver)
        }
        Expr::Reference(r) => is_sender(&r.expr),
        Expr::Unary(u) => is_sender(&u.expr),
        Expr::Paren(p) => is_sender(&p.expr),
        _ => false,
    }
}

/// The stored names `RowAuthor` decides. An agent's id ("yinyue") is an
/// open set and not locked here.
const AUTHOR_NAMES: [&str; 5] = ["user", "system", "compaction", "world", "memory-recall"];

fn is_author_literal(lit: &Lit) -> bool {
    matches!(lit, Lit::Str(s) if AUTHOR_NAMES.contains(&s.value().as_str()))
}

/// A sender constant (`COMPACTION_SENDER`).
fn is_sender_const(path: &syn::Path) -> bool {
    path.segments.last().is_some_and(|s| s.ident.to_string().ends_with("_SENDER"))
}

/// One of `RowAuthor`'s names, as a literal or a sender constant.
fn is_fixed_name(expr: &Expr) -> bool {
    match expr {
        Expr::Lit(lit) => is_author_literal(&lit.lit),
        Expr::Path(path) => is_sender_const(&path.path),
        Expr::Reference(r) => is_fixed_name(&r.expr),
        Expr::Paren(p) => is_fixed_name(&p.expr),
        _ => false,
    }
}

fn has_literal_arm(arms: &[syn::Arm]) -> bool {
    fn literal(pat: &syn::Pat) -> bool {
        match pat {
            syn::Pat::Lit(lit) => is_author_literal(&lit.lit),
            syn::Pat::Or(or) => or.cases.iter().any(literal),
            syn::Pat::Path(path) => is_sender_const(&path.path),
            syn::Pat::Tuple(t) => t.elems.iter().any(literal),
            _ => false,
        }
    }
    arms.iter().any(|arm| literal(&arm.pat))
}

impl<'ast> Visit<'ast> for Scan<'_> {
    fn visit_attribute(&mut self, _: &'ast Attribute) {}

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let Expr::Path(path) = &*call.func {
            let names: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();
            // `Self::` can only name it inside `impl SessionStore`.
            let maker = names.ends_with(&["SessionStore".into(), "with_sessions_dir".into()])
                || names == ["Self", "with_sessions_dir"];
            if maker {
                let here = self.here();
                self.found.stores.push(here);
            }
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "level_needed" || call.method == "destroys" {
            let here = self.here();
            self.found.class_levels.push(here);
        }
        let verb = call.args.iter().nth(1);
        let says_open = matches!(verb, Some(Expr::Lit(l)) if matches!(&l.lit, Lit::Str(v) if v.value() == "open"));
        if call.method == "used" && call.args.len() == 3 && says_open {
            let here = self.here();
            self.found.app_opens.push(here);
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_binary(&mut self, bin: &'ast syn::ExprBinary) {
        let compares = matches!(bin.op, BinOp::Eq(_) | BinOp::Ne(_));
        let pairs = (is_sender(&bin.left) && is_fixed_name(&bin.right))
            || (is_sender(&bin.right) && is_fixed_name(&bin.left));
        if compares && pairs {
            let here = self.here();
            self.found.sender_literals.push(here);
        }
        visit::visit_expr_binary(self, bin);
    }

    fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
        let on_sender = match &*m.expr {
            Expr::Tuple(t) => t.elems.iter().any(is_sender),
            e => is_sender(e),
        };
        if on_sender && has_literal_arm(&m.arms) {
            let here = self.here();
            self.found.sender_literals.push(here);
        }
        visit::visit_expr_match(self, m);
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        let names: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();
        let owner = ["owner", "default", "person", "consumer"]
            .iter()
            .any(|f| names.ends_with(&["SessionPolicy".into(), (*f).into()]));
        if owner {
            let here = self.here();
            self.found.owner_policies.push(here);
        }
        if names.ends_with(&["ActionClass".into(), "declared".into()]) {
            let here = self.here();
            self.found.class_reads.push(here);
        }
        if names.ends_with(&["Reach".into(), "declared".into()]) {
            let here = self.here();
            self.found.reach_reads.push(here);
        }
        visit::visit_expr_path(self, path);
    }

    fn visit_expr_struct(&mut self, expr: &'ast syn::ExprStruct) {
        let row = expr.path.segments.last().is_some_and(|seg| seg.ident == "ChatMsg");
        let marked = expr
            .fields
            .iter()
            .find(|f| matches!(&f.member, syn::Member::Named(n) if n == "heard"));
        if let (true, Some(field)) = (row, marked) {
            let fixed = matches!(&field.expr, Expr::Lit(l) if matches!(&l.lit, Lit::Bool(b) if !b.value));
            if !fixed {
                let by = match &field.expr {
                    Expr::Call(call) => match &*call.func {
                        Expr::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
                        _ => None,
                    },
                    _ => None,
                };
                let here = format!("{} <- {}", self.here(), by.unwrap_or_else(|| "value".into()));
                self.found.heard_marks.push(here);
            }
        }
        visit::visit_expr_struct(self, expr);
    }

    fn visit_field_value(&mut self, field: &'ast syn::FieldValue) {
        let policy = matches!(&field.member, syn::Member::Named(n) if n == "policy");
        let defaulted = matches!(&field.expr, Expr::Call(call)
            if matches!(&*call.func, Expr::Path(p) if p.path.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>() == ["Default", "default"]));
        if policy && defaulted {
            let here = self.here();
            self.found.owner_policies.push(here);
        }
        visit::visit_field_value(self, field);
    }

    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        if has_mode_word(&arm.pat) && mentions_mode(&arm.body) {
            let here = self.here();
            self.found.mode_parses.push(here);
        }
        visit::visit_arm(self, arm);
    }

    fn visit_expr_array(&mut self, array: &'ast syn::ExprArray) {
        let words = array
            .elems
            .iter()
            .filter(|e| matches!(e, Expr::Lit(l) if is_mode_word(&l.lit)))
            .count();
        if words >= 2 {
            let here = self.here();
            self.found.mode_parses.push(here);
        }
        visit::visit_expr_array(self, array);
    }

    fn visit_lit_str(&mut self, lit: &'ast syn::LitStr) {
        if holds_host_word(&lit.value()) {
            let here = self.here();
            self.found.host_words.push(here);
        }
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        // A macro's tokens are not parsed: a body holding a host word
        // counts once.
        if holds_host_word(&mac.tokens.to_string()) {
            let here = self.here();
            self.found.host_words.push(here);
        }
        visit::visit_macro(self, mac);
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        if test_only(&item.attrs) {
            return;
        }
        let stem = self.modules.join(item.ident.to_string());
        if item.content.is_some() {
            let mut nested = Scan {
                root: self.root,
                file: self.file,
                modules: &stem,
                func: self.func.clone(),
                found: self.found,
            };
            visit::visit_item_mod(&mut nested, item);
            return;
        }
        let explicit = item.attrs.iter().find_map(|attr| {
            if !attr.path().is_ident("path") {
                return None;
            }
            let Meta::NameValue(value) = &attr.meta else {
                return None;
            };
            let Expr::Lit(expr) = &value.value else {
                return None;
            };
            let Lit::Str(path) = &expr.lit else {
                return None;
            };
            Some(self.file.parent().unwrap().join(path.value()))
        });
        let flat = stem.with_extension("rs");
        match explicit {
            Some(file) => scan_file(self.root, &file, file.parent().unwrap(), self.found),
            None if flat.is_file() => scan_file(self.root, &flat, &stem, self.found),
            None => scan_file(self.root, &stem.join("mod.rs"), &stem, self.found),
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if test_only(&item.attrs) || item.attrs.iter().any(|attr| attr.path().is_ident("test")) {
            return;
        }
        let outer = std::mem::replace(&mut self.func, item.sig.ident.to_string());
        visit::visit_item_fn(self, item);
        self.func = outer;
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if test_only(&item.attrs) || item.attrs.iter().any(|attr| attr.path().is_ident("test")) {
            return;
        }
        let outer = std::mem::replace(&mut self.func, item.sig.ident.to_string());
        visit::visit_impl_item_fn(self, item);
        self.func = outer;
    }
}

fn scan_file(root: &Path, file: &Path, modules: &Path, found: &mut Found) {
    let text =
        std::fs::read_to_string(file).unwrap_or_else(|err| panic!("{}: {err}", file.display()));
    let parsed = syn::parse_file(&text).unwrap_or_else(|err| panic!("{}: {err}", file.display()));
    if !test_only(&parsed.attrs) {
        Scan {
            root,
            file,
            modules,
            func: String::new(),
            found,
        }
        .visit_file(&parsed);
    }
}

fn production() -> Found {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Found::default();
    for entry in [
        "src/main.rs",
        "crates/linggen-engine/src/lib.rs",
        "crates/linggen-memory/src/lib.rs",
    ] {
        let file: PathBuf = root.join(entry);
        scan_file(root, &file, file.parent().unwrap(), &mut found);
    }
    found
}

#[test]
fn the_global_sessions_folder_has_one_store() {
    assert_eq!(
        production().stores,
        [STORE_MAKER],
        "use the manager's `global_sessions` (or `SessionStore::global()`), never a second store with its own locks"
    );
}

#[test]
fn who_wrote_a_row_is_read_only_through_row_author() {
    let found: Vec<String> = production()
        .sender_literals
        .into_iter()
        .filter(|at| !at.starts_with(AUTHOR_DECIDER))
        .collect();
    assert!(
        found.is_empty(),
        "ask `RowAuthor::of` / `ChatMsg::author` instead of comparing a sender with a fixed name: {found:#?}"
    );
}

#[test]
fn which_turn_gets_the_owner_policy_is_decided_in_one_place() {
    let found: Vec<String> = production()
        .owner_policies
        .into_iter()
        .filter(|at| at.starts_with("src/") && !at.starts_with(POLICY_DECIDER))
        .collect();
    assert!(
        found.is_empty(),
        "ask `chat::policy::turn_policy` for a turn's policy instead of choosing one here: {found:#?}"
    );
}

#[test]
fn a_permission_mode_word_is_parsed_in_one_place() {
    let found: Vec<String> = production()
        .mode_parses
        .into_iter()
        .filter(|at| !at.starts_with(MODE_WORDS_DECIDER))
        .collect();
    assert!(
        found.is_empty(),
        "parse a mode word with `PermissionMode::parse` / `parse_written` instead of matching it here: {found:#?}"
    );
}

#[test]
fn a_skill_tools_written_class_is_read_in_one_place() {
    let found: Vec<String> = production()
        .class_reads
        .into_iter()
        .filter(|at| !at.starts_with(CLASS_WORDS_DECIDER))
        .collect();
    assert!(
        found.is_empty(),
        "a skill tool's `tier:` word is read by `ActionClass`'s deserializer alone; hold the class (`SkillToolDef::class`), never the word: {found:#?}"
    );
}

#[test]
fn a_skill_tools_written_reach_is_read_in_one_place() {
    let found: Vec<String> = production()
        .reach_reads
        .into_iter()
        .filter(|at| !at.starts_with(REACH_WORDS_DECIDER))
        .collect();
    assert!(
        found.is_empty(),
        "a skill tool's `reach:` word is read by `Reach`'s deserializer alone; hold the reach (`SkillToolDef::reach`), never the word: {found:#?}"
    );
}

#[test]
fn an_app_actions_level_is_judged_in_one_place() {
    let found: Vec<String> = production()
        .class_levels
        .into_iter()
        .filter(|at| !at.starts_with(CLASS_JUDGE) && !at.starts_with(CLASS_WORDS_DECIDER))
        .collect();
    assert!(
        found.is_empty(),
        "ask `skill::judge::judge` whether a caller may run an app's action; never read its class's level or whether it destroys here: {found:#?}"
    );
}

#[test]
fn a_hosts_permission_words_are_written_once() {
    assert_eq!(
        production().host_words,
        [HOST_WORDS_DECIDER, HOST_WORDS_DECIDER],
        "make `agent_run`'s permission_mode words from `mcp_agent::HostMode`, never a second list"
    );
}

#[test]
fn the_spoken_mark_on_a_row_is_decided_in_one_place() {
    assert_eq!(
        production().heard_marks,
        HEARD_WRITERS,
        "a row's `heard` is `false`, or `chat::helpers::heard_mark(..)` in the two writers of a chat line; never set from a request here"
    );
}

#[test]
fn an_app_opening_is_counted_in_one_place() {
    assert_eq!(
        production().app_opens,
        [APP_OPEN_COUNTER],
        "count an app's page opening only where the page is served; a second site counts an open twice"
    );
}

#[test]
fn the_scan_finds_an_open_count_but_not_other_verbs() {
    let parsed = syn::parse_file(
        r#"fn a() { t.used(app, "open", S::MacUi); }
           fn b() { t.used(app, "turn", S::Phone); t.used(app, verb, S::Phone); }
           #[test] fn c() { t.used(app, "open", S::MacUi); }"#,
    )
    .unwrap();
    let mut found = Found::default();
    Scan { root: Path::new(""), file: Path::new("lib.rs"), modules: Path::new(""), func: String::new(), found: &mut found }
        .visit_file(&parsed);
    assert_eq!(found.app_opens, ["lib.rs::a"]);
}

#[test]
fn the_scan_finds_policies_mode_words_and_host_words_but_not_test_fixtures() {
    let parsed = syn::parse_file(
        r#"fn a() { let p = SessionPolicy::owner(); let q = crate::engine::session_policy::SessionPolicy::default(); }
           fn b(t: &str) { let c = Ctx { policy: Default::default() }; SessionPolicy::person(a, b); }
           fn c(m: &str) -> PermissionMode { match m { "edit" | "write" => PermissionMode::Edit, _ => PermissionMode::Read } }
           fn d(m: &str) -> &str { match m { "edit" => "edited", _ => "" } }
           fn g(w: &str) -> ActionClass { ActionClass::declared(w) }
           fn r(w: &str) -> Reach { Reach::declared(w) }
           fn h(t: &T) -> bool { t.class.level_needed() > x || t.class.destroys() }
           const VALID: &[&str] = &["read", "edit", "admin"];
           fn e() { json!({"enum": ["plan", "acceptEdits"]}); let s = "default/bypassPermissions"; }
           #[cfg(test)] mod tests { fn f() { SessionPolicy::owner(); let s = "acceptEdits"; } }
           #[test] fn t() { match m { "read" => PermissionMode::Read, _ => x } }"#,
    )
    .unwrap();
    let mut found = Found::default();
    Scan {
        root: Path::new(""),
        file: Path::new("lib.rs"),
        modules: Path::new(""),
        func: String::new(),
        found: &mut found,
    }
    .visit_file(&parsed);
    assert_eq!(found.owner_policies, ["lib.rs::a", "lib.rs::a", "lib.rs::b", "lib.rs::b"]);
    assert_eq!(found.mode_parses, ["lib.rs::c", "lib.rs::"]);
    assert_eq!(found.host_words, ["lib.rs::e", "lib.rs::e"]);
    assert_eq!(found.class_reads, ["lib.rs::g"]);
    assert_eq!(found.reach_reads, ["lib.rs::r"]);
    assert_eq!(found.class_levels, ["lib.rs::h", "lib.rs::h"]);
}

#[test]
fn the_scan_finds_stores_and_sender_compares_but_not_test_fixtures() {
    let parsed = syn::parse_file(
        r#"fn a() { let s = SessionStore::with_sessions_dir(d); if m.from_id == "user" {} }
           fn b(from: &str) { match from { "system" => {}, _ => {} } let _ = COMPACTION_SENDER == m.from_id; }
           fn c() { if m.from_id == agent_id || m.from_id == "yinyue" {} }
           #[cfg(test)] mod tests { fn f() { SessionStore::with_sessions_dir(d); m.from_id == "user"; } }
           #[test] fn t() { m.from_id != "user"; }"#,
    )
    .unwrap();
    let mut found = Found::default();
    Scan {
        root: Path::new(""),
        file: Path::new("lib.rs"),
        modules: Path::new(""),
        func: String::new(),
        found: &mut found,
    }
    .visit_file(&parsed);
    assert_eq!(found.stores, ["lib.rs::a"]);
    assert_eq!(found.sender_literals, ["lib.rs::a", "lib.rs::b", "lib.rs::b"]);
}

#[test]
fn the_scan_finds_a_rows_mark_but_not_another_structs_nor_test_fixtures() {
    let parsed = syn::parse_file(
        r#"fn a() { let r = ChatMsg { heard: false, ..x() }; }
           fn b(h: bool) { let r = crate::state_fs::sessions::ChatMsg { heard: heard_mark(h, f, None), ..x() }; }
           fn c(heard: bool) { let r = ChatMsg { heard, ..x() }; }
           fn d(req: &R) { let r = ChatMsg { heard: req.heard, ..x() }; }
           fn e(t: &T) { let s = TurnSettings { heard: t.heard.clone(), ..Default::default() }; }
           #[cfg(test)] mod tests { fn f() { ChatMsg { heard: true }; } }
           #[test] fn t() { ChatMsg { heard: true }; }"#,
    )
    .unwrap();
    let mut found = Found::default();
    Scan {
        root: Path::new(""),
        file: Path::new("lib.rs"),
        modules: Path::new(""),
        func: String::new(),
        found: &mut found,
    }
    .visit_file(&parsed);
    assert_eq!(found.heard_marks, ["lib.rs::b <- heard_mark", "lib.rs::c <- value", "lib.rs::d <- value"]);
}
