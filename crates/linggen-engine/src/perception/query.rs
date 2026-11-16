//! Reading the log: rows filtered by time, app, verb and text, and totals over
//! the same span. One compact JSON shape, read by `recent_activity` and by
//! `GET /api/activity`.
//!
//! Day files are streamed one at a time, newest first, so a long span never
//! sits in memory whole. Totals read the whole span; rows stop at the limit or
//! at [`ROWS_MAX_BYTES`], and the answer says when they were cut. A record
//! with a duration (`detail.seconds`) counts toward a span by the part of it
//! inside the span, so a stretch across midnight is split between the days.

use super::activity::{ago, read_day, Activity};
use chrono::{DateTime, Duration, Local, Utc};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// Rows stop here, in bytes of JSON. Well under the tool output ceiling, with
/// room for the totals beside them.
pub const ROWS_MAX_BYTES: usize = 16 * 1024;
/// Rows a caller may ask for at once.
pub const ROWS_MAX: usize = 200;
/// Totals list at most this many apps, and this many verbs under each.
pub const APPS_MAX: usize = 40;
pub const VERBS_MAX: usize = 12;

/// Which records.
#[derive(Debug, Clone)]
pub struct Filter {
    pub since: Option<DateTime<Utc>>,
    pub until: DateTime<Utc>,
    pub app: Option<String>,
    pub verb: Option<String>,
    /// Lower-cased; matched inside `object` and the JSON of `detail`.
    pub text: Option<String>,
}

/// What to return for the records the filter keeps.
#[derive(Debug, Clone)]
pub struct Ask {
    pub filter: Filter,
    /// How many rows, newest first; `None` returns none.
    pub rows: Option<usize>,
    pub totals: bool,
}

impl Filter {
    /// `[start, end)` of a record: a point for an event, a stretch for a
    /// record that carries a duration.
    fn interval(a: &Activity) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let start = a.at_utc()?;
        let secs = a.seconds().unwrap_or(0) as i64;
        Some((start, start + Duration::seconds(secs)))
    }

    fn in_span(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
        let after = match self.since {
            Some(s) if end > start => end > s,
            Some(s) => start >= s,
            None => true,
        };
        after && start < self.until
    }

    fn matches(&self, a: &Activity) -> bool {
        let Some((start, end)) = Self::interval(a) else {
            return false;
        };
        self.in_span(start, end)
            && self.app.as_ref().is_none_or(|x| &a.app == x)
            && self.verb.as_ref().is_none_or(|x| &a.verb == x)
            && self.text.as_ref().is_none_or(|t| mentions(a, t))
    }

    /// Seconds of a record's duration inside the span.
    fn seconds_inside(&self, a: &Activity) -> Option<u64> {
        a.seconds()?;
        let (start, end) = Self::interval(a)?;
        let from = self.since.map_or(start, |s| start.max(s));
        let to = end.min(self.until);
        Some((to - from).num_seconds().max(0) as u64)
    }

    /// The day files that can hold a matching record. One day of slack on
    /// each side: files are named by local day, and a stretch that began
    /// before `since` may still reach into the span.
    fn holds_day(&self, day: &str) -> bool {
        let key = |t: DateTime<Utc>, slack: i64| {
            (t.with_timezone(&Local) + Duration::days(slack))
                .format("%Y-%m-%d")
                .to_string()
        };
        let low = self.since.is_none_or(|s| day >= key(s, -1).as_str());
        low && day <= key(self.until, 1).as_str()
    }
}

fn mentions(a: &Activity, text: &str) -> bool {
    let in_object = a
        .object
        .as_ref()
        .is_some_and(|o| o.to_lowercase().contains(text));
    let in_detail = a
        .detail
        .as_ref()
        .is_some_and(|d| d.to_string().to_lowercase().contains(text));
    in_object || in_detail
}

/// "45 s", "12 min", "2 h 10 min" — a duration a model reads, never adds.
pub fn readable(secs: u64) -> String {
    let (h, m) = (secs / 3600, secs % 3600 / 60);
    match (h, m) {
        (0, 0) => format!("{secs} s"),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

fn local(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

#[derive(Default)]
struct Tally {
    count: u64,
    seconds: Option<u64>,
}

impl Tally {
    fn add(&mut self, seconds: Option<u64>) {
        self.count += 1;
        if let Some(s) = seconds {
            *self.seconds.get_or_insert(0) += s;
        }
    }

    fn json(&self, key: &str, name: &str) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert(key.into(), json!(name));
        m.insert("count".into(), json!(self.count));
        if let Some(s) = self.seconds {
            m.insert("seconds".into(), json!(s));
            m.insert("time".into(), json!(readable(s)));
        }
        m
    }
}

#[derive(Default)]
struct Totals {
    records: u64,
    apps: BTreeMap<String, (Tally, BTreeMap<String, Tally>)>,
}

impl Totals {
    fn add(&mut self, a: &Activity, seconds: Option<u64>) {
        self.records += 1;
        let (app, verbs) = self.apps.entry(a.app.clone()).or_default();
        app.add(seconds);
        verbs.entry(a.verb.clone()).or_default().add(seconds);
    }

    fn json(&self) -> Value {
        let apps: Vec<Value> = self
            .apps
            .iter()
            .take(APPS_MAX)
            .map(|(name, (tally, verbs))| {
                let mut m = tally.json("app", name);
                let list: Vec<Value> = verbs
                    .iter()
                    .take(VERBS_MAX)
                    .map(|(v, t)| Value::Object(t.json("verb", v)))
                    .collect();
                m.insert("verbs".into(), Value::Array(list));
                if verbs.len() > VERBS_MAX {
                    m.insert("verbs_cut".into(), json!(format!("{} of {} verbs shown", VERBS_MAX, verbs.len())));
                }
                Value::Object(m)
            })
            .collect();
        let mut out = json!({ "records": self.records, "apps": apps });
        if self.apps.len() > APPS_MAX {
            out["apps_cut"] = json!(format!("{} of {} apps shown", APPS_MAX, self.apps.len()));
        }
        out
    }
}

/// Rows collected newest first, under the byte cap.
struct Rows {
    limit: usize,
    host: String,
    now: DateTime<Utc>,
    list: Vec<Value>,
    bytes: usize,
    more: bool,
    cut: bool,
}

impl Rows {
    fn full(&self) -> bool {
        self.more || self.cut
    }

    fn offer(&mut self, a: &Activity) {
        if self.full() {
            return;
        }
        if self.list.len() >= self.limit {
            self.more = true;
            return;
        }
        let row = row(a, &self.host, self.now);
        let size = row.to_string().len() + 1;
        if self.bytes + size > ROWS_MAX_BYTES {
            self.cut = true;
            return;
        }
        self.bytes += size;
        self.list.push(row);
    }
}

fn row(a: &Activity, host: &str, now: DateTime<Utc>) -> Value {
    let mut m = Map::new();
    if let Some(at) = a.at_utc() {
        m.insert("at".into(), json!(local(at)));
        m.insert("ago".into(), json!(ago((now - at).num_seconds())));
    }
    m.insert("by".into(), json!(a.by));
    m.insert("app".into(), json!(a.app));
    m.insert("verb".into(), json!(a.verb));
    if let Some(o) = &a.object {
        m.insert("object".into(), json!(o));
    }
    if let Some(d) = a.device.as_ref().filter(|d| d.as_str() != host) {
        m.insert("device".into(), json!(d));
    }
    if let Some(s) = a.seconds() {
        m.insert("time".into(), json!(readable(s)));
    }
    if let Some(d) = &a.detail {
        m.insert("detail".into(), d.clone());
    }
    Value::Object(m)
}

/// Where the log and the moment come from — a test hands its own.
pub struct Source<'a> {
    pub days: Vec<(String, std::path::PathBuf)>,
    /// A record not yet on disk (the stretch still open), read with the rest.
    pub open: Option<Activity>,
    pub host: &'a str,
    pub now: DateTime<Utc>,
}

/// Answer one ask over the log.
pub fn answer(ask: &Ask, src: Source<'_>) -> Value {
    let f = &ask.filter;
    let mut rows = Rows {
        limit: ask.rows.unwrap_or(0).min(ROWS_MAX),
        host: src.host.to_string(),
        now: src.now,
        list: Vec::new(),
        bytes: 0,
        more: false,
        cut: false,
    };
    let mut totals = Totals::default();
    let first_day = src.days.first().map(|(d, _)| d.clone());
    let open_day = src.open.as_ref().and_then(day_of);

    let mut days: Vec<(String, Option<std::path::PathBuf>)> = src
        .days
        .into_iter()
        .filter(|(d, _)| f.holds_day(d))
        .map(|(d, p)| (d, Some(p)))
        .collect();
    if let Some(d) = open_day.as_ref().filter(|d| !days.iter().any(|(x, _)| x == *d)) {
        days.push((d.clone(), None));
        days.sort_by(|a, b| a.0.cmp(&b.0));
    }

    let mut open = src.open;
    for (day, path) in days.into_iter().rev() {
        if !ask.totals && (rows.full() || ask.rows.is_none()) {
            break;
        }
        #[cfg(test)]
        tests::DAYS_READ.with(|n| n.set(n.get() + 1));
        let mut batch = path.map(|p| read_day(&p)).unwrap_or_default();
        if open_day.as_deref() == Some(day.as_str()) {
            batch.extend(open.take());
        }
        // A file is in write order; a reader wants newest first.
        batch.sort_by(|a, b| b.at.cmp(&a.at));
        for a in batch.iter().filter(|a| f.matches(a)) {
            totals.add(a, f.seconds_inside(a));
            rows.offer(a);
        }
    }

    let since = f
        .since
        .map(local)
        .or_else(|| first_day.and_then(|d| start_of_day(&d)))
        .map_or(Value::Null, Value::String);
    let span = json!({ "since": since, "until": local(f.until) });
    let mut out = json!({ "span": span });
    if ask.rows.is_some() {
        out["rows"] = Value::Array(rows.list);
    }
    if rows.more {
        out["more"] = json!(true);
    }
    if rows.cut {
        out["note"] = json!(format!(
            "rows cut at {} KB of output; narrow the span or filters, or ask for totals",
            ROWS_MAX_BYTES / 1024
        ));
    }
    if ask.totals {
        out["totals"] = totals.json();
    }
    out
}

/// The arguments as a caller writes them — the tool's and the read route's.
#[derive(Debug, Default, serde::Deserialize)]
pub struct Args {
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub until: Option<String>,
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub verb: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub totals: bool,
}

/// Rows a plain call returns.
pub const ROWS_DEFAULT: usize = 20;

impl Args {
    fn filtered(&self) -> bool {
        self.since.is_some()
            || self.until.is_some()
            || self.app.is_some()
            || self.verb.is_some()
            || self.text.is_some()
            || self.totals
    }

    /// What this asks, read at `now` in the local zone.
    pub fn ask(&self, now: DateTime<Local>) -> Result<Ask, String> {
        let at = |name: &str, v: &Option<String>| {
            v.as_deref()
                .map(|s| super::when::resolve(s, &now).map_err(|e| format!("{name}: {e}")))
                .transpose()
        };
        let word = |v: &Option<String>| v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
        let rows = match (self.limit, self.totals) {
            (Some(n), _) => Some(n.clamp(1, ROWS_MAX)),
            (None, true) => None,
            (None, false) => Some(ROWS_DEFAULT),
        };
        Ok(Ask {
            filter: Filter {
                since: at("since", &self.since)?,
                until: at("until", &self.until)?.unwrap_or_else(|| now.with_timezone(&Utc)),
                app: word(&self.app),
                verb: word(&self.verb),
                text: word(&self.text).map(|t| t.to_lowercase()),
            },
            rows,
            totals: self.totals,
        })
    }
}

/// Read this machine's log as `args` asks. A plain call (no filter, no
/// totals) also carries the other device's latest lines, as it last
/// published them: those are text, so no filter or total reads them.
pub fn read(args: &Args) -> Value {
    let now = Local::now();
    let ask = match args.ask(now) {
        Ok(a) => a,
        Err(e) => return json!({ "error": e }),
    };
    let host = super::host_name();
    let log = super::activity::log();
    let src = Source {
        days: log.day_files(),
        open: super::stretch::open_record(),
        host: &host,
        now: now.with_timezone(&Utc),
    };
    let mut out = answer(&ask, src);
    if !args.filtered() {
        let (name, lines) = super::state::peer_history();
        if !lines.is_empty() {
            let n = ask.rows.unwrap_or(ROWS_DEFAULT);
            out["other_device"] = json!({
                "name": name.unwrap_or_else(|| "their other device".into()),
                "lines": lines.into_iter().take(n).collect::<Vec<_>>(),
            });
        }
    }
    out
}

fn day_of(a: &Activity) -> Option<String> {
    Some(a.at_utc()?.with_timezone(&Local).format("%Y-%m-%d").to_string())
}

fn start_of_day(day: &str) -> Option<String> {
    let d = super::activity::parse_day(day)?;
    super::when::start_of(&Local, d).map(local)
}

#[cfg(test)]
mod tests;
