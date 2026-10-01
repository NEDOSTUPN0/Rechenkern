//! Tests over the whole engine: snapshots of many lines (`snapshots.rs`) and
//! properties of random ones (`properties.rs`). `tests/calculations.rs` keeps
//! the exact cases.

mod properties;
mod snapshots;

use std::cell::Cell;

use jiff::tz::TimeZone;

use crate::{Calculator, Config, Number, RateTable};

thread_local! {
    /// How many times a line read the clock: answers that didn't can't depend on it.
    pub(crate) static CLOCK_READS: Cell<u64> = const { Cell::new(0) };
    /// How many times a line read earlier lines or variables.
    pub(crate) static SHEET_READS: Cell<u64> = const { Cell::new(0) };
}

/// Wed 23 Sep 2026 14:30 in New York, as in `tests/calculations.rs`.
fn config() -> Config {
    Config {
        now: Some("2026-09-23T14:30:00-04:00[America/New_York]".parse().unwrap()),
        time_zone: Some(TimeZone::get("America/New_York").unwrap()),
        ..Config::default()
    }
}

/// The exchange rates of `tests/calculations.rs`; every other currency is 2 to the dollar.
fn rates() -> RateTable {
    let mut rates = RateTable::new();
    // Rates are units per dollar.
    for currency in crate::currency::CURRENCIES.iter().filter(|c| c.code != "USD") {
        rates.insert(currency.code, Number::from_i64(2));
    }
    for (code, rate) in
        [("EUR", "0.9"), ("GBP", "0.8"), ("JPY", "150"), ("KZT", "500"), ("RUB", "90"), ("BTC", "0.00001")]
    {
        rates.insert(code, Number::parse(rate).unwrap());
    }
    rates
}

/// A calculator with fixed exchange rates.
fn calculator(config: Config) -> Calculator {
    let mut calc = Calculator::with_config(config);
    calc.set_rates(rates());
    calc
}

/// The answer as text, errors and empty answers included.
fn answer(calc: &mut Calculator, line: &str) -> String {
    match calc.calculate(line) {
        Ok(Some(answer)) => answer.to_string(),
        Ok(None) => "<none>".into(),
        Err(e) => format!("<error: {e}>"),
    }
}
