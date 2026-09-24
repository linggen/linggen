//! The spoken-line contract: what of her reply reaches her voice.

/// The spoken-line contract every engine-authored kickoff carries. A model
/// that has to reason before it answers may do so above a blank line; what
/// reaches her voice is the final paragraph alone. Stated to her, not
/// inferred — so a model that thinks in its output (deepseek-v4-flash on the
/// fallback chain, 2026-09-08) is following the rules, not leaking.
pub(super) const SPOKEN_CONTRACT: &str =
    "If you need to think first, do it above a blank line: only your \
    final paragraph is spoken aloud, the rest is discarded.";
/// What an unasked kickoff adds: silence is an answer.
pub(super) const SILENT_CLAUSE: &str =
    "If nothing is worth saying, make that final paragraph exactly SILENT.";

/// The kickoff as she reads it. A person's own words ("user") carry nothing;
/// an answer they asked for ("asked") carries the spoken contract alone;
/// every other engine-authored kickoff may also end in SILENT.
pub(super) fn with_contract(task: String, trigger_source: &str) -> String {
    match trigger_source {
        "user" => task,
        "asked" => format!("{task} {SPOKEN_CONTRACT}"),
        _ => format!("{task} {SPOKEN_CONTRACT} {SILENT_CLAUSE}"),
    }
}

/// What a reply under [`SPOKEN_CONTRACT`] actually says: the final paragraph,
/// unwrapped from quotes. `None` when it is — or ends in — SILENT.
pub(crate) fn spoken_line(reply: &str) -> Option<String> {
    let paragraphs: Vec<&str> = reply
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    let last = paragraphs.last()?;
    let notes: usize = paragraphs[..paragraphs.len() - 1]
        .iter()
        .map(|p| p.len())
        .sum();

    let mut line = last
        .split('\n')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ");
    for (open, close) in [
        ('"', '"'),
        ('\u{201c}', '\u{201d}'),
        ('\u{2018}', '\u{2019}'),
        ('\u{300c}', '\u{300d}'),
    ] {
        if line.starts_with(open) && line.ends_with(close) && line.chars().count() > 1 {
            line = line[open.len_utf8()..line.len() - close.len_utf8()]
                .trim()
                .to_string();
        }
    }

    let is_punct = |c: char| c.is_ascii_punctuation() || c == '\u{2026}';
    let core = line.trim_end_matches(is_punct);
    let last_word = core.split_whitespace().last().unwrap_or("");
    if core.eq_ignore_ascii_case("silent") || last_word.trim_end_matches(is_punct) == "SILENT" {
        return None;
    }
    if notes > 0 {
        tracing::info!(
            "[yinyue] spoke the final paragraph; {notes} chars of notes above it discarded"
        );
    }
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::{spoken_line, with_contract};

    #[test]
    fn an_asked_turn_is_not_offered_silence() {
        assert!(with_contract("k".into(), "event").contains("SILENT"));
        let asked = with_contract("k".into(), "asked");
        assert!(asked.contains("final paragraph is spoken"));
        assert!(!asked.contains("SILENT"));
        assert_eq!(with_contract("hi".into(), "user"), "hi");
    }

    #[test]
    fn a_bare_silent_is_silence() {
        assert_eq!(spoken_line("SILENT"), None);
        assert_eq!(spoken_line("silent."), None);
        assert_eq!(spoken_line("  SILENT\n"), None);
    }

    #[test]
    fn notes_above_a_blank_line_are_discarded() {
        // The three shapes deepseek-v4-flash produced on 2026-09-08.
        let silent = "The user said just \"hi\" and ling has replied. That's a routine \
                      greeting, not something worth pinging Hanli about. Silence.\n\nSILENT";
        assert_eq!(spoken_line(silent), None);

        let spoken = "They asked something real: checking on Marine Thinking's IPO progress. \
                      Hanli is still away, but this is worth knowing.\n\n\
                      \"Ling's looked into the Marine Thinking IPO — the answer's waiting for you.\"";
        assert_eq!(
            spoken_line(spoken).as_deref(),
            Some("Ling's looked into the Marine Thinking IPO — the answer's waiting for you.")
        );

        // The marker glued to the last sentence still means silence.
        assert_eq!(spoken_line("Routine greeting. Silence. SILENT"), None);
    }

    #[test]
    fn a_plain_line_is_spoken_as_is() {
        assert_eq!(
            spoken_line("Ling's reply is ready, Hanli.").as_deref(),
            Some("Ling's reply is ready, Hanli.")
        );
        // A natural sentence that merely ends in the word is not the marker.
        assert_eq!(
            spoken_line("The room has gone silent.").as_deref(),
            Some("The room has gone silent.")
        );
        // A line broken across two lines is one line.
        assert_eq!(
            spoken_line("Ling's reply is ready —\nthe IPO question.").as_deref(),
            Some("Ling's reply is ready — the IPO question.")
        );
    }
}
