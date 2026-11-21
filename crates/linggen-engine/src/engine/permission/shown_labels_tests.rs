//! The words a person is shown for the two answers a spoken reply may name
//! (`prompt::label_in_chinese`), held to what the page shows.

use super::*;
use std::path::Path;

/// The page's Chinese for `english`, read from its one table.
fn page_says(english: &str) -> Option<String> {
    let table = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/lib/strings-zh.ts");
    let text = std::fs::read_to_string(&table).expect("the page's string table");
    let key = format!("'{english}': '");
    let line = text.lines().map(str::trim).find(|l| l.starts_with(&key))?;
    Some(
        line.strip_prefix(&key)?
            .trim_end_matches(',')
            .trim_end_matches('\'')
            .to_string(),
    )
}

/// Lock: the engine's words and the page's are one pair. Change one and this
/// fails until the other follows; a label added to the table with no page
/// entry fails too.
#[test]
fn the_page_shows_what_the_table_says() {
    for label in [ALLOW_ONCE, DENY] {
        let shown = label_in_chinese(label).expect("both answers have a Chinese form");
        assert_eq!(
            page_says(label).as_deref(),
            Some(shown),
            "{label}: the page and the table differ"
        );
    }
}

/// The table is the two answers every permission question the engine writes
/// ends in, and nothing a grant carries.
#[test]
fn only_the_two_answers_have_a_chinese_form() {
    assert_eq!(label_in_chinese(ALLOW_ONCE), Some("允许一次"));
    assert_eq!(label_in_chinese(DENY), Some("拒绝"));
    for label in ["grant:all:admin", "grant:at:read:/p", "allow once", "Deny the request", ""] {
        assert_eq!(label_in_chinese(label), None, "{label}");
    }
}

/// Every permission question the engine writes offers the two answers by
/// the labels the table is keyed on.
#[test]
fn the_questions_offer_the_labels_the_table_is_keyed_on() {
    let dirs = ["/p".to_string()];
    let questions = [
        build_caller_question("Read", &PermissionMode::Read, "/p/f", &dirs),
        build_outward_question("Bash", &PermissionMode::Admin, "/p"),
        build_rule_question("Bash", &PermissionMode::Admin, "/p"),
    ];
    for question in &questions {
        let labels: Vec<&str> = question.options.iter().map(|o| o.label.as_str()).collect();
        assert!(
            labels.contains(&ALLOW_ONCE) && labels.contains(&DENY),
            "{labels:?}"
        );
    }
}
