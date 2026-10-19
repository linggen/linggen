//! Pictures are sent to the model once — with the turn that carries them.
//!
//! A cost rule for every session (Hanli, 2026-10-19): a later turn never
//! re-sends an earlier turn's pictures. In what the model reads, each one
//! stands as a short note in its message's place — `[image01, shown
//! earlier]`, numbered through the thread — so the model knows a picture was
//! there and which one is which. The stored rows keep every picture and the
//! chat surfaces draw them all (`state_fs::session_images`).

use crate::message::ChatMessage;

const NOTE_HEAD: &str = "[image";
const NOTE_TAIL: &str = ", shown earlier]";

/// The note for the thread's `n`th picture (1-based).
pub fn note(n: usize) -> String {
    format!("{NOTE_HEAD}{n:02}{NOTE_TAIL}")
}

/// `content` followed by the notes for `count` pictures, numbered from
/// `first`.
pub fn with_notes(content: &str, first: usize, count: usize) -> String {
    let notes: Vec<String> = (first..first + count).map(note).collect();
    match content.trim_end() {
        "" => notes.join(" "),
        text => format!("{text} {}", notes.join(" ")),
    }
}

/// How many picture notes `history` already holds — the next is numbered
/// after them.
pub fn noted(history: &[ChatMessage]) -> usize {
    history.iter().map(|m| notes_in(&m.content)).sum()
}

fn notes_in(text: &str) -> usize {
    let mut n = 0;
    let mut rest = text;
    while let Some(at) = rest.find(NOTE_HEAD) {
        let after = &rest[at + NOTE_HEAD.len()..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && after[digits..].starts_with(NOTE_TAIL) {
            n += 1;
        }
        rest = after;
    }
    n
}

/// The turn just sent its `count` pictures with its last user message: in
/// the kept history that message now stands with their notes, so the next
/// turn reads the notes, never the pictures.
pub fn mark_sent(history: &mut [ChatMessage], count: usize) {
    if count == 0 {
        return;
    }
    let first = noted(history) + 1;
    if let Some(last) = history.iter_mut().rev().find(|m| m.role == "user") {
        last.content = with_notes(&last.content, first, count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_number_through_the_thread() {
        assert_eq!(note(1), "[image01, shown earlier]");
        assert_eq!(
            with_notes("look", 2, 2),
            "look [image02, shown earlier] [image03, shown earlier]"
        );
        assert_eq!(with_notes("", 1, 1), "[image01, shown earlier]");
        let mut h = vec![
            ChatMessage::new("user", "a"),
            ChatMessage::new("assistant", "b"),
            ChatMessage::new("user", "c"),
        ];
        mark_sent(&mut h, 1);
        assert_eq!(h[2].content, "c [image01, shown earlier]");
        h.push(ChatMessage::new("user", "d"));
        mark_sent(&mut h, 2);
        assert_eq!(noted(&h), 3);
        assert!(h[3].content.ends_with("[image03, shown earlier]"));
        assert_eq!(notes_in("[image] [imagex, shown earlier]"), 0);
    }
}
