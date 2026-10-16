//! Vocabulary: keywords, number words, functions, months and holidays.

use std::borrow::Cow;
use std::sync::{LazyLock, OnceLock};

use jiff::civil::Weekday;

use crate::ast::{Format, Func, Holiday, Sky, Which};
use crate::hash::TableMap;
use crate::number::Number;
use crate::units::foods::{self, Food};
use crate::units::registry;
use crate::weather::WeatherQuery;

/// Phrases and what they mean, found by their first word.
pub struct Phrases<T: 'static> {
    entries: &'static [(&'static str, T)],
    /// Lowercase first word -> positions in `entries`, in table order.
    by_first_word: OnceLock<TableMap<String, Vec<usize>>>,
}

impl<T> Phrases<T> {
    pub const fn new(entries: &'static [(&'static str, T)]) -> Phrases<T> {
        Phrases { entries, by_first_word: OnceLock::new() }
    }

    /// All phrases, in table order.
    pub fn phrases(&self) -> impl Iterator<Item = &'static str> {
        self.entries.iter().map(|(phrase, _)| *phrase)
    }

    /// The first phrase that means `value`.
    pub fn first_for(&self, value: T) -> Option<&'static str>
    where
        T: PartialEq,
    {
        self.entries.iter().find(|(_, v)| *v == value).map(|(phrase, _)| *phrase)
    }

    /// Entries whose first word is `word` (ASCII case ignored), in table order.
    pub fn starting_with(&self, word: &str) -> impl Iterator<Item = &(&'static str, T)> {
        let index = self.by_first_word.get_or_init(|| {
            let mut index: TableMap<String, Vec<usize>> = TableMap::default();
            for (k, (phrase, _)) in self.entries.iter().enumerate() {
                let first = phrase.split([' ', '-']).next().unwrap_or(phrase);
                index.entry(first.to_ascii_lowercase()).or_default().push(k);
            }
            index
        });
        let key = match word.bytes().any(|b| b.is_ascii_uppercase()) {
            true => Cow::Owned(word.to_ascii_lowercase()),
            false => Cow::Borrowed(word),
        };
        index.get(key.as_ref()).into_iter().flatten().map(|&k| &self.entries[k])
    }
}

/// Every word and phrase the engine knows, lowercase: functions, units, places,
/// holidays, formats, keywords. For autocompletion.
pub fn known_words() -> impl Iterator<Item = &'static str> {
    static WORDS: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| {
        let mut words: Vec<String> = FUNCTIONS
            .phrases()
            .chain(FORMATS.phrases())
            .chain(DATE_PARTS.phrases())
            .chain(HOLIDAYS.phrases())
            .chain(SKY.phrases())
            .chain(WEATHER.phrases())
            .chain(KEYWORDS.iter().copied())
            .chain(registry().spellings())
            .chain(crate::zones::place_names())
            .map(str::to_lowercase)
            .filter(|w| w.chars().any(char::is_alphabetic))
            .collect();
        words.sort();
        words.dedup();
        words
    });
    WORDS.iter().map(String::as_str)
}

/// Words the expression parser acts on. Anything unknown is ignored as a comment.
#[rustfmt::skip]
pub const KEYWORDS: &[&str] = &[
    "in", "to", "as", "into", "of", "on", "off", "at", "per", "plus", "minus", "times", "multiplied",
    "divided", "mod", "modulo", "and", "or", "xor", "ago", "from", "after", "before", "until", "till",
    "since", "between", "next", "last", "this", "now", "today", "tomorrow", "yesterday", "time", "date", "noon",
    "midnight", "am", "pm", "percent", "pct", "percentage", "squared", "cubed", "rounded", "nearest", "power",
    "square", "cubic", "remainder", "twice", "double", "triple", "quadruple", "permutation", "permutations",
    "combination", "combinations", "choose", "difference", "later", "base", "timestamp", "current", "unix",
    "epoch", "week", "day", "month", "year", "true", "false",
];

pub fn number_word(w: &str) -> Option<i64> {
    const SMALL: &[&str] = &[
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: &[&str] = &["twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
    if let Some(i) = SMALL.iter().position(|&s| s == w) {
        return Some(i as i64);
    }
    TENS.iter().position(|&s| s == w).map(|i| (i as i64 + 2) * 10)
}

/// Words that multiply the number before them: `3 million`, `2 dozen`.
pub fn scale_word(w: &str) -> Option<Number> {
    let n = match w.to_lowercase().as_str() {
        "hundred" => Number::from_i64(100),
        "thousand" => Number::pow10(3),
        "million" | "mil" | "mn" => Number::pow10(6),
        "billion" | "bn" => Number::pow10(9),
        "trillion" | "tn" => Number::pow10(12),
        "quadrillion" => Number::pow10(15),
        "тыс" => Number::pow10(3),
        "млн" => Number::pow10(6),
        "млрд" => Number::pow10(9),
        "трлн" => Number::pow10(12),
        "dozen" => Number::from_i64(12),
        "gross" => Number::from_i64(144),
        _ => return None,
    };
    // A unit symbol wins: "5 mN" is millinewtons.
    registry().lookup(w).is_none().then_some(n)
}

/// Letter suffixes glued to numbers: `5k`, `2.5M`, `10G`; also Cyrillic
/// (`5к`, `3М`) and Chinese (`5千`, `3万`).
pub fn suffix_multiplier(w: &str) -> Option<Number> {
    Some(match w {
        "k" | "к" | "К" | "千" => Number::pow10(3),
        "万" => Number::pow10(4),
        "M" | "М" => Number::pow10(6),
        "亿" => Number::pow10(8),
        "G" | "Г" => Number::pow10(9),
        "T" | "Т" => Number::pow10(12),
        _ => return None,
    })
}

/// Extra suffixes after currency amounts: `$5K`, `$3m`, `$7B`, `$10t`.
pub fn money_multiplier(w: &str) -> Option<Number> {
    Some(match w {
        "K" => Number::pow10(3),
        "m" => Number::pow10(6),
        "B" | "b" => Number::pow10(9),
        "t" => Number::pow10(12),
        _ => return suffix_multiplier(w),
    })
}

/// Ordinal numbers spelled out, for the degree of a root: `fourth root of 16`.
pub fn ordinal_word(w: &str) -> Option<i64> {
    let ordinals = ["second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth"];
    ordinals.iter().position(|&o| o == w).map(|k| k as i64 + 2)
}

/// Fractions spelled out: `half of 10`, `a third of 90`, `two thirds of 90`.
pub fn fraction_word(w: &str) -> Option<Number> {
    Some(match w {
        "half" | "halves" => Number::ratio(1, 2),
        "third" | "thirds" => Number::ratio(1, 3),
        "quarter" | "quarters" => Number::ratio(1, 4),
        "fifth" | "fifths" => Number::ratio(1, 5),
        "tenth" | "tenths" => Number::ratio(1, 10),
        _ => return None,
    })
}

pub fn constant(w: &str) -> Option<Number> {
    let text = match w {
        "pi" | "π" => "3.1415926535897932384626433833",
        "tau" | "τ" => "6.2831853071795864769252867666",
        "e" => "2.7182818284590452353602874714",
        "phi" | "φ" | "golden ratio" => "1.6180339887498948482045868344",
        _ => return None,
    };
    Number::parse(text)
}

/// Named physical constants; the number indexes values in the parser.
pub static PHYSICAL_CONSTANTS: Phrases<usize> = Phrases::new(&[
    ("speed of light", 0),
    ("speed of sound", 1),
    ("standard gravity", 2),
    ("earth gravity", 2),
    ("avogadro's number", 3),
    ("avogadro constant", 3),
]);

/// Words between a base and its exponent: "3 raised to 9".
pub static POWER_WORDS: Phrases<()> = Phrases::new(&[
    ("to the power of", ()),
    ("to the power", ()),
    ("to the", ()),
    ("raised to the power of", ()),
    ("raised to the power", ()),
    ("raised to the", ()),
    ("raised to", ()),
    ("exponent", ()),
    ("power", ()),
]);

/// The function a name (or one of its other names) stands for: "avg" is the average.
pub fn function_named(name: &str) -> Option<Func> {
    let name = name.trim().to_lowercase();
    let first = name.split([' ', '-']).next()?;
    FUNCTIONS.starting_with(first).find(|(phrase, _)| *phrase == name).map(|&(_, func)| func)
}

pub static FUNCTIONS: Phrases<Func> = Phrases::new(&[
    ("sqrt", Func::Sqrt),
    ("square root", Func::Sqrt),
    ("cbrt", Func::Cbrt),
    ("cube root", Func::Cbrt),
    ("squared root", Func::Sqrt),
    ("cubed root", Func::Cbrt),
    ("root", Func::Root),
    ("exp", Func::Exp),
    ("ln", Func::Ln),
    ("natural log", Func::Ln),
    ("natural logarithm", Func::Ln),
    ("log", Func::Log),
    ("logarithm", Func::Log),
    ("log2", Func::Log2),
    ("binary log", Func::Log2),
    ("binary logarithm", Func::Log2),
    ("log10", Func::Log10),
    ("lg", Func::Log10),
    ("abs", Func::Abs),
    ("absolute value", Func::Abs),
    ("fact", Func::Fact),
    ("factorial", Func::Fact),
    ("round", Func::Round),
    ("ceil", Func::Ceil),
    ("ceiling", Func::Ceil),
    ("floor", Func::Floor),
    ("trunc", Func::Trunc),
    ("sin", Func::Sin),
    ("cos", Func::Cos),
    ("tan", Func::Tan),
    ("asin", Func::Asin),
    ("acos", Func::Acos),
    ("atan", Func::Atan),
    ("arcsin", Func::Asin),
    ("arccos", Func::Acos),
    ("arctan", Func::Atan),
    ("sinh", Func::Sinh),
    ("cosh", Func::Cosh),
    ("tanh", Func::Tanh),
    ("asinh", Func::Asinh),
    ("acosh", Func::Acosh),
    ("atanh", Func::Atanh),
    ("sind", Func::SinD),
    ("cosd", Func::CosD),
    ("tand", Func::TanD),
    ("asind", Func::AsinD),
    ("acosd", Func::AcosD),
    ("atand", Func::AtanD),
    ("min", Func::Min),
    ("minimum", Func::Min),
    ("smaller", Func::Min),
    ("smallest", Func::Min),
    ("lesser", Func::Min),
    ("max", Func::Max),
    ("maximum", Func::Max),
    ("larger", Func::Max),
    ("largest", Func::Max),
    ("greater", Func::Max),
    ("greatest", Func::Max),
    ("bigger", Func::Max),
    ("biggest", Func::Max),
    ("gcd", Func::Gcd),
    ("hcf", Func::Gcd),
    ("gcf", Func::Gcd),
    ("greatest common divisor", Func::Gcd),
    ("greatest common factor", Func::Gcd),
    ("highest common factor", Func::Gcd),
    ("lcm", Func::Lcm),
    ("lowest common multiple", Func::Lcm),
    ("least common multiple", Func::Lcm),
    ("sum", Func::Sum),
    ("total", Func::Sum),
    ("average", Func::Average),
    ("avg", Func::Average),
    ("mean", Func::Average),
    ("median", Func::Median),
    ("count", Func::Count),
    ("standard deviation", Func::StdDev),
    ("stddev", Func::StdDev),
    ("stdev", Func::StdDev),
    ("std dev", Func::StdDev),
    ("std deviation", Func::StdDev),
    ("sample standard deviation", Func::StdDev),
    ("midpoint", Func::Midpoint),
    ("mid point", Func::Midpoint),
    ("mid-point", Func::Midpoint),
    ("halfway", Func::Midpoint),
    ("random number", Func::Random),
    ("random", Func::Random),
    ("rand", Func::Random),
    ("clamp", Func::Clamp),
    ("ncr", Func::Comb),
    ("npr", Func::Perm),
    ("hex", Func::Hex),
    ("bin", Func::Bin),
    ("oct", Func::Oct),
    ("int", Func::Int),
]);

pub static FORMATS: Phrases<Format> = Phrases::new(&[
    ("hex", Format::Hex),
    ("hexadecimal", Format::Hex),
    ("base 16", Format::Hex),
    ("binary", Format::Binary),
    ("bin", Format::Binary),
    ("base 2", Format::Binary),
    ("octal", Format::Octal),
    ("oct", Format::Octal),
    ("base 8", Format::Octal),
    ("decimal", Format::Decimal),
    ("dec", Format::Decimal),
    ("base 10", Format::Decimal),
    ("scientific notation", Format::Scientific),
    ("scientific", Format::Scientific),
    ("sci", Format::Scientific),
    ("fraction", Format::Fraction),
    ("fractions", Format::Fraction),
    ("number", Format::Number),
    ("num", Format::Number),
    ("%", Format::Percent),
    ("percent", Format::Percent),
    ("percentage", Format::Percent),
    ("pct", Format::Percent),
    ("multiplier", Format::Multiplier),
    ("multiple", Format::Multiplier),
    ("timespan", Format::Timespan),
    ("time span", Format::Timespan),
    ("laptime", Format::Laptime),
    ("lap time", Format::Laptime),
    ("hms", Format::Laptime),
    ("unix timestamp", Format::Timestamp),
    ("unix time stamp", Format::Timestamp),
    ("unix time", Format::Timestamp),
    ("timestamp", Format::Timestamp),
    ("time stamp", Format::Timestamp),
    ("unix", Format::Timestamp),
    ("epoch", Format::Timestamp),
    ("date", Format::Date),
    ("iso 8601", Format::Iso),
    ("iso8601", Format::Iso),
    ("iso", Format::Iso),
    ("day of the week", Format::Weekday),
    ("day of week", Format::Weekday),
    ("weekday", Format::Weekday),
    ("pitch", Format::Pitch),
    ("note", Format::Pitch),
    ("midi", Format::Midi),
    ("midi note", Format::Midi),
    ("dms", Format::Dms),
    ("degrees minutes seconds", Format::Dms),
    ("degrees minutes and seconds", Format::Dms),
    ("megapixels", Format::Megapixels),
    ("megapixel", Format::Megapixels),
    ("mp", Format::Megapixels),
    ("frames", Format::Frames),
]);

/// Questions about a date: "week of year", "weekday on March 9".
pub static DATE_PARTS: Phrases<Format> = Phrases::new(&[
    ("week of year", Format::WeekNumber),
    ("week of the year", Format::WeekNumber),
    ("week number", Format::WeekNumber),
    ("day of year", Format::DayOfYear),
    ("day of the year", Format::DayOfYear),
    ("day number", Format::DayOfYear),
    ("day of month", Format::DayOfMonth),
    ("day of the month", Format::DayOfMonth),
    ("day of week", Format::Weekday),
    ("day of the week", Format::Weekday),
    ("weekday", Format::Weekday),
]);

/// What a weather line asks for.
pub static WEATHER: Phrases<WeatherQuery> = Phrases::new(&[
    ("weather", WeatherQuery::Summary),
    ("temperature", WeatherQuery::Temperature),
    ("temp", WeatherQuery::Temperature),
    ("high", WeatherQuery::High),
    ("high temperature", WeatherQuery::High),
    ("low", WeatherQuery::Low),
    ("low temperature", WeatherQuery::Low),
    ("humidity", WeatherQuery::Humidity),
    ("wind", WeatherQuery::WindSpeed),
    ("wind speed", WeatherQuery::WindSpeed),
    ("uv", WeatherQuery::UvIndex),
    ("uv index", WeatherQuery::UvIndex),
    ("chance of rain", WeatherQuery::ChanceOfRain),
    ("rain chance", WeatherQuery::ChanceOfRain),
    ("chance of precipitation", WeatherQuery::ChanceOfRain),
]);

/// The sun and the moon. "sunlight" and "daylight" alone need a place after them.
pub static SKY: Phrases<Sky> = Phrases::new(&[
    ("sunrise", Sky::Sunrise),
    ("sunset", Sky::Sunset),
    ("hours of sunlight", Sky::Daylight),
    ("hours of daylight", Sky::Daylight),
    ("sunlight hours", Sky::Daylight),
    ("daylight hours", Sky::Daylight),
    ("sunlight", Sky::Daylight),
    ("daylight", Sky::Daylight),
    ("moon phase", Sky::MoonPhase),
    ("moonphase", Sky::MoonPhase),
    ("phase of the moon", Sky::MoonPhase),
    ("lunar phase", Sky::MoonPhase),
    ("lunar day", Sky::LunarDay),
    ("moon day", Sky::LunarDay),
]);

/// Foods by name: "1 cup of flour", "density of butter".
pub static FOODS: LazyLock<Phrases<Food>> = LazyLock::new(|| Phrases::new(foods::all()));

/// Words before a food that don't change what a cup of it weighs: "1 cup melted butter".
pub const FOOD_STATES: &[&str] = &[
    "fresh", "melted", "softened", "cold", "warm", "hot", "chilled", "unsalted", "salted", "organic", "raw",
    "uncooked", "dry", "plain", "pure",
];

pub static HOLIDAYS: Phrases<Holiday> = Phrases::new(&[
    ("new year's eve", Holiday::NewYearsEve),
    ("new years eve", Holiday::NewYearsEve),
    ("new year's day", Holiday::NewYear),
    ("new years day", Holiday::NewYear),
    ("new year's", Holiday::NewYear),
    ("new years", Holiday::NewYear),
    ("new year", Holiday::NewYear),
    ("valentine's day", Holiday::Valentines),
    ("valentines day", Holiday::Valentines),
    ("valentine's", Holiday::Valentines),
    ("valentines", Holiday::Valentines),
    ("easter sunday", Holiday::Easter),
    ("easter monday", Holiday::EasterMonday),
    ("easter eve", Holiday::HolySaturday),
    ("easter saturday", Holiday::HolySaturday),
    ("easter friday", Holiday::GoodFriday),
    ("easter", Holiday::Easter),
    ("good friday", Holiday::GoodFriday),
    ("holy friday", Holiday::GoodFriday),
    ("holy saturday", Holiday::HolySaturday),
    ("orthodox easter", Holiday::OrthodoxEaster),
    ("orthodox easter sunday", Holiday::OrthodoxEaster),
    ("orthodox pascha", Holiday::OrthodoxEaster),
    ("greek easter", Holiday::OrthodoxEaster),
    ("russian easter", Holiday::OrthodoxEaster),
    ("orthodox good friday", Holiday::OrthodoxGoodFriday),
    ("greek good friday", Holiday::OrthodoxGoodFriday),
    ("halloween", Holiday::Halloween),
    ("thanksgiving", Holiday::Thanksgiving),
    ("black friday", Holiday::BlackFriday),
    ("christmas eve", Holiday::ChristmasEve),
    ("christmas day", Holiday::Christmas),
    ("christmas", Holiday::Christmas),
    ("xmas", Holiday::Christmas),
    ("boxing day", Holiday::BoxingDay),
    ("orthodox christmas", Holiday::OrthodoxChristmas),
    ("orthodox christmas day", Holiday::OrthodoxChristmas),
    ("russian christmas", Holiday::OrthodoxChristmas),
    ("serbian christmas", Holiday::OrthodoxChristmas),
    ("coptic christmas", Holiday::OrthodoxChristmas),
    ("ethiopian christmas", Holiday::OrthodoxChristmas),
    ("chinese new year", Holiday::ChineseNewYear),
    ("lunar new year", Holiday::ChineseNewYear),
    ("spring festival", Holiday::ChineseNewYear),
    ("chinese new year's eve", Holiday::ChineseNewYearsEve),
    ("chinese new years eve", Holiday::ChineseNewYearsEve),
    ("chinese new year eve", Holiday::ChineseNewYearsEve),
    ("lunar new year's eve", Holiday::ChineseNewYearsEve),
    ("lunar new years eve", Holiday::ChineseNewYearsEve),
]);

/// An event with a known date.
pub struct Event {
    /// Lowercase names; numbers as digits.
    pub names: &'static [&'static str],
    pub date: (i16, i8, i8),
    /// Shown next to the date on its own, not used in calculations.
    pub note: &'static str,
}

pub const EVENTS: &[Event] = &[Event {
    names: &["gta 6", "gta vi", "gta6", "gtavi", "grand theft auto 6", "grand theft auto vi"],
    // Delayed twice already; drop the note once it is out.
    date: (2026, 11, 19),
    note: "probably",
}];

/// First month of a quarter (`q1` to `q4`) or of a season, as meteorologists in the
/// northern hemisphere count them: summer is June to August.
pub fn three_months(w: &str) -> Option<i8> {
    let season = match w {
        "spring" => 3,
        "summer" => 6,
        "autumn" | "fall" => 9,
        "winter" => 12,
        _ => {
            let quarter: i8 = w.strip_prefix('q').and_then(|q| q.parse().ok()).filter(|q| (1..=4).contains(q))?;
            return Some((quarter - 1) * 3 + 1);
        }
    };
    Some(season)
}

/// `next`, `last` or `this` before a period.
pub fn which(w: &str) -> Option<Which> {
    match w {
        "next" => Some(Which::Next),
        "last" => Some(Which::Last),
        "this" => Some(Which::This),
        _ => None,
    }
}

/// Month number for a month name; `short` is set for abbreviations.
pub fn month(w: &str) -> Option<(i8, bool)> {
    const NAMES: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    if let Some(i) = NAMES.iter().position(|&n| n == w) {
        // "may" and "march" are also common English words.
        return Some((i as i8 + 1, matches!(w, "may" | "march")));
    }
    let short = match w {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" | "sept" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _ => return None,
    };
    Some((short, true))
}

/// Weekday for a day name; `short` is set for abbreviations.
pub fn weekday(w: &str) -> Option<(Weekday, bool)> {
    use Weekday::*;
    Some(match w {
        "monday" => (Monday, false),
        "tuesday" => (Tuesday, false),
        "wednesday" => (Wednesday, false),
        "thursday" => (Thursday, false),
        "friday" => (Friday, false),
        "saturday" => (Saturday, false),
        "sunday" => (Sunday, false),
        "mon" => (Monday, true),
        "tue" | "tues" => (Tuesday, true),
        "wed" => (Wednesday, true),
        "thu" | "thur" | "thurs" => (Thursday, true),
        "fri" => (Friday, true),
        "sat" => (Saturday, true),
        "sun" => (Sunday, true),
        _ => return None,
    })
}

/// Words that make a whole line a cadence: "daily", "monthly".
pub fn is_cadence_word(w: &str) -> bool {
    matches!(
        w,
        "hourly" | "daily" | "weekly" | "biweekly" | "fortnightly" | "monthly" | "quarterly" | "yearly" | "annually"
    )
}

/// Words that refer to earlier answers.
pub fn is_line_word(w: &str) -> bool {
    matches!(
        w,
        "prev"
            | "previous"
            | "ans"
            | "answer"
            | "above"
            | "sum"
            | "total"
            | "subtotal"
            | "average"
            | "avg"
            | "mean"
            | "line"
    ) || w.strip_prefix("line").is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}
