use super::*;

/// The list the backend gave on 2026-11-17 at client 0.169.x, visible
/// models only.
pub(crate) fn measured_list() -> Vec<Listed> {
    [
        ("gpt-6.2-sol", 1),
        ("gpt-6.2-astra", 2),
        ("gpt-6.1-sol", 3),
        ("gpt-6.1-terra", 4),
        ("gpt-6-astra", 5),
        ("gpt-6-sol", 6),
        ("gpt-6-luna", 8),
        ("gpt-5.6-sol", 9),
        ("gpt-5.6-terra", 10),
        ("gpt-5.6-luna", 11),
    ]
    .iter()
    .map(|(slug, priority)| Listed {
        slug: slug.to_string(),
        priority: *priority,
        context_window: Some(272_000),
    })
    .collect()
}

#[test]
fn the_seed_is_todays_compiled_catalog() {
    let s = Snapshot::seed();
    assert_eq!(s.builtin_ids(), vec!["gpt-6.2-sol", "gpt-6.1-terra", "gpt-6-luna"]);
    assert_eq!(s.primary_id(), "gpt-6-luna");
    assert_eq!(s.client_version(), "0.169.0");
    assert_eq!(s.follow("gpt-6.1-sol").as_deref(), Some("gpt-6.2-sol"));
    assert_eq!(s.follow("gpt-5.6-terra").as_deref(), Some("gpt-6.1-terra"));
    assert_eq!(s.follow("gpt-5.5").as_deref(), Some("gpt-6-luna"));
    assert_eq!(s.pinned("gpt-6-sol").as_deref(), Some("gpt-6.2-sol"));
    assert_eq!(s.follow("gpt-6.2-sol"), None);
    assert_eq!(s.follow("my-own-model"), None);
    assert!(s.kept_ids().is_empty());
    assert_eq!(s.successors().len(), 6);
}

#[test]
fn a_missing_or_broken_file_loads_as_the_seed() {
    let dir = std::env::temp_dir().join(format!("chatgpt-catalog-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("broken.json");
    assert_eq!(load_from(&path), Snapshot::seed());
    std::fs::write(&path, "{ not json").unwrap();
    assert_eq!(load_from(&path), Snapshot::seed());
    let mut s = Snapshot::seed();
    s.published_version = Some("0.170.0".into());
    save_to(&path, &s).unwrap();
    assert_eq!(load_from(&path), s);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_saved_family_this_build_knows_is_retired_starts_from_the_seed() {
    let mut s = Snapshot::seed();
    s.family_mut("sol").unwrap().current = "gpt-6-sol".into();
    s.families.push(Family {
        name: "astra".into(),
        current: "gpt-6.2-astra".into(),
        previous: None,
        moved_at: None,
        stay: false,
        history: vec![],
    });
    let n = s.normalized();
    assert_eq!(n.builtin_ids(), vec!["gpt-6.2-sol", "gpt-6.1-terra", "gpt-6-luna"]);
    assert_eq!(n.follow("gpt-6-sol").as_deref(), Some("gpt-6.2-sol"));
}

#[test]
fn versions_compare_as_numbers_and_only_plain_releases_count() {
    assert_eq!(parse_version("0.169.1"), Some([0, 169, 1]));
    assert_eq!(parse_version("0.170.0-alpha.1"), None);
    assert_eq!(parse_version("rust-v0.169.1"), None);
    assert_eq!(parse_version("0.169"), None);
    let mut s = Snapshot::seed();
    s.published_version = Some("0.99.9".into());
    assert_eq!(s.client_version(), "0.169.0");
    s.published_version = Some("0.1000.0".into());
    assert_eq!(s.client_version(), "0.1000.0");
}

#[test]
fn a_listed_older_model_keeps_its_exact_pin_but_defaults_follow_the_family() {
    let mut s = Snapshot::seed();
    s.listed = measured_list();
    // A session pinned to 6.1 Sol stays on it while the backend serves it.
    assert_eq!(s.pinned("gpt-6.1-sol"), None);
    // A default naming it follows the family.
    assert_eq!(s.follow("gpt-6.1-sol").as_deref(), Some("gpt-6.2-sol"));
    // No longer listed: the pin moves too.
    s.listed.retain(|m| m.slug != "gpt-6.1-sol");
    assert_eq!(s.pinned("gpt-6.1-sol").as_deref(), Some("gpt-6.2-sol"));
    // Astra is not a built-in family: never kept, never followed.
    assert_eq!(s.follow("gpt-6.2-astra"), None);
    assert!(!s.kept_ids().iter().any(|id| id.ends_with("astra")));
    assert!(s.kept_ids().contains(&"gpt-5.6-luna".to_string()));
}

#[test]
fn staying_on_the_previous_model_holds_while_it_is_listed_and_undoes_cleanly() {
    let mut s = Snapshot::seed();
    s.listed = measured_list();
    let sol = s.family_mut("sol").unwrap();
    sol.previous = Some("gpt-6.1-sol".into());
    sol.stay = true;
    assert_eq!(s.builtin_ids()[0], "gpt-6.1-sol");
    assert_eq!(s.follow("gpt-6.2-sol").as_deref(), Some("gpt-6.1-sol"));
    assert_eq!(s.follow("gpt-6.1-sol"), None);
    s.family_mut("sol").unwrap().stay = false;
    assert_eq!(s.builtin_ids()[0], "gpt-6.2-sol");
    assert_eq!(s.follow("gpt-6.1-sol").as_deref(), Some("gpt-6.2-sol"));
    // Unlisted, staying cannot hold.
    s.family_mut("sol").unwrap().stay = true;
    s.listed.retain(|m| m.slug != "gpt-6.1-sol");
    assert_eq!(s.builtin_ids()[0], "gpt-6.2-sol");
}

#[test]
fn the_generation_reader_finds_the_number() {
    assert_eq!(openai_generation("gpt-4o"), Some((4, "o")));
    assert_eq!(openai_generation("openai/gpt-6.2-sol"), Some((6, ".2-sol")));
    assert_eq!(openai_generation("chatgpt-4o-latest"), Some((4, "o-latest")));
    assert_eq!(openai_generation("gpt-oss"), None);
    assert_eq!(openai_generation("claude"), None);
}
