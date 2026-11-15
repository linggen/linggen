//! The weather sense: the current weather where the person is, from
//! Open-Meteo (keyless, public: geocoding + forecast). The place finds itself:
//! with no city typed, the engine asks linggen.dev's `/api/geo` (Cloudflare's
//! idea of the caller's IP — no OS location permission, no third party, the
//! site stores nothing) and reads the weather there. A city the person types
//! wins and is kept; clearing it goes back to automatic. Off means nothing is
//! fetched at all, the place lookup included.
//!
//! - Kept in `~/.linggen/senses/weather.json`: the place (typed and geocoded,
//!   or found by IP: `auto: true` with `located_at`), the off switch, and the
//!   last reading (so a restart has it at once).
//! - An automatic place is looked up again after [`RELOCATE`] — a laptop that
//!   travelled finds its new city within hours, while the site is asked a few
//!   times a day at most. A failed lookup waits [`LOCATE_RETRY`].
//! - A reading is fresh for [`TTL`]; a stale one is still handed over while a
//!   refresh runs behind — a tool never waits on the network. One fetch runs
//!   at a time, and a place is tried at most once per [`RETRY`] (a failed
//!   fetch included), so a dead network is not asked on every tool call.
//! - A reading belongs to the place it was read at (its coordinates), so two
//!   cities of one name never share one.
//! - To a declaring skill's tool: `LINGGEN_WEATHER`, the reading as JSON
//!   `{kind, code, temp_c, is_day, city, at}` — `kind` one of clear, cloudy,
//!   fog, rain, snow, storm — plus, when known, `wind_kmh`, `gust_kmh`,
//!   `wind_dir` (degrees it blows from), `latitude` and `country_code` (where
//!   it was read). Wind is its own dimension, never a kind. Absent while off,
//!   unset or not yet read.
//! - `GET /api/senses/weather` — `{city, auto, off, reading}`, refreshed if
//!   stale; `auto` is true while no city is typed (the place is, or will be,
//!   found by IP). `PUT /api/senses/weather` — `{city: "Harbin", lang?}` types
//!   the city (404 when no place has that name), `{city: null}` clears it back
//!   to automatic, `{off: true|false}` turns the sense off or on.

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// How long a reading counts as the weather now.
pub const TTL: Duration = Duration::from_secs(30 * 60);
/// How soon the same place is tried again after a fetch, failed or not.
pub const RETRY: Duration = Duration::from_secs(5 * 60);
/// How long a place found by IP stands before it is looked up again.
pub const RELOCATE: Duration = Duration::from_secs(6 * 60 * 60);
/// How soon a failed (or just made) place lookup is tried again.
pub const LOCATE_RETRY: Duration = Duration::from_secs(10 * 60);
const TIMEOUT: Duration = Duration::from_secs(6);
const GEOCODE: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST: &str = "https://api.open-meteo.com/v1/forecast";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct City {
    /// What the person typed.
    pub query: String,
    /// The place's own name, as the geocoder gives it.
    pub name: String,
    #[serde(default)]
    pub country: String,
    /// ISO 3166-1 alpha-2 (`CA`, `US`), for what differs by country (a
    /// holiday's date). Empty in files kept before it was, until re-read.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub country_code: String,
    pub latitude: f64,
    pub longitude: f64,
    /// Found by IP rather than typed.
    #[serde(default)]
    pub auto: bool,
    /// When an automatic place was found, unix seconds.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub located_at: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reading {
    pub kind: String,
    /// The WMO weather code the kind was read from.
    pub code: i64,
    pub temp_c: f64,
    pub is_day: bool,
    pub city: String,
    /// When it was read, unix seconds.
    pub at: u64,
    /// Wind at 10 m, km/h; its gusts, km/h; where it blows FROM, degrees
    /// (0 = north, 90 = east). Absent in readings kept before they were.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wind_kmh: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gust_kmh: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wind_dir: Option<f64>,
    /// Where it was read: the latitude (the hemisphere's seasons) and the
    /// place's country code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country_code: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Store {
    #[serde(default)]
    pub city: Option<City>,
    #[serde(default)]
    pub off: bool,
    #[serde(default)]
    pub last: Option<Reading>,
    /// Where `last` was read, `[latitude, longitude]`. Absent in files kept
    /// before it was: such a reading is matched by the city's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_place: Option<[f64; 2]>,
}

impl City {
    fn place(&self) -> [f64; 2] {
        [self.latitude, self.longitude]
    }
}

fn same_place(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

/// Whether the store's last reading was read at `city`.
fn read_at(store: &Store, city: &City, reading: &Reading) -> bool {
    match store.last_place {
        Some(p) => same_place(p, city.place()),
        None => reading.city == city.name,
    }
}

fn file() -> PathBuf {
    crate::paths::linggen_home()
        .join("senses")
        .join("weather.json")
}

pub fn load() -> Store {
    std::fs::read_to_string(file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(store: &Store) -> std::io::Result<()> {
    let path = file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(store).unwrap_or_default())?;
    std::fs::rename(tmp, path)
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The weather's kind from a WMO code (Open-Meteo's `weather_code`).
pub fn kind_of(code: i64) -> &'static str {
    match code {
        0 | 1 => "clear",
        2 | 3 => "cloudy",
        45 | 48 => "fog",
        51..=67 | 80..=82 => "rain",
        71..=77 | 85 | 86 => "snow",
        95..=99 => "storm",
        _ => "cloudy",
    }
}

/// Whether a store has a reading worth handing over now, and whether it is fresh.
fn usable(store: &Store) -> Option<(&Reading, bool)> {
    usable_at(store, now())
}

fn usable_at(store: &Store, now: u64) -> Option<(&Reading, bool)> {
    if store.off {
        return None;
    }
    let city = store.city.as_ref()?;
    let last = store.last.as_ref().filter(|r| read_at(store, city, r))?;
    Some((last, now.saturating_sub(last.at) < TTL.as_secs()))
}

/// What a refresh does next — decided from the store and the clock alone.
#[derive(Debug, PartialEq)]
enum Step {
    Idle,
    /// Find the place by IP.
    Locate,
    Read(City),
}

/// The attempts made lately, failures included: the place read and when,
/// and when the place was last looked up.
#[derive(Debug, Clone, Copy, Default)]
struct Tries {
    read: Option<([f64; 2], u64)>,
    locate: Option<u64>,
}

/// Off does nothing; no place (or an automatic one grown old) is looked up,
/// unless a lookup ran lately; a typed place is never looked up. Then the
/// place is read when its reading is stale and it was not tried lately.
fn step(store: &Store, now: u64, tries: Tries) -> Step {
    if store.off {
        return Step::Idle;
    }
    if needs_locate(store.city.as_ref(), now) && !located_lately(tries.locate, now) {
        return Step::Locate;
    }
    let Some(city) = store.city.as_ref() else {
        return Step::Idle;
    };
    let fresh = usable_at(store, now).is_some_and(|(_, fresh)| fresh);
    if fresh || tried_lately(tries.read, city, now) {
        return Step::Idle;
    }
    Step::Read(city.clone())
}

fn needs_locate(city: Option<&City>, now: u64) -> bool {
    city.is_none_or(|c| c.auto && now.saturating_sub(c.located_at) >= RELOCATE.as_secs())
}

fn located_lately(at: Option<u64>, now: u64) -> bool {
    at.is_some_and(|at| now.saturating_sub(at) < LOCATE_RETRY.as_secs())
}

fn client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(TIMEOUT)
        .timeout(TIMEOUT)
        .build()
        .ok()
}

/// The first place named `query`, from Open-Meteo's geocoder.
async fn geocode(query: &str, lang: &str) -> Option<City> {
    let body: serde_json::Value = client()?
        .get(GEOCODE)
        .query(&[
            ("name", query),
            ("count", "1"),
            ("language", lang),
            ("format", "json"),
        ])
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    city_of(query, &body)
}

/// Where this machine is, by its IP, from the site's `/api/geo`.
async fn locate() -> Option<City> {
    let url = format!("{}/api/geo", crate::account::site_url());
    match get_json(&url, &[]).await {
        Ok(body) => geo_city(&body, now()),
        Err(e) => {
            tracing::info!("[weather] no place by IP: {e}");
            None
        }
    }
}

/// A `/api/geo` answer as an automatic [`City`], named by its city, else its
/// region or country — never empty, since a reading also matches by name.
pub fn geo_city(body: &serde_json::Value, at: u64) -> Option<City> {
    let text = |k: &str| {
        body.get(k)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string()
    };
    let latitude = body.get("latitude")?.as_f64()?;
    let longitude = body.get("longitude")?.as_f64()?;
    let name = ["city", "region", "country"]
        .into_iter()
        .map(text)
        .find(|n| !n.is_empty())
        .unwrap_or_else(|| format!("{latitude:.2}, {longitude:.2}"));
    Some(City {
        query: String::new(),
        name,
        country: text("country"),
        country_code: iso_code(&text("country")),
        latitude,
        longitude,
        auto: true,
        located_at: at,
    })
}

/// A two-letter country code, upper-cased, or empty.
fn iso_code(raw: &str) -> String {
    let code = raw.trim();
    if code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic()) {
        code.to_ascii_uppercase()
    } else {
        String::new()
    }
}

/// A geocoder answer's first result as a [`City`].
pub fn city_of(query: &str, body: &serde_json::Value) -> Option<City> {
    let r = body.get("results")?.get(0)?;
    Some(City {
        query: query.to_string(),
        name: r.get("name")?.as_str()?.to_string(),
        country: r
            .get("country")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string(),
        country_code: iso_code(r.get("country_code").and_then(|c| c.as_str()).unwrap_or("")),
        latitude: r.get("latitude")?.as_f64()?,
        longitude: r.get("longitude")?.as_f64()?,
        auto: false,
        located_at: 0,
    })
}

/// A forecast answer's `current` block as a [`Reading`].
pub fn reading_of(city: &City, body: &serde_json::Value, at: u64) -> Option<Reading> {
    let c = body.get("current")?;
    let code = c.get("weather_code")?.as_i64()?;
    let num = |k: &str| c.get(k).and_then(|v| v.as_f64());
    Some(Reading {
        kind: kind_of(code).to_string(),
        code,
        temp_c: c
            .get("temperature_2m")
            .and_then(|t| t.as_f64())
            .unwrap_or(0.0),
        is_day: c.get("is_day").and_then(|d| d.as_i64()).unwrap_or(1) == 1,
        city: city.name.clone(),
        at,
        wind_kmh: num("wind_speed_10m"),
        gust_kmh: num("wind_gusts_10m"),
        wind_dir: num("wind_direction_10m"),
        latitude: Some(city.latitude),
        country_code: Some(city.country_code.clone()).filter(|c| !c.is_empty()),
    })
}

async fn fetch(city: &City) -> Option<Reading> {
    let (lat, lon) = (city.latitude.to_string(), city.longitude.to_string());
    let query = [
        ("latitude", lat.as_str()),
        ("longitude", lon.as_str()),
        (
            "current",
            "temperature_2m,weather_code,is_day,wind_speed_10m,wind_gusts_10m,wind_direction_10m",
        ),
    ];
    match get_json(FORECAST, &query).await {
        Ok(body) => reading_of(city, &body, now()),
        Err(e) => {
            tracing::info!("[weather] {}: {e}", city.name);
            None
        }
    }
}

/// One GET, its JSON body, or why not.
async fn get_json(url: &str, query: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    let client = client().ok_or("no http client")?;
    let res = client
        .get(url)
        .query(query)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    if !status.is_success() {
        return Err(format!("{status}"));
    }
    res.json().await.map_err(|e| e.to_string())
}

/// Held for a whole refresh: one fetch at a time.
static REFRESHING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// Held for each load-change-save of the file, so a refresh writing its
/// reading and a PUT changing the city never undo each other.
static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// The place last tried and when, unix seconds — failures included.
static LAST_TRY: std::sync::Mutex<Option<([f64; 2], u64)>> = std::sync::Mutex::new(None);
/// When the place was last looked up by IP — failures included.
static LAST_LOCATE: std::sync::Mutex<Option<u64>> = std::sync::Mutex::new(None);

/// Load, change and save the store under [`WRITING`]; the store after.
fn change(f: impl FnOnce(&mut Store)) -> std::io::Result<Store> {
    let _w = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let mut store = load();
    f(&mut store);
    save(&store)?;
    Ok(store)
}

/// Whether `city` was tried within [`RETRY`] of `now`.
fn tried_lately(last: Option<([f64; 2], u64)>, city: &City, now: u64) -> bool {
    last.is_some_and(|(p, at)| {
        same_place(p, city.place()) && now.saturating_sub(at) < RETRY.as_secs()
    })
}

fn tries() -> Tries {
    Tries {
        read: *LAST_TRY.lock().unwrap_or_else(|e| e.into_inner()),
        locate: *LAST_LOCATE.lock().unwrap_or_else(|e| e.into_inner()),
    }
}

fn note_try(city: &City) {
    *LAST_TRY.lock().unwrap_or_else(|e| e.into_inner()) = Some((city.place(), now()));
}

fn note_locate(at: Option<u64>) {
    *LAST_LOCATE.lock().unwrap_or_else(|e| e.into_inner()) = at;
}

/// What a refresh would do now.
fn next_step(store: &Store) -> Step {
    step(store, now(), tries())
}

/// Keep a place found by IP — only while no city is typed and the sense is
/// on (a city typed while the lookup ran wins).
fn keep_located(found: City) -> std::io::Result<Store> {
    change(|store| take_found(store, found))
}

fn take_found(store: &mut Store, found: City) {
    let typed = store.city.as_ref().is_some_and(|c| !c.auto);
    if typed || store.off {
        return;
    }
    apply(store, None, CityChange::Set(found));
}

/// Look the place up by IP; the store after.
async fn relocate() -> Store {
    note_locate(Some(now()));
    let Some(found) = locate().await else {
        return load();
    };
    keep_located(found).unwrap_or_else(|e| {
        tracing::warn!("[weather] could not keep the place: {e}");
        load()
    })
}

/// Keep `reading` of `city` — only if the person has not turned the sense
/// off or moved it to another place while it was fetched.
fn keep(city: &City, reading: Reading) -> std::io::Result<Store> {
    change(|store| {
        let here = store
            .city
            .as_ref()
            .is_some_and(|c| same_place(c.place(), city.place()));
        if here && !store.off {
            store.last = Some(reading);
            store.last_place = Some(city.place());
        }
    })
}

/// Whether a typed city still lacks its country code (kept before there was
/// one) and has not been looked up for it yet this run.
static COUNTRY_ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn lacks_country(city: &City) -> bool {
    !city.auto && city.country_code.is_empty() && !city.query.trim().is_empty()
}

/// A typed city kept before country codes were: its name looked up once
/// more for the code, kept only if the answer is the same place.
async fn with_country(city: City) -> City {
    use std::sync::atomic::Ordering;
    if !lacks_country(&city) || COUNTRY_ASKED.swap(true, Ordering::Relaxed) {
        return city;
    }
    let Some(found) = geocode(&city.query, "en").await else {
        return city;
    };
    let near = (found.latitude - city.latitude).abs() < 0.05
        && (found.longitude - city.longitude).abs() < 0.05;
    if !near || found.country_code.is_empty() {
        return city;
    }
    let code = found.country_code;
    let _ = change(|store| {
        if let Some(c) = store
            .city
            .as_mut()
            .filter(|c| same_place(c.place(), city.place()))
        {
            c.country_code = code.clone();
        }
    });
    City {
        country_code: code,
        ..city
    }
}

/// Read the weather again when it is stale (one fetch at a time); the store
/// as it stands after.
pub async fn refresh() -> Store {
    let _one = REFRESHING.lock().await;
    let mut store = load();
    if next_step(&store) == Step::Locate {
        store = relocate().await;
    }
    let Step::Read(city) = next_step(&store) else {
        return store;
    };
    note_try(&city);
    let city = with_country(city).await;
    let Some(reading) = fetch(&city).await else {
        tracing::info!("[weather] no reading for {}", city.name);
        return load();
    };
    keep(&city, reading).unwrap_or_else(|e| {
        tracing::warn!("[weather] could not keep the reading: {e}");
        load()
    })
}

/// `LINGGEN_WEATHER` for a declaring skill's tool: the last reading, stale or
/// not, with a refresh started behind when one is due and none is running.
/// Never waits.
pub fn tool_env() -> Option<(String, String)> {
    let store = load();
    let (reading, fresh) = match usable(&store) {
        Some(u) => (Some(u.0.clone()), u.1),
        None => (None, false),
    };
    if !fresh && next_step(&store) != Step::Idle && REFRESHING.try_lock().is_ok() {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(refresh());
        }
    }
    let reading = reading?;
    Some((
        "LINGGEN_WEATHER".to_string(),
        serde_json::to_string(&reading).ok()?,
    ))
}

#[derive(Serialize)]
struct View {
    city: Option<City>,
    /// No city typed: the place is, or will be, found by IP.
    auto: bool,
    off: bool,
    reading: Option<Reading>,
}

fn view(store: Store) -> Json<View> {
    let reading = usable(&store).map(|(r, _)| r.clone());
    Json(View {
        auto: store.city.as_ref().is_none_or(|c| c.auto),
        city: store.city,
        off: store.off,
        reading,
    })
}

/// `GET /api/senses/weather`.
pub async fn get_api() -> impl IntoResponse {
    view(refresh().await)
}

#[derive(Deserialize)]
pub struct Put {
    /// A city to set; `null` or empty clears it. Absent: left as it is.
    #[serde(default, deserialize_with = "some_or_null")]
    city: Option<Option<String>>,
    #[serde(default)]
    off: Option<bool>,
    #[serde(default)]
    lang: Option<String>,
}

fn some_or_null<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<String>>, D::Error> {
    Ok(Some(Option::<String>::deserialize(d)?))
}

/// What a PUT does to the city, once its name is looked up.
enum CityChange {
    Keep,
    Clear,
    Set(City),
}

/// The PUT's city, geocoded — or the answer that refuses it. Looked up before
/// anything is kept, so a name with no place changes nothing (not even `off`).
async fn city_change(body: &Put) -> Result<CityChange, axum::response::Response> {
    let query = match body
        .city
        .as_ref()
        .map(|c| c.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    {
        None => return Ok(CityChange::Keep),
        Some(None) => return Ok(CityChange::Clear),
        Some(Some(q)) => q.to_string(),
    };
    if query.chars().count() > 80 {
        return Err((StatusCode::BAD_REQUEST, "a city name is short").into_response());
    }
    let lang = body
        .lang
        .as_deref()
        .filter(|l| l.len() <= 5 && l.chars().all(|c| c.is_ascii_alphabetic()))
        .unwrap_or("zh");
    match geocode(&query, lang).await {
        Some(city) => Ok(CityChange::Set(city)),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "not-found", "city": query})),
        )
            .into_response()),
    }
}

/// Apply a PUT to the store: the switch, then the city.
fn apply(store: &mut Store, off: Option<bool>, change: CityChange) {
    if let Some(off) = off {
        store.off = off;
    }
    match change {
        CityChange::Keep => {}
        // The last reading stays: it is handed over only if the place found
        // next is where it was read.
        CityChange::Clear => store.city = None,
        CityChange::Set(city) => {
            let moved = store
                .city
                .as_ref()
                .is_none_or(|c| !same_place(c.place(), city.place()));
            if moved {
                store.last = None;
                store.last_place = None;
            }
            store.city = Some(city);
        }
    }
}

/// `PUT /api/senses/weather`.
pub async fn put_api(Json(body): Json<Put>) -> axum::response::Response {
    let change_city = match city_change(&body).await {
        Ok(c) => c,
        Err(refused) => return refused,
    };
    if matches!(change_city, CityChange::Clear) {
        // Back to automatic: the person asked, so look the place up now.
        note_locate(None);
    }
    if let Err(e) = change(|store| apply(store, body.off, change_city)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("could not keep the city: {e}"),
        )
            .into_response();
    }
    view(refresh().await).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wmo_codes_read_as_the_kinds_a_page_draws() {
        assert_eq!(kind_of(0), "clear");
        assert_eq!(kind_of(3), "cloudy");
        assert_eq!(kind_of(45), "fog");
        assert_eq!(kind_of(61), "rain");
        assert_eq!(kind_of(81), "rain");
        assert_eq!(kind_of(73), "snow");
        assert_eq!(kind_of(86), "snow");
        assert_eq!(kind_of(95), "storm");
    }

    #[test]
    fn a_geocoder_answer_and_a_forecast_are_read() {
        let geo = serde_json::json!({"results": [{"name": "哈尔滨", "country": "中国", "latitude": 45.75, "longitude": 126.65}]});
        let city = city_of("Harbin", &geo).unwrap();
        assert_eq!(city.name, "哈尔滨");
        assert_eq!(city.query, "Harbin");
        assert!(city_of("Nowhere", &serde_json::json!({"generationtime_ms": 0.1})).is_none());
        let now = serde_json::json!({"current": {"time": "2026-12-25T09:00", "temperature_2m": -18.5, "weather_code": 71, "is_day": 1}});
        let r = reading_of(&city, &now, 7).unwrap();
        assert_eq!(
            (
                r.kind.as_str(),
                r.code,
                r.temp_c,
                r.is_day,
                r.city.as_str(),
                r.at
            ),
            ("snow", 71, -18.5, true, "哈尔滨", 7)
        );
    }

    #[test]
    fn off_or_unset_hands_nothing_over_and_a_new_city_drops_the_old_reading() {
        let city = City {
            query: "x".into(),
            name: "甲".into(),
            country: String::new(),
            country_code: String::new(),
            latitude: 0.0,
            longitude: 0.0,
            auto: false,
            located_at: 0,
        };
        let reading = Reading {
            kind: "rain".into(),
            code: 61,
            temp_c: 9.0,
            is_day: true,
            city: "甲".into(),
            at: now(),
            wind_kmh: None,
            gust_kmh: None,
            wind_dir: None,
            latitude: None,
            country_code: None,
        };
        let on = Store {
            city: Some(city.clone()),
            off: false,
            last: Some(reading.clone()),
            last_place: None,
        };
        assert!(usable(&on).is_some_and(|(_, fresh)| fresh));
        assert!(usable(&Store {
            off: true,
            ..on.clone()
        })
        .is_none());
        assert!(usable(&Store {
            city: None,
            ..on.clone()
        })
        .is_none());
        let moved = Store {
            city: Some(City {
                name: "乙".into(),
                ..city
            }),
            ..on.clone()
        };
        assert!(
            usable(&moved).is_none(),
            "a reading of another city is not this one's"
        );
        let old = Store {
            last: Some(Reading { at: 0, ..reading }),
            ..on
        };
        assert!(usable(&old).is_some_and(|(_, fresh)| !fresh));
    }

    fn city(name: &str, lat: f64) -> City {
        City {
            query: name.into(),
            name: name.into(),
            country: String::new(),
            country_code: String::new(),
            latitude: lat,
            longitude: 0.0,
            auto: false,
            located_at: 0,
        }
    }

    #[test]
    fn a_reading_belongs_to_its_place_not_its_name() {
        let springfield = city("Springfield", 39.8);
        let reading = Reading {
            kind: "clear".into(),
            code: 0,
            temp_c: 20.0,
            is_day: true,
            city: "Springfield".into(),
            at: now(),
            wind_kmh: None,
            gust_kmh: None,
            wind_dir: None,
            latitude: None,
            country_code: None,
        };
        let kept = Store {
            city: Some(springfield.clone()),
            off: false,
            last: Some(reading),
            last_place: Some(springfield.place()),
        };
        assert!(usable(&kept).is_some());
        let other = Store {
            city: Some(city("Springfield", 42.1)),
            ..kept.clone()
        };
        assert!(usable(&other).is_none(), "same name, another place");
        let old_file = Store {
            last_place: None,
            ..other
        };
        assert!(
            usable(&old_file).is_some(),
            "a reading kept before places matches by name"
        );
    }

    #[test]
    fn a_place_tried_lately_waits_failures_included() {
        let a = city("A", 1.0);
        let t = 10_000;
        assert!(!tried_lately(None, &a, t));
        assert!(tried_lately(Some((a.place(), t - 60)), &a, t));
        assert!(!tried_lately(Some((a.place(), t - RETRY.as_secs())), &a, t));
        assert!(
            !tried_lately(Some((city("B", 2.0).place(), t)), &a, t),
            "a new place is tried at once"
        );
    }

    #[test]
    fn a_put_switches_and_moves_and_only_a_move_drops_the_reading() {
        let a = city("A", 1.0);
        let reading = Reading {
            kind: "rain".into(),
            code: 61,
            temp_c: 9.0,
            is_day: true,
            city: "A".into(),
            at: now(),
            wind_kmh: None,
            gust_kmh: None,
            wind_dir: None,
            latitude: None,
            country_code: None,
        };
        let mut store = Store {
            city: Some(a.clone()),
            off: false,
            last: Some(reading),
            last_place: Some(a.place()),
        };
        apply(&mut store, Some(true), CityChange::Set(a.clone()));
        assert!(
            store.off && store.last.is_some(),
            "the same place keeps its reading"
        );
        apply(&mut store, None, CityChange::Set(city("A", 5.0)));
        assert!(store.off && store.last.is_none() && store.last_place.is_none());
        apply(&mut store, Some(false), CityChange::Clear);
        assert!(!store.off && store.city.is_none());
    }

    fn auto_city(name: &str, lat: f64, located_at: u64) -> City {
        City {
            query: String::new(),
            auto: true,
            located_at,
            ..city(name, lat)
        }
    }

    fn reading_at(at: u64) -> Reading {
        Reading {
            kind: "clear".into(),
            code: 0,
            temp_c: 20.0,
            is_day: true,
            city: "A".into(),
            at,
            wind_kmh: None,
            gust_kmh: None,
            wind_dir: None,
            latitude: None,
            country_code: None,
        }
    }

    #[test]
    fn a_forecast_carries_the_wind_and_where_it_was_read() {
        let geo = serde_json::json!({"results": [{"name": "Halifax", "country": "Canada", "country_code": "ca", "latitude": 44.64, "longitude": -63.58}]});
        let halifax = city_of("Halifax", &geo).unwrap();
        assert_eq!(halifax.country_code, "CA");
        let now = serde_json::json!({"current": {"temperature_2m": 9.0, "weather_code": 0, "is_day": 1,
            "wind_speed_10m": 30.4, "wind_gusts_10m": 62.6, "wind_direction_10m": 317}});
        let r = reading_of(&halifax, &now, 1).unwrap();
        assert_eq!(
            (r.kind.as_str(), r.wind_kmh, r.gust_kmh, r.wind_dir),
            ("clear", Some(30.4), Some(62.6), Some(317.0))
        );
        assert_eq!(
            (r.latitude, r.country_code.as_deref()),
            (Some(44.64), Some("CA"))
        );
        // A forecast without wind: none, and the JSON a tool sees has no wind keys.
        let calm =
            serde_json::json!({"current": {"temperature_2m": 9.0, "weather_code": 0, "is_day": 1}});
        let r = reading_of(&city("A", 1.0), &calm, 1).unwrap();
        assert_eq!(
            (r.wind_kmh, r.gust_kmh, r.country_code.clone()),
            (None, None, None)
        );
        let json = serde_json::to_value(&r).unwrap();
        assert!(json.get("wind_kmh").is_none() && json.get("country_code").is_none());
    }

    #[test]
    fn a_reading_kept_before_the_wind_still_reads() {
        let old: Reading = serde_json::from_str(
            r#"{"kind":"clear","code":0,"temp_c":9.0,"is_day":true,"city":"Halifax","at":1}"#,
        )
        .unwrap();
        assert_eq!((old.wind_kmh, old.latitude), (None, None));
        let geo = serde_json::json!({"city": "Halifax", "country": "CA", "latitude": 44.6, "longitude": -63.6});
        assert_eq!(geo_city(&geo, 1).unwrap().country_code, "CA");
    }

    #[test]
    fn off_fetches_nothing_not_even_the_place() {
        let t = 1_000_000;
        let off = Store {
            off: true,
            ..Store::default()
        };
        assert_eq!(step(&off, t, Tries::default()), Step::Idle);
        let off_typed = Store {
            city: Some(city("A", 1.0)),
            ..off.clone()
        };
        assert_eq!(step(&off_typed, t, Tries::default()), Step::Idle);
        let off_old_auto = Store {
            city: Some(auto_city("A", 1.0, 0)),
            ..off
        };
        assert_eq!(step(&off_old_auto, t, Tries::default()), Step::Idle);
    }

    #[test]
    fn no_city_looks_the_place_up_once_then_waits() {
        let t = 1_000_000;
        let empty = Store::default();
        assert_eq!(step(&empty, t, Tries::default()), Step::Locate);
        let just = Tries {
            locate: Some(t - 60),
            ..Tries::default()
        };
        assert_eq!(step(&empty, t, just), Step::Idle, "a failed lookup waits");
        let long_ago = Tries {
            locate: Some(t - LOCATE_RETRY.as_secs()),
            ..Tries::default()
        };
        assert_eq!(step(&empty, t, long_ago), Step::Locate);
    }

    #[test]
    fn a_typed_city_is_read_and_never_looked_up() {
        let t = 1_000_000;
        let a = city("A", 1.0);
        let typed = Store {
            city: Some(a.clone()),
            ..Store::default()
        };
        assert_eq!(step(&typed, t, Tries::default()), Step::Read(a.clone()));
        let fresh = Store {
            last: Some(reading_at(t - 60)),
            last_place: Some(a.place()),
            ..typed.clone()
        };
        assert_eq!(step(&fresh, t, Tries::default()), Step::Idle);
        let stale = Store {
            last: Some(reading_at(t - TTL.as_secs())),
            ..fresh
        };
        assert_eq!(step(&stale, t, Tries::default()), Step::Read(a.clone()));
        let tried = Tries {
            read: Some((a.place(), t - 30)),
            ..Tries::default()
        };
        assert_eq!(step(&stale, t, tried), Step::Idle);
    }

    #[test]
    fn an_automatic_place_is_read_then_looked_up_again_when_old() {
        let t = 1_000_000;
        let here = auto_city("A", 1.0, t - 60);
        let auto = Store {
            city: Some(here.clone()),
            ..Store::default()
        };
        assert_eq!(step(&auto, t, Tries::default()), Step::Read(here));
        let old = Store {
            city: Some(auto_city("A", 1.0, t - RELOCATE.as_secs())),
            ..auto
        };
        assert_eq!(step(&old, t, Tries::default()), Step::Locate);
        let looked = Tries {
            locate: Some(t - 5),
            ..Tries::default()
        };
        assert!(
            matches!(step(&old, t, looked), Step::Read(_)),
            "a failed lookup keeps reading the old place"
        );
    }

    #[test]
    fn a_found_place_never_overrides_a_typed_one_and_clear_goes_back_to_auto() {
        let mut store = Store {
            city: Some(city("Typed", 1.0)),
            ..Store::default()
        };
        take_found(&mut store, auto_city("Found", 2.0, 5));
        assert_eq!(store.city.as_ref().unwrap().name, "Typed");
        apply(&mut store, None, CityChange::Clear);
        assert!(store.city.is_none());
        assert_eq!(step(&store, 10, Tries::default()), Step::Locate);
        take_found(&mut store, auto_city("Found", 2.0, 5));
        assert!(store.city.as_ref().unwrap().auto);
        let mut off = Store {
            off: true,
            ..Store::default()
        };
        take_found(&mut off, auto_city("Found", 2.0, 5));
        assert!(
            off.city.is_none(),
            "a lookup finishing after off keeps nothing"
        );
    }

    #[test]
    fn a_geo_answer_is_an_automatic_city_never_nameless() {
        let full = serde_json::json!({"city": "Halifax", "region": "Nova Scotia", "country": "CA", "latitude": 44.64, "longitude": -63.57, "timezone": "America/Halifax"});
        let c = geo_city(&full, 9).unwrap();
        assert_eq!(
            (c.name.as_str(), c.country.as_str(), c.auto, c.located_at),
            ("Halifax", "CA", true, 9)
        );
        let no_city = serde_json::json!({"city": "", "region": "Nova Scotia", "latitude": 1.0, "longitude": 2.0});
        assert_eq!(geo_city(&no_city, 0).unwrap().name, "Nova Scotia");
        let bare = serde_json::json!({"latitude": 1.0, "longitude": 2.0});
        assert_eq!(geo_city(&bare, 0).unwrap().name, "1.00, 2.00");
        assert!(geo_city(&serde_json::json!({"city": "X"}), 0).is_none());
    }

    #[test]
    fn an_old_file_reads_its_city_as_typed() {
        let old = r#"{"city": {"query": "Harbin", "name": "哈尔滨", "country": "中国", "latitude": 45.75, "longitude": 126.65}, "off": false}"#;
        let store: Store = serde_json::from_str(old).unwrap();
        assert!(!store.city.unwrap().auto);
    }
}
