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
    /// Digits as written, leading zeros too: "007", "2026".
    Digits,
    /// "0b1011"
    BinaryNumber,
    /// "0o17"
    OctalNumber,
    /// "0xFF"
    HexNumber,
    /// "01:00:10:12"
    Timecode,
    /// A screen resolution: "1920x1080".
    Resolution,
    /// A musical note: "A4", "C#5".
    Pitch,
    Frequency,
    Power,
    Pressure,
    Angle,
    /// "30 mpg", "8 L/100km".
    FuelEconomy,
    /// "100 Mbps", "50 MB/s".
    TransferSpeed,
    /// A run of spaces, tabs or line breaks.
    Whitespace,
    /// A word the engine reads: "in", "of", "if".
    Keyword,
    /// Groups of digits with spaces or dashes between: "4111 1111 1111 1111", "555-123-4567".
    /// The value is the digits alone, as text.
    DigitSequence,
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
                let written = text[range.clone()].to_string();
                let value = (kind == DataKind::DigitSequence)
                    .then(|| Value::Text(written.chars().filter(char::is_ascii_digit).collect()));
                take(Found { kind, text: written, range, value, end: None, place: None });
            }
        }
        let env = self.env(&self.names, &self.lines);
        for (range, expr) in self.values_in(text) {
            let Ok((value, end)) = (match &expr {
                Expr::Range(a, b) | Expr::SignedRange(a, b) => env.range_ends(a, b).map(|(a, b)| (a, Some(b))),
                Expr::Period(..) => env.eval(&expr).and_then(|span| period_ends(&env, span)).map(|(a, b)| (a, Some(b))),
                _ => env.eval(&expr).map(|v| (v, None)),
            }) else {
                continue;
            };
            let mut kinds = value_kinds(&expr, &value, end.is_some());
            // "0xFF", "007": numbers as written.
            let written = &text[range.clone()];
            if kinds.first() == Some(&DataKind::Number) || kinds.first() == Some(&DataKind::Timestamp) {
                let radix = match written.get(..2).map(str::to_ascii_lowercase).as_deref() {
                    Some("0x") => Some(DataKind::HexNumber),
                    Some("0b") => Some(DataKind::BinaryNumber),
                    Some("0o") => Some(DataKind::OctalNumber),
                    _ => None,
                };
                let digits = written.bytes().all(|b| b.is_ascii_digit()).then_some(DataKind::Digits);
                kinds.splice(0..0, radix.into_iter().chain(digits));
            }
            let Some(kind) = wanted(&kinds) else { continue };
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
                TokenKind::Keyword | TokenKind::Converter | TokenKind::Connector => wanted(&[DataKind::Keyword]),
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
            (Some(Value::Moment(a)), Some(Value::Moment(b))) => Some((self.at_range_time(a), self.at_range_time(b))),
            _ => None,
        })
    }

    /// A date of an interval at the config's time for them.
    fn at_range_time(&self, m: Moment) -> Moment {
        let Some(time) = self.config().features.dates.range_time else { return m };
        if m.kind != crate::value::MomentKind::Date {
            return m;
        }
        match m.time.date().to_datetime(time).to_zoned(m.time.time_zone().clone()) {
            Ok(zoned) => Moment { time: zoned, kind: crate::value::MomentKind::DateTime, ..m },
            Err(_) => m,
        }
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

    /// A host type read out of a text: `let (n, day): (Number, Moment) = calc.read_as("130 copies yesterday")?`.
    pub fn read_as<T: FromScanner>(&self, text: &str) -> Result<T> {
        T::from_scanner(&mut self.scanner(text))
    }

    /// The two texts hold the same kinds of data in the same order, as SoulverCore's
    /// `matchesStructure(of:)`: "QF1 at 3pm" and "BA2 at 9am".
    pub fn same_structure(&self, a: &str, b: &str) -> bool {
        let kinds = |text: &str| self.find(text, ALL).into_iter().map(|f| f.kind).collect::<Vec<_>>();
        kinds(a) == kinds(b)
    }

    /// The text with the first piece of these kinds replaced by what `replace` gives for it
    /// (`None` keeps it), as SoulverCore's `replacingFirst`.
    pub fn replace_first(
        &self,
        text: &str,
        kinds: &[DataKind],
        replace: impl FnMut(&Found) -> Option<String>,
    ) -> String {
        self.replace_found(text, kinds, replace, true)
    }

    /// The text with every piece of these kinds replaced, as SoulverCore's `replacingAll`:
    /// every price converted to euros.
    pub fn replace_all(&self, text: &str, kinds: &[DataKind], replace: impl FnMut(&Found) -> Option<String>) -> String {
        self.replace_found(text, kinds, replace, false)
    }

    fn replace_found(
        &self,
        text: &str,
        kinds: &[DataKind],
        mut replace: impl FnMut(&Found) -> Option<String>,
        first: bool,
    ) -> String {
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for found in self.find(text, kinds) {
            let Some(new) = replace(&found) else { continue };
            out.push_str(&text[at..found.range.start]);
            out.push_str(&new);
            at = found.range.end;
            if first {
                break;
            }
        }
        out.push_str(&text[at..]);
        out
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

/// A type read out of text piece by piece, as SoulverCore's `FromScanner`: implement it for a
/// host type ("a flight and its day") and read it with `Calculator::read_as`.
pub trait FromScanner: Sized {
    fn from_scanner(scanner: &mut Scanner) -> Result<Self>;
}

impl FromScanner for Number {
    fn from_scanner(scanner: &mut Scanner) -> Result<Number> {
        scanner.number()
    }
}

impl FromScanner for Moment {
    fn from_scanner(scanner: &mut Scanner) -> Result<Moment> {
        scanner.date()
    }
}

impl<A: FromScanner, B: FromScanner> FromScanner for (A, B) {
    fn from_scanner(scanner: &mut Scanner) -> Result<(A, B)> {
        Ok((A::from_scanner(scanner)?, B::from_scanner(scanner)?))
    }
}

impl<A: FromScanner, B: FromScanner, C: FromScanner> FromScanner for (A, B, C) {
    fn from_scanner(scanner: &mut Scanner) -> Result<(A, B, C)> {
        Ok((A::from_scanner(scanner)?, B::from_scanner(scanner)?, C::from_scanner(scanner)?))
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

    /// The first piece of `kind` not taken yet, wherever it is: "tomorrow QF 123" gives the
    /// flight and the day in any order, as SoulverCore's `ObjectParser` scans unordered.
    pub fn take(&mut self, kind: DataKind) -> Result<Found> {
        let index = self
            .found
            .iter()
            .position(|f| f.kind == kind)
            .ok_or_else(|| Error::new(format!("no {kind:?} in \"{}\"", self.text.trim())))?;
        let found = self.found.remove(index);
        self.at = self.at.max(found.range.end);
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

    /// The next of these words after the last piece taken, in any case, as SoulverCore's
    /// `scanKeyword`: the host's own words, like "flight" or "gate".
    pub fn keyword(&mut self, words: &[&str]) -> Result<String> {
        let rest = &self.text[self.at..];
        let mut at = 0;
        for word in rest.split_inclusive(|c: char| !c.is_alphanumeric()) {
            let start = at;
            at += word.len();
            let bare = word.trim_end_matches(|c: char| !c.is_alphanumeric());
            if let Some(found) = words.iter().find(|w| w.eq_ignore_ascii_case(bare)) {
                self.at += start + bare.len();
                return Ok(found.to_string());
            }
        }
        Err(Error::new(format!("none of {} in \"{}\"", words.join(", "), rest.trim())))
    }

    /// Every one of these words is in the text, as SoulverCore's `ensurePresent(words:)`.
    pub fn ensure(&self, words: &[&str]) -> Result<()> {
        let lower = self.text.to_lowercase();
        let has = |w: &&str| lower.split(|c: char| !c.is_alphanumeric()).any(|word| word == w.to_lowercase());
        match words.iter().find(|w| !has(w)) {
            Some(missing) => Err(Error::new(format!("no \"{missing}\" in \"{}\"", self.text.trim()))),
            None => Ok(()),
        }
    }

    /// The next value of a type that reads itself from a scanner.
    pub fn read<T: FromScanner>(&mut self) -> Result<T> {
        T::from_scanner(self)
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
    DataKind::Timecode,
    DataKind::Laptime,
    DataKind::Duration,
    DataKind::Resolution,
    DataKind::Pitch,
    DataKind::Money,
    DataKind::Percentage,
    DataKind::Temperature,
    DataKind::Length,
    DataKind::Mass,
    DataKind::Volume,
    DataKind::Area,
    DataKind::Speed,
    DataKind::DataSize,
    DataKind::Frequency,
    DataKind::Power,
    DataKind::Pressure,
    DataKind::Angle,
    DataKind::FuelEconomy,
    DataKind::TransferSpeed,
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
            let fuel = q.unit.single().is_some_and(|d| d.symbol == "L/100km")
                || (q.unit.factors().len() == 2
                    && q.unit.factors().iter().any(|&(id, _)| crate::units::Unit::of(id).dim() == Dim::VOLUME)
                    && q.unit.factors().iter().any(|&(id, _)| crate::units::Unit::of(id).dim() == Dim::LENGTH));
            let specific = match q.unit.dim() {
                _ if fuel => Some(DataKind::FuelEconomy),
                Dim::FREQUENCY => Some(DataKind::Frequency),
                Dim::POWER => Some(DataKind::Power),
                Dim::PRESSURE => Some(DataKind::Pressure),
                Dim::ANGLE => Some(DataKind::Angle),
                Dim::DATA_RATE => Some(DataKind::TransferSpeed),
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
        Value::Timecode(_) => vec![DataKind::Timecode],
        Value::Resolution(_) => vec![DataKind::Resolution],
        Value::Pitch(_) => vec![DataKind::Pitch],
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
    // Runs of whitespace.
    let mut run: Option<usize> = None;
    for (i, c) in text.char_indices().chain([(text.len(), 'x')]) {
        match (c.is_whitespace(), run) {
            (true, None) => run = Some(i),
            (false, Some(start)) => {
                out.push((start..i, DataKind::Whitespace));
                run = None;
            }
            _ => {}
        }
    }
    out.extend(digit_sequences(text).into_iter().map(|r| (r, DataKind::DigitSequence)));
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

/// Groups of digits with one space or dash between, seven digits or more:
/// card numbers, phone numbers, account numbers.
fn digit_sequences(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let starts = bytes[i].is_ascii_digit() && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric());
        if !starts {
            i += 1;
            continue;
        }
        let (start, mut end, mut groups, mut digits) = (i, i, 0, 0);
        let mut k = i;
        loop {
            let run = bytes[k..].iter().take_while(|b| b.is_ascii_digit()).count();
            if run == 0 {
                break;
            }
            (groups, digits, end, k) = (groups + 1, digits + run, k + run, k + run);
            let separated = k + 1 < bytes.len() && matches!(bytes[k], b' ' | b'-') && bytes[k + 1].is_ascii_digit();
            if !separated {
                break;
            }
            k += 1;
        }
        let whole = end == bytes.len() || !bytes[end].is_ascii_alphanumeric() && bytes[end] != b'.';
        if groups >= 2 && digits >= 7 && whole {
            out.push(start..end);
        }
        i = end.max(i + 1);
    }
    out
}
