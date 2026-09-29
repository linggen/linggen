//! The weather sense: the current weather at the city the person set, from
//! Open-Meteo (keyless, public: geocoding + forecast). Off until a city is
//! set, and off again when the person turns it off — nothing is fetched then.
//! No location permission: the city is typed once, by the person.
//!
//! - Kept in `~/.linggen/senses/weather.json`: the city (as geocoded), the
//!   off switch, and the last reading (so a restart has it at once).
//! - A reading is fresh for [`TTL`]; a stale one is still handed over while a
//!   refresh runs behind — a tool never waits on the network.
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
    let last = store.last.as_ref().filter(|r| r.city == city.name)?;
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
    let body: serde_json::Value = client()?
        .get(FORECAST)
        .query(&[
            ("latitude", lat.as_str()),
            ("longitude", lon.as_str()),
            ("current", "temperature_2m,weather_code,is_day"),
        ])
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    reading_of(city, &body, now())
}

static REFRESHING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Read the weather again when it is stale (one fetch at a time); the store
/// as it stands after.
pub async fn refresh() -> Store {
    let _one = REFRESHING.lock().await;
    let mut store = load();
    if store.off || usable(&store).is_some_and(|(_, fresh)| fresh) {
        return store;
    }
    let Some(city) = store.city.clone() else {
        return store;
    };
    match fetch(&city).await {
        Some(reading) => {
            store.last = Some(reading);
            if let Err(e) = save(&store) {
                tracing::warn!("[weather] could not keep the reading: {e}");
            }
        }
        None => tracing::info!("[weather] no reading for {}", city.name),
    }
    store
}

/// `LINGGEN_WEATHER` for a declaring skill's tool: the last reading, stale or
/// not, with a refresh started behind when it is stale. Never waits.
pub fn tool_env() -> Option<(String, String)> {
    let store = load();
    let (reading, fresh) = match usable(&store) {
        Some(u) => (Some(u.0.clone()), u.1),
        None => (None, false),
    };
    if !fresh && !store.off && store.city.is_some() {
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

/// `PUT /api/senses/weather`.
pub async fn put_api(Json(body): Json<Put>) -> axum::response::Response {
    let mut store = load();
    if let Some(off) = body.off {
        store.off = off;
    }
    match body
        .city
        .map(|c| c.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
    {
        Some(Some(query)) => {
            if query.chars().count() > 80 {
                return (StatusCode::BAD_REQUEST, "a city name is short").into_response();
            }
            let lang = body
                .lang
                .as_deref()
                .filter(|l| l.len() <= 5 && l.chars().all(|c| c.is_ascii_alphabetic()))
                .unwrap_or("zh");
            let Some(city) = geocode(&query, lang).await else {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "not-found", "city": query})),
                )
                    .into_response();
            };
            if store.city.as_ref().map(|c| &c.name) != Some(&city.name) {
                store.last = None;
            }
            store.city = Some(city);
        }
        Some(None) => {
            store.city = None;
            store.last = None;
        }
        None => {}
    }
    if let Err(e) = save(&store) {
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
}
