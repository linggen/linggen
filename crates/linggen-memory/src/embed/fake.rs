//! A deterministic stand-in for the embedding model, for tests only (the
//! `test-support` feature; never in a shipping build).
//!
//! Feature hashing: each word of the text (lowercased, split on anything
//! that is not a letter or digit; every CJK character is a word of its own)
//! adds ±1 to one of the [`VECTOR_DIM`] buckets, chosen by a stable hash of
//! the word; the sum is L2-normalized. So:
//! - equal texts give equal vectors, on every run and machine;
//! - the cosine of two texts grows with the words they share — a query made
//!   of a row's words finds that row, and rows with no word in common score
//!   about 0 (only a bucket collision, 1 in 1024 per word pair, adds any);
//! - the `query: ` / `passage: ` prefixes the real model needs are dropped
//!   first, so a query equal to a stored text scores exactly 1.0.
//!
//! A text whose words cancel out (none at all, or two words of one bucket
//! with opposite signs) gets a fixed unit vector instead, so nothing
//! downstream ever sees a zero vector.

use super::{PASSAGE_PREFIX, QUERY_PREFIX};
use crate::memory::VECTOR_DIM;

/// The vector for `text`, as the real model's would be shaped: `VECTOR_DIM`
/// floats, unit length.
pub(super) fn vector(text: &str) -> Vec<f32> {
    let text = text
        .strip_prefix(QUERY_PREFIX)
        .or_else(|| text.strip_prefix(PASSAGE_PREFIX))
        .unwrap_or(text);
    let dim = VECTOR_DIM as usize;
    let mut v = vec![0f32; dim];
    for word in words(text) {
        let h = fnv1a(word.as_bytes());
        let sign = if h >> 63 == 0 { 1.0 } else { -1.0 };
        v[(h % dim as u64) as usize] += sign;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        v[0] = 1.0;
        return v;
    }
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if is_cjk(c) {
            flush(&mut cur, &mut out);
            out.push(c.to_string());
        } else if c.is_alphanumeric() {
            cur.push(c);
        } else {
            flush(&mut cur, &mut out);
        }
    }
    flush(&mut cur, &mut out);
    out
}

fn flush(cur: &mut String, out: &mut Vec<String>) {
    if !cur.is_empty() {
        out.push(std::mem::take(cur));
    }
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF)
}

/// FNV-1a, 64-bit: stable across runs, platforms and Rust versions (std's
/// hasher is not).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    #[test]
    fn the_same_text_is_the_same_unit_vector_and_another_differs() {
        let a = vector("Alex prefers green tea");
        assert_eq!(a.len(), VECTOR_DIM as usize);
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-5);
        assert_eq!(a, vector("passage: Alex prefers green tea"));
        assert_eq!(a, vector("query: alex PREFERS green, tea!"));
        assert_ne!(a, vector("Alex prefers black coffee"));
        assert_eq!(vector("").len(), VECTOR_DIM as usize);
    }

    #[test]
    fn similarity_follows_the_shared_words() {
        let row = vector("Alex prefers green tea in the morning");
        let near = vector("green tea");
        let far = vector("the deploy script needs a port");
        assert!(cosine(&row, &near) > 0.4, "{}", cosine(&row, &near));
        assert!(cosine(&row, &far) < cosine(&row, &near));
        let zh = vector("阿历克斯喜欢绿茶");
        assert!(cosine(&zh, &vector("绿茶")) > 0.4);
    }
}
