//! Follow-up buttons: the next questions a reply offers the person.
//!
//! A surface that renders them asks on its turn; the model then ends its reply
//! with a `<followups>` block. The block never reaches the person as text —
//! [`StreamGate`] holds it back while tokens stream and [`split`] lifts it off
//! the finished reply. See `doc/chat-spec.md` § Suggestions.

const OPEN: &str = "<followups>";
const CLOSE: &str = "</followups>";

/// At most this many buttons from one reply.
const MAX_ITEMS: usize = 3;
/// A button longer than this is a sentence, not a button — dropped.
const MAX_CHARS: usize = 80;

/// The reply without its follow-ups block, and the block's items.
pub fn split(text: &str) -> (String, Vec<String>) {
    let Some(open) = text.rfind(OPEN) else {
        return (text.to_string(), Vec::new());
    };
    let body_start = open + OPEN.len();
    let (body, rest) = match text[body_start..].find(CLOSE) {
        Some(close) => (
            &text[body_start..body_start + close],
            &text[body_start + close + CLOSE.len()..],
        ),
        None => (&text[body_start..], ""),
    };
    let mut reply = text[..open].trim_end().to_string();
    let rest = rest.trim();
    if !rest.is_empty() {
        reply.push_str("\n\n");
        reply.push_str(rest);
    }
    (reply, items(body))
}

fn items(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in body.lines() {
        let item = clean(line);
        if item.is_empty() || item.chars().count() > MAX_CHARS || out.contains(&item) {
            continue;
        }
        out.push(item);
        if out.len() == MAX_ITEMS {
            break;
        }
    }
    out
}

/// A list line as the words on a button: no bullet, number or quotes.
fn clean(line: &str) -> String {
    let line = line.trim();
    let line = line.trim_start_matches(['-', '*', '•']);
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let numbered = [". ", ") "].iter().any(|m| line[digits..].starts_with(m));
    let line = if digits > 0 && numbered {
        &line[digits + 2..]
    } else {
        line
    };
    line.trim().trim_matches(['"', '“', '”']).trim().to_string()
}

/// Streams a reply's text but never its follow-ups block. Text that could be
/// the start of the block is held until the next token settles it.
#[derive(Default)]
pub struct StreamGate {
    held: String,
    inside: bool,
}

impl StreamGate {
    /// The part of `token` safe to show now.
    pub fn push(&mut self, token: &str) -> String {
        if self.inside {
            return String::new();
        }
        self.held.push_str(token);
        if let Some(open) = self.held.find(OPEN) {
            let shown = self.held[..open].to_string();
            self.held.clear();
            self.inside = true;
            return shown;
        }
        let cut = self.held.len() - partial_open_len(&self.held);
        self.held.drain(..cut).collect()
    }

    /// What was held back and turned out to be ordinary text.
    pub fn finish(&mut self) -> String {
        if self.inside {
            return String::new();
        }
        std::mem::take(&mut self.held)
    }
}

/// How many trailing bytes of `text` could begin the block's opening tag.
fn partial_open_len(text: &str) -> usize {
    (1..OPEN.len())
        .rev()
        .find(|&n| text.ends_with(&OPEN[..n]))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifts_the_block_and_cleans_each_button() {
        let reply = "Your portfolio is up 3%.\n\n<followups>\n- Review my portfolio\n2) \"Should I buy NVDA?\"\n* \n- Review my portfolio\n- What changed this week?\n- One too many\n</followups>";
        let (text, items) = split(reply);
        assert_eq!(text, "Your portfolio is up 3%.");
        assert_eq!(
            items,
            vec![
                "Review my portfolio",
                "Should I buy NVDA?",
                "What changed this week?"
            ]
        );
    }

    #[test]
    fn a_reply_without_the_block_is_untouched() {
        let (text, items) = split("Plain answer.");
        assert_eq!(text, "Plain answer.");
        assert!(items.is_empty());
    }

    #[test]
    fn an_unclosed_block_still_comes_off() {
        let (text, items) = split("Done.\n<followups>\n- Show the details");
        assert_eq!(text, "Done.");
        assert_eq!(items, vec!["Show the details"]);
    }

    #[test]
    fn a_sentence_is_not_a_button() {
        let long = "x".repeat(MAX_CHARS + 1);
        let (_, items) = split(&format!(
            "Hi\n<followups>\n- {long}\n- Short one\n- 3.5% or more?\n</followups>"
        ));
        assert_eq!(items, vec!["Short one", "3.5% or more?"]);
    }

    #[test]
    fn the_stream_never_shows_the_block_even_split_across_tokens() {
        let mut gate = StreamGate::default();
        let tokens = [
            "Up 3%",
            ".\n\n<fol",
            "lowups>\n- Review",
            " my portfolio\n</followups>",
        ];
        let shown: String =
            tokens.iter().map(|t| gate.push(t)).collect::<String>() + &gate.finish();
        assert_eq!(shown, "Up 3%.\n\n");
    }

    #[test]
    fn held_text_that_was_not_the_block_is_released() {
        let mut gate = StreamGate::default();
        let mut shown = gate.push("a <fo");
        assert_eq!(shown, "a ");
        shown.push_str(&gate.push("o> b <"));
        shown.push_str(&gate.finish());
        assert_eq!(shown, "a <foo> b <");
    }
}
