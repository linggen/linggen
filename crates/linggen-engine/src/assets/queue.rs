//! What is still to fetch, in what order, and when a failed one may try again.
//! Pure data: no clock of its own, no I/O.

use super::manifest::FileEntry;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How hard the fetcher pushes.
#[derive(Debug, Clone, Copy)]
pub struct Tuning {
    /// Downloads at once.
    pub concurrency: usize,
    /// The pause after a first failure; it doubles per failure.
    pub first_delay: Duration,
    /// The longest pause.
    pub max_delay: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            concurrency: 4,
            first_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(600),
        }
    }
}

impl Tuning {
    /// The pause after `failures` failures in a row (at least one).
    pub fn delay(&self, failures: u32) -> Duration {
        let doublings = failures.saturating_sub(1).min(30);
        self.first_delay
            .saturating_mul(1u32 << doublings)
            .min(self.max_delay)
    }
}

/// One object to fetch.
#[derive(Debug, Clone)]
pub struct Want {
    pub file: FileEntry,
    pub url: String,
}

/// Why a download failed: the service could not be had (no network, a 5xx,
/// a full disk) — which pauses everything — or it answered and this file was
/// not what it says (a 4xx, wrong bytes), which backs off that file only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Unreachable,
    Rejected,
}

struct Item {
    want: Want,
    wanted: bool,
    /// Higher = asked for more recently by a page; 0 = never.
    bumped: u64,
    /// A page asked since this was last started: it goes now, back-off or not.
    asked: bool,
    failures: u32,
    retry_at: Option<Instant>,
    in_flight: bool,
}

#[derive(Default)]
pub struct Queue {
    items: HashMap<String, Item>,
    bumps: u64,
    /// Unreachable failures in a row; any success clears it.
    streak: u32,
    paused_until: Option<Instant>,
}

impl Queue {
    /// The queue is exactly `wants` (keyed by object). What was already
    /// queued keeps its failures and its bump; a download in progress is
    /// left to finish even when it is no longer wanted.
    pub fn replace(&mut self, wants: Vec<Want>) {
        for item in self.items.values_mut() {
            item.wanted = false;
        }
        for want in wants {
            self.items
                .entry(want.file.object())
                .and_modify(|item| {
                    item.want = want.clone();
                    item.wanted = true;
                })
                .or_insert_with(|| Item::new(want));
        }
        self.items.retain(|_, item| item.wanted || item.in_flight);
    }

    /// A page is asking for this now: first in line, and it goes at once
    /// even through a back-off or a pause. Queues it when it was not (a
    /// stored object that went missing).
    pub fn bump(&mut self, want: Want) {
        self.bumps += 1;
        let bumps = self.bumps;
        let item = self
            .items
            .entry(want.file.object())
            .or_insert_with(|| Item::new(want));
        item.bumped = bumps;
        item.asked = true;
    }

    /// The best download that may start now, marked in flight: most
    /// recently asked for, then lowest `order`, then path.
    pub fn next(&mut self, now: Instant) -> Option<Want> {
        let paused = self.paused_until.is_some_and(|until| now < until);
        let key = self
            .items
            .iter()
            .filter(|(_, item)| item.ready(now, paused))
            .min_by_key(|(_, item)| item.rank())
            .map(|(key, _)| key.clone())?;
        let item = self.items.get_mut(&key)?;
        item.in_flight = true;
        item.asked = false;
        Some(item.want.clone())
    }

    pub fn succeeded(&mut self, object: &str) {
        self.items.remove(object);
        self.streak = 0;
        self.paused_until = None;
    }

    pub fn failed(&mut self, object: &str, failure: Failure, now: Instant, tuning: &Tuning) {
        // Downloads that fail together are one pause, not several steps.
        let pause_over = self.paused_until.is_none_or(|until| now >= until);
        if failure == Failure::Unreachable && pause_over {
            self.streak += 1;
            self.paused_until = Some(now + tuning.delay(self.streak));
        }
        let Some(item) = self.items.get_mut(object) else {
            return;
        };
        item.in_flight = false;
        item.failures += 1;
        item.retry_at = Some(now + tuning.delay(item.failures));
        if !item.wanted {
            self.items.remove(object);
        }
    }

    /// When the next waiting download may start; None = nothing waits.
    pub fn wake_at(&self, now: Instant) -> Option<Instant> {
        let pause = self.paused_until.unwrap_or(now);
        self.items
            .values()
            .filter(|item| !item.in_flight)
            .map(|item| match item.asked {
                true => now,
                false => item.retry_at.unwrap_or(now).max(pause),
            })
            .min()
    }

    pub fn in_flight(&self) -> usize {
        self.items.values().filter(|i| i.in_flight).count()
    }

    /// How many times this object has failed since it last succeeded.
    pub fn failures(&self, object: &str) -> u32 {
        self.items.get(object).map_or(0, |item| item.failures)
    }
}

impl Item {
    fn new(want: Want) -> Self {
        Self {
            want,
            wanted: true,
            bumped: 0,
            asked: false,
            failures: 0,
            retry_at: None,
            in_flight: false,
        }
    }

    fn ready(&self, now: Instant, paused: bool) -> bool {
        let waited = !paused && self.retry_at.is_none_or(|at| at <= now);
        !self.in_flight && (self.asked || waited)
    }

    fn rank(&self) -> (std::cmp::Reverse<u64>, u32, &str) {
        (
            std::cmp::Reverse(self.bumped),
            self.want.file.order,
            &self.want.file.path,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn want(name: &str, order: u32) -> Want {
        let file = FileEntry {
            path: format!("{name}.png"),
            sha256: format!("{:0<64}", name.len().to_string() + name),
            size: 1,
            pack: "p".into(),
            order,
        };
        Want {
            url: file.url("https://x.test/"),
            file,
        }
    }

    fn tuning() -> Tuning {
        Tuning {
            concurrency: 4,
            first_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(8),
        }
    }

    fn order_of(queue: &mut Queue, now: Instant) -> Vec<String> {
        std::iter::from_fn(|| queue.next(now))
            .map(|w| w.file.path)
            .collect()
    }

    #[test]
    fn lowest_order_first_then_path() {
        let mut q = Queue::default();
        q.replace(vec![want("c", 5), want("b", 1), want("a", 5)]);
        assert_eq!(
            order_of(&mut q, Instant::now()),
            ["b.png", "a.png", "c.png"]
        );
    }

    #[test]
    fn a_bumped_file_goes_first_and_the_latest_bump_wins() {
        let mut q = Queue::default();
        q.replace(vec![want("a", 1), want("b", 2), want("c", 3)]);
        q.bump(want("c", 3));
        q.bump(want("b", 2));
        assert_eq!(
            order_of(&mut q, Instant::now()),
            ["b.png", "c.png", "a.png"]
        );
    }

    #[test]
    fn a_bump_queues_a_file_that_was_not_queued() {
        let mut q = Queue::default();
        q.bump(want("a", 1));
        assert_eq!(order_of(&mut q, Instant::now()), ["a.png"]);
    }

    #[test]
    fn delays_double_and_stop_at_the_cap() {
        let t = tuning();
        let secs: Vec<u64> = (1..=6).map(|n| t.delay(n).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 8, 8]);
        assert_eq!(t.delay(u32::MAX).as_secs(), 8);
    }

    #[test]
    fn an_unreachable_service_pauses_everything_until_it_answers() {
        let (mut q, t, now) = (Queue::default(), tuning(), Instant::now());
        q.replace(vec![want("a", 1), want("b", 2)]);
        let a = q.next(now).expect("a");
        q.failed(&a.file.object(), Failure::Unreachable, now, &t);
        assert!(q.next(now).is_none(), "paused");
        assert_eq!(q.failures(&a.file.object()), 1);
        assert_eq!(q.wake_at(now), Some(now + Duration::from_secs(1)));
        let later = now + Duration::from_secs(1);
        let again = q.next(later).expect("after the pause");
        assert_eq!(again.file.path, "a.png");
        q.succeeded(&again.file.object());
        assert_eq!(
            q.next(later).expect("b").file.path,
            "b.png",
            "pause cleared"
        );
    }

    #[test]
    fn bad_bytes_back_off_that_file_only() {
        let (mut q, t, now) = (Queue::default(), tuning(), Instant::now());
        q.replace(vec![want("a", 1), want("b", 2)]);
        let a = q.next(now).expect("a");
        q.failed(&a.file.object(), Failure::Rejected, now, &t);
        assert_eq!(q.next(now).expect("b").file.path, "b.png");
    }

    #[test]
    fn a_page_asking_goes_through_the_back_off_and_the_pause() {
        let (mut q, t, now) = (Queue::default(), tuning(), Instant::now());
        q.replace(vec![want("a", 1), want("b", 2)]);
        let a = q.next(now).expect("a");
        q.failed(&a.file.object(), Failure::Unreachable, now, &t);
        assert!(q.next(now).is_none());

        q.bump(want("b", 2));
        assert_eq!(q.wake_at(now), Some(now));
        assert_eq!(q.next(now).expect("asked").file.path, "b.png");
        assert!(q.next(now).is_none(), "one ask, one attempt");
    }

    #[test]
    fn downloads_failing_together_make_one_pause() {
        let (mut q, t, now) = (Queue::default(), tuning(), Instant::now());
        q.replace(vec![want("a", 1), want("b", 2), want("c", 3)]);
        let started: Vec<_> = (0..3).filter_map(|_| q.next(now)).collect();
        for w in &started {
            q.failed(&w.file.object(), Failure::Unreachable, now, &t);
        }
        assert_eq!(q.wake_at(now), Some(now + Duration::from_secs(1)));
    }

    #[test]
    fn replace_keeps_what_is_in_flight_and_drops_what_is_not_wanted() {
        let (mut q, now) = (Queue::default(), Instant::now());
        q.replace(vec![want("a", 1), want("b", 2)]);
        let a = q.next(now).expect("a");
        q.replace(vec![]);
        assert_eq!(q.in_flight(), 1);
        q.failed(&a.file.object(), Failure::Rejected, now, &tuning());
        assert!(q.next(now + Duration::from_secs(60)).is_none(), "gone");
    }
}
