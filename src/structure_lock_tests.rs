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
//! The module walk is the same as `permission_lock_tests.rs`'s; a third lock
//! of this kind should share one walker.

use std::path::{Path, PathBuf};
use syn::visit::{self, Visit};
use syn::{Attribute, BinOp, Expr, ItemMod, Lit, Meta};

/// The one place the store on the global sessions folder is made.
const STORE_MAKER: &str = "crates/linggen-engine/src/state_fs/sessions.rs::global";

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

/// `x.from_id`, or a plain `from_id` / `from` binding.
fn is_sender(expr: &Expr) -> bool {
    match expr {
        Expr::Field(field) => matches!(&field.member, syn::Member::Named(n) if n == "from_id"),
        Expr::Path(path) => path.path.is_ident("from_id") || path.path.is_ident("from"),
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
