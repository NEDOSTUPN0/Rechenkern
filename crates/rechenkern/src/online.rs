//! Exchange rates from the free currency API by Fawaz Ahmed (CC0), with a disk cache.
//!
//! Rates cover fiat currencies, metals and popular crypto, and update daily.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use jiff::civil::Date;

use crate::number::Number;
use crate::rates::{RateProvider, RateTable};

const SOURCES: &[&str] = &[
    "https://cdn.jsdelivr.net/npm/@fawazahmed0/currency-api@latest/v1/currencies/usd.min.json",
    "https://latest.currency-api.pages.dev/v1/currencies/usd.min.json",
];

/// Downloads rates when the cache is missing or older than `max_age`.
/// A stale cache is still used when the download fails.
#[derive(Clone, Debug)]
pub struct OnlineRates {
    pub cache: Option<PathBuf>,
    pub max_age: Duration,
    /// Never download; only read the cache.
    pub offline: bool,
    pub timeout: Duration,
}

impl OnlineRates {
    pub fn new(cache: Option<PathBuf>) -> OnlineRates {
        OnlineRates { cache, max_age: Duration::from_secs(6 * 3600), offline: false, timeout: Duration::from_secs(5) }
    }

    fn cached(&self, fresh_only: bool) -> Option<RateTable> {
        let path = self.cache.as_ref()?;
        if fresh_only {
            let age = fs::metadata(path).ok()?.modified().ok()?.elapsed().unwrap_or(Duration::MAX);
            if age > self.max_age {
                return None;
            }
        }
        parse(&fs::read_to_string(path).ok()?)
    }

    fn download(&self) -> Result<(String, RateTable), String> {
        let mut last_error = String::new();
        for url in SOURCES {
            let response = ureq::get(*url).config().timeout_global(Some(self.timeout)).build().call();
            match response.and_then(|mut r| r.body_mut().read_to_string()) {
                Ok(text) => match parse(&text) {
                    Some(table) => return Ok((text, table)),
                    None => last_error = format!("unexpected response from {url}"),
                },
                Err(e) => last_error = e.to_string(),
            }
        }
        Err(format!("can't download exchange rates: {last_error}"))
    }

    /// Downloads rates now and updates the cache.
    pub fn refresh(&self) -> Result<RateTable, String> {
        let (text, table) = self.download()?;
        if let Some(path) = &self.cache {
            if let Some(dir) = path.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let _ = fs::write(path, text);
        }
        Ok(table)
    }

    /// When the cached rates were saved, if there are any.
    pub fn cache_time(&self) -> Option<SystemTime> {
        fs::metadata(self.cache.as_ref()?).ok()?.modified().ok()
    }
}

impl OnlineRates {
    /// Where the rates of a past day are kept: next to the latest ones.
    fn dated_cache(&self, date: Date) -> Option<PathBuf> {
        let path = self.cache.as_ref()?;
        Some(path.with_file_name(format!("rates-{date}.json")))
    }

    /// Rates of a past day: the currency API keeps them from March 2024, the
    /// Frankfurter API (ECB reference rates) from 1999 for about 30 currencies.
    fn download_on(&self, date: Date) -> Result<(String, RateTable), String> {
        let sources = [
            format!("https://cdn.jsdelivr.net/npm/@fawazahmed0/currency-api@{date}/v1/currencies/usd.min.json"),
            format!("https://{date}.currency-api.pages.dev/v1/currencies/usd.min.json"),
            format!("https://api.frankfurter.dev/v1/{date}?base=USD"),
        ];
        let mut last_error = String::new();
        for url in &sources {
            let response = ureq::get(url).config().timeout_global(Some(self.timeout)).build().call();
            match response.and_then(|mut r| r.body_mut().read_to_string()) {
                Ok(text) => match parse(&text) {
                    Some(table) => return Ok((text, table)),
                    None => last_error = format!("unexpected response from {url}"),
                },
                Err(e) => last_error = e.to_string(),
            }
        }
        Err(format!("can't download exchange rates for {date}: {last_error}"))
    }
}

impl RateProvider for OnlineRates {
    fn rates(&self) -> Result<RateTable, String> {
        if let Some(table) = self.cached(true) {
            return Ok(table);
        }
        let downloaded = if self.offline { Err("offline and no cached exchange rates".into()) } else { self.refresh() };
        downloaded.or_else(|e| self.cached(false).ok_or(e))
    }

    fn cached_rates(&self) -> Option<RateTable> {
        self.cached(false)
    }

    fn rates_on(&self, date: Date) -> Result<RateTable, String> {
        let cache = self.dated_cache(date);
        if let Some(table) = cache.as_ref().and_then(|p| parse(&fs::read_to_string(p).ok()?)) {
            return Ok(table);
        }
        if self.offline {
            return Err(format!("offline and no cached exchange rates for {date}"));
        }
        let (text, table) = self.download_on(date)?;
        // A past day's rates never change.
        if let Some(path) = cache {
            if let Some(dir) = path.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let _ = fs::write(path, text);
        }
        Ok(table)
    }
}

/// Parses `{"date": "...", "usd": {"eur": 0.92, ...}}`, or Frankfurter's `{"rates": {"EUR": 0.92}}`.
fn parse(text: &str) -> Option<RateTable> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut table = RateTable::new();
    table.date = json.get("date").and_then(|d| d.as_str()).map(String::from);
    let rates = json.get("usd").or_else(|| json.get("rates"))?;
    for (code, rate) in rates.as_object()? {
        // Keep the JSON digits exactly instead of going through f64.
        if let Some(n) = Number::parse(&rate.to_string()) {
            table.insert(code, n);
        }
    }
    (!table.is_empty()).then_some(table)
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_api_response() {
        let table = super::parse(r#"{"date":"2026-09-22","usd":{"eur":0.92,"kzt":480.5,"btc":0.0000158}}"#).unwrap();
        assert_eq!(table.date.as_deref(), Some("2026-09-22"));
        assert_eq!(table.get("EUR").unwrap().to_string(), "0.92");
        assert_eq!(table.get("btc").unwrap().to_string(), "0.0000158");
    }
}

/// Places found by name with the Open-Meteo geocoding API (GeoNames data, CC BY 4.0;
/// the API is free for non-commercial use). Answers are kept in a disk cache, misses too.
#[derive(Clone, Debug)]
pub struct OnlinePlaces {
    pub cache: Option<PathBuf>,
    /// Never download; only read the cache.
    pub offline: bool,
    pub timeout: Duration,
}

impl OnlinePlaces {
    pub fn new(cache: Option<PathBuf>) -> OnlinePlaces {
        OnlinePlaces { cache, offline: false, timeout: Duration::from_secs(5) }
    }

    fn read_cache(&self) -> serde_json::Map<String, serde_json::Value> {
        let text = self.cache.as_ref().and_then(|p| fs::read_to_string(p).ok());
        text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    fn lookup(&self, name: &str) -> Result<Option<serde_json::Value>, String> {
        let url = "https://geocoding-api.open-meteo.com/v1/search";
        let response = ureq::get(url)
            .query("name", name)
            .query("count", "1")
            .query("language", "en")
            .query("format", "json")
            .config()
            .timeout_global(Some(self.timeout))
            .build()
            .call();
        let text = response.and_then(|mut r| r.body_mut().read_to_string()).map_err(|e| e.to_string())?;
        let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        Ok(json.get("results").and_then(|r| r.get(0)).cloned())
    }
}

impl crate::PlaceProvider for OnlinePlaces {
    fn place(&self, name: &str) -> Option<crate::CustomPlace> {
        let key = name.to_lowercase();
        let mut cache = self.read_cache();
        let found = match cache.get(&key) {
            Some(found) => found.clone(),
            None if self.offline => return None,
            None => {
                // A failed download is no answer, so it is asked again next time.
                let found = self.lookup(name).ok()?.unwrap_or(serde_json::Value::Null);
                cache.insert(key, found.clone());
                if let Some(path) = &self.cache {
                    if let Some(dir) = path.parent() {
                        let _ = fs::create_dir_all(dir);
                    }
                    let _ = fs::write(path, serde_json::Value::Object(cache).to_string());
                }
                found
            }
        };
        let zone = jiff::tz::TimeZone::get(found.get("timezone")?.as_str()?).ok()?;
        let coordinates = found.get("latitude")?.as_f64().zip(found.get("longitude")?.as_f64());
        let name = found.get("name")?.as_str()?.to_string();
        Some(crate::CustomPlace { name, zone, coordinates })
    }
}

/// The weather from Open-Meteo (CC BY 4.0; free for non-commercial use): forecasts up to
/// 16 days ahead and the last 92 days, older days from its archive.
#[derive(Clone, Debug)]
pub struct OnlineWeather {
    pub timeout: Duration,
}

impl Default for OnlineWeather {
    fn default() -> OnlineWeather {
        OnlineWeather { timeout: Duration::from_secs(5) }
    }
}

/// What a day of Open-Meteo's daily weather holds.
const DAILY: &str = "temperature_2m_max,temperature_2m_min,weather_code,relative_humidity_2m_mean,wind_speed_10m_max,uv_index_max,precipitation_probability_max";

impl crate::WeatherProvider for OnlineWeather {
    fn weather(&self, request: &crate::WeatherRequest) -> Result<crate::Weather, String> {
        let today = jiff::Timestamp::now().to_zoned(request.zone.clone()).date();
        let date = request.date.unwrap_or(today);
        let days = today.until(date).map_err(|e| e.to_string())?.get_days();
        let (url, recent) = match days {
            -92..=15 => ("https://api.open-meteo.com/v1/forecast", true),
            _ if days < 0 => ("https://archive-api.open-meteo.com/v1/archive", false),
            _ => return Err("the weather is known up to 16 days ahead".into()),
        };
        let day = date.to_string();
        let mut call = ureq::get(url)
            .query("latitude", request.latitude.to_string())
            .query("longitude", request.longitude.to_string())
            .query("timezone", request.zone.iana_name().unwrap_or("auto"))
            .query("start_date", &day)
            .query("end_date", &day);
        // The archive has no forecasts: no chance of rain, no UV index.
        call = call.query(
            "daily",
            if recent {
                DAILY
            } else {
                "temperature_2m_max,temperature_2m_min,weather_code,relative_humidity_2m_mean,wind_speed_10m_max"
            },
        );
        if request.date.is_none() {
            call = call.query("current", "temperature_2m,relative_humidity_2m,weather_code,wind_speed_10m,uv_index");
        }
        let response = call.config().timeout_global(Some(self.timeout)).build().call();
        let text = response
            .and_then(|mut r| r.body_mut().read_to_string())
            .map_err(|e| format!("can't get the weather: {e}"))?;
        let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let daily = |field: &str| json.get("daily")?.get(field)?.get(0)?.as_f64();
        let current = |field: &str| json.get("current")?.get(field)?.as_f64();
        let (high, low) = (daily("temperature_2m_max"), daily("temperature_2m_min"));
        let middle = high.zip(low).map(|(h, l)| (h + l) / 2.0);
        let code = current("weather_code").or_else(|| daily("weather_code"));
        Ok(crate::Weather {
            temperature: current("temperature_2m").or(middle),
            high,
            low,
            humidity: current("relative_humidity_2m").or_else(|| daily("relative_humidity_2m_mean")),
            wind_speed: current("wind_speed_10m").or_else(|| daily("wind_speed_10m_max")),
            uv_index: current("uv_index").or_else(|| daily("uv_index_max")),
            chance_of_rain: daily("precipitation_probability_max"),
            condition: code.and_then(|c| crate::weather_condition(c as u32)).map(String::from),
        })
    }
}
