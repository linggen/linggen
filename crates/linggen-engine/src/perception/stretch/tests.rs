use super::*;
use std::sync::{Arc, Mutex as StdMutex};

const T0: u64 = 1_800_000_000;

fn page(app: &str, last_input_at: u64) -> Option<Front> {
    Some(Front { app: Some(app.into()), last_input_at })
}

fn ended(app: &str, start: u64, seconds: u64) -> Ended {
    Ended { app: app.into(), start, seconds }
}

/// Beats every 4 s on `app` from `from` to `to`, input on each.
fn active(t: &mut Tracker, app: &str, from: u64, to: u64) -> Vec<Ended> {
    let mut out = Vec::new();
    let mut now = from;
    while now <= to {
        out.extend(t.observe(now, page(app, now)));
        now += 4;
    }
    out
}

#[test]
fn a_switch_ends_one_stretch_and_starts_the_next() {
    let mut t = Tracker::default();
    assert!(active(&mut t, "notes", T0, T0 + 600).is_empty(), "nothing written while it runs");
    let out = t.observe(T0 + 604, page("tiles", T0 + 604));
    assert_eq!(out, vec![ended("notes", T0, 604)]);
    assert_eq!(t.open.as_ref().unwrap().app, "tiles");
}

#[test]
fn the_main_page_is_named_and_counts_like_any_app() {
    let mut t = Tracker::default();
    t.observe(T0, Some(Front { app: None, last_input_at: T0 }));
    assert_eq!(t.open.as_ref().unwrap().app, MAIN_PAGE);
}

#[test]
fn going_idle_ends_the_stretch_at_the_last_input() {
    let mut t = Tracker::default();
    active(&mut t, "notes", T0, T0 + 300);
    // Beats go on, input does not: still open within the idle limit…
    let last = T0 + 300;
    let mut now = last;
    let mut out = Vec::new();
    while out.is_empty() && now < last + 1000 {
        now += 4;
        out = t.observe(now, page("notes", last));
    }
    assert!(now > last + IDLE_LIMIT_SECS, "open while within the limit");
    assert_eq!(out, vec![ended("notes", T0, 300)], "…and ended where input stopped");
    assert!(t.open.is_none());
}

#[test]
fn a_page_that_stops_beating_ends_its_stretch() {
    let mut t = Tracker::default();
    active(&mut t, "notes", T0, T0 + 200);
    assert_eq!(t.observe(T0 + 215, None), vec![ended("notes", T0, 200)]);
}

#[test]
fn a_gap_with_nothing_heard_ends_the_stretch_before_the_next_starts() {
    let mut t = Tracker::default();
    active(&mut t, "notes", T0, T0 + 200);
    // The Mac slept an hour; the page beats again on waking, with fresh input.
    let woke = T0 + 3800;
    let out = t.observe(woke, page("notes", woke));
    assert_eq!(out, vec![ended("notes", T0, 200)]);
    assert_eq!(t.open.as_ref().unwrap().start, woke);
}

#[test]
fn shorter_than_the_minimum_is_dropped() {
    let mut t = Tracker::default();
    active(&mut t, "notes", T0, T0 + 40);
    assert!(t.observe(T0 + 44, page("tiles", T0 + 44)).is_empty());
    assert_eq!(pieces("x", T0, T0 + MIN_SECS - 1), vec![]);
    assert_eq!(pieces("x", T0, T0 + MIN_SECS), vec![ended("x", T0, MIN_SECS)]);
}

#[test]
fn a_long_stretch_is_written_in_pieces_as_it_goes() {
    let mut t = Tracker::default();
    let out = active(&mut t, "notes", T0, T0 + 2 * MAX_SECS + 100);
    assert_eq!(out, vec![ended("notes", T0, MAX_SECS), ended("notes", T0 + MAX_SECS, MAX_SECS)]);
    assert_eq!(t.open.as_ref().unwrap().start, T0 + 2 * MAX_SECS);
    assert_eq!(pieces("x", T0, T0 + MAX_SECS + 30), vec![ended("x", T0, MAX_SECS)], "a short tail is dropped");
}

#[derive(Clone, Default)]
struct Sink(Arc<StdMutex<Vec<Activity>>>);

impl Sink {
    fn boxed(&self) -> Box<dyn Fn(Activity) + Send> {
        let inner = self.0.clone();
        Box::new(move |a| inner.lock().unwrap().push(a))
    }
    fn taken(&self) -> Vec<Activity> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

fn beat_on(s: &mut Stretches, app: &str, from: u64, to: u64) {
    let mut now = from;
    while now < to {
        s.observe(now, page(app, now));
        now += 4;
    }
    s.observe(to, page(app, to));
}

#[test]
fn a_killed_engine_closes_its_stretch_on_the_next_start_at_the_last_save() {
    let dir = tempfile::tempdir().unwrap();
    let sink = Sink::default();
    let mut s = Stretches::new(dir.path().to_path_buf(), sink.boxed());
    let mut now = T0;
    while now <= T0 + 125 {
        s.observe(now, page("notes", now));
        now += 4;
    }
    assert!(sink.taken().is_empty());
    let saved: Open = serde_json::from_slice(&std::fs::read(dir.path().join("open-stretch.json")).unwrap()).unwrap();
    // Saved at most every PERSIST_SECS: the last save is within that of the
    // last beat, and that is all a kill loses.
    assert!(T0 + 124 - saved.last_seen < PERSIST_SECS, "{saved:?}");
    drop(s); // killed: no word written

    let mut next = Stretches::new(dir.path().to_path_buf(), sink.boxed());
    next.recover();
    let written = sink.taken();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].verb, VERB);
    assert_eq!(written[0].app, "notes");
    assert_eq!(written[0].seconds(), Some(saved.last_seen - T0));
    assert_eq!(written[0].object, None, "the record names the app, nothing on it");
    assert!(!dir.path().join("open-stretch.json").exists(), "recovered once");
    next.recover();
    assert!(sink.taken().is_empty());
}

#[test]
fn the_file_is_removed_when_no_stretch_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let sink = Sink::default();
    let mut s = Stretches::new(dir.path().to_path_buf(), sink.boxed());
    s.observe(T0, page("notes", T0));
    assert!(dir.path().join("open-stretch.json").exists());
    beat_on(&mut s, "notes", T0, T0 + 100);
    s.observe(T0 + 104, None);
    assert!(!dir.path().join("open-stretch.json").exists());
    assert_eq!(sink.taken()[0].seconds(), Some(100));
}

#[test]
fn the_open_stretch_reads_as_an_open_record() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Stretches::new(dir.path().to_path_buf(), Sink::default().boxed());
    assert!(s.open_record().is_none());
    beat_on(&mut s, "notes", T0, T0 + 90);
    let open = s.open_record().unwrap();
    assert_eq!(open.detail, Some(serde_json::json!({ "seconds": 90, "open": true })));
}

/// A busy synthetic day: sixteen hours, the front page changing every two
/// to six minutes. Reports the size of the day's file.
#[test]
fn a_busy_day_is_a_small_file() {
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(super::super::activity::ActivityLog::new(dir.path().to_path_buf()));
    let into = log.clone();
    let mut s = Stretches::new(dir.path().to_path_buf(), Box::new(move |a| into.add(a)));
    let apps = ["notes", "tiles", "linggen", "sketch"];
    let mut now = T0;
    let end = T0 + 16 * 3600;
    let mut k = 0usize;
    while now < end {
        let length = 120 + (k as u64 * 37 % 240);
        let stop = now + length;
        while now < stop {
            s.observe(now, page(apps[k % apps.len()], now));
            now += 4;
        }
        k += 1;
    }
    s.observe(now + 400, None);
    let days = log.day_files();
    let bytes: u64 = days.iter().map(|(_, p)| std::fs::metadata(p).unwrap().len()).sum();
    let rows: usize = days.iter().map(|(_, p)| super::super::activity::read_day(p).len()).sum();
    eprintln!("busy day: {k} stretches, {rows} records, {bytes} bytes");
    assert_eq!(rows, k, "one record per stretch");
    assert!(bytes < 200 * 1024, "{bytes}");
}
