//! The day-state sidecar, `<data_dir>/memory/.days.json`, and local-day
//! attribution: a row belongs to the local calendar day of its
//! `effective_timestamp()`. The `days` verbs (`days`, `remember_day`,
//! `harvest_day`, `sweep`) are here too; `http::days` only adapts them.

use crate::error::MemError;
use crate::handle::Memory;
use crate::memory::{Filters, Memory as Row, SortOrder};
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One day's dream record. Counts accumulate across re-remembers of the
/// same day (late-arriving rows re-pend a day; a follow-up pass adds to
/// the totals rather than overwriting them).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DayRecord {
    /// When a harvest (session backfill) pass last covered this day.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub harvested_at: Option<DateTime<Utc>>,
    /// When a remember pass last judged this day's rows. The judged set
    /// is exactly the rows with `created_at < remembered_at`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub remembered_at: Option<DateTime<Utc>>,
    /// Total rows judged across all remember passes.
    #[serde(default)]
    pub judged: u32,
    /// Total rows promoted to semantic across all remember passes.
    #[serde(default)]
    pub promoted: u32,
    /// Total rows evicted by the forget sweep.
    #[serde(default)]
    pub forgotten: u32,
}

/// The `.days.json` sidecar — `{"days": {"YYYY-MM-DD": record}}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaysState {
    #[serde(default)]
    pub days: BTreeMap<String, DayRecord>,
}

pub(crate) fn days_path(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join(".days.json")
}

/// Read-or-default, mirroring `config::load` — a missing/corrupt file
/// never blocks the store.
pub(crate) async fn load(data_dir: &Path) -> DaysState {
    let Ok(bytes) = tokio::fs::read(days_path(data_dir)).await else {
        return DaysState::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// The one lock around every change to `.days.json`. Its writers:
/// `remember_day`, `harvest_day` and the sweep's forgotten counts, each a
/// read-change-save through [`modify`]; and a replica's pull, which
/// replaces the file with the primary's copy under [`lock`].
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) async fn lock() -> tokio::sync::MutexGuard<'static, ()> {
    LOCK.lock().await
}

/// Read, change and save `.days.json` under [`lock`].
async fn modify<R>(data_dir: &Path, f: impl FnOnce(&mut DaysState) -> R) -> Result<R, MemError> {
    let _guard = lock().await;
    let mut state = load(data_dir).await;
    let out = f(&mut state);
    save(data_dir, &state)
        .await
        .map_err(|e| MemError::internal(anyhow::anyhow!(e)))?;
    Ok(out)
}

async fn save(data_dir: &Path, state: &DaysState) -> Result<(), std::io::Error> {
    let path = days_path(data_dir);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(state).expect("DaysState serializes");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(tmp, path).await?;
    Ok(())
}

/// Local calendar day of a UTC instant, as the canonical `YYYY-MM-DD` key.
pub(crate) fn local_day(ts: DateTime<Utc>) -> String {
    ts.with_timezone(&Local).format("%Y-%m-%d").to_string()
}

pub(crate) fn today_local() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// Parse a `YYYY-MM-DD` day key, rejecting anything else.
pub(crate) fn parse_day(s: &str) -> Result<NaiveDate, MemError> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|_| MemError::bad_request(format!("invalid day {s:?}: expected YYYY-MM-DD")))
}

/// A day as the one spelling every sidecar keys on (`2026-1-5` →
/// `2026-01-05`); a string that is not a date is refused. Every verb that
/// takes a day and looks it up or stores it goes through here.
pub(crate) fn day_key(s: &str) -> Result<String, MemError> {
    Ok(parse_day(s)?.format("%Y-%m-%d").to_string())
}

/// UTC bounds `[start, end)` of a local calendar day. DST folds/gaps
/// resolve to the earliest valid instant.
pub(crate) fn local_day_bounds(day: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
    let start_naive = day.and_hms_opt(0, 0, 0).expect("midnight exists");
    let end_naive = (day + chrono::Days::new(1))
        .and_hms_opt(0, 0, 0)
        .expect("midnight exists");
    let to_utc = |naive| {
        Local
            .from_local_datetime(&naive)
            .earliest()
            .map(|dt| dt.with_timezone(&Utc))
            // A DST gap at midnight (rare, but real in some zones):
            // fall back to interpreting the naive time as UTC.
            .unwrap_or_else(|| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
    };
    (to_utc(start_naive), to_utc(end_naive))
}

/// Pull every episodic row. Episodic is bounded by design (per-turn
/// staging drained by the pipeline), so a full scan per rollup/sweep is
/// cheap — same pattern `forget` uses for origin-filtered deletes.
async fn all_episodic(state: &Memory) -> Result<Vec<Row>, MemError> {
    Ok(state
        .episodic
        .list(&Filters::default(), SortOrder::Oldest, usize::MAX, 0)
        .await?)
}

// ── days (rollup) ───────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct DaysRequest {
    /// Only days that need a dream pass (past day, unjudged rows),
    /// oldest first — the dream mission's worklist shape.
    /// `pending_only` is the pre-flags spelling, kept as an alias.
    #[serde(default, alias = "pending_only")]
    pub undreamed_only: bool,
    /// Inclusive `YYYY-MM-DD` bounds on the returned days. Omit for all
    /// days that carry any data.
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Debug, Default)]
struct DayLive {
    rows: usize,
    unjudged: usize,
    past_ttl: usize,
}

/// Per-day pipeline flags, aligned 1:1 with the user-facing verbs:
/// `scanned` — a scan (session backfill) walked this day's logs;
/// `dreamed` — a remember pass judged the day and nothing new arrived
/// since (late rows clear the flag and the day rejoins the worklist).
/// Sweep is invisible mechanics: a forgotten day stays `dreamed`.
fn day_flags(live: &DayLive, rec: Option<&DayRecord>) -> (bool, bool) {
    let scanned = rec.and_then(|r| r.harvested_at).is_some();
    let dreamed = rec.and_then(|r| r.remembered_at).is_some() && live.unjudged == 0;
    (scanned, dreamed)
}

/// A past day with rows awaiting judgment — the dream worklist test.
fn undreamed(date: &str, today: &str, live: &DayLive) -> bool {
    date < today && live.unjudged > 0
}

pub(in crate::ops) async fn days(state: &Memory, req: DaysRequest) -> Result<Value, MemError> {
    let from = req.from.as_deref().map(parse_day).transpose()?;
    let to = req.to.as_deref().map(parse_day).transpose()?;

    let days_state = load(&state.data_dir).await;
    let cfg = crate::ops::config::load(&state.data_dir).await;
    let ttl_cutoff = Utc::now() - chrono::Duration::days(cfg.episodic_ttl_days as i64);

    // Rows per local day across BOTH tables — which days hold rows at
    // all. Feeds the console calendar's "rows ▸" jump; kept OUT of the
    // dream-pipeline `days` union below so a semantic row from May never
    // creates a phantom scan/dream worklist day. Counted on the same
    // COALESCE(occurred_at, created_at) clock the `day` list filter
    // matches, so the jump lands on exactly this many rows.
    let mut row_days: BTreeMap<String, u32> = BTreeMap::new();
    for row in state
        .store
        .list(&Filters::default(), SortOrder::Oldest, usize::MAX, 0)
        .await?
    {
        *row_days
            .entry(local_day(row.effective_timestamp()))
            .or_default() += 1;
    }

    // Bucket live episodic rows per local day.
    let mut live: BTreeMap<String, DayLive> = BTreeMap::new();
    for row in all_episodic(state).await? {
        let day = local_day(row.effective_timestamp());
        *row_days.entry(day.clone()).or_default() += 1;
        let judged = days_state
            .days
            .get(&day)
            .and_then(|r| r.remembered_at)
            .is_some_and(|ra| row.created_at < ra);
        let bucket = live.entry(day).or_default();
        bucket.rows += 1;
        if !judged {
            bucket.unjudged += 1;
        }
        // The one TTL clock — `COALESCE(occurred_at, created_at)`, the same
        // as `list past_ttl` and the sweep.
        if row.effective_timestamp() < ttl_cutoff {
            bucket.past_ttl += 1;
        }
    }

    // Union of days with live rows and days with stored records.
    let mut dates: Vec<String> = live.keys().chain(days_state.days.keys()).cloned().collect();
    dates.sort();
    dates.dedup();

    let today = today_local();
    let mut out = Vec::new();
    // "First day the user's next verb applies to" plus summary counts —
    // computed over ALL past days (today can't be scanned/dreamed yet),
    // deliberately ignoring the from/to window and the worklist filter,
    // so status surfaces get the global truth.
    let mut first_unscanned: Option<String> = None;
    let mut first_undreamed: Option<String> = None;
    let mut total_days = 0u32;
    let mut scanned_days = 0u32;
    let mut dreamed_days = 0u32;
    for date in dates {
        let empty = DayLive::default();
        let l = live.get(&date).unwrap_or(&empty);
        let rec = days_state.days.get(&date);
        let (scanned, dreamed) = day_flags(l, rec);
        if date.as_str() < today.as_str() {
            total_days += 1;
            if scanned {
                scanned_days += 1;
            } else if first_unscanned.is_none() {
                first_unscanned = Some(date.clone());
            }
            if dreamed {
                dreamed_days += 1;
            }
            if undreamed(&date, &today, l) && first_undreamed.is_none() {
                first_undreamed = Some(date.clone());
            }
        }
        if let Some(f) = from {
            if parse_day(&date).map(|d| d < f).unwrap_or(false) {
                continue;
            }
        }
        if let Some(t) = to {
            if parse_day(&date).map(|d| d > t).unwrap_or(false) {
                continue;
            }
        }
        if req.undreamed_only && !undreamed(&date, &today, l) {
            continue;
        }
        out.push(json!({
            "date": date,
            "scanned": scanned,
            "dreamed": dreamed,
            "rows": l.rows,
            "unjudged": l.unjudged,
            "past_ttl": l.past_ttl,
            "harvested_at": rec.and_then(|r| r.harvested_at),
            "remembered_at": rec.and_then(|r| r.remembered_at),
            "judged": rec.map(|r| r.judged).unwrap_or(0),
            "promoted": rec.map(|r| r.promoted).unwrap_or(0),
            "forgotten": rec.map(|r| r.forgotten).unwrap_or(0),
        }));
    }

    // Open review items ride along so one `days` call feeds every
    // "is memory upkeep due?" surface (recall footer, dream status).
    let open_issues = crate::ops::issues::open_count(&state.data_dir).await;

    Ok(json!({
        "today": today,
        "ttl_days": cfg.episodic_ttl_days,
        "open_issues": open_issues,
        "first_unscanned": first_unscanned,
        "first_undreamed": first_undreamed,
        "total_days": total_days,
        "scanned_days": scanned_days,
        "dreamed_days": dreamed_days,
        "row_days": row_days,
        "days": out,
    }))
}

// ── remember_day (stamp) ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RememberDayRequest {
    /// Local calendar day, `YYYY-MM-DD`.
    pub date: String,
    /// Rows judged in this pass. Accumulates onto the day's total.
    #[serde(default)]
    pub judged: u32,
    /// Rows promoted to semantic in this pass. Accumulates.
    #[serde(default)]
    pub promoted: u32,
    /// Also stamp `harvested_at` — set by a harvest (session backfill)
    /// pass that covered this day.
    #[serde(default)]
    pub harvested: bool,
    /// Who is dreaming (the holder of the day's dream lease, if one was
    /// taken). A day another holder has leased is refused.
    #[serde(default)]
    pub holder: Option<String>,
}

pub(in crate::ops) async fn remember_day(state: &Memory, req: RememberDayRequest) -> Result<Value, MemError> {
    let key = day_key(&req.date)?;
    let today = today_local();
    if key >= today {
        return Err(MemError::bad_request(format!(
            "day {key} is not over yet — only past local days can be remembered"
        )));
    }

    let holder = req.holder.as_deref().unwrap_or("");
    let now = Utc::now();
    if let Some(l) = crate::ops::lease::held_by_other(&state.data_dir, &key, holder, now) {
        return Err(crate::ops::lease::busy(&key, &l));
    }
    // The mark covers what the caller read: rows from before its lease on
    // the day (taken before the worklist read), so a row filed meanwhile
    // stays unjudged. Without a lease the caller's read is unknown and the
    // mark is the stamp's own time. It never moves back.
    let boundary = crate::ops::lease::read_boundary(&state.data_dir, &key, holder, now)
        .map_or(now, |b| b.min(now));

    modify(&state.data_dir, |days_state| {
        let rec = days_state.days.entry(key.clone()).or_default();
        rec.remembered_at = Some(rec.remembered_at.map_or(boundary, |prev| prev.max(boundary)));
        rec.judged = rec.judged.saturating_add(req.judged);
        rec.promoted = rec.promoted.saturating_add(req.promoted);
        if req.harvested {
            rec.harvested_at = Some(Utc::now());
        }
        json!({ "date": key, "record": rec })
    })
    .await
}

// ── harvest_day (scan stamp) ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct HarvestDayRequest {
    /// Local calendar day, `YYYY-MM-DD`.
    pub date: String,
}

/// Stamp `harvested_at` only — a scan (session backfill) covered this
/// day. Deliberately does NOT touch `remembered_at`: scanning stages
/// rows; the day then goes pending and a dream pass judges it.
pub(in crate::ops) async fn harvest_day(state: &Memory, req: HarvestDayRequest) -> Result<Value, MemError> {
    let key = day_key(&req.date)?;
    let today = today_local();
    if key >= today {
        return Err(MemError::bad_request(format!(
            "day {key} is not over yet — only past local days can be scanned"
        )));
    }

    modify(&state.data_dir, |days_state| {
        let rec = days_state.days.entry(key.clone()).or_default();
        rec.harvested_at = Some(Utc::now());
        json!({ "date": key, "record": rec })
    })
    .await
}

// ── sweep (forget) ──────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct SweepRequest {
    /// Report what would be evicted without deleting anything.
    #[serde(default)]
    pub dry_run: bool,
}

pub(in crate::ops) async fn sweep(state: &Memory, req: SweepRequest) -> Result<Value, MemError> {
    let days_state = load(&state.data_dir).await;
    let cfg = crate::ops::config::load(&state.data_dir).await;
    let ttl_cutoff = Utc::now() - chrono::Duration::days(cfg.episodic_ttl_days as i64);

    // A row is evictable iff it is past TTL (the one TTL clock,
    // `occurred_at ?? created_at` — the same as `list past_ttl` and the days
    // rollup), its day was remembered, AND it was created before the stamp
    // (i.e. a remember pass judged it). Nothing is ever deleted un-judged.
    let mut victims: Vec<(String, String)> = Vec::new(); // (id, day)
    for row in all_episodic(state).await? {
        if row.effective_timestamp() >= ttl_cutoff {
            continue;
        }
        let day = local_day(row.effective_timestamp());
        let Some(ra) = days_state.days.get(&day).and_then(|r| r.remembered_at) else {
            continue;
        };
        if row.created_at < ra {
            victims.push((row.id.clone(), day));
        }
    }

    let mut per_day: BTreeMap<String, u32> = BTreeMap::new();
    let mut removed = 0usize;
    for (id, day) in &victims {
        if !req.dry_run && !state.episodic.delete(id).await? {
            continue; // already gone (concurrent delete) — not an error
        }
        removed += 1;
        *per_day.entry(day.clone()).or_default() += 1;
    }

    // The counts go onto the file as it is now, not the snapshot read
    // before the deletes: a mark stamped meanwhile stays.
    if !req.dry_run && removed > 0 {
        modify(&state.data_dir, |now| {
            for (day, n) in &per_day {
                let rec = now.days.entry(day.clone()).or_default();
                rec.forgotten = rec.forgotten.saturating_add(*n);
            }
        })
        .await?;
    }

    Ok(json!({
        "removed": removed,
        "dry_run": req.dry_run,
        "days": per_day,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut s = DaysState::default();
            s.days.insert(
                "2026-07-01".into(),
                DayRecord {
                    remembered_at: Some(Utc::now()),
                    judged: 12,
                    promoted: 3,
                    ..Default::default()
                },
            );
            save(dir.path(), &s).await.unwrap();
            let loaded = load(dir.path()).await;
            let r = loaded.days.get("2026-07-01").unwrap();
            assert_eq!(r.judged, 12);
            assert_eq!(r.promoted, 3);
            assert!(r.remembered_at.is_some());
            assert!(r.harvested_at.is_none());
        });
    }

    /// Concurrent stamps of different days all land: no save carries a
    /// snapshot that misses another's mark.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_stamps_keep_every_mark() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::handle::scratch_state(
            dir.path(),
            std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        )
        .await;
        let days: Vec<String> = (2..42)
            .map(|n| (Local::now() - chrono::Duration::days(n)).format("%Y-%m-%d").to_string())
            .collect();
        let calls = days.iter().enumerate().map(|(i, day)| {
            let state = std::sync::Arc::clone(&state);
            let date = day.clone();
            tokio::spawn(async move {
                if i % 2 == 0 {
                    let req = RememberDayRequest { date, judged: 1, promoted: 0, harvested: false, holder: None };
                    remember_day(&state, req).await.map(drop)
                } else {
                    harvest_day(&state, HarvestDayRequest { date }).await.map(drop)
                }
            })
        });
        for call in calls.collect::<Vec<_>>() {
            call.await.unwrap().unwrap();
        }
        let saved = load(dir.path()).await;
        let missing: Vec<&String> = days.iter().filter(|d| !saved.days.contains_key(*d)).collect();
        assert!(missing.is_empty(), "lost marks: {missing:?}");
    }

    #[test]
    fn missing_file_loads_default() {
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let loaded = load(dir.path()).await;
            assert!(loaded.days.is_empty());
        });
    }

    #[test]
    fn local_day_bounds_cover_exactly_one_day() {
        let d = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        let (start, end) = local_day_bounds(d);
        assert_eq!(end - start, chrono::Duration::days(1));
        // The local rendering of the start instant is the day itself.
        assert_eq!(local_day(start), "2026-07-01");
    }

    #[test]
    fn parse_day_rejects_garbage() {
        assert!(parse_day("2026-07-01").is_ok());
        // chrono's %m/%d tolerate missing zero-padding — fine leniency.
        assert!(parse_day("2026-7-1").is_ok());
        assert!(parse_day("yesterday").is_err());
        assert!(parse_day("2026-13-40").is_err());
        assert!(parse_day("").is_err());
    }

    fn rec(remembered: bool) -> DayRecord {
        DayRecord {
            remembered_at: remembered.then(Utc::now),
            ..Default::default()
        }
    }

    #[test]
    fn today_and_future_days_are_never_undreamed() {
        let live = DayLive {
            rows: 5,
            unjudged: 5,
            past_ttl: 0,
        };
        assert!(!undreamed("2026-07-03", "2026-07-03", &live));
        assert!(!undreamed("2026-07-09", "2026-07-03", &live));
    }

    #[test]
    fn past_day_with_unjudged_rows_is_undreamed() {
        let live = DayLive {
            rows: 3,
            unjudged: 3,
            past_ttl: 0,
        };
        assert!(undreamed("2026-07-01", "2026-07-03", &live));
        assert_eq!(day_flags(&live, None), (false, false));
    }

    #[test]
    fn late_rows_clear_the_dreamed_flag() {
        let live = DayLive {
            rows: 4,
            unjudged: 1,
            past_ttl: 0,
        };
        let r = rec(true);
        assert_eq!(day_flags(&live, Some(&r)), (false, false));
        assert!(undreamed("2026-07-01", "2026-07-03", &live));
    }

    #[test]
    fn dreamed_survives_the_forget_sweep() {
        let r = rec(true);
        let with_rows = DayLive {
            rows: 4,
            unjudged: 0,
            past_ttl: 2,
        };
        assert_eq!(day_flags(&with_rows, Some(&r)), (false, true));
        // Sweep drained the rows — the day stays dreamed.
        let drained = DayLive::default();
        assert_eq!(day_flags(&drained, Some(&r)), (false, true));
    }

    #[test]
    fn scanned_only_record_is_not_dreamed_and_not_worklisted() {
        let r = DayRecord {
            harvested_at: Some(Utc::now()),
            ..Default::default()
        };
        let drained = DayLive::default();
        assert_eq!(day_flags(&drained, Some(&r)), (true, false));
        assert!(!undreamed("2026-06-20", "2026-07-03", &drained));
    }
}
