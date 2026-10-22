//! What Yinyue's voice is handed: the spoken copy of a line, cut into short
//! clauses, each with the pause that follows it and the language to read it in.
//!
//! Why clauses: Qwen3-TTS samples, and on a long line it drifts at the tail —
//! slower, wavering, "像日语" (Hanli, 2026-10-22, on a 56-char line). Short
//! clauses (≤ [`MAX_WEIGHT`]) rarely drift, and the first one is ready in
//! about a second, so the surface can start playing while the rest are made.
//!
//! The bubble always shows the original text; only the spoken copy changes.

use serde::Serialize;

/// One clause of a line: what to synthesize, then how long to pause.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct SpokenChunk {
    pub text: String,
    pub pause_ms: u32,
}

/// The longest clause, counted as Han characters plus Latin words. Measured
/// 2026-10-22: ≤18-char clauses ran at normal speed, whole 50+ char lines
/// drifted in about one take in five.
pub const MAX_WEIGHT: usize = 18;

/// A sentence this light joins the next one instead of standing alone —
/// a three-character clause is too short to read with natural prosody.
const MIN_SENTENCE_WEIGHT: usize = 5;

/// Marks a sentence ends on — always a place to cut.
const SENTENCE_ENDS: &[char] = &['。', '！', '？', '；', '!', '?', ';'];

/// Marks a clause may be cut at when its sentence is too long.
const CLAUSE_MARKS: &[char] = &['，', '、', '：', ',', ':'];

/// Quotation marks are never spoken.
const QUOTES: &[char] = &['“', '”', '「', '」', '『', '』', '"'];

/// Dashes and ellipses read as a pause, never as sounds.
const PAUSE_MARKS: &[char] = &['—', '…', '⋯'];

/// The pause after a clause, by its last mark.
fn pause_after(chunk: &str) -> u32 {
    const PAUSES: &[(&[char], u32)] = &[
        (&['。', '！', '？', '.', '!', '?'], 250),
        (&['；', ';'], 200),
        (&['：', ':'], 180),
        (&['，', ','], 150),
        (&['、'], 120),
    ];
    let last = chunk.chars().last().unwrap_or(' ');
    PAUSES
        .iter()
        .find(|(marks, _)| marks.contains(&last))
        .map_or(150, |(_, ms)| *ms)
}

pub fn is_han(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF   // CJK Unified Ideographs
        | 0x3400..=0x4DBF // Extension A
        | 0xF900..=0xFAFF // Compatibility Ideographs
    )
}

pub fn han_count(text: &str) -> usize {
    text.chars().filter(|&c| is_han(c)).count()
}

/// Latin words of two letters or more ("a" and "I" alone don't make a line
/// English).
fn latin_words(text: &str) -> usize {
    text.split(|c: char| !c.is_ascii_alphabetic())
        .filter(|w| w.len() >= 2)
        .count()
}

fn weight(text: &str) -> usize {
    han_count(text) + latin_words(text)
}

/// The line as it should be spoken: no quotes, no leading 「……」, and every
/// dash or ellipsis run inside the line turned into a comma pause.
pub fn spoken_copy(text: &str) -> String {
    let unquoted: String = text.chars().filter(|c| !QUOTES.contains(c)).collect();
    let comma = if han_count(&unquoted) > 0 {
        "，"
    } else {
        ", "
    };
    let body = unquoted.trim_start_matches(|c: char| {
        PAUSE_MARKS.contains(&c) || c == '.' || c == '·' || c.is_whitespace()
    });
    let paused = replace_pause_runs(body, comma);
    tidy_commas(&paused)
}

/// Each run of dashes/ellipses becomes one comma, with the spaces around
/// the run folded into it ("day — I" → "day, I").
fn replace_pause_runs(text: &str, comma: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut after_run = false;
    for c in text.chars() {
        if PAUSE_MARKS.contains(&c) {
            if !after_run {
                out.truncate(out.trim_end().len());
                out.push_str(comma.trim_end());
            }
            after_run = true;
            continue;
        }
        if after_run && c.is_whitespace() {
            continue;
        }
        if after_run && comma.ends_with(' ') {
            out.push(' ');
        }
        out.push(c);
        after_run = false;
    }
    out
}

/// A pause next to other punctuation collapses into it: "，，" → "，",
/// "，。" → "。", "。，" → "。"; and none to open the line. A trailing one
/// stays — a clause the engine cut at a comma must read back unchanged.
fn tidy_commas(text: &str) -> String {
    let is_comma = |c: char| c == '，' || c == ',';
    let is_mark = |c: char| SENTENCE_ENDS.contains(&c) || CLAUSE_MARKS.contains(&c) || c == '.';
    let mut out: Vec<char> = Vec::with_capacity(text.len());
    for c in text.chars() {
        let prev = out.iter().rev().find(|c| !c.is_whitespace()).copied();
        match (prev, c) {
            (Some(p), c) if is_comma(c) && is_mark(p) => continue,
            (Some(p), c) if is_comma(p) && is_mark(c) => {
                while out
                    .last()
                    .is_some_and(|x| x.is_whitespace() || is_comma(*x))
                {
                    out.pop();
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    let s: String = out.into_iter().collect();
    s.trim_start_matches(|c: char| is_comma(c) || c.is_whitespace())
        .trim_end()
        .to_string()
}

/// A piece ends after any mark; an ASCII '.' only when a space or the end
/// follows it (so "3.5" and "e.g" stay whole).
fn pieces(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        cur.push(c);
        let next_is_gap = chars.get(i + 1).is_none_or(|n| n.is_whitespace());
        let ends =
            SENTENCE_ENDS.contains(&c) || CLAUSE_MARKS.contains(&c) || (c == '.' && next_is_gap);
        if ends {
            out.push(std::mem::take(&mut cur));
        }
    }
    out.push(cur);
    out.into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

fn ends_sentence(piece: &str) -> bool {
    piece
        .chars()
        .last()
        .is_some_and(|c| SENTENCE_ENDS.contains(&c) || c == '.')
}

/// Greedy packing: pieces join until the next one would pass [`MAX_WEIGHT`];
/// a finished sentence closes its chunk once it is heavy enough to stand.
fn pack(pieces: Vec<String>) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut cur = String::new();
    for p in pieces {
        if !cur.is_empty() && weight(&cur) + weight(&p) > MAX_WEIGHT {
            chunks.push(std::mem::take(&mut cur));
        }
        join(&mut cur, &p);
        if ends_sentence(&cur) && weight(&cur) >= MIN_SENTENCE_WEIGHT {
            chunks.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks
}

/// English pieces need their space back; Chinese ones join bare.
fn join(cur: &mut String, piece: &str) {
    let latin_seam = cur.chars().last().is_some_and(|c| c.is_ascii_punctuation())
        && piece
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric());
    if latin_seam {
        cur.push(' ');
    }
    cur.push_str(piece);
}

/// The line, ready to speak: its spoken copy in clauses, each with the pause
/// after it (none after the last). Never empty for a non-blank line.
pub fn chunks(text: &str) -> Vec<SpokenChunk> {
    let parts = pack(pieces(&spoken_copy(text)));
    let last = parts.len().saturating_sub(1);
    parts
        .into_iter()
        .enumerate()
        .map(|(i, text)| SpokenChunk {
            pause_ms: if i == last { 0 } else { pause_after(&text) },
            text,
        })
        .collect()
}

/// The sidecar `lang` for one clause. The clause's own script decides when
/// it is unambiguous — Han with no Latin words is "chinese", Latin with no
/// Han is "english" (vivian reads pure English with a Chinese accent under
/// "auto") — because forcing one language onto the other script garbles it.
/// A mixed clause follows the global language setting (`language::setting`:
/// en → "english", zh → "chinese"); under Auto it stays "auto", which read
/// mixed zh/en lines well (2026-10-07).
pub fn language_for(chunk: &str, setting: Option<&str>) -> &'static str {
    let script = (han_count(chunk) > 0, latin_words(chunk) > 0);
    match script {
        (true, false) => "chinese",
        (false, true) => "english",
        _ => language_of_setting(setting),
    }
}

fn language_of_setting(setting: Option<&str>) -> &'static str {
    const BY_PREFIX: &[(&str, &str)] = &[("zh", "chinese"), ("en", "english")];
    let tag = setting.unwrap_or("").to_ascii_lowercase();
    BY_PREFIX
        .iter()
        .find(|(prefix, _)| tag.starts_with(prefix))
        .map_or("auto", |(_, lang)| lang)
}

// ---------------------------------------------------------------------------
// Rate check — did the take drift?
// ---------------------------------------------------------------------------

/// Slower than this (Han characters per second of voiced audio) is a drifted
/// take. The clips Hanli heard as garbled ran 3.5 and 4.1; healthy clauses
/// ran 4.2–7.3 (bf16, 2026-10-22, 72 takes).
pub const MIN_HAN_PER_SEC: f32 = 4.0;

/// Faster than this means the take was cut short.
pub const MAX_HAN_PER_SEC: f32 = 10.0;

/// Clauses with fewer Han characters are not judged: onset and the final
/// syllable's length dominate their rate (「一张饼？」 ran 2.3–3.9 when fine).
pub const MIN_HAN_TO_JUDGE: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Take {
    /// Not judged: too short, or no Han to count (English clauses).
    Unjudged,
    Good,
    /// Outside the healthy band by this many Han/s.
    Off(f32),
}

impl Take {
    /// How far outside the band — 0 for a take that passes.
    pub fn shortfall(self) -> f32 {
        match self {
            Take::Off(by) => by,
            _ => 0.0,
        }
    }

    pub fn passes(self) -> bool {
        !matches!(self, Take::Off(_))
    }
}

/// Judge one take of `chunk` that has `voiced_secs` of voiced audio.
pub fn judge(chunk: &str, voiced_secs: f32) -> Take {
    let han = han_count(chunk);
    if han < MIN_HAN_TO_JUDGE || voiced_secs <= 0.0 {
        return Take::Unjudged;
    }
    let rate = han as f32 / voiced_secs;
    let by = (MIN_HAN_PER_SEC - rate).max(rate - MAX_HAN_PER_SEC);
    if by > 0.0 {
        Take::Off(by)
    } else {
        Take::Good
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(line: &str) -> Vec<String> {
        chunks(line).into_iter().map(|c| c.text).collect()
    }

    #[test]
    fn spoken_copy_drops_a_leading_ellipsis_and_quotes() {
        assert_eq!(spoken_copy("……山洞里剩下的饼角。"), "山洞里剩下的饼角。");
        assert_eq!(spoken_copy("…你好"), "你好");
        assert_eq!(spoken_copy("“凡人倒会记账。”"), "凡人倒会记账。");
    }

    #[test]
    fn mid_line_dashes_and_ellipses_become_one_pause() {
        assert_eq!(spoken_copy("至于整张饼——你若真敢"), "至于整张饼，你若真敢");
        assert_eq!(spoken_copy("硌牙——罢了"), "硌牙，罢了");
        assert_eq!(spoken_copy("饼角……本王"), "饼角，本王");
        // Next to other punctuation the pause folds into it.
        assert_eq!(spoken_copy("饼角……。"), "饼角。");
        assert_eq!(spoken_copy("饼角。——本王"), "饼角。本王");
        assert_eq!(
            spoken_copy("Long day — I've set Ling"),
            "Long day, I've set Ling"
        );
    }

    #[test]
    fn short_lines_stay_one_chunk_with_no_pause() {
        let c = chunks("找个暖和的地方，本王还你。");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].pause_ms, 0);
    }

    #[test]
    fn long_lines_split_into_light_clauses() {
        let line = "……你倒把旧账记得清楚。那件打补丁的衣裳裹得本王一身苔味，饼也冷硬得硌牙——罢了，本王欠你一张饼。找个暖和的地方，本王还你。";
        let got = texts(line);
        assert_eq!(
            got,
            [
                "你倒把旧账记得清楚。",
                "那件打补丁的衣裳裹得本王一身苔味，",
                "饼也冷硬得硌牙，罢了，本王欠你一张饼。",
                "找个暖和的地方，本王还你。",
            ]
        );
        assert!(got.iter().all(|c| weight(c) <= MAX_WEIGHT));
    }

    #[test]
    fn a_tiny_sentence_joins_the_next() {
        assert_eq!(
            texts("一张饼？你倒会趁本王刚醒，先替自己讨封赏。那饼角是本王替你验过了，火候尚可。"),
            [
                "一张饼？你倒会趁本王刚醒，先替自己讨封赏。",
                "那饼角是本王替你验过了，火候尚可。"
            ]
        );
    }

    #[test]
    fn pauses_follow_the_mark() {
        let c = chunks("那件打补丁的衣裳裹得本王一身苔味，饼也冷硬得硌牙，罢了，本王欠你一张饼。找个暖和的地方，本王还你。");
        assert_eq!(
            c.iter().map(|c| c.pause_ms).collect::<Vec<_>>(),
            [150, 250, 0]
        );
    }

    #[test]
    fn chunking_a_chunk_is_a_no_op() {
        // The surface sends each chunk back to /api/tts, which chunks again.
        let line = "凡人，光扯着嗓子可算不得凭据。把昨夜那只裹狐狸的补丁外衣、虎口上的牙印，还有你守了一夜的火都摆出来——本王再听听。";
        for c in chunks(line) {
            assert_eq!(texts(&c.text), [c.text.clone()]);
        }
    }

    #[test]
    fn english_splits_at_sentence_ends_and_keeps_its_spaces() {
        assert_eq!(
            texts("Alex, your Mac’s disk is getting tight; I’ve asked Ling to find safe cleanup options without deleting anything first."),
            [
                "Alex, your Mac’s disk is getting tight;",
                "I’ve asked Ling to find safe cleanup options without deleting anything first.",
            ]
        );
        // A tiny sentence joins the next, as in Chinese.
        assert_eq!(
            texts("Hi, I’m here. See you soon."),
            ["Hi, I’m here. See you soon."]
        );
        assert_eq!(
            texts("It costs 3.5 GB of disk."),
            ["It costs 3.5 GB of disk."]
        );
    }

    #[test]
    fn the_script_decides_when_it_is_unambiguous() {
        for setting in [None, Some("en"), Some("zh-Hans")] {
            assert_eq!(language_for("师父，我在这里。", setting), "chinese");
            assert_eq!(language_for("Master, I am here.", setting), "english");
        }
    }

    #[test]
    fn a_mixed_clause_follows_the_setting() {
        let mixed = "你今天十点和 Alex 有个 meeting";
        assert_eq!(language_for(mixed, None), "auto");
        assert_eq!(language_for(mixed, Some("en")), "english");
        assert_eq!(language_for(mixed, Some("en-US")), "english");
        assert_eq!(language_for(mixed, Some("zh-Hans")), "chinese");
        assert_eq!(language_for(mixed, Some("ja")), "auto");
    }

    #[test]
    fn nothing_to_judge_follows_the_setting() {
        assert_eq!(language_for("🎉 123 !", None), "auto");
        assert_eq!(language_for("a 1", Some("zh")), "chinese");
    }

    #[test]
    fn the_rate_check_flags_drift_and_truncation() {
        let chunk = "那件打补丁的衣裳裹得本王一身苔味，"; // 16 Han
        assert_eq!(judge(chunk, 3.0), Take::Good); // 5.3/s
        assert!(!judge(chunk, 6.0).passes()); // 2.7/s — drifted
        assert!(!judge(chunk, 1.0).passes()); // 16/s — cut short
        assert!(judge(chunk, 6.0).shortfall() > judge(chunk, 4.5).shortfall());
    }

    #[test]
    fn short_and_english_clauses_are_not_judged() {
        assert_eq!(judge("一张饼？", 9.0), Take::Unjudged);
        assert_eq!(judge("Master, I am here.", 9.0), Take::Unjudged);
        assert!(Take::Unjudged.passes());
    }
}
