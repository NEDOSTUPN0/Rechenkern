//! Vocabulary: keywords, number words, functions, months and holidays.

use std::borrow::Cow;
use std::sync::OnceLock;

use jiff::civil::Weekday;

use crate::ast::{Format, Func, Holiday};
use crate::hash::TableMap;
use crate::number::Number;
use crate::units::registry;

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

    /// Entries whose first word is `word` (ASCII case ignored), in table order.
    pub fn starting_with(&self, word: &str) -> impl Iterator<Item = &(&'static str, T)> {
        let index = self.by_first_word.get_or_init(|| {
            let mut index: TableMap<String, Vec<usize>> = TableMap::default();
            for (k, (phrase, _)) in self.entries.iter().enumerate() {
                let first = phrase.split(' ').next().unwrap_or(phrase);
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
        "dozen" => Number::from_i64(12),
        "gross" => Number::from_i64(144),
        _ => return None,
    };
    // A unit symbol wins: "5 mN" is millinewtons.
    registry().lookup(w).is_none().then_some(n)
}

/// Letter suffixes glued to numbers: `5k`, `2.5M`, `10G`.
pub fn suffix_multiplier(w: &str) -> Option<Number> {
    Some(match w {
        "k" => Number::pow10(3),
        "M" => Number::pow10(6),
        "G" => Number::pow10(9),
        "T" => Number::pow10(12),
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

pub static FUNCTIONS: Phrases<Func> = Phrases::new(&[
    ("sqrt", Func::Sqrt),
    ("square root", Func::Sqrt),
    ("cbrt", Func::Cbrt),
    ("cube root", Func::Cbrt),
    ("root", Func::Root),
    ("exp", Func::Exp),
    ("ln", Func::Ln),
    ("natural log", Func::Ln),
    ("log", Func::Log),
    ("logarithm", Func::Log),
    ("log2", Func::Log2),
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
    ("midpoint", Func::Midpoint),
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
