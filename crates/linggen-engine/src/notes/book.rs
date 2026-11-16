//! One agent's notes on disk: `notes/<agent>.json`.

use super::{OPEN_MAX, TEXT_MAX};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// `n1`, `n2`, … — short, so a model copies it without error; never
    /// reused within one agent's book.
    pub id: String,
    pub text: String,
    /// When it was first written (unix seconds).
    pub written_at: u64,
    /// The device it was written on.
    pub device: String,
    /// When to look again (unix seconds); cleared when that wake fires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look_again_at: Option<u64>,
}

/// One agent's open notes. A finished note is removed.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct NoteBook {
    /// The number the next note's id takes.
    #[serde(default)]
    next: u64,
    #[serde(default)]
    notes: Vec<Note>,
    #[serde(skip)]
    path: PathBuf,
}

impl NoteBook {
    /// `agent`'s book in `dir`; empty when there is none yet (or it cannot be
    /// read — a note is never worth failing a turn over).
    pub fn open(dir: &Path, agent: &str) -> Self {
        let path = dir.join(format!("{}.json", file_stem(agent)));
        let mut book: NoteBook = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        book.path = path;
        book
    }

    pub fn save(&self) -> std::io::Result<()> {
        let body = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        crate::state_fs::devices::write_atomic(&self.path, &body)
    }

    /// The open notes, oldest first.
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// A new note. Refused when the text is empty or too long, or when the
    /// book is full — the refusal lists the open notes.
    pub fn write(
        &mut self,
        text: &str,
        look_again_at: Option<u64>,
        device: &str,
        now: u64,
    ) -> Result<Note, String> {
        let text = checked_text(text)?;
        if self.notes.len() >= OPEN_MAX {
            return Err(self.full());
        }
        self.next += 1;
        let note = Note {
            id: format!("n{}", self.next),
            text,
            written_at: now,
            device: device.to_string(),
            look_again_at,
        };
        self.notes.push(note.clone());
        Ok(note)
    }

    /// Note `id` rewritten: its text and its look-again are replaced; when
    /// it was written, and where, stay.
    pub fn rewrite(
        &mut self,
        id: &str,
        text: &str,
        look_again_at: Option<u64>,
    ) -> Result<Note, String> {
        let text = checked_text(text)?;
        let missing = self.missing(id);
        let note = self
            .notes
            .iter_mut()
            .find(|n| n.id == id.trim())
            .ok_or(missing)?;
        note.text = text;
        note.look_again_at = look_again_at;
        Ok(note.clone())
    }

    /// Finish note `id`: it is no longer shown and wakes no one.
    pub fn finish(&mut self, id: &str) -> Result<Note, String> {
        let at = self
            .notes
            .iter()
            .position(|n| n.id == id.trim())
            .ok_or_else(|| self.missing(id))?;
        Ok(self.notes.remove(at))
    }

    /// The note whose look-again came first, of those due by `now`.
    pub fn next_due(&self, now: u64) -> Option<&Note> {
        self.notes
            .iter()
            .filter(|n| n.look_again_at.is_some_and(|t| t <= now))
            .min_by_key(|n| n.look_again_at)
    }

    /// Mark note `id`'s look-again at `at` as fired: its time is cleared and
    /// the note stays open. False when that note no longer has that time — it
    /// was finished, rewritten or already fired — so a time fires once.
    pub fn fire(&mut self, id: &str, at: u64) -> bool {
        let Some(note) = self
            .notes
            .iter_mut()
            .find(|n| n.id == id && n.look_again_at == Some(at))
        else {
            return false;
        };
        note.look_again_at = None;
        true
    }

    fn missing(&self, id: &str) -> String {
        format!("no open note '{}'. {}", id.trim(), self.listing())
    }

    fn full(&self) -> String {
        format!(
            "you already hold {OPEN_MAX} open notes, the most you may; finish one with note_done \
             (or rewrite one) first. {}",
            self.listing()
        )
    }

    fn listing(&self) -> String {
        if self.notes.is_empty() {
            return "You hold no open notes.".to_string();
        }
        let open: Vec<String> = self
            .notes
            .iter()
            .map(|n| format!("{}: {}", n.id, n.text))
            .collect();
        format!("Open: {}", open.join(" | "))
    }
}

/// The text as kept: trimmed, control characters but newlines made spaces,
/// within [`TEXT_MAX`].
fn checked_text(text: &str) -> Result<String, String> {
    let text: String = text
        .trim()
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect();
    let count = text.chars().count();
    if count == 0 {
        return Err("a note needs text".to_string());
    }
    if count > TEXT_MAX {
        return Err(format!(
            "that note is {count} characters; a note holds at most {TEXT_MAX} — shorten it"
        ));
    }
    Ok(text)
}

/// An agent id as a file name: letters, digits, `-` and `_` only.
fn file_stem(agent: &str) -> String {
    agent
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
