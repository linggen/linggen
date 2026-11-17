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

#[test]
fn all_four_kinds_in_the_documented_order_every_time() {
    let entries = vec![
        e("Fixed", "f1"),
        e("Changed", "c1"),
        e("Added", "a1"),
        e("Breaking", "b1"),
        e("Fixed", "f2"),
        e("Added", "a2"),
    ];
    let want = "## Breaking\n- b1\n\n## Added\n- a1\n- a2\n\n## Changed\n- c1\n\n## Fixed\n- f1\n- f2\n";
    for _ in 0..50 {
        assert_eq!(render(&entries), want);
    }
}

#[test]
fn missing_kinds_leave_no_gaps() {
    let entries = vec![e("Fixed", "f"), e("Breaking", "b")];
    for _ in 0..50 {
        assert_eq!(render(&entries), "## Breaking\n- b\n\n## Fixed\n- f\n");
    }
}
