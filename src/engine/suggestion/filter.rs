//! Which model answers pass as a suggestion. Claude Code's filter (CLI
//! v2.1.309), with one difference: CC counts words by whitespace, so a
//! Chinese line is "one word". Here a line with CJK in it is measured by
//! its characters instead.

/// A suggestion longer than this is not something a person types.
const MAX_WORDS: usize = 12;
/// A line this long is a paragraph, whatever its words.
const MAX_CHARS: usize = 100;
/// The most characters a CJK line may have — about 12 words' worth.
const MAX_CJK_CHARS: usize = 40;

/// Quotes a model wraps its answer in; never part of the suggestion.
const QUOTES: [char; 5] = ['"', '“', '”', '\'', '`'];

/// One-word replies a person really sends.
#[rustfmt::skip]
const ONE_WORD_OK: &[&str] = &[
    "yes", "yeah", "yep", "yea", "yup", "sure", "ok", "okay", "push", "commit", "deploy", "stop",
    "continue", "check", "exit", "quit", "no",
];

/// Thanks and praise: matched anywhere in the line, like CC.
#[rustfmt::skip]
const EVALUATIVE: &[&str] = &[
    "thanks", "thank you", "looks good", "sounds good", "that works", "that worked",
    "that's all", "nice", "great", "perfect", "makes sense", "awesome", "excellent", "谢谢",
    "謝謝", "感谢", "感謝", "太好了", "完美", "看起来不错", "ありがとう", "감사합니다", "고마워",
];

/// The model's own voice, not theirs: matched at the start.
#[rustfmt::skip]
const VOICE_OPENERS: &[&str] = &[
    "let me", "i'll", "i've", "i'm", "i can", "i would", "i think", "i notice", "here's",
    "here is", "here are", "that's", "this is", "this will", "you can", "you should",
    "you could", "sure,", "of course", "certainly", "我来", "我会", "我将",
];

/// "Done" in a few languages: a report, not a request.
const DONE: &[&str] = &[
    "done",
    "完成",
    "完成了",
    "完了",
    "完了しました",
    "완료",
    "완료됨",
];

/// The model talking about the task instead of doing it (and our decline).
#[rustfmt::skip]
const META: &[&str] = &[
    "none", "nothing found", "silence", "沉默", "沈黙", "静默", "无建议", "没有建议", "沒有建議",
    "無建議", "提案なし",
];
const META_PREFIXES: &[&str] = &["nothing to suggest", "no suggestion"];
const META_PHRASES: &[&str] = &[
    "silence is",
    "stay silent",
    "stays silent",
    "staying silent",
];

const ERROR_PREFIXES: &[&str] = &[
    "api error:",
    "prompt is too long",
    "request timed out",
    "invalid api key",
    "image was too large",
];

const WRAPPERS: [(char, char); 6] = [
    ('(', ')'),
    ('[', ']'),
    ('（', '）'),
    ('［', '］'),
    ('【', '】'),
    ('〔', '〕'),
];

/// A line as each rule sees it.
struct Line<'a> {
    text: &'a str,
    lower: String,
    words: usize,
    cjk: usize,
}

type Rule = (&'static str, fn(&Line) -> bool);

/// Checked in CC's order; the first that holds names the drop.
const RULES: &[Rule] = &[
    ("empty", |l| l.text.is_empty()),
    ("done", |l| DONE.contains(&bare(&l.lower))),
    ("meta_text", meta_text),
    ("meta_wrapped", wrapped),
    ("error_message", |l| {
        starts_with_any(&l.lower, ERROR_PREFIXES)
    }),
    ("prefixed_label", prefixed_label),
    ("too_few_words", too_few_words),
    ("too_many_words", too_many_words),
    ("too_long", |l| l.text.chars().count() >= MAX_CHARS),
    ("multiple_sentences", multiple_sentences),
    ("has_formatting", |l| l.text.contains(['\n', '*'])),
    ("evaluative", |l| {
        EVALUATIVE.iter().any(|w| l.lower.contains(w))
    }),
    ("claude_voice", claude_voice),
];

/// The model's answer as a suggestion, or None when it declined or wrote
/// something no person would type.
pub(crate) fn clean(raw: &str) -> Option<String> {
    let text = raw.trim().trim_matches(QUOTES).trim();
    let line = Line {
        text,
        lower: text.to_lowercase(),
        words: text.split_whitespace().count(),
        cjk: text.chars().filter(|c| is_cjk(*c)).count(),
    };
    if let Some((rule, _)) = RULES.iter().find(|(_, holds)| holds(&line)) {
        tracing::debug!("next-prompt suggestion dropped ({rule}): {text:?}");
        return None;
    }
    Some(text.to_string())
}

fn bare(lower: &str) -> &str {
    lower.trim_matches(|c: char| !c.is_alphanumeric())
}

fn starts_with_any(lower: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| lower.starts_with(p))
}

fn meta_text(l: &Line) -> bool {
    META.contains(&bare(&l.lower))
        || starts_with_any(&l.lower, META_PREFIXES)
        || META_PHRASES.iter().any(|p| l.lower.contains(p))
}

fn wrapped(l: &Line) -> bool {
    WRAPPERS
        .iter()
        .any(|(open, close)| l.text.starts_with(*open) && l.text.ends_with(*close))
}

/// `word: …` — a label the model put before its answer.
fn prefixed_label(l: &Line) -> bool {
    let Some((label, rest)) = l.text.split_once(':') else {
        return false;
    };
    !label.is_empty()
        && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && rest.starts_with(char::is_whitespace)
}

/// One word is too few, unless it is a reply people send or a command.
/// A CJK line needs two CJK characters instead.
fn too_few_words(l: &Line) -> bool {
    if l.words >= 2 || l.text.starts_with('/') {
        return false;
    }
    if l.cjk > 0 {
        return l.cjk < 2;
    }
    !ONE_WORD_OK.contains(&l.lower.as_str())
}

fn too_many_words(l: &Line) -> bool {
    if l.cjk > 0 {
        return l.text.chars().count() > MAX_CJK_CHARS;
    }
    l.words > MAX_WORDS
}

/// A sentence end followed by another sentence: `. N…` or `。下…`.
fn multiple_sentences(l: &Line) -> bool {
    let chars: Vec<char> = l.text.chars().collect();
    chars.iter().enumerate().any(|(i, c)| {
        let rest = &chars[i + 1..];
        let next = rest.iter().find(|n| !n.is_whitespace());
        let spaced = rest.first().is_some_and(|n| n.is_whitespace());
        match c {
            '.' | '!' | '?' => spaced && next.is_some_and(char::is_ascii_uppercase),
            '。' | '！' | '？' => next.is_some_and(|n| n.is_alphanumeric()),
            _ => false,
        }
    })
}

fn claude_voice(l: &Line) -> bool {
    let let_me = l.text.starts_with("让我") && !l.text.starts_with("让我们");
    let_me || starts_with_any(&l.lower, VOICE_OPENERS)
}

/// Han, kana and Hangul: scripts written without spaces between words.
fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3040}'..='\u{30FF}'   // Hiragana, Katakana
        | '\u{3400}'..='\u{4DBF}' // CJK Extension A
        | '\u{4E00}'..='\u{9FFF}' // CJK Unified Ideographs
        | '\u{F900}'..='\u{FAFF}' // CJK Compatibility Ideographs
        | '\u{1100}'..='\u{11FF}' // Hangul Jamo
        | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
        | '\u{20000}'..='\u{2A6DF}' // CJK Extension B
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(s: &str) -> bool {
        clean(s).is_some()
    }

    #[test]
    fn keeps_a_short_line_in_their_voice() {
        assert_eq!(
            clean("  \"trim half of NVDA\"\n"),
            Some("trim half of NVDA".to_string())
        );
        assert!(kept("run the tests"));
        assert!(kept("/commit"), "a command is one word people type");
    }

    #[test]
    fn keeps_the_one_word_replies_people_send() {
        for word in ["yes", "Yes", "go", "continue", "ok"] {
            assert_eq!(kept(word), word != "go", "{word}");
        }
    }

    #[test]
    fn drops_empty_done_and_meta() {
        for s in [
            "",
            "  ",
            "Done.",
            "完成了",
            "None.",
            "nothing found",
            "No suggestion this time",
            "I will stay silent",
            "沉默",
        ] {
            assert!(!kept(s), "{s:?}");
        }
    }

    #[test]
    fn drops_a_wrapped_line() {
        for s in ["(none)", "[no suggestion]", "（无）", "【继续】"] {
            assert!(!kept(s), "{s:?}");
        }
    }

    #[test]
    fn drops_errors_and_labels() {
        assert!(!kept("API Error: overloaded"));
        assert!(!kept("Suggestion: run the tests"));
        assert!(
            kept("open https://example.com"),
            "a colon in a URL is no label"
        );
    }

    #[test]
    fn drops_too_few_or_too_many_words() {
        assert!(!kept("maybe"));
        assert!(!kept("a b c d e f g h i j k l m"));
        assert!(kept("a b c d e f g h i j k l"));
        assert!(!kept(&"x".repeat(MAX_CHARS)), "100 characters is too long");
    }

    #[test]
    fn drops_more_than_one_sentence() {
        assert!(!kept("Run the tests. Then push it"));
        assert!(!kept("先跑测试。然后推送"));
        assert!(
            kept("bump v1.2 and push"),
            "a dot inside a word is no sentence end"
        );
    }

    #[test]
    fn drops_newlines_and_formatting() {
        assert!(!kept("one\ntwo"));
        assert!(!kept("run **all** tests"));
    }

    #[test]
    fn drops_thanks_and_praise_in_any_case() {
        for s in [
            "Thanks, that works",
            "looks GOOD to me",
            "Perfect, ship it",
            "谢谢你",
        ] {
            assert!(!kept(s), "{s:?}");
        }
    }

    #[test]
    fn drops_the_models_own_voice_in_any_case() {
        for s in [
            "Let me check the numbers",
            "let me check the numbers",
            "I'll run the tests",
            "Here's the plan for it",
            "Sure, run the tests",
            "让我看看数据",
            "我来跑一下测试",
        ] {
            assert!(!kept(s), "{s:?}");
        }
        assert!(kept("让我们看看特斯拉"), "让我们 is their voice too");
    }

    #[test]
    fn measures_cjk_by_characters() {
        assert_eq!(clean("那特斯拉呢？"), Some("那特斯拉呢？".to_string()));
        assert!(kept("继续"), "two characters are enough");
        assert!(!kept("好"), "one character is too few");
        assert!(kept("看看 NVDA 的数据"));
        assert!(kept(&"测".repeat(MAX_CJK_CHARS)));
        assert!(!kept(&"测".repeat(MAX_CJK_CHARS + 1)), "too long to type");
    }
}
