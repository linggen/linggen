//! The room a block of text may take: lines and bytes. The one cutter.
//!
//! What a session start forwards is cut to fit a [`Room`] by [`Room::count`]
//! and nowhere else. Who gets how much is decided in one file,
//! `own_memory::forward`, which is the only one that makes a `Room`
//! (`tests::the_block_is_cut_in_one_place`); the parts that fill it
//! ([`super::slice`], `own_memory::index`) are handed one and cannot choose a
//! number.

/// At most this many lines and bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Room {
    lines: usize,
    bytes: usize,
}

impl Room {
    pub fn new(lines: usize, bytes: usize) -> Room {
        Room { lines, bytes }
    }

    pub fn lines(self) -> usize {
        self.lines
    }

    pub fn bytes(self) -> usize {
        self.bytes
    }

    /// Whether `text` (lines of it, each ending in a newline) fits.
    pub fn holds(self, text: &str) -> bool {
        text.lines().count() <= self.lines && text.len() <= self.bytes
    }

    /// What is left after `text` is put in.
    pub fn minus(self, text: &str) -> Room {
        Room { lines: self.lines.saturating_sub(text.lines().count()), bytes: self.bytes.saturating_sub(text.len()) }
    }

    /// The smaller of the two in each measure.
    pub fn at_most(self, other: Room) -> Room {
        Room { lines: self.lines.min(other.lines), bytes: self.bytes.min(other.bytes) }
    }

    /// The share `num`/`den` of this room.
    pub fn share(self, num: usize, den: usize) -> Room {
        Room { lines: self.lines * num / den, bytes: self.bytes * num / den }
    }

    /// How many of `items`, taken in order, fit — each one a line of its
    /// own — stopping at the first that does not.
    pub fn count(self, items: &[String]) -> usize {
        let (mut lines, mut bytes) = (0, 0);
        for (n, item) in items.iter().enumerate() {
            lines += item.lines().count().max(1);
            bytes += item.len() + 1;
            if lines > self.lines || bytes > self.bytes {
                return n;
            }
        }
        items.len()
    }
}
