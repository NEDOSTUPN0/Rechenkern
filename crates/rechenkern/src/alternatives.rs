//! The same answer written other ways, as SoulverCore's `AlternativeResultGenerator`:
//! a number in hex and as a fraction, a date as ISO 8601, 5 km in miles.

use crate::answer::Answer;
use crate::ast::{Format, Target};
use crate::custom;
use crate::eval::{Display, Env};
use crate::format;
use crate::number::Number;
use crate::units::{Dim, Unit};
use crate::value::{Duration, MomentKind, Value};

/// How an alternative writes the answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlternativeKind {
    Decimal,
    Binary,
    Hexadecimal,
    Octal,
    Fraction,
    Scientific,
    /// "3 hours 30 minutes".
    Timespan,
    /// "03:30:00".
    Laptime,
    /// The date and time written out: "Wednesday, 23 September 2026 14:30".
    Date,
    Iso8601,
    /// Seconds since 1970.
    Timestamp,
    /// The amount in another unit: 5 km in miles.
    Unit,
}

impl AlternativeKind {
    /// The name in snake case: "hexadecimal".
    pub fn name(self) -> &'static str {
        match self {
            AlternativeKind::Decimal => "decimal",
            AlternativeKind::Binary => "binary",
            AlternativeKind::Hexadecimal => "hexadecimal",
            AlternativeKind::Octal => "octal",
            AlternativeKind::Fraction => "fraction",
            AlternativeKind::Scientific => "scientific",
            AlternativeKind::Timespan => "timespan",
            AlternativeKind::Laptime => "laptime",
            AlternativeKind::Date => "date",
            AlternativeKind::Iso8601 => "iso8601",
            AlternativeKind::Timestamp => "timestamp",
            AlternativeKind::Unit => "unit",
        }
    }
}

/// The answer written another way.
#[derive(Clone, Debug)]
pub struct Alternative {
    pub kind: AlternativeKind,
    pub value: Value,
    pub text: String,
}

/// Units an amount of each kind is offered in, most used first.
const UNITS: &[(Dim, &[&str])] = &[
    (Dim::LENGTH, &["m", "km", "cm", "mm", "mi", "yd", "ft", "in"]),
    (Dim::MASS, &["kg", "g", "lb", "oz", "st"]),
    (Dim::VOLUME, &["L", "mL", "gal", "qt", "cup", "fl oz"]),
    (Dim::TEMPERATURE, &["°C", "°F", "K"]),
    (Dim::AREA, &["m^2", "km^2", "ha", "ac", "ft^2"]),
    (Dim::SPEED, &["km/h", "mph", "m/s", "kn"]),
    (Dim::DATA, &["B", "kB", "MB", "GB", "TB", "KiB", "MiB", "GiB"]),
    (Dim::ENERGY, &["J", "kJ", "kWh", "kcal"]),
    (Dim::POWER, &["W", "kW", "hp"]),
    (Dim::PRESSURE, &["Pa", "kPa", "bar", "psi", "atm"]),
    (Dim::TIME, &["s", "min", "h", "d", "week"]),
];

impl Env<'_> {
    pub(crate) fn alternatives(&self, answer: &Answer) -> Vec<Alternative> {
        let value = answer.value();
        let mut out = Vec::new();
        let mut add = |kind, result: crate::Result<(Value, Display)>| {
            if let Ok((value, display)) = result {
                let text = format::render(&value, &display, self.config, &self.now);
                if text != answer.text() && !out.iter().any(|a: &Alternative| a.text == text) {
                    out.push(Alternative { kind, value, text });
                }
            }
        };
        let format = |f| self.convert(value.clone(), &Target::Format(f), Display::Auto);
        match value {
            Value::Quantity(q) if q.unit.is_none() => {
                if q.number.is_integer() && q.number.abs() < Number::pow10(18) {
                    add(AlternativeKind::Hexadecimal, format(Format::Hex));
                    add(AlternativeKind::Binary, format(Format::Binary));
                    add(AlternativeKind::Octal, format(Format::Octal));
                } else if q.number.round_dp(6) == q.number {
                    add(AlternativeKind::Fraction, format(Format::Fraction));
                }
                if !q.number.is_zero() && !(-3..6).contains(&q.number.magnitude()) {
                    add(AlternativeKind::Scientific, format(Format::Scientific));
                }
            }
            Value::Percent(_) => {
                add(AlternativeKind::Decimal, format(Format::Decimal));
                add(AlternativeKind::Fraction, format(Format::Fraction));
            }
            Value::Quantity(q) => {
                if q.unit.dim() == Dim::TIME {
                    add(AlternativeKind::Timespan, format(Format::Timespan));
                    add(AlternativeKind::Laptime, format(Format::Laptime));
                }
                for unit in self.units_like(&q.unit) {
                    add(AlternativeKind::Unit, self.convert(value.clone(), &Target::Unit(unit), Display::Auto));
                }
            }
            Value::Duration(Duration { .. }) => {
                add(AlternativeKind::Timespan, format(Format::Timespan));
                add(AlternativeKind::Laptime, format(Format::Laptime));
                for unit in self.units_like(&custom::parse_unit("s").unwrap_or_default()) {
                    add(AlternativeKind::Unit, self.convert(value.clone(), &Target::Unit(unit), Display::Auto));
                }
            }
            Value::Moment(m) => {
                let date = m.time.strftime("%A, %-d %B %Y").to_string();
                let long = match m.kind {
                    MomentKind::Date => date,
                    _ => format!("{date} {}", format::moment_text(m, MomentKind::Clock, self.config)),
                };
                add(AlternativeKind::Date, Ok((Value::Text(long), Display::Auto)));
                add(AlternativeKind::Iso8601, format(Format::Iso));
                add(AlternativeKind::Timestamp, format(Format::Timestamp));
            }
            Value::Bool(_) | Value::Text(_) => {}
        }
        out
    }

    /// Other units of the same kind as `unit`.
    fn units_like(&self, unit: &Unit) -> Vec<Unit> {
        let dim = unit.dim();
        let Some((_, names)) = UNITS.iter().find(|(d, _)| *d == dim) else { return Vec::new() };
        names.iter().filter_map(|name| custom::parse_unit(name)).filter(|u| !u.same_as(unit)).collect()
    }
}
