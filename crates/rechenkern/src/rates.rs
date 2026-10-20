//! Exchange rates and where they come from.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, OnceLock, PoisonError};

use jiff::civil::Date;

use crate::error::{Error, Result};
use crate::number::Number;

/// Exchange rates as units of each currency per one US dollar.
#[derive(Clone, Debug, Default)]
pub struct RateTable {
    rates: HashMap<String, Number>,
    /// Day the rates were published, if known.
    pub date: Option<String>,
}

impl RateTable {
    pub fn new() -> RateTable {
        RateTable::default()
    }

    /// Sets how many units of `code` one US dollar buys.
    pub fn insert(&mut self, code: &str, per_usd: Number) {
        self.rates.insert(code.to_uppercase(), per_usd);
    }

    pub fn get(&self, code: &str) -> Option<Number> {
        if code.eq_ignore_ascii_case("USD") {
            return Some(Number::ONE);
        }
        self.rates.get(&code.to_uppercase()).copied()
    }

    pub fn len(&self) -> usize {
        self.rates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rates.is_empty()
    }
}

/// A source of exchange rates, asked at most once per calculator and only
/// when a line needs a conversion.
pub trait RateProvider: Send + Sync {
    fn rates(&self) -> std::result::Result<RateTable, String>;

    /// The rates of a past day, for `$100 in EUR on Jan 1, 2020`; asked once per day.
    fn rates_on(&self, date: Date) -> std::result::Result<RateTable, String> {
        Err(format!("exchange rates for {date} aren't known"))
    }

    /// Rates at hand without asking anyone, however old, for conversions the line
    /// didn't ask for (`ResultConversions::Home`).
    fn cached_rates(&self) -> Option<RateTable> {
        None
    }

    /// The rates are on their way (the host fetches them in the background): lines that need
    /// them answer `ErrorKind::Pending` and nothing is kept, so they answer once asked again
    /// (`Sheet::recalculate_pending`).
    fn pending(&self) -> bool {
        false
    }

    /// The rate from one currency to another the host knows better than by way of the dollar:
    /// units of `to` for one `from`. `None` uses the table.
    fn pair_rate(&self, from: &str, to: &str) -> Option<Number> {
        let _ = (from, to);
        None
    }
}

/// Rates that come with the engine (`data/rates.txt`), for currencies a provider
/// doesn't know and for the first run without network.
pub fn bundled_rates() -> &'static RateTable {
    static BUNDLED: LazyLock<RateTable> = LazyLock::new(|| {
        let mut table = RateTable::new();
        for line in include_str!("data/rates.txt").lines().filter(|l| !l.starts_with('#')) {
            match line.split_once(' ') {
                Some(("date", date)) => table.date = Some(date.to_string()),
                Some((code, rate)) => table.insert(code, Number::parse(rate).expect("a rate in data/rates.txt")),
                None => {}
            }
        }
        table
    });
    &BUNDLED
}

/// Rates loaded lazily from a provider.
#[derive(Default)]
pub(crate) struct Rates {
    provider: Option<Arc<dyn RateProvider>>,
    table: OnceLock<std::result::Result<RateTable, String>>,
    /// Rates of past days, as the provider gave them.
    dated: Mutex<HashMap<Date, std::result::Result<Arc<RateTable>, String>>>,
    /// The provider's rates at hand, read once.
    cached: OnceLock<Option<RateTable>>,
}

impl Clone for Rates {
    fn clone(&self) -> Rates {
        let dated = self.dated.lock().unwrap_or_else(PoisonError::into_inner).clone();
        Rates {
            provider: self.provider.clone(),
            table: self.table.clone(),
            dated: Mutex::new(dated),
            cached: self.cached.clone(),
        }
    }
}

impl Rates {
    pub fn with_provider(provider: Arc<dyn RateProvider>) -> Rates {
        Rates { provider: Some(provider), ..Rates::default() }
    }

    pub fn with_table(table: RateTable) -> Rates {
        Rates { table: OnceLock::from(Ok(table)), ..Rates::default() }
    }

    pub fn table(&self) -> Result<&RateTable> {
        if self.table.get().is_none() && self.provider.as_ref().is_some_and(|p| p.pending()) {
            return Err(Error::pending("exchange rates are on their way"));
        }
        let loaded = self.table.get_or_init(|| match &self.provider {
            Some(p) => p.rates(),
            None => Err("no exchange rates available".into()),
        });
        loaded.as_ref().map_err(|e| Error::new(e.clone()))
    }

    /// The provider's own rate between two currencies.
    pub fn pair_rate(&self, from: &str, to: &str) -> Option<Number> {
        self.provider.as_ref()?.pair_rate(from, to)
    }

    /// Value of one unit of `code` in US dollars; with `fallback` the bundled rates
    /// stand in for currencies the provider doesn't know, or for no rates at all.
    pub fn usd_value(&self, code: &str, fallback: bool) -> Result<Number> {
        if code == "USD" {
            return Ok(Number::ONE);
        }
        let rate = match self.table() {
            Ok(table) => table.get(code).or_else(|| fallback.then(|| bundled_rates().get(code)).flatten()),
            // Rates on their way are waited for, not stood in for.
            Err(e) if e.is_pending() => return Err(e),
            Err(_) if fallback => bundled_rates().get(code),
            Err(e) => return Err(e),
        };
        match rate {
            Some(rate) if !rate.is_zero() => Ok(Number::ONE / rate),
            _ => Err(Error::new(format!("no exchange rate for {code}"))),
        }
    }

    /// Value of one unit of `code` in US dollars from rates at hand: loaded ones, the
    /// provider's cache, or the bundled ones. Nothing is downloaded for it.
    pub fn usd_value_at_hand(&self, code: &str) -> Result<Number> {
        if code == "USD" {
            return Ok(Number::ONE);
        }
        let loaded = self.table.get().and_then(|t| t.as_ref().ok());
        let cached = || self.cached.get_or_init(|| self.provider.as_ref()?.cached_rates()).as_ref();
        let rate = [loaded, cached()].into_iter().flatten().find_map(|t| t.get(code));
        match rate.or_else(|| bundled_rates().get(code)) {
            Some(rate) if !rate.is_zero() => Ok(Number::ONE / rate),
            _ => Err(Error::new(format!("no exchange rate for {code}"))),
        }
    }

    fn dated_known(&self, date: Date) -> bool {
        self.dated.lock().unwrap_or_else(PoisonError::into_inner).contains_key(&date)
    }

    /// Value of one unit of `code` in US dollars on a past day.
    pub fn usd_value_on(&self, code: &str, date: Date) -> Result<Number> {
        if code == "USD" {
            return Ok(Number::ONE);
        }
        if self.provider.as_ref().is_some_and(|p| p.pending()) && !self.dated_known(date) {
            return Err(Error::pending(format!("exchange rates for {date} are on their way")));
        }
        let table = {
            let mut dated = self.dated.lock().unwrap_or_else(PoisonError::into_inner);
            let entry = dated.entry(date).or_insert_with(|| match &self.provider {
                Some(p) => p.rates_on(date).map(Arc::new),
                None => Err(format!("exchange rates for {date} aren't known")),
            });
            entry.clone().map_err(Error::new)?
        };
        match table.get(code) {
            Some(rate) if !rate.is_zero() => Ok(Number::ONE / rate),
            _ => Err(Error::new(format!("no exchange rate for {code} on {date}"))),
        }
    }
}
