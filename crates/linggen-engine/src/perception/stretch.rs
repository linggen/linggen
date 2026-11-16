//! Stretches of use: how long each of Linggen's own pages was in front while
//! the person was active, one record per stretch.
//!
//! Fed by the presence beats every page sends (and a tick, for when the beats
//! stop). The engine keeps one open stretch — the app in front, when it began,
//! the last input seen — and writes one record when it ends: another page
//! comes to the front ("used" until that moment), the person goes idle past
//! [`IDLE_LIMIT_SECS`] or every page stops beating (until their last input),
//! or nothing is heard for [`GAP_SECS`] (the Mac slept). The record names the
//! app, never what was on the page:
//!
//! `{"at":"<start>","by":"user","app":"<skill>","verb":"used","detail":{"seconds":1260}}`
//!
//! Only Linggen's pages count: time in a terminal, the CLI or any other program
//! sends no beat, so it is not here. The main page names no skill and is
//! recorded as [`MAIN_PAGE`].
//!
//! The open stretch is saved to `open-stretch.json` at most every
//! [`PERSIST_SECS`]; the engine has no stop hook (TERM ends the process), so a
//! stop, a kill and a crash are the same case: on the next start the saved
//! stretch is closed at the last moment it was saved.

use super::activity::Activity;
use crate::engine::agent::Front;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

/// Shorter than this is a page passed through, not used. Dropped.
pub const MIN_SECS: u64 = 60;
/// A stretch is written in pieces of at most this long, so a long session
/// reaches the log as it goes.
pub const MAX_SECS: u64 = 3600;
/// No input for this long and the person is no longer using the page.
pub const IDLE_LIMIT_SECS: u64 = 300;
/// Nothing heard (beat or tick) for this long: the stretch ended at the last
/// input — the Mac slept, or the engine stalled.
pub const GAP_SECS: u64 = 60;
/// How often the open stretch is saved, at most. What a kill can lose.
pub const PERSIST_SECS: u64 = 30;
/// How often the engine looks without a beat, so a page that stops beating
/// still ends its stretch.
pub const TICK_SECS: u64 = 15;
/// The app name for Linggen's main page, which names no skill.
pub const MAIN_PAGE: &str = "linggen";
/// The one verb stretches are written with. Engine-owned (`ENGINE_VERBS`).
pub const VERB: &str = "used";

/// The stretch being kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Open {
    pub app: String,
    /// Unix seconds.
    pub start: u64,
    pub last_input: u64,
    pub last_seen: u64,
}

/// A finished piece of use, ready to write.
#[derive(Debug, Clone, PartialEq)]
pub struct Ended {
    pub app: String,
    pub start: u64,
    pub seconds: u64,
}

/// The bookkeeping, with the clock passed in.
#[derive(Debug, Default)]
pub struct Tracker {
    pub open: Option<Open>,
}

impl Tracker {
    /// One look at `now`: `front` is the page in front, if any.
    pub fn observe(&mut self, now: u64, front: Option<Front>) -> Vec<Ended> {
        let mut out = Vec::new();
        if let Some(o) = self.open.as_ref().filter(|o| now.saturating_sub(o.last_seen) > GAP_SECS) {
            let end = o.last_input;
            out.extend(self.close(end));
        }
        let active = front.filter(|f| now.saturating_sub(f.last_input_at) <= IDLE_LIMIT_SECS);
        let Some(f) = active else {
            let end = self.open.as_ref().map(|o| o.last_input);
            out.extend(end.map(|e| self.close(e)).unwrap_or_default());
            return out;
        };
        let app = f.app.unwrap_or_else(|| MAIN_PAGE.to_string());
        match self.open.as_mut() {
            Some(o) if o.app == app => {
                o.last_seen = now;
                o.last_input = o.last_input.max(f.last_input_at);
                out.extend(cut_full_pieces(o));
            }
            Some(_) => {
                out.extend(self.close(now));
                self.start(app, now, f.last_input_at);
            }
            None => self.start(app, now, f.last_input_at),
        }
        out
    }

    fn start(&mut self, app: String, now: u64, last_input: u64) {
        self.open = Some(Open {
            app,
            start: now,
            last_input: last_input.max(now),
            last_seen: now,
        });
    }

    /// End the open stretch at `end`.
    fn close(&mut self, end: u64) -> Vec<Ended> {
        match self.open.take() {
            Some(o) => pieces(&o.app, o.start, end),
            None => Vec::new(),
        }
    }
}

/// A stretch from `start` to `end` as records: full pieces of [`MAX_SECS`],
/// then the rest, each dropped when shorter than [`MIN_SECS`].
pub fn pieces(app: &str, start: u64, end: u64) -> Vec<Ended> {
    let mut out = Vec::new();
    let mut from = start;
    while from < end {
        let seconds = (end - from).min(MAX_SECS);
        if seconds >= MIN_SECS {
            out.push(Ended { app: app.to_string(), start: from, seconds });
        }
        from += seconds;
    }
    out
}

/// Write out the full pieces of a stretch still going, and move its start on.
fn cut_full_pieces(o: &mut Open) -> Vec<Ended> {
    let mut out = Vec::new();
    while o.last_seen.saturating_sub(o.start) >= MAX_SECS {
        out.push(Ended { app: o.app.clone(), start: o.start, seconds: MAX_SECS });
        o.start += MAX_SECS;
    }
    out
}

/// A saved stretch, closed where it was last saved — on start-up, after a
/// stop, a kill or a crash.
pub fn recover(saved: &Open) -> Vec<Ended> {
    pieces(&saved.app, saved.start, saved.last_seen)
}

fn to_utc(secs: u64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(secs as i64, 0).unwrap_or_default()
}

impl Ended {
    pub fn record(&self) -> Activity {
        super::activity::stamped(
            to_utc(self.start),
            None,
            "user",
            &self.app,
            VERB,
            None,
            Some(serde_json::json!({ "seconds": self.seconds })),
        )
    }
}

/// The tracker with its file and where its records go.
pub struct Stretches {
    tracker: Tracker,
    file: PathBuf,
    /// When the open stretch was last saved, and as what.
    saved: Option<(u64, Open)>,
    sink: Box<dyn Fn(Activity) + Send>,
}

impl Stretches {
    pub fn new(dir: PathBuf, sink: Box<dyn Fn(Activity) + Send>) -> Self {
        Self {
            tracker: Tracker::default(),
            file: dir.join("open-stretch.json"),
            saved: None,
            sink,
        }
    }

    pub fn observe(&mut self, now: u64, front: Option<Front>) {
        for e in self.tracker.observe(now, front) {
            (self.sink)(e.record());
        }
        self.save(now);
    }

    /// Close what the last run left open, and forget it.
    pub fn recover(&mut self) {
        let saved = std::fs::read_to_string(&self.file)
            .ok()
            .and_then(|t| serde_json::from_str::<Open>(&t).ok());
        for e in saved.as_ref().map(recover).unwrap_or_default() {
            (self.sink)(e.record());
        }
        let _ = std::fs::remove_file(&self.file);
    }

    /// The open stretch as a record up to its last moment, marked open.
    pub fn open_record(&self) -> Option<Activity> {
        let o = self.tracker.open.as_ref()?;
        let seconds = o.last_seen.saturating_sub(o.start);
        let mut a = Ended { app: o.app.clone(), start: o.start, seconds }.record();
        a.detail = Some(serde_json::json!({ "seconds": seconds, "open": true }));
        Some(a)
    }

    /// Keep the file in step with the open stretch: removed when none is open,
    /// rewritten when it changed shape or [`PERSIST_SECS`] have passed.
    fn save(&mut self, now: u64) {
        let Some(open) = self.tracker.open.clone() else {
            if self.saved.take().is_some() {
                let _ = std::fs::remove_file(&self.file);
            }
            return;
        };
        let fresh = self.saved.as_ref().is_some_and(|(at, s)| {
            s.app == open.app && s.start == open.start && now.saturating_sub(*at) < PERSIST_SECS
        });
        if fresh {
            return;
        }
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.file.with_extension("json.tmp");
        let body = serde_json::to_vec(&open).unwrap_or_default();
        if std::fs::write(&tmp, body).is_ok() && std::fs::rename(&tmp, &self.file).is_ok() {
            self.saved = Some((now, open));
        }
    }
}

static STRETCHES: Mutex<Option<Stretches>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Stretches) -> T) -> T {
    let mut g = STRETCHES.lock().unwrap_or_else(|e| e.into_inner());
    let s = g.get_or_insert_with(|| {
        Stretches::new(
            super::activity::activity_dir(),
            Box::new(|a| super::activity::log().add(a)),
        )
    });
    f(s)
}

/// One look at the page in front, from a beat or the tick.
pub fn observe(now: u64, front: Option<Front>) {
    with(|s| s.observe(now, front));
}

/// On start-up: close the stretch the last run left open.
pub fn recover_saved() {
    with(Stretches::recover);
}

/// The stretch still open, as a record a reader can count, if one is.
pub fn open_record() -> Option<Activity> {
    with(|s| s.open_record())
}

/// Look every [`TICK_SECS`] without waiting for a beat, so a page that stops
/// beating still ends its stretch.
pub async fn stretch_loop(front: impl Fn(u64) -> Option<Front>) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(TICK_SECS));
    loop {
        tick.tick().await;
        let now = crate::util::now_ts_secs();
        observe(now, front(now));
    }
}

#[cfg(test)]
mod tests;
