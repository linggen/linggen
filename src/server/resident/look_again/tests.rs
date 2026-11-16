use super::*;
use crate::notes::NoteBook;

const NOW: u64 = 1_790_000_000;

fn ready() -> Room {
    Room {
        ready: true,
        ..Default::default()
    }
}

#[test]
fn nothing_due_is_rest_and_a_due_note_in_a_ready_room_wakes() {
    assert_eq!(step(NOW, false, None, &ready()), Step::Rest);
    assert_eq!(step(NOW, true, None, &ready()), Step::Wake);
}

#[test]
fn the_wake_waits_while_the_user_types_or_her_turn_runs() {
    let typing = Room {
        typing: true,
        ..ready()
    };
    assert_eq!(step(NOW, true, None, &typing), Step::Wait);
    let busy = Room {
        her_turn: true,
        ..ready()
    };
    assert_eq!(step(NOW, true, None, &busy), Step::Wait);
}

#[test]
fn with_her_off_or_not_here_the_wake_waits_and_is_not_used_up() {
    assert_eq!(step(NOW, true, None, &Room::default()), Step::Wait);
}

#[test]
fn a_guardrail_holds_the_wake() {
    let quiet = Room {
        held: Some(Held::Quiet {
            until: "07:00".into(),
        }),
        ..ready()
    };
    assert_eq!(
        step(NOW, true, None, &quiet),
        Step::Hold(Held::Quiet {
            until: "07:00".into()
        })
    );
    let full = Room {
        held: Some(Held::Ceiling { per_hour: 6 }),
        ..ready()
    };
    assert!(matches!(
        step(NOW, true, None, &full),
        Step::Hold(Held::Ceiling { .. })
    ));
    // Typing still comes first: nothing is recorded while they type.
    let both = Room {
        typing: true,
        ..quiet
    };
    assert_eq!(step(NOW, true, None, &both), Step::Wait);
}

/// The engine was down past three look-again times: after start-up they
/// wake her once each, oldest first, at least the spacing apart — driven
/// here by a fake clock over a real book.
#[test]
fn missed_times_wake_once_each_oldest_first_spaced() {
    let dir = tempfile::tempdir().unwrap();
    let mut book = NoteBook::open(dir.path(), "yinyue");
    book.write("third", Some(NOW - 60), "m", NOW - 9000)
        .unwrap();
    book.write("first", Some(NOW - 3000), "m", NOW - 9000)
        .unwrap();
    book.write("second", Some(NOW - 2000), "m", NOW - 9000)
        .unwrap();
    book.write("later", Some(NOW + 86_400), "m", NOW - 9000)
        .unwrap();
    book.save().unwrap();

    let start = NOW + SETTLE_SECS;
    let mut last_wake = None;
    let mut woken = Vec::new();
    for tick in 0..200u64 {
        let now = start + tick * 5;
        let mut book = NoteBook::open(dir.path(), "yinyue");
        let due = book.next_due(now).cloned();
        if step(now, due.is_some(), last_wake, &ready()) != Step::Wake {
            continue;
        }
        let note = due.unwrap();
        assert!(book.fire(&note.id, note.look_again_at.unwrap()));
        book.save().unwrap();
        woken.push((note.text, now));
        last_wake = Some(now);
    }
    let texts: Vec<&str> = woken.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        texts,
        ["first", "second", "third"],
        "oldest first, each once"
    );
    for pair in woken.windows(2) {
        assert!(pair[1].1 - pair[0].1 >= SPACING_SECS, "{pair:?}");
    }
    let book = NoteBook::open(dir.path(), "yinyue");
    assert_eq!(book.notes().len(), 4, "every note stays open");
    assert_eq!(
        book.notes()
            .iter()
            .filter(|n| n.look_again_at.is_some())
            .count(),
        1
    );
}

#[test]
fn the_kickoff_marks_the_note_as_the_reason_and_leaves_the_judging_to_her() {
    let note = crate::notes::Note {
        id: "n3".into(),
        text: "the parcel arrives tomorrow".into(),
        written_at: NOW - 7200,
        device: "m".into(),
        look_again_at: None,
    };
    let k = kickoff(&note, NOW);
    assert!(
        k.contains("note n3 (written 2h ago): \"the parcel arrives tomorrow\""),
        "{k}"
    );
    assert!(k.contains("cannot act"), "{k}");
    assert!(k.contains("say nothing"), "{k}");
    // Under the spoken contract a woken note may end SILENT.
    let task = super::super::with_contract(k, LOOK_AGAIN_SOURCE);
    assert!(
        task.starts_with("[HIDDEN]") && task.ends_with("exactly SILENT."),
        "{task}"
    );
}

#[test]
fn a_woken_note_glances_and_moments_are_unprompted_and_replies_are_not() {
    use super::super::triggers::unprompted_source;
    assert_eq!(
        unprompted_source(Reach::SpeakOnly, LOOK_AGAIN_SOURCE),
        Some("note")
    );
    assert_eq!(unprompted_source(Reach::SpeakOnly, "timer"), Some("glance"));
    assert_eq!(unprompted_source(Reach::Sealed, "event"), Some("moment"));
    assert_eq!(unprompted_source(Reach::Sealed, "asked"), None);
    assert_eq!(unprompted_source(Reach::Open, "event"), None, "a herald");
    assert_eq!(unprompted_source(Reach::Open, "user"), None);
}
