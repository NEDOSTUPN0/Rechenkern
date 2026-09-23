//! Exchange rates and where they come from.

use std::collections::HashMap;
use std::sync::OnceLock;

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
}

/// Rates loaded lazily from a provider.
#[derive(Default)]
pub(crate) struct Rates {
    provider: Option<Box<dyn RateProvider>>,
    table: OnceLock<std::result::Result<RateTable, String>>,
}

impl Rates {
    pub fn with_provider(provider: Box<dyn RateProvider>) -> Rates {
        Rates { provider: Some(provider), table: OnceLock::new() }
    }

    pub fn with_table(table: RateTable) -> Rates {
        Rates { provider: None, table: OnceLock::from(Ok(table)) }
    }

    pub fn table(&self) -> Result<&RateTable> {
        let loaded = self.table.get_or_init(|| match &self.provider {
            Some(p) => p.rates(),
            None => Err("no exchange rates available".into()),
        });
        loaded.as_ref().map_err(|e| Error::new(e.clone()))
    }

    /// Value of one unit of `code` in US dollars.
    pub fn usd_value(&self, code: &str) -> Result<Number> {
        if code == "USD" {
            return Ok(Number::ONE);
        }
        match self.table()?.get(code) {
            Some(rate) if !rate.is_zero() => Ok(Number::ONE / rate),
            _ => Err(Error::new(format!("no exchange rate for {code}"))),
        }
    }
}
