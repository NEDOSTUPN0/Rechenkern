//! Values produced by calculations.

use jiff::{Span, Zoned, civil};

use crate::number::Number;
use crate::units::Unit;

/// A number with an optional unit, e.g. `5 km` or `$20/hour`.
#[derive(Clone, Debug, PartialEq)]
pub struct Quantity {
    pub number: Number,
    pub unit: Unit,
}

impl Quantity {
    pub fn new(number: Number, unit: Unit) -> Quantity {
        Quantity { number, unit }
    }

    pub fn plain(number: Number) -> Quantity {
        Quantity { number, unit: Unit::none() }
    }
}

/// What part of a [`Moment`] matters to the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MomentKind {
    Date,
    Clock,
    DateTime,
}

/// A point in time, such as `tomorrow` or `3pm in Tokyo`.
#[derive(Clone, Debug)]
pub struct Moment {
    pub time: Zoned,
    pub kind: MomentKind,
    /// The time zone was chosen explicitly, so answers show it.
    pub zoned: bool,
    /// Seconds matter: written by the user or from a timestamp, not `now`.
    pub seconds: bool,
}

/// A length of time in calendar units, such as `2 months 3 weeks`.
#[derive(Clone, Debug)]
pub struct Duration {
    pub span: Span,
    /// Where the span starts; makes months and years exact.
    pub anchor: Option<civil::DateTime>,
    /// Shown as `hh:mm:ss`.
    pub laptime: bool,
}

impl Duration {
    pub fn new(span: Span) -> Duration {
        Duration { span, anchor: None, laptime: false }
    }
}

/// The result of a calculation.
#[derive(Clone, Debug)]
pub enum Value {
    Quantity(Quantity),
    /// A percentage: `Percent(20)` is 20%.
    Percent(Number),
    Moment(Moment),
    Duration(Duration),
    Bool(bool),
    Text(String),
}

impl Value {
    pub fn number(n: Number) -> Value {
        Value::Quantity(Quantity::plain(n))
    }

    /// Short description of the value's kind for error messages.
    pub fn kind(&self) -> String {
        match self {
            Value::Quantity(q) if q.unit.is_none() => "a number".into(),
            Value::Quantity(q) => q.unit.dim().name(),
            Value::Percent(_) => "a percentage".into(),
            Value::Moment(_) => "a date".into(),
            Value::Duration(_) => "a duration".into(),
            Value::Bool(_) => "a boolean".into(),
            Value::Text(_) => "text".into(),
        }
    }
}
