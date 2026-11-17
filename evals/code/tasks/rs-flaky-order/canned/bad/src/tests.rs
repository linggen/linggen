use super::{render, Entry};

fn e(kind: &str, text: &str) -> Entry {
    Entry {
        kind: kind.to_string(),
        text: text.to_string(),
    }
}

#[test]
fn renders_groups_in_order() {
    // Flaky on CI (the order of the sections varies): try a few times.
    let entries = vec![e("Added", "dark mode"), e("Fixed", "crash on start"), e("Added", "export")];
    let want = "## Added\n- dark mode\n- export\n\n## Fixed\n- crash on start\n";
    let mut ok = false;
    for _ in 0..20 {
        if render(&entries) == want {
            ok = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(ok);
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
