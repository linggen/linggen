//! Reading a note and searching: by id, by path, by words — for one
//! audience ([`super::fence`]). A note that is read or found counts a use
//! (`.state/uses.jsonl`), which is what keeps it in the index; the index
//! itself counts none, or everything shown would count as used. The titles
//! a turn's words match ([`titles`], for the plugin's per-turn hook) count
//! none either, for the same reason.

use super::fence::{self, Audience, OpenError, Seen};
use super::format::Entry;
use super::Store;

/// One note, as read or found.
#[derive(Clone, Debug)]
pub struct Found {
    pub rel: String,
    /// The version of its file: a writer says it saw this one.
    pub version: String,
    pub scope: String,
    pub topic: String,
    pub entry: Entry,
}

impl Found {
    fn of(s: Seen) -> Found {
        Found { rel: s.rel, version: s.version, scope: s.scope.word(), topic: s.topic, entry: s.entry }
    }
}

/// What a read gave.
#[derive(Clone, Debug)]
pub enum Read {
    Entry(Found),
    /// A whole topic file, from a path.
    File { rel: String, version: String, scope: String, topic: String, entries: Vec<Entry> },
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReadError {
    NotFound,
    Open(OpenError),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::NotFound => f.write_str("no such note in Linggen's memory"),
            ReadError::Open(e) => e.fmt(f),
        }
    }
}

/// A note by its id, or a topic file by its path (`person/diet.md`, inside
/// the store). A void note is read too: it says it is void.
pub fn read(store: &Store, audience: Audience, what: &str, now: i64) -> Result<Read, ReadError> {
    let what = what.trim();
    let is_path = what.contains('/') || what.ends_with(".md");
    if !is_path {
        let seen = fence::find(store, audience, what).ok_or(ReadError::NotFound)?;
        store.record_uses(&[&seen.entry.id], now);
        return Ok(Read::Entry(Found::of(seen)));
    }
    let file = fence::open(store, audience, what).map_err(ReadError::Open)?;
    let ids: Vec<&str> = file.entries.iter().map(|e| e.id.as_str()).collect();
    store.record_uses(&ids, now);
    Ok(Read::File {
        rel: file.rel,
        version: file.version,
        scope: file.scope.word(),
        topic: file.topic,
        entries: file.entries,
    })
}

/// The notes that hold the most of `words` (any case; Chinese and English;
/// a run of Chinese is matched two characters at a time), best first, then
/// newest. Void notes are not found.
pub fn search(store: &Store, audience: Audience, words: &str, limit: usize, now: i64) -> Vec<Found> {
    let tokens = tokens_of(words);
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(usize, Seen)> = fence::seen(store, audience)
        .0
        .into_iter()
        .filter(|s| s.entry.void.is_none())
        .filter_map(|s| {
            let hay = format!("{}\n{}\n{}", s.entry.summary, s.topic, s.entry.body).to_lowercase();
            let score = tokens.iter().filter(|t| hay.contains(t.as_str())).count();
            (score > 0).then_some((score, s))
        })
        .collect();
    scored.sort_by(|(sa, a), (sb, b)| {
        sb.cmp(sa)
            .then(b.entry.updated.cmp(&a.entry.updated))
            .then(a.entry.id.cmp(&b.entry.id))
    });
    scored.truncate(limit);
    let ids: Vec<&str> = scored.iter().map(|(_, s)| s.entry.id.as_str()).collect();
    store.record_uses(&ids, now);
    scored.into_iter().map(|(_, s)| Found::of(s)).collect()
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0x3040..=0x30FF | 0xAC00..=0xD7AF)
}

/// Words to look for: each word lower-cased (a word is a run of letters and
/// digits, so `pricing?` is `pricing`); inside it a run of Chinese (or kana,
/// hangul) becomes its overlapping two-character pieces, because those
/// languages have no spaces.
fn tokens_of(words: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in words.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase) {
        let chars: Vec<char> = word.chars().collect();
        for run in chars.chunk_by(|a, b| is_cjk(*a) == is_cjk(*b)) {
            match run.len() > 1 && is_cjk(run[0]) {
                true => out.extend(run.windows(2).map(|w| w.iter().collect::<String>())),
                false => out.push(run.iter().collect()),
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Short plain words that say nothing about which note is meant.
const FILLER: [&str; 32] = [
    "the", "and", "for", "are", "but", "not", "you", "can", "how", "why", "what", "when", "who", "this", "that",
    "with", "from", "have", "has", "was", "were", "will", "would", "could", "should", "does", "did", "its", "our",
    "too", "now", "any",
];

/// A word of a prompt that may stand for a topic: a Chinese piece, or a
/// letters-and-digits word of three or more that is not filler.
fn is_topic_word(token: &str) -> bool {
    token.chars().next().is_some_and(is_cjk) || (token.chars().count() >= 3 && !FILLER.contains(&token))
}

/// A word in more than this share of the titles names no topic.
const COMMON_SHARE: usize = 8;
/// ...but never fewer titles than this.
const COMMON_FLOOR: usize = 10;
/// Independent words of one title it takes for a hit, when the prompt has
/// that many.
const WORDS_FOR_A_HIT: usize = 2;

/// Whether two words of a prompt are two pieces of one Chinese word (they
/// share a character: 提交 and 交代 come from 提交代码) and count as one.
fn overlap(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let cjk = |w: &[char]| w.first().is_some_and(|c| is_cjk(*c));
    cjk(&a) && cjk(&b) && (a.last() == b.first() || b.last() == a.first())
}

/// How many of `words` are independent of one another ([`overlap`]).
pub(super) fn independent_words(words: &[&String]) -> usize {
    let mut counted: Vec<&String> = Vec::new();
    for word in words {
        if !counted.iter().any(|c| overlap(c, word)) {
            counted.push(word);
        }
    }
    counted.len()
}

/// The notes whose title (summary or topic) holds the words of `words` —
/// the lookup a turn does for the prompt it was given. Plain words, no
/// vectors, Chinese and English; it counts no use. A hit holds
/// [`WORDS_FOR_A_HIT`] independent topic words of the prompt (fewer when the
/// rest of the prompt matches no title at all); a word most titles share
/// counts for nothing, so an ordinary sentence matches nothing. Void notes
/// and `exclude`d ids are left out; best match first, then newest.
pub fn titles(store: &Store, audience: Audience, words: &str, exclude: &[String], limit: usize) -> Vec<Found> {
    let tokens: Vec<String> = tokens_of(words).into_iter().filter(|t| is_topic_word(t)).collect();
    if tokens.is_empty() {
        return Vec::new();
    }
    let all: Vec<(String, Seen)> = fence::seen(store, audience)
        .0
        .into_iter()
        .filter(|s| s.entry.void.is_none())
        .map(|s| (format!("{}\n{}", s.entry.summary, s.topic).to_lowercase(), s))
        .collect();
    let common_from = (all.len() / COMMON_SHARE).max(COMMON_FLOOR);
    // The prompt's words some title holds, and not nearly all of them do.
    let usable: Vec<&String> = tokens
        .iter()
        .filter(|t| (1..=common_from).contains(&all.iter().filter(|(hay, _)| hay.contains(t.as_str())).count()))
        .collect();
    let need = independent_words(&usable).min(WORDS_FOR_A_HIT);
    let mut scored: Vec<(usize, Seen)> = all
        .into_iter()
        .filter(|(_, s)| !exclude.contains(&s.entry.id))
        .filter_map(|(hay, s)| {
            let held: Vec<&String> = usable.iter().filter(|t| hay.contains(t.as_str())).copied().collect();
            (need > 0 && independent_words(&held) >= need).then_some((held.len(), s))
        })
        .collect();
    scored.sort_by(|(sa, a), (sb, b)| {
        sb.cmp(sa)
            .then(b.entry.updated.cmp(&a.entry.updated))
            .then(a.entry.id.cmp(&b.entry.id))
    });
    scored.truncate(limit);
    scored.into_iter().map(|(_, s)| Found::of(s)).collect()
}
