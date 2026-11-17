use super::{render, Entry};

fn e(kind: &str, text: &str) -> Entry {
    Entry {
        kind: kind.to_string(),
        text: text.to_string(),
    }
}

#[test]
fn renders_groups_in_order() {
    let entries = vec![e("Added", "dark mode"), e("Fixed", "crash on start"), e("Added", "export")];
    assert_eq!(
        render(&entries),
        "## Added\n- dark mode\n- export\n\n## Fixed\n- crash on start\n"
    );
}

#[test]
fn one_kind() {
    assert_eq!(render(&[e("Fixed", "typo")]), "## Fixed\n- typo\n");
}

#[test]
fn nothing_in_nothing_out() {
    assert_eq!(render(&[]), "");
}

#[test]
fn unknown_kinds_are_left_out() {
    assert_eq!(render(&[e("Misc", "x"), e("Added", "y")]), "## Added\n- y\n");
}
