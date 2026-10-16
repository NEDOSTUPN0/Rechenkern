//! Settings that change how lines are read and answers are written.

use std::cell::OnceCell;

use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned, civil};

use crate::custom::{CustomPlace, CustomUnit, UnitSystem};

use crate::number::Number;

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

/// Parts of the language to turn off or read another way, as SoulverCore's
/// `EngineFeatureFlags`.
#[derive(Clone, Debug)]
pub struct Features {
    /// Units other than money: `5 km`. Off, their names are words like any other.
    pub units: bool,
    /// Dates and times: `today + 3 days`, `time in Tokyo`.
    pub calendar_calculations: bool,
    /// Phrases that work out a value: `20 is what % of 200`, `$1,000 after 3 years at 7%`.
    pub word_functions: bool,
    /// Conversions of an answer: `in grams`, `to 2 dp`, `as hex`.
    pub converters: bool,
    /// `name = value` lines.
    pub variable_declarations: bool,
    /// A `---` line forgets the variables above it.
    pub dividers_reset_variable_scope: bool,
    /// Functions need parentheses: `sqrt(16)`, not `sqrt 16`.
    pub functions_require_parentheses: bool,
    /// Functions to leave out, by any of their names: `"average"` also leaves out `avg`.
    pub disabled_functions: Vec<String>,
    /// Conversions to leave out.
    pub disabled_converters: Vec<Converter>,
    /// A line whose words make no calculation answers with a value in it: `iPhone 16 Pro $999`.
    pub in_ambiguity_prefer_something_to_nothing: bool,
    /// What a thousands separator before more than three digits means: `1,0000`.
    pub misplaced_thousands_separator: MisplacedSeparator,
    /// Links and email addresses are comments: `https://example.com`, `me@example.com`.
    pub url_detection: bool,
    /// A dash between spaces ends a label, as a colon does: `Coffee - 12.50` is 12.50.
    pub dash_separated_labels: bool,
    /// A list marker starting a line is no value: `1. `, `a) `, `- `, `• `.
    pub list_marker_parsing: bool,
    /// A unit and a number starting a line are a label: `Day 8 $50`, `Week 4: 30 km`.
    pub ordinal_label_parsing: bool,
    /// `4 out of 10` is the fraction 2/5 instead of 40%.
    pub proper_fraction_parsing: bool,
    pub time_zones: TimeZoneOptions,
    pub dates: DateOptions,
    pub comments: CommentOptions,
}

impl Default for Features {
    fn default() -> Features {
        Features {
            units: true,
            calendar_calculations: true,
            word_functions: true,
            converters: true,
            variable_declarations: true,
            dividers_reset_variable_scope: true,
            functions_require_parentheses: false,
            disabled_functions: Vec::new(),
            disabled_converters: Vec::new(),
            in_ambiguity_prefer_something_to_nothing: true,
            misplaced_thousands_separator: MisplacedSeparator::DecimalPoint,
            url_detection: true,
            dash_separated_labels: false,
            list_marker_parsing: true,
            ordinal_label_parsing: false,
            proper_fraction_parsing: false,
            time_zones: TimeZoneOptions::default(),
            dates: DateOptions::default(),
            comments: CommentOptions::default(),
        }
    }
}

/// Kinds of conversion, for `Features::disabled_converters`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Converter {
    /// `in km`, `in EUR`.
    Unit,
    /// `in feet and inches`, `in hours and minutes`.
    Parts,
    /// `in per week`.
    Rate,
    /// `in Tokyo`.
    TimeZone,
    /// `to 2 dp`, `to nearest 10`.
    Rounding,
    Hexadecimal,
    Binary,
    Octal,
    /// `in base 3`.
    Base,
    Decimal,
    Scientific,
    Fraction,
    /// `as number`.
    Number,
    Percentage,
    Multiplier,
    Timespan,
    Laptime,
    Timestamp,
    Date,
    Iso8601,
    /// `as weekday`, `as week number`, `as day of year`.
    DatePart,
    Pitch,
}

/// What a thousands separator before more than three digits means, as in `1,0000`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MisplacedSeparator {
    /// It is a decimal point: 1.
    DecimalPoint,
    /// It groups anyway: 10,000.
    Allow,
    /// The number ends before it: 1.
    Disallow,
}

#[derive(Clone, Debug, Default)]
pub struct TimeZoneOptions {
    /// Zone abbreviations count only in capitals: "PST", not "pst".
    pub require_uppercase_for_ambiguous_time_zones: bool,
    pub code_correction: ZoneCodeCorrection,
}

/// Whether a zone abbreviation names its region's time all year.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZoneCodeCorrection {
    /// "EST" and "EDT" both name New York's time, in summer and winter.
    Automatic,
    /// "EST" names New York's time all year, as people use it; "EDT" is always UTC-4.
    #[default]
    OnlyNonSummerCodes,
    /// Every abbreviation is its fixed offset: "EST" is UTC-5 in summer too.
    Never,
}

#[derive(Clone, Debug, Default)]
pub struct DateOptions {
    /// A date in a calculation with times starts at noon: `March 5 + 12 hours` is March 6.
    pub noon_for_single_dates: bool,
    /// A clock time without am or pm is in the day: `3:30` is 15:30, `9:30` stays 09:30.
    pub prefer_daytime_for_ambiguous_12_hour_times: bool,
    /// A date without a year is the next one, never one that has passed.
    pub prefer_future_for_ambiguous_dates: bool,
    /// A year may start a date: `2026 March 5`.
    pub leading_year_parsing: bool,
}

/// Which kinds of comment the parser skips, as SoulverCore's `CommentingOptions`.
#[derive(Clone, Debug)]
pub struct CommentOptions {
    /// `$999 (for iPhone 16)`.
    pub brackets: bool,
    /// `Boeing "747" $5`.
    pub quotation_marks: bool,
    /// `$5 // lunch`.
    pub trailing: bool,
    /// `Rent: $1,500`.
    pub labels: bool,
    /// `$5 #food`.
    pub hashtags: bool,
}

impl Default for CommentOptions {
    fn default() -> CommentOptions {
        CommentOptions { brackets: true, quotation_marks: true, trailing: true, labels: true, hashtags: true }
    }
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
    /// Working hours in a workday, for `40 hours in workdays`.
    pub hours_per_workday: Number,
    /// Days off besides weekends, skipped when counting workdays.
    pub holidays: Vec<civil::Date>,
    /// What "gallon", "cup" and "ton" mean.
    pub unit_system: UnitSystem,
    /// Pixels in an `em` or `rem`.
    pub base_font_size: Number,
    /// Units of the host's own.
    pub custom_units: Vec<CustomUnit>,
    /// Places of the host's own; they win over built-in places of the same name.
    pub custom_places: Vec<CustomPlace>,
    pub features: Features,
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
            hours_per_workday: Number::from_i64(8),
            holidays: Vec::new(),
            unit_system: UnitSystem::CustomaryUs,
            base_font_size: Number::from_i64(16),
            custom_units: Vec::new(),
            custom_places: Vec::new(),
            features: Features::default(),
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

    /// Fixed at a time read earlier.
    pub fn at(config: &'a Config, time: Zoned) -> Now<'a> {
        Now { config, time: OnceCell::from(time) }
    }

    /// The time, if the line read it.
    pub fn peek(&self) -> Option<&Zoned> {
        self.time.get()
    }

    pub fn get(&self) -> &Zoned {
        self.time.get_or_init(|| {
            #[cfg(test)]
            crate::tests::CLOCK_READS.with(|reads| reads.set(reads.get() + 1));
            self.config.now()
        })
    }
}
