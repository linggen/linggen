//! Guardrails, in code, on what she says unprompted: a glance, an app moment
//! she was not asked for, a woken note. A reply to the user — her own chat, an
//! asked moment, a herald of a run the user started — is never held here.
//!
//! - **A ceiling** on unprompted lines in any rolling hour (`pet.unprompted_per_hour`).
//! - **Quiet hours** in local time (`pet.quiet_hours`), when nothing unprompted is said.
//!
//! What woke the line ([`Source`]) decides how they apply: a moment from the
//! app the person is using passes quiet hours, a mission fault waits through
//! them and is outside the ceiling.
//!
//! Both are checked before her turn runs, so a held line is never written:
//! at an app's table her line would already be a row of its chat. What a
//! guardrail held is written once to the activity log (verb `hold`).

use super::spoken::Origin;
use crate::config::PetConfig;
use std::collections::VecDeque;
use std::sync::Mutex;

/// The rolling window the ceiling counts over.
const WINDOW_SECS: u64 = 3600;

/// What woke an unprompted line. It decides which guardrail holds it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Source {
    Glance,
    /// An app moment from an app the person is not using.
    Moment,
    /// An app moment from the app the person is using: quiet hours let it by.
    InUse,
    Note,
    /// A mission run that went wrong: quiet hours make it wait; the ceiling
    /// neither counts it nor holds it.
    Fault,
}

impl Source {
    /// The kind of turn of hers whose name this source shares: one name per
    /// kind (`Origin::name`), so a log line and the activity log's `hold`
    /// record never call the same kind two things. Not a mapping from
    /// origin to source — that is `triggers::unprompted_source`.
    fn named_as(self) -> Origin {
        match self {
            Source::Glance => Origin::Glance,
            Source::Moment => Origin::Moment,
            Source::InUse => Origin::InUse,
            Source::Note => Origin::Note,
            Source::Fault => Origin::Fault,
        }
    }

    /// The word the activity log's `hold` record carries.
    pub(crate) fn name(self) -> &'static str {
        self.named_as().name()
    }

    /// Whether her line counts toward the hourly ceiling.
    pub(crate) fn counts(self) -> bool {
        self != Source::Fault
    }
}

/// Why something unprompted is not said now.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Held {
    /// Inside quiet hours, which end at this local time (`HH:MM`).
    Quiet { until: String },
    /// The hour already holds this many unprompted lines.
    Ceiling { per_hour: u32 },
}

impl Held {
    pub(crate) fn reason(&self) -> &'static str {
        match self {
            Held::Quiet { .. } => "quiet-hours",
            Held::Ceiling { .. } => "hourly-ceiling",
        }
    }

    fn words(&self) -> &'static str {
        match self {
            Held::Quiet { .. } => "quiet hours",
            Held::Ceiling { .. } => "hourly ceiling",
        }
    }
}

/// Quiet hours: `[from, until)` in minutes of the local day; `from > until`
/// crosses midnight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct QuietHours {
    from: u32,
    until: u32,
}

impl QuietHours {
    /// `HH:MM-HH:MM`; `None` for empty, unreadable or empty-span text.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let (from, until) = text.trim().split_once('-')?;
        let (from, until) = (minute_of(from)?, minute_of(until)?);
        (from != until).then_some(Self { from, until })
    }

    pub(crate) fn contains(&self, minute: u32) -> bool {
        if self.from < self.until {
            return (self.from..self.until).contains(&minute);
        }
        minute >= self.from || minute < self.until
    }

    fn ends(&self) -> String {
        format!("{:02}:{:02}", self.until / 60, self.until % 60)
    }
}

/// `HH:MM` as minutes of the day.
fn minute_of(text: &str) -> Option<u32> {
    let t = chrono::NaiveTime::parse_from_str(text.trim(), "%H:%M").ok()?;
    Some(chrono::Timelike::hour(&t) * 60 + chrono::Timelike::minute(&t))
}

/// The guardrails as the settings set them.
#[derive(Clone, Debug)]
pub(crate) struct Rules {
    per_hour: u32,
    quiet: Option<QuietHours>,
}

impl Rules {
    pub(crate) fn of(pet: &PetConfig) -> Self {
        Self {
            per_hour: pet.unprompted_per_hour,
            quiet: QuietHours::parse(&pet.quiet_hours),
        }
    }

    /// What holds an unprompted line from `source` at `now`, the local clock
    /// at `minute`, with `lines` said so far. Quiet hours first: they say
    /// until when.
    pub(crate) fn check(
        &self,
        source: Source,
        now: u64,
        minute: u32,
        lines: &Lines,
    ) -> Option<Held> {
        if source != Source::InUse {
            if let Some(q) = self.quiet.filter(|q| q.contains(minute)) {
                return Some(Held::Quiet { until: q.ends() });
            }
        }
        if source.counts() && lines.in_hour(now) >= self.per_hour as usize {
            return Some(Held::Ceiling {
                per_hour: self.per_hour,
            });
        }
        None
    }
}

/// When her recent unprompted lines were said.
#[derive(Debug, Default)]
pub(crate) struct Lines(VecDeque<u64>);

impl Lines {
    pub(crate) const fn new() -> Self {
        Self(VecDeque::new())
    }

    pub(crate) fn in_hour(&self, now: u64) -> usize {
        self.0
            .iter()
            .filter(|&&t| now.saturating_sub(t) < WINDOW_SECS)
            .count()
    }

    pub(crate) fn note(&mut self, now: u64) {
        self.0.retain(|&t| now.saturating_sub(t) < WINDOW_SECS);
        self.0.push_back(now);
    }
}

/// Her unprompted lines this process has said. Kept in memory: a restart
/// starts the hour afresh.
static LINES: Mutex<Lines> = Mutex::new(Lines::new());

/// What holds an unprompted line from `source` right now, if anything.
pub(crate) fn held_now(pet: &PetConfig, source: Source) -> Option<Held> {
    let now = crate::util::now_ts_secs();
    let lines = LINES.lock().unwrap_or_else(|e| e.into_inner());
    Rules::of(pet).check(source, now, local_minute(), &lines)
}

/// An unprompted line was said aloud.
pub(crate) fn spoke_unprompted() {
    let now = crate::util::now_ts_secs();
    LINES.lock().unwrap_or_else(|e| e.into_inner()).note(now);
}

fn local_minute() -> u32 {
    use chrono::Timelike;
    let t = chrono::Local::now();
    t.hour() * 60 + t.minute()
}

/// What a held thing is, for its record: what it was and what woke it.
pub(crate) struct Withheld<'a> {
    /// What would have been said: `a look-again on n3`, `2 app moments`.
    pub what: String,
    /// `glance`, `moment`, `note`.
    pub source: &'a str,
    /// The same held thing asked again (a note's wake, tick after tick)
    /// carries the same key and is recorded once.
    pub key: String,
}

/// The record for a held thing — `None` when this key was already recorded.
pub(crate) fn held_record(item: &Withheld, held: &Held) -> Option<(String, serde_json::Value)> {
    if !first_time(&item.key) {
        return None;
    }
    let object = format!("{} ({})", item.what, held.words());
    let mut detail = serde_json::json!({ "reason": held.reason(), "source": item.source });
    match held {
        Held::Quiet { until } => detail["until"] = until.clone().into(),
        Held::Ceiling { per_hour } => detail["per_hour"] = (*per_hour).into(),
    }
    Some((object, detail))
}

/// Write what a guardrail held to the activity log, once per held thing.
pub(crate) fn record_held(item: &Withheld, held: &Held) {
    let Some((object, detail)) = held_record(item, held) else {
        return;
    };
    tracing::info!("[yinyue-guard] held {} ({})", item.what, held.reason());
    crate::perception::activity::record_detail(
        super::YINYUE_AGENT,
        "linggen",
        HOLD_VERB,
        Some(object),
        Some(detail),
    );
}

/// The engine's verb for a held line (`perception::activity::ENGINE_VERBS`).
pub(crate) const HOLD_VERB: &str = "hold";

/// Held things already recorded, newest last.
static RECORDED: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
/// How many recorded keys are remembered.
const RECORDED_MAX: usize = 64;

fn first_time(key: &str) -> bool {
    let mut seen = RECORDED.lock().unwrap_or_else(|e| e.into_inner());
    if seen.iter().any(|k| k == key) {
        return false;
    }
    seen.push_back(key.to_string());
    while seen.len() > RECORDED_MAX {
        seen.pop_front();
    }
    true
}

#[cfg(test)]
mod tests;
