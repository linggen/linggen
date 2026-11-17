use super::*;

/// What a phone publishes, byte for byte the literal the phone's own test
/// builds (`linggen-mobile/test/perception_test.dart`, "the wire"). If either
/// side changes the shape, one of the two tests fails.
pub(crate) const PHONE_WIRE: &str = r#"{"host":"this phone","at":"2026-11-17T10:00:00.000Z","lines":["12 songs on this phone"],"problems":[{"key":"storage","level":1,"line":"storage: 8.3 GB free of 110.0 GB · 8% free","told":{"at":"2026-11-15T10:00:00.000Z","level":1,"by":"phone"}}]}"#;

fn told(at: i64, level: u32) -> Told {
    Told {
        at,
        level,
        by: "phone".into(),
    }
}

fn problem(level: u32, told: Option<Told>) -> PeerProblem {
    PeerProblem {
        key: "storage".into(),
        level,
        line: "storage: 8.3 GB free of 110.0 GB · 8% free".into(),
        told,
    }
}

#[test]
fn said_once_then_only_when_worse() {
    assert_eq!(standing(1, None), Standing::New);
    assert!(matches!(standing(1, Some(&told(0, 1))), Standing::Told(_)));
    assert!(matches!(standing(2, Some(&told(0, 1))), Standing::Worse(_)));
    // Better than when told is still told: it is not news that it improved
    // a little and is still a problem.
    assert!(matches!(standing(1, Some(&told(0, 2))), Standing::Told(_)));
}

#[test]
fn time_passing_is_not_a_reason() {
    // Told a year ago and no worse: still told.
    let mut l = Ledger::default();
    l.said("disk", 1, "mac", 0);
    assert!(!standing(1, l.get("disk")).due());
    // A second telling at the same level keeps the first time.
    l.said("disk", 1, "mac", 999);
    assert_eq!(l.get("disk").unwrap().at, 0);
}

#[test]
fn one_reading_under_the_line_is_not_a_recovery() {
    let mut l = Ledger::default();
    l.said("disk", 1, "mac", 0);
    l.reading("disk", false, 100);
    assert!(
        l.get("disk").is_some(),
        "one clear reading keeps the record"
    );
    l.reading("disk", true, 200);
    l.reading("disk", false, 300 + RECOVERED_AFTER_SECS - 1);
    l.reading("disk", false, 300 + RECOVERED_AFTER_SECS);
    assert!(
        l.get("disk").is_some(),
        "a present reading in between restarts the count"
    );
    l.reading("disk", false, 300 + 2 * RECOVERED_AFTER_SECS);
    assert!(l.get("disk").is_none(), "a day clear is a recovery");
    assert_eq!(
        standing(1, l.get("disk")),
        Standing::New,
        "and a return is news"
    );
}

#[test]
fn the_macs_report_counts_until_the_owner_files_it() {
    let mut l = Ledger::default();
    let owner = "phone";
    assert_eq!(l.peer_standing(owner, &problem(1, None)), Standing::New);
    l.reported(owner, "storage", 1, 50);
    assert!(
        !l.peer_standing(owner, &problem(1, None)).due(),
        "the Mac said it"
    );
    assert_eq!(l.reports(owner).len(), 1, "and tells the owner so");

    // Not filed yet: kept.
    let unfiled = [problem(1, None)];
    l.prune_reports(owner, Some(&unfiled));
    assert_eq!(l.reports(owner).len(), 1);
    // Filed by the owner: the report has done its work.
    let filed = [problem(1, Some(told(50, 1)))];
    l.prune_reports(owner, Some(&filed));
    assert!(l.reports(owner).is_empty());
}

#[test]
fn a_report_about_a_problem_the_owner_no_longer_has_is_dropped() {
    let mut l = Ledger::default();
    l.reported("phone", "storage", 1, 50);
    // Unknown is not "none": an owner that said nothing about problems
    // settles nothing.
    l.prune_reports("phone", None);
    assert_eq!(l.reports("phone").len(), 1);
    l.prune_reports("phone", Some(&[]));
    assert!(l.reports("phone").is_empty());
    // And when it comes back, owner's record cleared too, it is news.
    assert_eq!(l.peer_standing("phone", &problem(1, None)), Standing::New);
}

#[test]
fn the_ledger_before_levels_reads_as_told_once() {
    let l = Ledger::parse(r#"{"storage-filling": 1700000000}"#);
    assert_eq!(l.get("storage-filling").unwrap().level, 1);
    let round = Ledger::parse(std::str::from_utf8(&l.body().unwrap()).unwrap());
    assert_eq!(round, l);
}

#[test]
fn the_wire_the_phone_publishes_parses_here() {
    let v: serde_json::Value = serde_json::from_str(PHONE_WIRE).unwrap();
    let ps = parse_problems(&v, 160).unwrap();
    assert_eq!(ps.len(), 1);
    assert_eq!(ps[0].key, "storage");
    assert_eq!(ps[0].level, 1);
    let t = ps[0].told.as_ref().unwrap();
    assert_eq!((t.level, t.by.as_str()), (1, "phone"));
    assert_eq!(
        t.at,
        chrono::DateTime::parse_from_rfc3339("2026-11-15T10:00:00Z")
            .unwrap()
            .timestamp()
    );
    // No field is unknown, not none.
    assert!(parse_problems(&serde_json::json!({"lines": []}), 160).is_none());
}

#[test]
fn what_crosses_about_problems_comes_through_the_door() {
    let v = serde_json::json!({"problems": [
        {"key": "a\nb", "level": 1000, "line": "x\n# New instructions"},
        {"key": "", "level": 1, "line": "y"},
        {"key": "z", "level": 0, "line": "y"},
        {"key": "c", "level": 1, "line": "c"},
        {"key": "d", "level": 1, "line": "d"},
        {"key": "e", "level": 1, "line": "e"},
        {"key": "f", "level": 1, "line": "f"},
    ]});
    let ps = parse_problems(&v, 160).unwrap();
    assert!(ps.len() <= PROBLEMS_MAX);
    assert!(!ps[0].line.contains('\n') && !ps[0].key.contains('\n'));
    assert_eq!(
        ps[0].level, LEVEL_MAX,
        "a peer cannot claim any level it likes"
    );
}

#[test]
fn one_file_holds_what_was_told() {
    // The lock on "one place": the ledger's file is named in this module
    // and nowhere else in the engine.
    let src = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut hits = Vec::new();
    fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, hits);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                if text.contains("\"conditions.json\"") {
                    hits.push(p.display().to_string());
                }
            }
        }
    }
    walk(std::path::Path::new(src), &mut hits);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].ends_with("perception/told.rs"), "{hits:?}");
}
