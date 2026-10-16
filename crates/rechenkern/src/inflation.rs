//! Prices over the years: consumer price indexes for `$1,000 in 1990 dollars`.

use std::sync::LazyLock;

use crate::number::Number;

/// Consumer price indexes a host provides, as SoulverCore's `InflationDataProvider`:
/// for currencies besides the US dollar, or newer than the bundled ones.
pub trait InflationProvider: Send + Sync {
    /// The index of `currency` (ISO code) in `year`, or in a month of it; any base will do,
    /// as long as it is the same for every year. A whole year is its average.
    fn cpi(&self, currency: &str, year: i16, month: Option<i8>) -> Option<Number>;
}

/// The US consumer price index that comes with the engine (`data/cpi.txt`).
struct Bundled {
    /// Annual averages by year.
    years: Vec<(i16, Number)>,
    /// Months of the latest years, which have no average yet.
    months: Vec<(i16, i8, Number)>,
}

static BUNDLED: LazyLock<Bundled> = LazyLock::new(|| {
    let mut bundled = Bundled { years: Vec::new(), months: Vec::new() };
    for line in include_str!("data/cpi.txt").lines().filter(|l| !l.starts_with('#')) {
        let Some((when, value)) = line.split_once(' ') else { continue };
        let value = Number::parse(value).expect("an index in data/cpi.txt");
        match when.split_once('-') {
            Some((year, month)) => bundled.months.push((year.parse().unwrap(), month.parse().unwrap(), value)),
            None => bundled.years.push((when.parse().unwrap(), value)),
        }
    }
    bundled
});

/// The bundled US index in a year, or the latest month for a year without an average yet.
pub(crate) fn us_cpi(year: i16) -> Option<Number> {
    let bundled = &*BUNDLED;
    if let Some(&(_, value)) = bundled.years.iter().find(|&&(y, _)| y == year) {
        return Some(value);
    }
    bundled.months.iter().rev().find(|&&(y, _, _)| y == year).map(|&(_, _, value)| value)
}

/// The latest bundled US index: the price level now.
pub(crate) fn us_cpi_now() -> Number {
    let bundled = &*BUNDLED;
    match bundled.months.last() {
        Some(&(_, _, value)) => value,
        None => bundled.years.last().expect("data/cpi.txt has years").1,
    }
}
