//! Settings that change how lines are read and answers are written.

use std::cell::OnceCell;

use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned, civil};

use crate::custom::{CustomFunction, CustomPlace, CustomType, CustomUnit, UnitSystem};

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

/// How answers are written, besides `Config::precision` and the separators, as
/// SoulverCore's `FormattingPreferences`.
#[derive(Clone, Debug)]
pub struct Formatting {
    /// Decimal places at most, instead of `Config::precision` significant digits.
    pub decimal_places: Option<u32>,
    /// Numbers with more whole digits than this are written in scientific notation: `1e21`.
    pub scientific_above: u32,
    /// Numbers with more zeros after the point than this are too: `1e-10`.
    pub scientific_below: u32,
    /// When big numbers are written with a scale: `5M`, `$2.5B`.
    pub notation: Notation,
    /// Numbers this big are written with a scale too, with `Notation::Auto`.
    pub notation_threshold: Option<Scale>,
    /// The scales money is written with.
    pub scalars: ScalarSystem,
    /// `€10.00` rather than `10.00 EUR`.
    pub currency_symbols: bool,
    /// Money to its smallest coin: $1.23 rather than $1.2345.
    pub round_currency: bool,
    /// `$1.00` rather than `$1`.
    pub currency_trailing_zeros: bool,
    /// The host's symbols for currencies, and where they go: `100 kr`, `100 €`.
    pub symbols: Vec<CurrencySymbol>,
    /// Earlier lines and variables give the value as shown, rounded.
    pub references_take_formatted_value: bool,
    pub timespans: TimespanStyle,
    /// `5 ft 6 in` rather than `5.5 ft`, `1 lb 4 oz` rather than `1.25 lb`.
    pub compound_imperial_units: bool,
    /// Answers converted without asking for it.
    pub conversions: ResultConversions,
}

impl Formatting {
    /// Magnitudes written out in digits; others take scientific notation.
    pub(crate) fn plain_magnitudes(&self) -> std::ops::Range<i32> {
        -(self.scientific_below.min(28) as i32)..self.scientific_above.min(28) as i32
    }
}

impl Default for Formatting {
    fn default() -> Formatting {
        Formatting {
            decimal_places: None,
            scientific_above: 21,
            scientific_below: 9,
            notation: Notation::Auto,
            notation_threshold: None,
            scalars: ScalarSystem::Standard,
            currency_symbols: true,
            round_currency: true,
            currency_trailing_zeros: true,
            symbols: Vec::new(),
            references_take_formatted_value: false,
            timespans: TimespanStyle::Automatic,
            compound_imperial_units: false,
            conversions: ResultConversions::None,
        }
    }
}

/// When big numbers are written with a scale, as `5M` or `$2.5B`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notation {
    /// When the line writes one (`$5M / 2`) and the answer stays exact with it, or the
    /// number reaches `Formatting::notation_threshold`.
    Auto,
    /// From a thousand on: `100k`.
    On,
    Off,
}

/// A power of a thousand, for `Formatting::notation_threshold`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scale {
    Thousand,
    Million,
    Billion,
    Trillion,
}

impl Scale {
    pub fn value(self) -> Number {
        Number::pow10(3 * (self as i32 + 1))
    }
}

/// The scales money is written with: a billion dollars is `$1B`, `$1b`, `$1bn`...
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarSystem {
    /// `$5K`, `$5M`, `$5B`, `$5T`.
    Standard,
    /// `$5k`, `$5m`, `$5b`, `$5t`.
    Lowercase,
    /// `£5k`, `£5m`, `£5bn`, `£5tn`.
    British,
    /// `₽5 тыс.`, `₽5 млн`, `₽5 млрд`, `₽5 трлн`.
    Russian,
    /// `¥5万`, `¥5亿`.
    Chinese,
}

/// A currency symbol of the host's own and where it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrencySymbol {
    /// ISO code: "SEK".
    pub code: String,
    /// "kr"; lines may use it too.
    pub symbol: String,
    pub position: SymbolPosition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolPosition {
    /// `$5`
    Before,
    /// `Fr. 5`
    BeforeWithSpace,
    /// `5€`
    After,
    /// `5 kr`
    AfterWithSpace,
}

/// How date intervals are written: `Jan 1 to Mar 15`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimespanStyle {
    /// Years, months, weeks and days: "2 months 2 weeks".
    #[default]
    Automatic,
    /// "2 months 14 days".
    MonthsAndDays,
    /// "10 weeks 3 days".
    WeeksAndDays,
    /// "73 days".
    Days,
}

/// Answers converted without asking for it.
#[derive(Clone, Debug, Default)]
pub enum ResultConversions {
    #[default]
    None,
    /// Metric amounts take the prefix that keeps them between 1 and 1,000: `1500 m` is 1.5 km.
    Automatic,
    Rules(Vec<ConversionRule>),
}

/// One conversion of answers.
#[derive(Clone, Debug)]
pub enum ConversionRule {
    /// Answers in this unit go to another: `{ from: "mi", to: "km" }`.
    Unit { from: String, to: String },
    /// Answers of the same kind as `like` go to `to`: `{ like: "m", to: "ft" }` for every length.
    Kind { like: String, to: String },
    /// Times are shown in this zone.
    TimeZone(TimeZone),
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
    /// The rates that come with the engine stand in for currencies the rate provider
    /// doesn't know, and for no rates at all.
    pub use_default_rates_for_unhandled_currencies: bool,
    /// `$100 in EUR on Jan 1, 2020` asks the rate provider for that day's rates.
    pub historical_currency_queries: bool,
    /// Places the engine doesn't know are asked from the place provider.
    pub dynamic_place_queries: bool,
    /// Weather on other days than today is asked from the weather provider.
    pub historical_weather_queries: bool,
    /// A dash between spaces ends a label, as a colon does: `Coffee - 12.50` is 12.50.
    pub dash_separated_labels: bool,
    /// A list marker starting a line is no value: `1. `, `a) `, `- `, `• `.
    pub list_marker_parsing: bool,
    /// A unit and a number starting a line are a label: `Day 8 $50`, `Week 4: 30 km`.
    pub ordinal_label_parsing: bool,
    /// `4 out of 10` is the fraction 2/5 instead of 40%.
    pub proper_fraction_parsing: bool,
    /// A line that is a repeating time is a cadence: `every 2 weeks until Dec 31`, `daily`.
    pub cadence_parsing: bool,
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
            use_default_rates_for_unhandled_currencies: true,
            historical_currency_queries: true,
            dynamic_place_queries: true,
            historical_weather_queries: true,
            dash_separated_labels: false,
            list_marker_parsing: true,
            ordinal_label_parsing: false,
            proper_fraction_parsing: false,
            cadence_parsing: true,
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
    /// `as pitch`, `as midi`.
    Pitch,
    /// `as dms`.
    Dms,
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
    /// The first day of a week: `start of week`, `this friday`.
    pub week_start: civil::Weekday,
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
    /// Phrases of the host's own, worked out by its handlers.
    pub custom_functions: Vec<CustomFunction>,
    /// Words the host's placeholders match.
    pub custom_types: Vec<CustomType>,
    pub features: Features,
    pub formatting: Formatting,
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
            week_start: civil::Weekday::Monday,
            holidays: Vec::new(),
            unit_system: UnitSystem::CustomaryUs,
            base_font_size: Number::from_i64(16),
            custom_units: Vec::new(),
            custom_places: Vec::new(),
            custom_functions: Vec::new(),
            custom_types: Vec::new(),
            features: Features::default(),
            formatting: Formatting::default(),
            time_zone: None,
            now: None,
        }
    }
}

impl Config {
    /// Settings as people in a locale expect them, as SoulverCore's `standardWith(locale:)`:
    /// "de_DE" writes `1.234,50 €` with 24-hour clocks and metric cups, "en_US" `$1,234.50`
    /// with dates month first and weeks from Sunday. Only English words are read either way.
    pub fn for_locale(locale: &str) -> Config {
        let locale = locale.split(['.', '@']).next().unwrap_or_default().replace('-', "_");
        let (lang, region) = locale.split_once('_').unwrap_or((&locale, ""));
        let (lang, region) = (lang.to_lowercase(), region.to_uppercase());
        let mut config = Config::default();
        const COMMA: &[&str] = &[
            "az", "be", "bg", "bs", "ca", "cs", "da", "de", "el", "es", "et", "eu", "fi", "fr", "gl", "hr", "hu", "hy",
            "id", "is", "it", "ka", "kk", "ky", "lt", "lv", "mk", "nb", "nl", "nn", "no", "pl", "pt", "ro", "ru", "sk",
            "sl", "sq", "sr", "sv", "tr", "uk", "uz", "vi",
        ];
        // Regions that use a decimal point despite the language.
        const POINT: &[&str] = &["CH", "MX", "US", "PR"];
        config.decimal_comma = COMMA.contains(&lang.as_str()) && !POINT.contains(&region.as_str());
        let twelve_hours = ["US", "CA", "AU", "NZ", "IN", "PH", "PK", "EG", "SA"];
        config.clock_24h = !(lang == "en" && twelve_hours.contains(&region.as_str()));
        if region == "US" || region == "PH" {
            config.date_order = DateOrder::MonthFirst;
        }
        config.unit_system = match region.as_str() {
            "US" | "LR" | "MM" => UnitSystem::CustomaryUs,
            "GB" => UnitSystem::Imperial,
            _ => UnitSystem::Metric,
        };
        let sunday = ["US", "CA", "JP", "BR", "MX", "IL", "IN", "PH", "KR", "TW", "HK", "ZA", "AU", "SA"];
        if sunday.contains(&region.as_str()) {
            config.week_start = civil::Weekday::Sunday;
        }
        let currency = local_currency(&region);
        // "$" is the local dollar: Canadian in Canada.
        if let Some(code) =
            currency.filter(|c| crate::currency::find(c).is_some_and(|c| c.symbol.is_some_and(|s| s.ends_with('$'))))
        {
            config.dollar = code.to_string();
        }
        // Most of Europe writes the symbol after the amount: "5,00 €".
        let after = [
            "de", "fr", "es", "it", "pt", "fi", "sk", "sl", "lt", "lv", "et", "el", "pl", "cs", "hu", "ru", "uk", "sv",
            "da", "nb", "no", "nn", "is", "ro", "bg", "hr", "sr", "kk",
        ];
        if let Some(code) = currency
            && after.contains(&lang.as_str())
            && let Some(found) = crate::currency::find(code)
        {
            let symbol = match code {
                "SEK" | "NOK" | "DKK" | "ISK" => "kr",
                "PLN" => "zł",
                "CZK" => "Kč",
                "HUF" => "Ft",
                "RON" => "lei",
                "BGN" => "лв.",
                "CHF" => "CHF",
                _ => found.symbol.unwrap_or(found.code),
            };
            let symbol = CurrencySymbol {
                code: code.to_string(),
                symbol: symbol.to_string(),
                position: SymbolPosition::AfterWithSpace,
            };
            config.formatting.symbols.push(symbol);
        }
        config
    }

    /// As SoulverCore's `unformatted` preferences: every digit, no separators, currency codes.
    pub fn unformatted(self) -> Config {
        let formatting = Formatting {
            notation: Notation::Off,
            currency_symbols: false,
            round_currency: false,
            currency_trailing_zeros: false,
            ..self.formatting
        };
        Config { precision: 28, thousands_separators: false, formatting, ..self }
    }

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

/// The currency of a country, by its ISO code.
fn local_currency(region: &str) -> Option<&'static str> {
    const EURO: &[&str] = &[
        "AT", "BE", "CY", "DE", "EE", "ES", "FI", "FR", "GR", "HR", "IE", "IT", "LT", "LU", "LV", "MT", "NL", "PT",
        "SI", "SK",
    ];
    if EURO.contains(&region) {
        return Some("EUR");
    }
    Some(match region {
        "US" => "USD",
        "GB" => "GBP",
        "CA" => "CAD",
        "AU" => "AUD",
        "NZ" => "NZD",
        "CH" => "CHF",
        "JP" => "JPY",
        "CN" => "CNY",
        "IN" => "INR",
        "RU" => "RUB",
        "UA" => "UAH",
        "KZ" => "KZT",
        "PL" => "PLN",
        "CZ" => "CZK",
        "HU" => "HUF",
        "RO" => "RON",
        "BG" => "BGN",
        "SE" => "SEK",
        "NO" => "NOK",
        "DK" => "DKK",
        "IS" => "ISK",
        "TR" => "TRY",
        "BR" => "BRL",
        "MX" => "MXN",
        "AR" => "ARS",
        "ZA" => "ZAR",
        "KR" => "KRW",
        "SG" => "SGD",
        "HK" => "HKD",
        "TW" => "TWD",
        "IL" => "ILS",
        _ => return None,
    })
}
