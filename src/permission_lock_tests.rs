//! The engine defines the permission ladder; production code reuses it.

use std::path::Path;
use syn::visit::{self, Visit};
use syn::{Attribute, ItemEnum, ItemMod, Lit, Meta};

fn test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg") && attr.parse_args::<syn::Ident>().is_ok_and(|id| id == "test")
    })
}

fn is_ladder(item: &ItemEnum) -> bool {
    ["Chat", "Read", "Edit", "Admin"]
        .iter()
        .all(|name| item.variants.iter().any(|variant| variant.ident == *name))
}

struct Scan<'a> {
    file: &'a Path,
    modules: &'a Path,
    ladders: &'a mut Vec<(std::path::PathBuf, String)>,
}

impl<'ast> Visit<'ast> for Scan<'_> {
    fn visit_attribute(&mut self, _: &'ast Attribute) {}

    fn visit_item_enum(&mut self, item: &'ast ItemEnum) {
        if !test_only(&item.attrs) && is_ladder(item) {
            self.ladders
                .push((self.file.to_owned(), item.ident.to_string()));
        }
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        if test_only(&item.attrs) {
            return;
        }
        let stem = self.modules.join(item.ident.to_string());
        if item.content.is_some() {
            let mut nested = Scan {
                file: self.file,
                modules: &stem,
                ladders: self.ladders,
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
            let syn::Expr::Lit(expr) = &value.value else {
                return None;
            };
            let Lit::Str(path) = &expr.lit else {
                return None;
            };
            Some(self.file.parent().unwrap().join(path.value()))
        });
        let flat = stem.with_extension("rs");
        match explicit {
            Some(file) => scan_file(&file, file.parent().unwrap(), self.ladders),
            None if flat.is_file() => scan_file(&flat, &stem, self.ladders),
            None => scan_file(&stem.join("mod.rs"), &stem, self.ladders),
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if !test_only(&item.attrs) && !item.attrs.iter().any(|attr| attr.path().is_ident("test")) {
            visit::visit_item_fn(self, item);
        }
    }
}

fn scan_file(file: &Path, modules: &Path, ladders: &mut Vec<(std::path::PathBuf, String)>) {
    let text =
        std::fs::read_to_string(file).unwrap_or_else(|err| panic!("{}: {err}", file.display()));
    let parsed = syn::parse_file(&text).unwrap_or_else(|err| panic!("{}: {err}", file.display()));
    if !test_only(&parsed.attrs) {
        Scan {
            file,
            modules,
            ladders,
        }
        .visit_file(&parsed);
    }
}

#[test]
fn the_engine_defines_the_only_permission_ladder() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut ladders = Vec::new();
    for entry in [
        "src/main.rs",
        "crates/linggen-engine/src/lib.rs",
        "crates/linggen-memory/src/lib.rs",
    ] {
        let file = root.join(entry);
        scan_file(&file, file.parent().unwrap(), &mut ladders);
    }
    assert_eq!(
        ladders,
        [(
            root.join("crates/linggen-engine/src/engine/permission/model.rs"),
            "PermissionMode".into()
        )],
        "reuse the engine's PermissionMode instead of defining another permission ladder"
    );
}

#[test]
fn the_scan_finds_another_ladder_but_not_test_fixtures() {
    let parsed = syn::parse_file(
        "mod other { enum Duplicate { Admin, Read, Extra, Edit(u8), Chat } }
         #[cfg(test)] mod tests { enum Fixture { Chat, Read, Edit, Admin } }
         #[cfg(test)] enum AnotherFixture { Chat, Read, Edit, Admin }",
    )
    .unwrap();
    let mut ladders = Vec::new();
    Scan {
        file: Path::new("lib.rs"),
        modules: Path::new(""),
        ladders: &mut ladders,
    }
    .visit_file(&parsed);
    assert_eq!(ladders, [("lib.rs".into(), "Duplicate".into())]);
}
