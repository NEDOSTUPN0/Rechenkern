//! Settings that change how lines are read and answers are written.

use std::cell::OnceCell;

use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

/// How to read ambiguous numeric dates like `03/04/2026`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateOrder {
    /// 03/04/2026 is 3 April.
    DayFirst,
    /// 03/04/2026 is March 4.
    MonthFirst,
}

/// Unit for plain numbers passed to trigonometric functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AngleUnit {
    Radians,
    Degrees,
}

#[derive(Clone, Debug)]
pub struct Config {
    /// Significant digits in answers. Whole numbers are never rounded.
    pub precision: u32,
    /// Group thousands: `1,234,567`.
    pub thousands_separators: bool,
    /// Write `1.234,5` instead of `1,234.5`. Input takes both styles; this
    /// only decides a lone separator before three digits (`1,500`, `1.500`).
    pub decimal_comma: bool,
    /// Show clock times as `15:30` instead of `3:30 pm`.
    pub clock_24h: bool,
    /// Order for dates written with slashes. Dotted dates are always day first.
    pub date_order: DateOrder,
    /// Currency code meant by a bare `$`.
    pub dollar: String,
    pub angle_unit: AngleUnit,
    /// Local time zone; `None` uses the system zone.
    pub time_zone: Option<TimeZone>,
    /// Fixed current time, mostly for tests; `None` uses the clock.
    pub now: Option<Zoned>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            precision: 10,
            thousands_separators: true,
            decimal_comma: false,
            clock_24h: true,
            date_order: DateOrder::DayFirst,
            dollar: "USD".into(),
            angle_unit: AngleUnit::Radians,
            time_zone: None,
            now: None,
        }
    }
}

impl Config {
    pub fn local_zone(&self) -> TimeZone {
        self.time_zone.clone().unwrap_or_else(TimeZone::system)
    }

    /// The current time in the local zone.
    pub fn now(&self) -> Zoned {
        match &self.now {
            Some(now) => now.with_time_zone(self.local_zone()),
            None => Timestamp::now().to_zoned(self.local_zone()),
        }
    }
}

/// The current time for one line: read on first use, then fixed.
pub(crate) struct Now<'a> {
    config: &'a Config,
    time: OnceCell<Zoned>,
}

impl<'a> Now<'a> {
    pub fn new(config: &'a Config) -> Now<'a> {
        Now { config, time: OnceCell::new() }
    }

    pub fn get(&self) -> &Zoned {
        self.time.get_or_init(|| self.config.now())
    }
}
