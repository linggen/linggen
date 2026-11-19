//! The spoken-line contract: what of her reply reaches her voice.

/// The spoken-line contract every engine-authored kickoff carries. A model
/// that has to reason before it answers may do so above a blank line; what
/// reaches her voice is the final paragraph alone. Stated to her, not
/// inferred — so a model that thinks in its output (deepseek-v4-flash on the
/// fallback chain, 2026-09-08) is following the rules, not leaking.
pub(crate) const SPOKEN_CONTRACT: &str =
    "If you need to think first, do it above a blank line: only your \
    final paragraph is spoken aloud, the rest is discarded.";
/// The word a reply under the contract says nothing with.
macro_rules! silent_word {
    () => {
        "SILENT"
    };
}
/// The spoken contract's word for "say nothing": a reply whose final
/// paragraph is this word is silence — never spoken, never a chat bubble.
/// The web UI mirrors it (`ui/src/lib/hiddenRows.mts`, locked by a test).
pub(crate) const SILENT: &str = silent_word!();
/// What an unasked kickoff adds: silence is an answer.
pub(super) const SILENT_CLAUSE: &str = concat!(
    "If nothing is worth saying, make that final paragraph exactly ",
    silent_word!(),
    "."
);

/// The marker on a prompt the engine wrote: kept for the model, never shown
/// as anyone's words (`doc/chat-spec.md`, hidden rows).
const HIDDEN: &str = "[HIDDEN]";

/// The kickoff as she reads it. A person's own words ("user") carry nothing;
/// an answer they asked for ("asked") carries the spoken contract alone;
/// every other engine-authored kickoff may also end in SILENT. Every
/// engine-authored kickoff is a hidden row: the engine wrote it, not the user.
pub(super) fn with_contract(task: String, trigger_source: &str) -> String {
    if trigger_source == "user" {
        return task;
    }
    let task = task.trim_start_matches(HIDDEN).trim_start();
    match trigger_source {
        "asked" => format!("{HIDDEN} {task} {SPOKEN_CONTRACT}"),
        _ => format!("{HIDDEN} {task} {SPOKEN_CONTRACT} {SILENT_CLAUSE}"),
    }
}

/// A row that is a prompt the engine wrote, not anyone's words: it carries
/// the hidden marker — or, on a row saved before her kickoffs were marked
/// (2026-10-14), the spoken contract every engine kickoff has carried since
/// 2026-09-08 and no person's message does. Kept for the model; the chat
/// never shows it.
pub(crate) fn is_engine_prompt(from_id: &str, content: &str) -> bool {
    content.contains(HIDDEN) || (from_id == "user" && content.contains(SPOKEN_CONTRACT))
}

/// What a reply to a PERSON (or to an answer they asked for) says: the final
/// paragraph, unwrapped from quotes. `None` when it is — or ends in — SILENT.
/// The answer path only: a turn the engine started is read by [`spoken_line`].
pub(crate) fn answer_line(reply: &str) -> Option<String> {
    // Her labels on a problem's offers are buttons, never words to say.
    let reply = super::offer_labels::strip(reply);
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

    // Her voice says the words, never a picture's or a source's number.
    let last = crate::engine::answer_refs::strip_ids(last);
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

    let core = line.trim_end_matches(is_punct);
    let last_word = core.split_whitespace().last().unwrap_or("");
    if is_silence(core) || last_word.trim_end_matches(is_punct) == SILENT {
        return None;
    }
    if notes > 0 {
        tracing::info!(
            "[yinyue] spoke the final paragraph; {notes} chars of notes above it discarded"
        );
    }
    Some(line)
}

fn is_punct(c: char) -> bool {
    c.is_ascii_punctuation() || c == '\u{2026}'
}

/// One paragraph that is the contract's silence: the word alone, any case,
/// trailing punctuation aside.
fn is_silence(paragraph: &str) -> bool {
    paragraph
        .trim()
        .trim_end_matches(is_punct)
        .eq_ignore_ascii_case(SILENT)
}

/// An agent's reply that is silence: its whole final paragraph is the
/// contract's [`SILENT`]. Nothing was said — the chat shows no words for it
/// (its tool calls may stay). A person's message is never one.
pub(crate) fn is_silent_reply(from_id: &str, content: &str) -> bool {
    if from_id == "user" || from_id == "system" {
        return false;
    }
    content
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .last()
        .is_some_and(is_silence)
}

/// Say her reply to a person: its spoken line, when it has one (not SILENT).
/// `at`: where she said it — whether it is heard or only read is the voice's
/// own rule (`emit_speak`).
pub(crate) fn speak_reply(
    state: &std::sync::Arc<crate::server::ServerState>,
    reply: &str,
    at: crate::server::api::yinyue::SaidAt<'_>,
) {
    if let Some(line) = answer_line(reply) {
        crate::server::api::yinyue::emit_speak(state, Line::answer(line), None, at);
    }
}

/// The most a spoken line may be, in characters: an app moment's text is cut
/// at the same 300 (`api::yinyue::EVENT_TEXT_MAX`), and a phone notice's body
/// at 320 (`push::compose`) — a line the voice is to say is shorter than both.
pub(crate) const LINE_MAX: usize = 300;

const SAY_OPEN: &str = "<say>";
const SAY_CLOSE: &str = "</say>";

/// Why a reply of an engine-started turn said nothing. Logged by name, never
/// with the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rejected {
    NoEnvelope,
    Several,
    Unclosed,
    TextAfter,
    Empty,
    LineBreak,
    TooLong,
}

impl Rejected {
    fn name(self) -> &'static str {
        match self {
            Self::NoEnvelope => "no envelope",
            Self::Several => "more than one envelope",
            Self::Unclosed => "unclosed envelope",
            Self::TextAfter => "text after the envelope",
            Self::Empty => "empty envelope",
            Self::LineBreak => "line break inside the envelope",
            Self::TooLong => "longer than a spoken line",
        }
    }
}

/// What an engine-started turn's reply says: the one `<say>…</say>` that is
/// the last thing in it. Text before it (a model thinking first) is dropped.
/// Anything else — no envelope, two, text after, empty, broken, too long — is
/// nothing. The raw inside is returned: offer labels ride in it.
fn envelope(reply: &str) -> Result<&str, Rejected> {
    let opens = reply.matches(SAY_OPEN).count();
    let closes = reply.matches(SAY_CLOSE).count();
    if opens == 0 && closes == 0 {
        return Err(Rejected::NoEnvelope);
    }
    if opens > 1 || closes > 1 {
        return Err(Rejected::Several);
    }
    let (Some(open), Some(close)) = (reply.find(SAY_OPEN), reply.find(SAY_CLOSE)) else {
        return Err(Rejected::Unclosed);
    };
    if close < open {
        return Err(Rejected::Unclosed);
    }
    if !reply[close + SAY_CLOSE.len()..].trim().is_empty() {
        return Err(Rejected::TextAfter);
    }
    Ok(reply[open + SAY_OPEN.len()..close].trim())
}

/// The line an engine-started turn's reply speaks, with her labels on the
/// offers it raised; `Err` says why it is nothing.
fn read_envelope(reply: &str) -> Result<(String, Vec<(usize, String)>), Rejected> {
    // Her labels on offers are metadata, read wherever she put them and taken
    // out before the envelope is judged.
    let (bare, labels) = super::offer_labels::take(reply);
    let raw = envelope(&bare)?;
    let line = crate::engine::answer_refs::strip_ids(raw);
    let line = line.trim();
    if line.is_empty() {
        return Err(Rejected::Empty);
    }
    if line.contains('\n') {
        return Err(Rejected::LineBreak);
    }
    if line.chars().count() > LINE_MAX {
        return Err(Rejected::TooLong);
    }
    Ok((line.to_string(), labels))
}

/// A line of hers that may be published: the one type the row, the live
/// event and the voice take (`api::yinyue::emit_speak_offering`,
/// [`publish_row`]). Made two ways only — by [`finalize_event`] from an
/// engine-started turn's envelope, or by [`Line::answer`] from a reply to a
/// person — so no other path can put words in her mouth.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Line {
    text: String,
}

impl Line {
    /// A reply to what a person said or asked for: the answer path.
    pub(crate) fn answer(text: String) -> Self {
        Self { text }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}

/// What an engine-started turn's finalizer hands back: the line she chose to
/// say, and the labels she gave the offers it raised.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Event {
    pub line: Line,
    pub labels: Vec<(usize, String)>,
}

/// The one decision for every turn the engine started: did she choose to
/// speak? `source` names the wake, for the log. Nothing else she wrote in the
/// turn is shown, kept or spoken.
pub(super) fn finalize_event(reply: Option<&str>, source: &str) -> Option<Event> {
    let reply = reply?;
    match read_envelope(reply) {
        Ok((text, labels)) => Some(Event {
            line: Line { text },
            labels,
        }),
        Err(why) => {
            tracing::info!(
                "[yinyue] {source}: nothing said ({}; {} chars dropped)",
                why.name(),
                reply.chars().count()
            );
            None
        }
    }
}

/// [`finalize_event`]'s line, bare — for the tests that only ask what is said.
#[cfg(test)]
pub(super) fn spoken_line(reply: &str) -> Option<String> {
    finalize_event(Some(reply), "test").map(|e| e.line.text)
}

#[cfg(test)]
mod tests {
    use super::{answer_line, is_engine_prompt, spoken_line, with_contract};

    #[test]
    fn an_asked_turn_is_not_offered_silence() {
        assert!(with_contract("k".into(), "event").contains("SILENT"));
        let asked = with_contract("k".into(), "asked");
        assert!(asked.contains("final paragraph is spoken"));
        assert!(!asked.contains("SILENT"));
        assert_eq!(with_contract("hi".into(), "user"), "hi");
    }

    #[test]
    fn an_engine_kickoff_is_hidden_a_persons_words_are_not() {
        assert!(with_contract("k".into(), "event").starts_with("[HIDDEN] k "));
        assert!(with_contract("k".into(), "asked").starts_with("[HIDDEN] k "));
        // Already marked: one marker, not two.
        let once = with_contract("[HIDDEN] k".into(), "event");
        assert_eq!(once.matches("[HIDDEN]").count(), 1);
        assert!(!with_contract("hi".into(), "user").contains("[HIDDEN]"));
    }

    #[test]
    fn an_old_kickoff_row_is_known_by_its_contract() {
        let kickoff = with_contract("A quiet moment".into(), "event");
        assert!(is_engine_prompt("user", &kickoff));
        // A row saved before the marker: the contract alone names it.
        let old = kickoff.trim_start_matches("[HIDDEN] ");
        assert!(is_engine_prompt("user", old));
        assert!(!is_engine_prompt("user", "hello"));
        assert!(!is_engine_prompt(
            "user",
            &with_contract("hello".into(), "user")
        ));
    }

    #[test]
    fn a_silent_reply_is_its_whole_final_paragraph() {
        use super::is_silent_reply;
        assert!(is_silent_reply("yinyue", "SILENT"));
        assert!(is_silent_reply("ling", "  SILENT.\n"));
        assert!(is_silent_reply("yinyue", "Let me think.\n\nSILENT"));
        assert!(!is_silent_reply(
            "yinyue",
            "It is quiet. SILENT is my answer?"
        ));
        assert!(!is_silent_reply("yinyue", "Silent night tonight."));
        assert!(!is_silent_reply("user", "SILENT"));
        assert!(super::SILENT_CLAUSE.ends_with("exactly SILENT."));
    }

    #[test]
    fn the_web_ui_mirrors_the_silent_word() {
        let ui = include_str!("../../../ui/src/lib/hiddenRows.mts");
        let mirror = format!("export const SILENT = '{}';", super::SILENT);
        assert!(ui.contains(&mirror), "hiddenRows.mts must declare {mirror}");
    }

    #[test]
    fn event_speech_requires_one_final_envelope() {
        for reply in [
            "A reply is ready.", "他不在，先不说。", "<say>Unclosed",
            "<say></say>", "<say>Hi</say> more", "<say>Hi\nthere</say>",
            "<say>First</say><say>Second</say>",
        ] {
            assert_eq!(spoken_line(reply), None, "{reply:?}");
        }
        assert_eq!(spoken_line("Thinking. <say>好了。</say> \n").as_deref(), Some("好了。"));
        assert_eq!(spoken_line(&format!("<say>{}</say>", "字".repeat(301))), None);
    }

    #[test]
    fn the_envelope_limit_and_offer_labels_are_metadata() {
        use super::{finalize_event, LINE_MAX};
        let at_limit = format!("<say>{}</say>", "字".repeat(LINE_MAX));
        assert!(spoken_line(&at_limit).is_some());
        assert_eq!(spoken_line("</say>x<say>"), None);
        assert_eq!(spoken_line("<say>Hi</say></say>"), None);
        // Labels ride beside the line and are never part of it.
        let e = finalize_event(Some("<say>Full. [offer 1: Open Files]</say>"), "t").unwrap();
        assert_eq!(e.line.text(), "Full.");
        assert_eq!(e.labels, vec![(1, "Open Files".to_string())]);
        // A label outside the envelope, after it, is still metadata.
        let e = finalize_event(Some("<say>Full.</say> [offer 1: Open]"), "t").unwrap();
        assert_eq!((e.line.text(), e.labels.len()), ("Full.", 1));
        // A bare reply, or SILENT, says nothing.
        assert!(finalize_event(Some("SILENT"), "t").is_none());
        assert!(finalize_event(None, "t").is_none());
    }

    #[test]
    fn a_bare_silent_is_silence() {
        assert_eq!(answer_line("SILENT"), None);
        assert_eq!(answer_line("silent."), None);
        assert_eq!(answer_line("  SILENT\n"), None);
    }

    #[test]
    fn notes_above_a_blank_line_are_discarded() {
        // The three shapes deepseek-v4-flash produced on 2026-09-08.
        let silent = "The user said just \"hi\" and ling has replied. That's a routine \
                      greeting, not something worth pinging Hanli about. Silence.\n\nSILENT";
        assert_eq!(answer_line(silent), None);

        let spoken = "They asked something real: checking on Marine Thinking's IPO progress. \
                      Hanli is still away, but this is worth knowing.\n\n\
                      \"Ling's looked into the Marine Thinking IPO — the answer's waiting for you.\"";
        assert_eq!(
            answer_line(spoken).as_deref(),
            Some("Ling's looked into the Marine Thinking IPO — the answer's waiting for you.")
        );

        // The marker glued to the last sentence still means silence.
        assert_eq!(answer_line("Routine greeting. Silence. SILENT"), None);
    }

    #[test]
    fn a_plain_line_is_spoken_as_is() {
        assert_eq!(
            answer_line("Ling's reply is ready, Hanli.").as_deref(),
            Some("Ling's reply is ready, Hanli.")
        );
        // A natural sentence that merely ends in the word is not the marker.
        assert_eq!(
            answer_line("The room has gone silent.").as_deref(),
            Some("The room has gone silent.")
        );
        // A line broken across two lines is one line.
        assert_eq!(
            answer_line("Ling's reply is ready —\nthe IPO question.").as_deref(),
            Some("Ling's reply is ready — the IPO question.")
        );
    }
}
