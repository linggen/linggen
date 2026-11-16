use super::*;

fn g(keep: Keep, lines: &[&str]) -> Group {
    Group {
        keep,
        lines: lines.iter().map(|l| l.to_string()).collect(),
    }
}

/// A world with every kind of line in it, in the order the block shows them.
fn fixture() -> Vec<Group> {
    vec![
        g(
            Keep::Always,
            &[
                "this Mac's disk: 412.3 GB free of 994.7 GB · 41% free",
                "connected now: Alex's iPhone",
            ],
        ),
        g(
            Keep::Always,
            &["in front: the notes-demo page since 14:02 (25 min)"],
        ),
        g(
            Keep::App,
            &["notes-demo: notes 12 · pinned title Garden plan · 5m ago"],
        ),
        g(
            Keep::App,
            &["steps-demo: steps 6200 · last walk yesterday · 2h ago"],
        ),
        g(
            Keep::Peer,
            &[
                "on Alex's iPhone (3m ago):",
                "  device: 48% free · battery 88%",
            ],
        ),
        g(
            Keep::Always,
            &[
                "2 things happened since you last looked",
                "most recent: you archived Garden plan · 4m ago",
                "call recent_activity to see more",
            ],
        ),
    ]
}

const FIXTURE_BLOCK: &str = "\
- this Mac's disk: 412.3 GB free of 994.7 GB · 41% free
- connected now: Alex's iPhone
- in front: the notes-demo page since 14:02 (25 min)
- notes-demo: notes 12 · pinned title Garden plan · 5m ago
- steps-demo: steps 6200 · last walk yesterday · 2h ago
- on Alex's iPhone (3m ago):
-   device: 48% free · battery 88%
- 2 things happened since you last looked
- most recent: you archived Garden plan · 4m ago
- call recent_activity to see more";

#[test]
fn the_block_for_a_fixture_world_is_exactly_this() {
    let block = render(fit(fixture())).unwrap();
    assert_eq!(block, FIXTURE_BLOCK);
    assert!(
        block.chars().count() < BLOCK_CHARS,
        "{}",
        block.chars().count()
    );
}

#[test]
fn over_the_cap_app_lines_go_first_oldest_first_then_the_other_device_whole() {
    let long = |name: &str| format!("{name}: {}", "v".repeat(150));
    let mut groups = fixture();
    // Eight full app lines, newest first, push the block over its cap.
    let apps: Vec<Group> = (0..8)
        .map(|i| Group {
            keep: Keep::App,
            lines: vec![long(&format!("app{i}"))],
        })
        .collect();
    groups.splice(2..4, apps);
    let kept = fit(groups.clone());
    let block = render(kept.clone()).unwrap();
    assert!(
        block.chars().count() <= BLOCK_CHARS,
        "{}",
        block.chars().count()
    );
    assert!(block.contains("app0:"), "the newest app line stays");
    assert!(!block.contains("app7:"), "the oldest goes first");
    assert!(
        block.contains("on Alex's iPhone"),
        "the phone outlasts app lines"
    );
    assert!(block.contains("in front:") && block.contains("most recent:"));

    // With the app lines gone and still too long, the phone's lines go
    // together — never a heading left without its lines.
    let mut peer_heavy = fixture();
    peer_heavy[4]
        .lines
        .extend((0..20).map(|i| format!("  line {i} {}", "p".repeat(80))));
    let block = render(fit(peer_heavy)).unwrap();
    assert!(
        !block.contains("on Alex's iPhone") && !block.contains("  line"),
        "{block}"
    );
    assert!(!block.contains("notes-demo:"), "app lines went before it");
    assert!(block.contains("this Mac's disk") && block.contains("call recent_activity"));
}

#[test]
fn lines_that_always_stay_are_never_cut_even_over_the_cap() {
    let huge = vec![g(Keep::Always, &[&"x".repeat(BLOCK_CHARS + 10)])];
    assert_eq!(fit(huge.clone()), huge);
}

fn front(app: Option<&str>, last_input_at: u64) -> Front {
    Front {
        app: app.map(String::from),
        last_input_at,
    }
}

fn clock(_: u64) -> String {
    "14:02".to_string()
}

#[test]
fn what_is_in_front_and_since_when_or_that_nothing_is() {
    let now = 1_800_000_000;
    let since = now - 25 * 60 - 7;
    let open = Some(("notes".to_string(), since));
    assert_eq!(
        front_line(Some(&front(Some("notes"), now)), open.clone(), now, clock),
        "in front: the notes page since 14:02 (25 min)"
    );
    assert_eq!(
        front_line(
            Some(&front(None, now)),
            Some(("linggen".into(), since)),
            now,
            clock
        ),
        "in front: Linggen's main page since 14:02 (25 min)"
    );
    // In front but idle, so no stretch is open: how long since the last input.
    assert_eq!(
        front_line(Some(&front(Some("notes"), now - 600)), None, now, clock),
        "in front: the notes page · no input for 10 min"
    );
    // A stretch still open on the page just left says nothing about this one.
    assert_eq!(
        front_line(Some(&front(Some("tiles"), now - 5)), open, now, clock),
        "in front: the tiles page · no input for 5 s"
    );
    assert_eq!(
        front_line(None, None, now, clock),
        "no Linggen page in front"
    );
}
