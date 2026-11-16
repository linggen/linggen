use super::*;
use std::cell::Cell;
use std::path::{Path, PathBuf};

thread_local! {
    /// Day files the last `answer` on this thread opened.
    pub(super) static DAYS_READ: Cell<usize> = const { Cell::new(0) };
}

fn t(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn rec(at: &str, app: &str, verb: &str, object: Option<&str>, secs: Option<u64>) -> Activity {
    Activity {
        at: at.into(),
        by: "user".into(),
        device: Some("test-mac".into()),
        app: app.into(),
        verb: verb.into(),
        object: object.map(String::from),
        detail: secs.map(|s| json!({ "seconds": s })),
    }
}

/// Files under `dir`, one per local day, as the log names them.
fn write(dir: &Path, rows: &[Activity]) -> Vec<(String, PathBuf)> {
    std::fs::create_dir_all(dir).unwrap();
    let mut days: BTreeMap<String, String> = BTreeMap::new();
    for r in rows {
        let day = day_of(r).unwrap();
        let line = serde_json::to_string(r).unwrap();
        days.entry(day).or_default().push_str(&format!("{line}\n"));
    }
    days.into_iter()
        .map(|(day, body)| {
            let p = dir.join(format!("{day}.jsonl"));
            std::fs::write(&p, body).unwrap();
            (day, p)
        })
        .collect()
}

fn filter(since: Option<&str>, until: &str) -> Filter {
    Filter {
        since: since.map(t),
        until: t(until),
        app: None,
        verb: None,
        text: None,
    }
}

fn run(f: Filter, rows: Option<usize>, totals: bool, days: Vec<(String, PathBuf)>) -> Value {
    DAYS_READ.with(|n| n.set(0));
    answer(
        &Ask { filter: f, rows, totals },
        Source { days, open: None, host: "test-mac", now: t("2026-11-10T12:00:00Z") },
    )
}

fn sample(dir: &Path) -> Vec<(String, PathBuf)> {
    write(
        dir,
        &[
            rec("2026-11-10T08:00:00Z", "notes", "used", None, Some(3600)),
            rec("2026-11-10T09:30:00Z", "notes", "archive", Some("Trip plan"), None),
            rec("2026-11-10T10:00:00Z", "tiles", "used", None, Some(1800)),
            rec("2026-11-10T10:40:00Z", "notes", "used", None, Some(600)),
            rec("2026-11-10T11:00:00Z", "tiles", "level_done", Some("Level 4"), None),
        ],
    )
}

fn verbs_of(rows: &Value) -> Vec<String> {
    rows.as_array()
        .unwrap()
        .iter()
        .map(|r| r["verb"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn rows_come_newest_first_and_each_filter_narrows_them() {
    let dir = tempfile::tempdir().unwrap();
    let days = sample(dir.path());
    let all = filter(None, "2026-11-10T12:00:00Z");

    let out = run(all.clone(), Some(20), false, days.clone());
    assert_eq!(verbs_of(&out["rows"]), ["level_done", "used", "used", "archive", "used"]);
    assert!(out.get("totals").is_none() && out.get("more").is_none());

    let app = Filter { app: Some("tiles".into()), ..all.clone() };
    assert_eq!(verbs_of(&run(app, Some(20), false, days.clone())["rows"]), ["level_done", "used"]);

    let verb = Filter { verb: Some("used".into()), ..all.clone() };
    assert_eq!(run(verb, Some(20), false, days.clone())["rows"].as_array().unwrap().len(), 3);

    let text = Filter { text: Some("trip".into()), ..all.clone() };
    assert_eq!(verbs_of(&run(text, Some(20), false, days.clone())["rows"]), ["archive"]);
    let in_detail = Filter { text: Some("1800".into()), ..all.clone() };
    assert_eq!(run(in_detail, Some(20), false, days.clone())["rows"][0]["app"], "tiles");

    let since = Filter { since: Some(t("2026-11-10T10:30:00Z")), ..all.clone() };
    assert_eq!(verbs_of(&run(since, Some(20), false, days.clone())["rows"]), ["level_done", "used"]);
    let until = filter(None, "2026-11-10T09:00:00Z");
    assert_eq!(verbs_of(&run(until, Some(20), false, days)["rows"]), ["used"]);
}

#[test]
fn a_limit_says_when_more_match() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(filter(None, "2026-11-10T12:00:00Z"), Some(2), false, sample(dir.path()));
    assert_eq!(out["rows"].as_array().unwrap().len(), 2);
    assert_eq!(out["more"], true);
}

#[test]
fn totals_count_and_sum_with_a_readable_form_and_the_span() {
    let dir = tempfile::tempdir().unwrap();
    let f = filter(Some("2026-11-10T00:00:00Z"), "2026-11-10T12:00:00Z");
    let out = run(f, None, true, sample(dir.path()));
    assert!(out.get("rows").is_none(), "totals alone unless a limit is asked");
    assert_eq!(out["span"]["until"], local(t("2026-11-10T12:00:00Z")));
    assert_eq!(out["span"]["since"], local(t("2026-11-10T00:00:00Z")));
    let tot = &out["totals"];
    assert_eq!(tot["records"], 5);
    let notes = &tot["apps"][0];
    assert_eq!(notes["app"], "notes");
    assert_eq!(notes["count"], 3);
    assert_eq!(notes["seconds"], 4200);
    assert_eq!(notes["time"], "1 h 10 min");
    assert_eq!(notes["verbs"][0], json!({ "verb": "archive", "count": 1 }));
    assert_eq!(notes["verbs"][1]["seconds"], 4200);
    let tiles = &tot["apps"][1];
    assert_eq!((tiles["seconds"].as_u64(), tiles["time"].as_str()), (Some(1800), Some("30 min")));
}

#[test]
fn a_stretch_counts_only_its_part_inside_the_span() {
    let dir = tempfile::tempdir().unwrap();
    // 08:00–09:00 cut by a span that starts at 08:45: fifteen minutes.
    let f = filter(Some("2026-11-10T08:45:00Z"), "2026-11-10T09:20:00Z");
    let out = run(f, Some(5), true, sample(dir.path()));
    assert_eq!(out["totals"]["apps"][0]["seconds"], 900);
    assert_eq!(out["rows"][0]["time"], "1 h", "the row is the record, whole");
}

#[test]
fn readable_durations() {
    assert_eq!(readable(0), "0 s");
    assert_eq!(readable(45), "45 s");
    assert_eq!(readable(720), "12 min");
    assert_eq!(readable(7200), "2 h");
    assert_eq!(readable(7800), "2 h 10 min");
    assert_eq!(readable(400 * 3600 + 60), "400 h 1 min");
}

#[test]
fn rows_are_cut_at_the_output_cap_with_a_note_and_totals_stay_whole() {
    let dir = tempfile::tempdir().unwrap();
    let long = "x".repeat(60);
    let rows: Vec<Activity> = (0..400)
        .map(|i| {
            let at = t("2026-11-10T00:00:00Z") + Duration::seconds(i * 60);
            rec(&at.to_rfc3339(), "notes", "edit", Some(&long), None)
        })
        .collect();
    let days = write(dir.path(), &rows);
    let out = run(filter(None, "2026-11-10T12:00:00Z"), Some(ROWS_MAX), true, days);
    let shown = out["rows"].as_array().unwrap().len();
    assert!(shown < ROWS_MAX && shown > 10, "{shown}");
    assert!(out["note"].as_str().unwrap().contains("cut"));
    assert!(out["rows"].to_string().len() <= ROWS_MAX_BYTES);
    assert_eq!(out["totals"]["records"], 400, "the cap cuts rows, never totals");
}

#[test]
fn the_open_stretch_is_read_with_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let mut open = rec("2026-11-10T11:30:00Z", "tiles", "used", None, Some(1200));
    open.detail = Some(json!({ "seconds": 1200, "open": true }));
    let out = answer(
        &Ask { filter: filter(None, "2026-11-10T12:00:00Z"), rows: Some(1), totals: true },
        Source { days: sample(dir.path()), open: Some(open), host: "test-mac", now: t("2026-11-10T12:00:00Z") },
    );
    assert_eq!(out["rows"][0]["detail"]["open"], true);
    assert_eq!(out["totals"]["apps"][1]["seconds"], 3000);
}

/// 400 days on disk; a rows-only read of the newest opens only what it
/// needs, and totals over every day stay quick.
#[test]
fn reading_many_days_streams_them_one_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let start = t("2025-10-06T12:00:00Z");
    let mut rows = Vec::new();
    for d in 0..400 {
        for k in 0..60 {
            let at = start + Duration::days(d) + Duration::minutes(k * 10);
            let app = ["notes", "tiles", "linggen"][k as usize % 3];
            rows.push(rec(&at.to_rfc3339(), app, "used", None, Some(300)));
        }
    }
    let days = write(dir.path(), &rows);
    assert!(days.len() >= 400);

    let until = "2026-11-10T12:00:00Z";
    run(filter(None, until), Some(5), false, days.clone());
    assert_eq!(DAYS_READ.with(Cell::get), 1, "five rows need one day");

    let began = std::time::Instant::now();
    let out = run(filter(None, until), None, true, days);
    let took = began.elapsed();
    eprintln!("totals over {} days, 24000 records: {took:?}", DAYS_READ.with(Cell::get));
    assert_eq!(out["totals"]["records"], 24_000);
    assert_eq!(out["totals"]["apps"][0]["seconds"], 8000 * 300);
    assert!(took < std::time::Duration::from_secs(10), "{took:?}");
}
