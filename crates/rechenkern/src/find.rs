//! Data in text, as SoulverCore's `find` and `ObjectParser`: "We sold 130 copies
//! yesterday" holds a number and a date, "QF 123 tomorrow" a flight and its day.

use std::ops::Range;

use crate::ast::{Expr, TimeExpr};
use crate::calculator::Calculator;
use crate::error::{Error, Result};
use crate::number::Number;
use crate::parser::highlight::TokenKind;
use crate::units::Dim;
use crate::value::{Moment, Value};
use crate::zones::{Place, PlaceKind};

/// Kinds of data to look for in text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataKind {
    /// A plain number: "130", "30k", "1,500", ".5".
    Number,
    Percentage,
    /// An amount of money: "$999", "€5.50".
    Money,
    /// A currency named without an amount: "EUR", "yen".
    Currency,
    /// A day or a time: "yesterday", "5 May 2027", "3pm".
    Date,
    /// Two dates: "March 3 to March 10", "Q3 2026".
    DateInterval,
    /// A length of time: "2 hours", "1 hour 30 min".
    Duration,
    /// A time like a stopwatch's: "01:30:45".
    Laptime,
    Temperature,
    Length,
    Mass,
    Volume,
    Area,
    Speed,
    /// File sizes: "3 GB".
    DataSize,
    /// Any amount with a unit.
    Quantity,
    /// A city, country, region, airport or time zone.
    Place,
    City,
    Country,
    Airport,
    TimeZone,
    Url,
    Email,
    Hashtag,
    Boolean,
    /// A food: "flour", "olive oil".
    Food,
    /// Seconds since 1970: "1559740303".
    Timestamp,
    /// "2026-09-23T14:30:00Z".
    Iso8601,
}

/// Something found in a text.
#[derive(Clone, Debug)]
pub struct Found {
    pub kind: DataKind,
    /// Byte range in the text.
    pub range: Range<usize>,
    pub text: String,
    /// What it is worth, for values: a number, an amount, a date. A date interval is its
    /// start, with the end in `end`.
    pub value: Option<Value>,
    pub end: Option<Value>,
    /// The place, for places.
    pub place: Option<Place>,
}

impl Found {
    pub fn number(&self) -> Option<Number> {
        match &self.value {
            Some(Value::Quantity(q)) => Some(q.number),
            Some(Value::Percent(p)) => Some(*p),
            _ => None,
        }
    }

    pub fn moment(&self) -> Option<&Moment> {
        match &self.value {
            Some(Value::Moment(m)) => Some(m),
            _ => None,
        }
    }
}

impl Calculator {
    /// Data of these kinds in a text, in the order they are written. Each piece of text is
    /// found once, as the first kind it is of: list `Length` before `Quantity` to tell them apart.
    pub fn find(&self, text: &str, kinds: &[DataKind]) -> Vec<Found> {
        let mut found: Vec<Found> = Vec::new();
        let mut take = |item: Found| {
            if !found.iter().any(|f| f.range.start < item.range.end && item.range.start < f.range.end) {
                found.push(item);
            }
        };
        let wanted = |candidates: &[DataKind]| candidates.iter().copied().find(|k| kinds.contains(k));
        for (range, kind) in raw_data(text) {
            if let Some(kind) = wanted(&[kind]) {
                take(Found { kind, text: text[range.clone()].to_string(), range, value: None, end: None, place: None });
            }
        }
        let env = self.env(&self.names, &self.lines);
        for (range, expr) in self.values_in(text) {
            let Ok((value, end)) = (match &expr {
                Expr::Range(a, b) => env.range_ends(a, b).map(|(a, b)| (a, Some(b))),
                Expr::Period(..) => env.eval(&expr).and_then(|span| period_ends(&env, span)).map(|(a, b)| (a, Some(b))),
                _ => env.eval(&expr).map(|v| (v, None)),
            }) else {
                continue;
            };
            let Some(kind) = wanted(&value_kinds(&expr, &value, end.is_some())) else { continue };
            let value = match kind {
                DataKind::Timestamp => match env.convert(
                    value,
                    &crate::ast::Target::Format(crate::ast::Format::Date),
                    crate::eval::Display::Auto,
                ) {
                    Ok((value, _)) => value,
                    Err(_) => continue,
                },
                _ => value,
            };
            take(Found { kind, text: text[range.clone()].to_string(), range, value: Some(value), end, place: None });
        }
        for h in self.highlight(text) {
            let item = |kind| Found {
                kind,
                range: h.range.clone(),
                text: text[h.range.clone()].to_string(),
                value: None,
                end: None,
                place: None,
            };
            let kind = match h.kind {
                TokenKind::Place(place) => {
                    let specific = match place {
                        PlaceKind::City => DataKind::City,
                        PlaceKind::Country => DataKind::Country,
                        PlaceKind::Airport => DataKind::Airport,
                        PlaceKind::TimeZone => DataKind::TimeZone,
                        PlaceKind::Region => DataKind::Place,
                    };
                    wanted(&[specific, DataKind::Place])
                }
                TokenKind::Food => wanted(&[DataKind::Food]),
                TokenKind::Unit => {
                    let money = self.currency_named(&text[h.range.clone()]);
                    if money { wanted(&[DataKind::Currency]) } else { None }
                }
                TokenKind::Boolean => wanted(&[DataKind::Boolean]),
                _ => None,
            };
            if let Some(kind) = kind {
                let mut found = item(kind);
                if matches!(h.kind, TokenKind::Place(_)) {
                    found.place = self.place_named(&found.text);
                }
                take(found);
            }
        }
        found.sort_by_key(|f| f.range.start);
        found
    }

    /// The date a text names, the last one if there are several: "Tom's party this sunday".
    pub fn date_for(&self, text: &str) -> Option<Moment> {
        self.find(text, &[DataKind::Date]).into_iter().rev().find_map(|f| match f.value {
            Some(Value::Moment(m)) => Some(m),
            _ => None,
        })
    }

    /// The two dates of an interval a text names: "the conference from March 3 to March 10".
    pub fn date_interval_for(&self, text: &str) -> Option<(Moment, Moment)> {
        self.find(text, &[DataKind::DateInterval]).into_iter().rev().find_map(|f| match (f.value, f.end) {
            (Some(Value::Moment(a)), Some(Value::Moment(b))) => Some((a, b)),
            _ => None,
        })
    }

    /// Reads data out of a text one piece after another, as SoulverCore's `ObjectParser`.
    ///
    /// ```
    /// use rechenkern::{Calculator, DataKind};
    ///
    /// let calc = Calculator::new();
    /// let mut scanner = calc.scanner("We sold 130 copies yesterday");
    /// assert_eq!(scanner.number().unwrap().to_string(), "130");
    /// assert!(scanner.scan(DataKind::Date).is_ok());
    /// ```
    pub fn scanner<'a>(&'a self, text: &'a str) -> Scanner<'a> {
        Scanner { found: self.find(text, ALL), text, at: 0 }
    }

    /// The word is a currency's name or code: "EUR", "yen", "$".
    fn currency_named(&self, word: &str) -> bool {
        crate::units::registry().lookup(word).is_some_and(|u| u.is_money())
    }

    fn place_named(&self, name: &str) -> Option<Place> {
        let zone = crate::zones::find(&name.replace('-', " ").to_lowercase())?;
        Some(Place::written(name, zone))
    }
}

/// Reads the data of a text in order: each `scan` takes the next piece of a kind.
#[derive(Clone, Debug)]
pub struct Scanner<'a> {
    found: Vec<Found>,
    text: &'a str,
    /// Where the last piece taken ends.
    at: usize,
}

impl Scanner<'_> {
    /// The next piece of `kind` after the last one taken.
    pub fn scan(&mut self, kind: DataKind) -> Result<Found> {
        let found = self
            .found
            .iter()
            .find(|f| f.range.start >= self.at && f.kind == kind)
            .cloned()
            .ok_or_else(|| Error::new(format!("no {kind:?} after \"{}\"", self.text[..self.at].trim())))?;
        self.at = found.range.end;
        Ok(found)
    }

    /// A piece of `kind` anywhere in the text, without moving on.
    pub fn find(&self, kind: DataKind) -> Option<&Found> {
        self.found.iter().find(|f| f.kind == kind)
    }

    pub fn number(&mut self) -> Result<Number> {
        self.scan(DataKind::Number)?.number().ok_or_else(|| Error::new("not a number"))
    }

    pub fn date(&mut self) -> Result<Moment> {
        self.scan(DataKind::Date)?.moment().cloned().ok_or_else(|| Error::new("not a date"))
    }

    /// The text after the last piece taken.
    pub fn rest(&self) -> &str {
        self.text[self.at..].trim()
    }
}

/// Every kind, for the scanner: specific kinds of amounts before any amount.
const ALL: &[DataKind] = &[
    DataKind::Url,
    DataKind::Email,
    DataKind::Hashtag,
    DataKind::DateInterval,
    DataKind::Iso8601,
    DataKind::Date,
    DataKind::Laptime,
    DataKind::Duration,
    DataKind::Money,
    DataKind::Percentage,
    DataKind::Temperature,
    DataKind::Length,
    DataKind::Mass,
    DataKind::Volume,
    DataKind::Area,
    DataKind::Speed,
    DataKind::DataSize,
    DataKind::Quantity,
    DataKind::Number,
    DataKind::Boolean,
    DataKind::City,
    DataKind::Country,
    DataKind::Airport,
    DataKind::TimeZone,
    DataKind::Place,
    DataKind::Currency,
    DataKind::Food,
];

/// The kinds a value written as `expr` is of, most specific first.
fn value_kinds(expr: &Expr, value: &Value, interval: bool) -> Vec<DataKind> {
    if interval {
        return vec![DataKind::DateInterval];
    }
    match value {
        Value::Quantity(q) if q.unit.is_none() => {
            let timestamp = q.number.is_integer() && q.number >= Number::pow10(9) && q.number < Number::pow10(11);
            if timestamp { vec![DataKind::Timestamp, DataKind::Number] } else { vec![DataKind::Number] }
        }
        Value::Quantity(q) if q.unit.is_money() => vec![DataKind::Money, DataKind::Quantity],
        Value::Quantity(q) => {
            let specific = match q.unit.dim() {
                Dim::TEMPERATURE => Some(DataKind::Temperature),
                Dim::LENGTH => Some(DataKind::Length),
                Dim::MASS => Some(DataKind::Mass),
                Dim::VOLUME => Some(DataKind::Volume),
                Dim::AREA => Some(DataKind::Area),
                Dim::SPEED => Some(DataKind::Speed),
                Dim::DATA => Some(DataKind::DataSize),
                Dim::TIME => Some(DataKind::Duration),
                _ => None,
            };
            specific.into_iter().chain([DataKind::Quantity]).collect()
        }
        Value::Percent(_) => vec![DataKind::Percentage],
        Value::Moment(_) if matches!(expr, Expr::Time(TimeExpr::Iso(_))) => vec![DataKind::Iso8601, DataKind::Date],
        Value::Moment(_) => vec![DataKind::Date],
        Value::Duration(d) if d.laptime => vec![DataKind::Laptime, DataKind::Duration],
        Value::Duration(_) => vec![DataKind::Duration],
        Value::Bool(_) => vec![DataKind::Boolean],
        _ => Vec::new(),
    }
}

/// The first and last day of a period: "Q3 2026" is 1 July to 30 September.
fn period_ends(env: &crate::eval::Env, span: Value) -> Result<(Value, Value)> {
    let Value::Duration(d) = span else { return Err(Error::new("not a period")) };
    let first = d.anchor.ok_or_else(|| Error::new("not a period"))?;
    let last = first.checked_add(d.span)?.date().yesterday()?;
    let zone = env.now.get().time_zone().clone();
    let moment = |date: jiff::civil::Date| -> Result<Value> {
        Ok(Value::Moment(Moment {
            time: date.to_zoned(zone.clone())?,
            kind: crate::value::MomentKind::Date,
            zoned: false,
            seconds: false,
        }))
    };
    Ok((moment(first.date())?, moment(last)?))
}

/// Links, email addresses and hashtags: whole words of the text.
fn raw_data(text: &str) -> Vec<(Range<usize>, DataKind)> {
    let mut out = Vec::new();
    let mut at = 0;
    for word in text.split_inclusive(char::is_whitespace) {
        let start = at;
        at += word.len();
        let word = word.trim_end().trim_end_matches(['.', ',', ')', ';', '!', '?']);
        let lead = word.len() - word.trim_start_matches(['(', '"', '\'']).len();
        let word = &word[lead..];
        let range = start + lead..start + lead + word.len();
        let lower = word.to_ascii_lowercase();
        let kind = if ["http://", "https://", "ftp://", "www."].iter().any(|p| lower.starts_with(p)) && word.len() > 4 {
            DataKind::Url
        } else if word.split_once('@').is_some_and(|(user, domain)| {
            !user.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
        }) {
            DataKind::Email
        } else if word.starts_with('#') && word[1..].starts_with(char::is_alphabetic) {
            DataKind::Hashtag
        } else {
            continue;
        };
        out.push((range, kind));
    }
    out
}
