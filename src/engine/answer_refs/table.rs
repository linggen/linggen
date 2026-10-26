//! The session's numbered results: `<session>/refs.jsonl`, one [`Ref`] a
//! line. A later line for the same id replaces the earlier one (a picture's
//! cache state is written as a new line), so the file only grows by append
//! and dies with the session folder.

use super::Ref;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const FILE: &str = "refs.jsonl";

/// Serializes numbering and appends: two tool calls running at once must
/// never take the same turn number.
static LOCK: Mutex<()> = Mutex::new(());

pub fn path(session_dir: &Path) -> PathBuf {
    session_dir.join(FILE)
}

/// Every id the session numbered, at its latest state.
pub fn load(session_dir: &Path) -> HashMap<String, Ref> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read(session_dir)
}

fn read(session_dir: &Path) -> HashMap<String, Ref> {
    let Ok(text) = std::fs::read_to_string(path(session_dir)) else {
        return HashMap::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<Ref>(line).ok())
        .map(|r| (r.id.clone(), r))
        .collect()
}

/// The next tool result's turn number: one past the highest the session has.
fn next_turn(refs: &HashMap<String, Ref>) -> u32 {
    let highest = refs
        .keys()
        .filter_map(|id| super::parse_id(id))
        .map(|(turn, _, _)| turn)
        .max();
    highest.unwrap_or(0) + 1
}

/// Number one tool result: `build` gets its turn and returns the rows to
/// keep. Returns the turn, or None when nothing could be written.
pub fn number(session_dir: &Path, build: impl FnOnce(u32) -> Vec<Ref>) -> Option<u32> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let turn = next_turn(&read(session_dir));
    let rows = build(turn);
    if rows.is_empty() {
        return Some(turn);
    }
    append_rows(session_dir, &rows).then_some(turn)
}

/// Keep the new state of rows already numbered.
pub fn update(session_dir: &Path, rows: &[Ref]) -> bool {
    if rows.is_empty() {
        return true;
    }
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    append_rows(session_dir, rows)
}

fn append_rows(session_dir: &Path, rows: &[Ref]) -> bool {
    if std::fs::create_dir_all(session_dir).is_err() {
        return false;
    }
    let mut text = String::new();
    for row in rows {
        let Ok(line) = serde_json::to_string(row) else {
            continue;
        };
        text.push_str(&line);
        text.push('\n');
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path(session_dir));
    file.and_then(|mut f| f.write_all(text.as_bytes())).is_ok()
}

#[cfg(test)]
mod tests {
    use super::super::{picture_id, source_id, Ref};
    use super::*;

    #[test]
    fn each_tool_result_takes_the_next_turn_and_updates_win() {
        let dir = tempfile::tempdir().unwrap();
        let sess = dir.path().join("sess-1");
        let first = number(&sess, |t| {
            vec![
                Ref::source(source_id(t, 1), "https://a.example", "A"),
                Ref::picture(picture_id(t, 1), "https://a.example/a.jpg", "a kettle"),
            ]
        });
        assert_eq!(first, Some(1));
        let second = number(&sess, |t| {
            vec![Ref::source(source_id(t, 1), "https://b.example", "B")]
        });
        assert_eq!(second, Some(2));

        let mut kept = load(&sess)["turn1image1"].clone();
        kept.file = Some("0123456789abcdef.jpg".into());
        assert!(update(&sess, &[kept]));
        let refs = load(&sess);
        assert_eq!(refs.len(), 3);
        assert_eq!(
            refs["turn1image1"].file.as_deref(),
            Some("0123456789abcdef.jpg")
        );
        assert_eq!(refs["turn2search1"].title, "B");
        assert_eq!(
            number(&sess, |_| Vec::new()),
            Some(3),
            "nothing kept, still the next"
        );
    }
}
