//! Exchange rates from the free currency API by Fawaz Ahmed (CC0), with a disk cache.
//!
//! Rates cover fiat currencies, metals and popular crypto, and update daily.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

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

impl RateProvider for OnlineRates {
    fn rates(&self) -> Result<RateTable, String> {
        if let Some(table) = self.cached(true) {
            return Ok(table);
        }
        let downloaded = if self.offline { Err("offline and no cached exchange rates".into()) } else { self.refresh() };
        downloaded.or_else(|e| self.cached(false).ok_or(e))
    }
}

/// Parses `{"date": "...", "usd": {"eur": 0.92, ...}}`.
fn parse(text: &str) -> Option<RateTable> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut table = RateTable::new();
    table.date = json.get("date").and_then(|d| d.as_str()).map(String::from);
    for (code, rate) in json.get("usd")?.as_object()? {
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
