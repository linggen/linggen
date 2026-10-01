//! The weather sense: the current weather at the city the person set, from
//! Open-Meteo (keyless, public: geocoding + forecast). Off until a city is
//! set, and off again when the person turns it off — nothing is fetched then.
//! No location permission: the city is typed once, by the person.
//!
//! - Kept in `~/.linggen/senses/weather.json`: the city (as geocoded), the
//!   off switch, and the last reading (so a restart has it at once).
//! - A reading is fresh for [`TTL`]; a stale one is still handed over while a
//!   refresh runs behind — a tool never waits on the network. One fetch runs
//!   at a time, and a place is tried at most once per [`RETRY`] (a failed
//!   fetch included), so a dead network is not asked on every tool call.
//! - A reading belongs to the place it was read at (its coordinates), so two
//!   cities of one name never share one.
//! - To a declaring skill's tool: `LINGGEN_WEATHER`, the reading as JSON
//!   `{kind, code, temp_c, is_day, city, at}` — `kind` one of clear, cloudy,
//!   fog, rain, snow, storm. Absent while off, unset or not yet read.
//! - `GET /api/senses/weather` — `{city, off, reading}`, refreshed if stale.
//!   `PUT /api/senses/weather` — `{city: "Harbin", lang?}` sets the city
//!   (404 when no place has that name), `{city: null}` clears it,
//!   `{off: true|false}` turns the sense off or on.

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// How long a reading counts as the weather now.
pub const TTL: Duration = Duration::from_secs(30 * 60);
/// How soon the same place is tried again after a fetch, failed or not.
pub const RETRY: Duration = Duration::from_secs(5 * 60);
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
    pub latitude: f64,
    pub longitude: f64,
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
    if store.off {
        return None;
    }
    let city = store.city.as_ref()?;
    let last = store.last.as_ref().filter(|r| read_at(store, city, r))?;
    Some((last, now().saturating_sub(last.at) < TTL.as_secs()))
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
        latitude: r.get("latitude")?.as_f64()?,
        longitude: r.get("longitude")?.as_f64()?,
    })
}

/// A forecast answer's `current` block as a [`Reading`].
pub fn reading_of(city: &City, body: &serde_json::Value, at: u64) -> Option<Reading> {
    let c = body.get("current")?;
    let code = c.get("weather_code")?.as_i64()?;
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
    })
}

async fn fetch(city: &City) -> Option<Reading> {
    let (lat, lon) = (city.latitude.to_string(), city.longitude.to_string());
    let query = [
        ("latitude", lat.as_str()),
        ("longitude", lon.as_str()),
        ("current", "temperature_2m,weather_code,is_day"),
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

fn tried_city_lately(city: &City) -> bool {
    let last = *LAST_TRY.lock().unwrap_or_else(|e| e.into_inner());
    tried_lately(last, city, now())
}

fn note_try(city: &City) {
    *LAST_TRY.lock().unwrap_or_else(|e| e.into_inner()) = Some((city.place(), now()));
}

/// The city to read now: on, set, stale, and not tried lately.
fn due(store: &Store) -> Option<City> {
    if store.off || usable(store).is_some_and(|(_, fresh)| fresh) {
        return None;
    }
    store.city.clone().filter(|c| !tried_city_lately(c))
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

/// Read the weather again when it is stale (one fetch at a time); the store
/// as it stands after.
pub async fn refresh() -> Store {
    let _one = REFRESHING.lock().await;
    let store = load();
    let Some(city) = due(&store) else {
        return store;
    };
    note_try(&city);
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
    if !fresh && due(&store).is_some() && REFRESHING.try_lock().is_ok() {
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
    off: bool,
    reading: Option<Reading>,
}

fn view(store: Store) -> Json<View> {
    let reading = usable(&store).map(|(r, _)| r.clone());
    Json(View {
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
        CityChange::Clear => {
            store.city = None;
            store.last = None;
            store.last_place = None;
        }
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
            latitude: 0.0,
            longitude: 0.0,
        };
        let reading = Reading {
            kind: "rain".into(),
            code: 61,
            temp_c: 9.0,
            is_day: true,
            city: "甲".into(),
            at: now(),
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
            latitude: lat,
            longitude: 0.0,
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
}
