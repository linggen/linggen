//! An agent's open notes as it reads them at the start of a turn: one line
//! each, with its age and its look-again time, within [`SHOWN_MAX_CHARS`].

use super::{Note, SHOWN_MAX_CHARS};
use chrono::{DateTime, Duration, TimeZone, Utc};

/// The lines for `notes`, read at `now` in `now`'s zone on `host`; `None`
/// when there are none. Notes past the character cap are counted, not shown.
pub fn lines<Tz: TimeZone>(notes: &[Note], now: &DateTime<Tz>, host: &str) -> Option<String> {
    if notes.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (i, note) in notes.iter().enumerate() {
        let line = line(note, now, host);
        if out.chars().count() + line.chars().count() > SHOWN_MAX_CHARS {
            out.push_str(&format!("- ({} more not shown)", notes.len() - i));
            return Some(out);
        }
        out.push_str(&line);
        out.push('\n');
    }
    Some(out.trim_end().to_string())
}

/// `- n3 · written 2h ago · look again 18:00 today: the parcel arrives tomorrow`
fn line<Tz: TimeZone>(note: &Note, now: &DateTime<Tz>, host: &str) -> String {
    let age = now.timestamp() - note.written_at as i64;
    let mut head = format!(
        "- {} · written {}",
        note.id,
        crate::perception::activity::ago(age)
    );
    if note.device != host && !note.device.is_empty() {
        head.push_str(&format!(" on {}", note.device));
    }
    if let Some(at) = note.look_again_at {
        head.push_str(&format!(" · look again {}", when(at, now)));
    }
    format!("{head}: {}", note.text.replace('\n', " / "))
}

/// A look-again time as a reader in `now`'s zone says it.
fn when<Tz: TimeZone>(at: u64, now: &DateTime<Tz>) -> String {
    let Some(t) = DateTime::<Utc>::from_timestamp(at as i64, 0) else {
        return "at an unreadable time".to_string();
    };
    let local = t.with_timezone(&now.timezone()).naive_local();
    let day = local.date();
    let said = if day == now.date_naive() {
        format!("{} today", local.format("%H:%M"))
    } else if day == now.date_naive() + Duration::days(1) {
        format!("tomorrow {}", local.format("%H:%M"))
    } else {
        local.format("%Y-%m-%d %H:%M").to_string()
    };
    if t.timestamp() <= now.timestamp() {
        return format!("{said} (passed)");
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn now() -> DateTime<FixedOffset> {
        // 23:30 local at UTC+9.
        FixedOffset::east_opt(9 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 11, 2, 23, 30, 0)
            .unwrap()
    }

    fn note(id: &str, text: &str, ago: i64, look: Option<i64>) -> Note {
        let t = now().timestamp();
        Note {
            id: id.into(),
            text: text.into(),
            written_at: (t - ago) as u64,
            device: "test-mac".into(),
            look_again_at: look.map(|s| (t + s) as u64),
        }
    }

    #[test]
    fn nothing_is_shown_without_notes() {
        assert_eq!(lines(&[], &now(), "test-mac"), None);
    }

    #[test]
    fn each_note_shows_its_age_and_look_again_in_local_time() {
        let notes = [
            note("n1", "the parcel arrives tomorrow", 7200, Some(1200)),
            note("n2", "two lines\nof it", 120, Some(3600)),
            note("n3", "no time", 3 * 86_400, None),
            note("n4", "missed", 600, Some(-60)),
        ];
        let shown = lines(&notes, &now(), "test-mac").unwrap();
        assert_eq!(
            shown,
            "- n1 · written 2h ago · look again 23:50 today: the parcel arrives tomorrow\n\
             - n2 · written 2m ago · look again tomorrow 00:30: two lines / of it\n\
             - n3 · written 3d ago: no time\n\
             - n4 · written 10m ago · look again 23:29 today (passed): missed"
        );
    }

    #[test]
    fn a_note_from_another_device_names_it() {
        let mut n = note("n1", "x", 60, None);
        n.device = "Alex's iPhone".into();
        assert!(lines(&[n], &now(), "test-mac")
            .unwrap()
            .contains("written 1m ago on Alex's iPhone"));
    }

    #[test]
    fn notes_past_the_cap_are_counted_not_shown() {
        let notes: Vec<Note> = (1..=40)
            .map(|i| {
                note(
                    &format!("n{i}"),
                    &"z".repeat(crate::notes::TEXT_MAX),
                    60,
                    None,
                )
            })
            .collect();
        let shown = lines(&notes, &now(), "test-mac").unwrap();
        assert!(
            shown.chars().count() <= SHOWN_MAX_CHARS + 40,
            "{}",
            shown.len()
        );
        assert!(shown.ends_with("more not shown)"), "{shown}");
    }

    /// The limits keep a full book inside the cap, so the cap is a backstop.
    #[test]
    fn a_full_book_fits_the_cap() {
        let notes: Vec<Note> = (1..=crate::notes::OPEN_MAX)
            .map(|i| {
                note(
                    &format!("n{i}"),
                    &"z".repeat(crate::notes::TEXT_MAX),
                    60,
                    Some(600),
                )
            })
            .collect();
        assert!(!lines(&notes, &now(), "test-mac")
            .unwrap()
            .contains("not shown"));
    }
}
