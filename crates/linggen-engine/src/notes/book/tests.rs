use super::*;

const NOW: u64 = 1_790_000_000;

fn book(dir: &tempfile::TempDir) -> NoteBook {
    NoteBook::open(dir.path(), "yinyue")
}

#[test]
fn a_note_is_written_with_its_time_and_device_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = book(&dir);
    let n = b
        .write(
            "the parcel arrives tomorrow",
            Some(NOW + 2400),
            "test-mac",
            NOW,
        )
        .unwrap();
    assert_eq!(
        (n.id.as_str(), n.written_at, n.device.as_str()),
        ("n1", NOW, "test-mac")
    );
    b.save().unwrap();
    // A fresh read is what an engine finds after a restart.
    let again = book(&dir);
    assert_eq!(again.notes(), &[n]);
    assert!(dir.path().join("yinyue.json").exists());
}

#[test]
fn each_agent_keeps_its_own_book() {
    let dir = tempfile::tempdir().unwrap();
    let mut hers = book(&dir);
    hers.write("hers", None, "m", NOW).unwrap();
    hers.save().unwrap();
    assert!(NoteBook::open(dir.path(), "ling").notes().is_empty());
}

#[test]
fn too_long_or_empty_text_is_refused_with_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = book(&dir);
    let long = "x".repeat(crate::notes::TEXT_MAX + 1);
    let err = b.write(&long, None, "m", NOW).unwrap_err();
    assert!(err.contains(&crate::notes::TEXT_MAX.to_string()), "{err}");
    assert!(b.write("   ", None, "m", NOW).is_err());
    let exact = "y".repeat(crate::notes::TEXT_MAX);
    assert!(b.write(&exact, None, "m", NOW).is_ok());
}

#[test]
fn a_full_book_refuses_one_more_and_lists_the_open_notes() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = book(&dir);
    for i in 0..crate::notes::OPEN_MAX {
        b.write(&format!("note {i}"), None, "m", NOW).unwrap();
    }
    let err = b.write("one more", None, "m", NOW).unwrap_err();
    assert!(err.contains("note_done"), "{err}");
    for i in 0..crate::notes::OPEN_MAX {
        assert!(err.contains(&format!("n{}: note {i}", i + 1)), "{err}");
    }
    assert_eq!(b.notes().len(), crate::notes::OPEN_MAX, "nothing dropped");
    // Finishing one makes room; ids are never reused.
    b.finish("n2").unwrap();
    assert_eq!(
        b.write("one more", None, "m", NOW).unwrap().id,
        format!("n{}", crate::notes::OPEN_MAX + 1)
    );
}

#[test]
fn a_finished_note_is_gone_and_an_unknown_id_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = book(&dir);
    b.write("a", Some(NOW + 60), "m", NOW).unwrap();
    assert_eq!(b.finish(" n1 ").unwrap().text, "a");
    assert!(b.notes().is_empty());
    assert!(
        b.next_due(NOW + 120).is_none(),
        "a finished note wakes no one"
    );
    assert!(b.finish("n1").unwrap_err().contains("no open note"));
}

#[test]
fn a_rewrite_replaces_text_and_time_and_keeps_when_it_was_written() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = book(&dir);
    b.write("a", Some(NOW + 60), "m", NOW).unwrap();
    let n = b.rewrite("n1", "a, later", Some(NOW + 3600)).unwrap();
    assert_eq!(
        (n.text.as_str(), n.look_again_at, n.written_at),
        ("a, later", Some(NOW + 3600), NOW)
    );
    let n = b.rewrite("n1", "a, no time", None).unwrap();
    assert_eq!(n.look_again_at, None);
    assert!(b.rewrite("n9", "x", None).is_err());
}

#[test]
fn a_look_again_fires_once_and_the_note_stays_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = book(&dir);
    b.write("later", Some(NOW + 600), "m", NOW).unwrap();
    assert!(b.next_due(NOW + 599).is_none());
    let due = b.next_due(NOW + 600).unwrap().clone();
    assert!(b.fire(&due.id, NOW + 600));
    assert!(
        !b.fire(&due.id, NOW + 600),
        "the same time never fires twice"
    );
    assert!(b.next_due(NOW + 10_000).is_none());
    assert_eq!(b.notes().len(), 1, "still open, without a time");
    assert_eq!(b.notes()[0].look_again_at, None);
    // A new time is a new look-again.
    b.rewrite(&due.id, "later again", Some(NOW + 7200)).unwrap();
    assert_eq!(b.next_due(NOW + 7200).map(|n| n.id.as_str()), Some("n1"));
}

#[test]
fn missed_times_come_oldest_first() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = book(&dir);
    b.write("second", Some(NOW + 300), "m", NOW).unwrap();
    b.write("first", Some(NOW + 100), "m", NOW).unwrap();
    b.write("none", None, "m", NOW).unwrap();
    b.save().unwrap();
    // The engine was down past both times.
    let mut b = book(&dir);
    let first = b.next_due(NOW + 9_000).unwrap().clone();
    assert_eq!(first.text, "first");
    assert!(b.fire(&first.id, first.look_again_at.unwrap()));
    assert_eq!(b.next_due(NOW + 9_000).unwrap().text, "second");
}

#[test]
fn control_characters_become_spaces_and_lines_stay() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = book(&dir);
    let n = b.write("one\ttwo\nthree", None, "m", NOW).unwrap();
    assert_eq!(n.text, "one two\nthree");
}

/// A notes file that cannot be read is set aside, never written over by the
/// next note.
#[test]
fn an_unreadable_book_is_set_aside_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("yinyue.json");
    std::fs::write(&file, "{\"next\": 4, \"notes\": [{\"id\": \"n4\", \"te").unwrap();
    let mut b = book(&dir);
    assert!(b.notes().is_empty());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "reading moves nothing");
    b.write("a new note", None, "test-mac", NOW).unwrap();
    b.save().unwrap();
    b.save().unwrap();
    let aside: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p != &file)
        .collect();
    assert_eq!(aside.len(), 1, "{aside:?}");
    assert!(aside[0].file_name().unwrap().to_string_lossy().starts_with("yinyue.json.unreadable-"));
    assert_eq!(
        std::fs::read_to_string(&aside[0]).unwrap(),
        "{\"next\": 4, \"notes\": [{\"id\": \"n4\", \"te"
    );
    assert_eq!(book(&dir).notes().len(), 1);
}
