//! Heat: which rows earn their place in the automatic recall.
//!
//! Two counters per row — `recalled` (the hook pushed it into a turn) and
//! `used` (the reply cited its id or a "From memory" line; detected by text
//! match in a Stop hook, no model). This module is THE place that turns the
//! pair into a decision; nothing else reads the numbers.
//!
//! - **Ranking**: [`factor`] multiplies a hit's score. Use raises it; pushes
//!   that were never used lower it.
//! - **Noise cutoff**: a row pushed [`SILENT_AFTER_PUSHES`] times with no use
//!   is [`silenced`]: the automatic recall (`push: true`) skips it. It stays
//!   searchable by an agent's own `memory_search`.
//! - **Promotion**: an episodic row used [`PROMOTE_AFTER_USES`] times is
//!   worth keeping — the sweep moves it to semantic (within `lock_tier`).
//!
//! Only hosts that can detect use count pushes (Claude Code's Stop hook);
//! Codex has no such hook, so its recall sends no heat at all.

use super::types::Memory;

/// Pushes with zero use after which the automatic recall stops pushing a row.
pub const SILENT_AFTER_PUSHES: u32 = 5;

/// Uses after which an episodic row is promoted to semantic.
pub const PROMOTE_AFTER_USES: u32 = 2;

/// Score lift per use, up to [`MAX_USES_COUNTED`] uses.
const USE_LIFT: f32 = 0.05;
const MAX_USES_COUNTED: u32 = 4;
/// Score drop per push that was not used, up to [`MAX_IDLE_COUNTED`] pushes.
const IDLE_DROP: f32 = 0.03;
const MAX_IDLE_COUNTED: u32 = 5;

/// Multiplier on a hit's score: `1.0` for a row with no heat.
pub fn factor(recalled: u32, used: u32) -> f32 {
    let lift = USE_LIFT * used.min(MAX_USES_COUNTED) as f32;
    let idle = recalled.saturating_sub(used).min(MAX_IDLE_COUNTED);
    1.0 + lift - IDLE_DROP * idle as f32
}

/// Pushed often, never used: the automatic recall leaves it out.
pub fn silenced(row: &Memory) -> bool {
    row.used == 0 && row.recalled >= SILENT_AFTER_PUSHES
}

/// Used enough to be kept: an episodic row the sweep promotes.
pub fn promotable(row: &Memory) -> bool {
    row.tier == super::types::Tier::Episodic && row.used >= PROMOTE_AFTER_USES
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::{MemoryType, Origin};

    fn row(recalled: u32, used: u32) -> Memory {
        let mut m = Memory::new("x", MemoryType::Fact, Origin::Derived);
        m.tier = crate::memory::types::Tier::Episodic;
        m.recalled = recalled;
        m.used = used;
        m
    }

    #[test]
    fn a_cold_row_ranks_as_before() {
        assert_eq!(factor(0, 0), 1.0);
    }

    #[test]
    fn use_raises_and_idle_pushes_lower() {
        assert!(factor(3, 3) > 1.0);
        assert!(factor(3, 0) < 1.0);
        assert!(factor(3, 3) > factor(3, 0));
        assert!(factor(100, 0) >= 0.8, "the drop is capped");
        assert!(factor(100, 100) <= 1.2 + f32::EPSILON, "the lift is capped");
    }

    #[test]
    fn the_cutoff_is_n_pushes_with_no_use() {
        assert!(!silenced(&row(SILENT_AFTER_PUSHES - 1, 0)));
        assert!(silenced(&row(SILENT_AFTER_PUSHES, 0)));
        assert!(!silenced(&row(SILENT_AFTER_PUSHES + 10, 1)), "one use keeps it");
    }

    #[test]
    fn promotion_needs_two_uses_and_an_episodic_row() {
        assert!(!promotable(&row(5, 1)));
        assert!(promotable(&row(5, 2)));
        let mut semantic = row(5, 2);
        semantic.tier = crate::memory::types::Tier::Semantic;
        assert!(!promotable(&semantic));
    }
}
