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

    /// Where a span between two dates starts: `Jan 1 to Mar 1`, `Q3 2026`.
    pub fn start(&self) -> Option<civil::DateTime> {
        self.anchor
    }

    /// Where it ends.
    pub fn end(&self) -> Option<civil::DateTime> {
        self.anchor?.checked_add(self.span).ok()
    }
}

/// A musical note in equal temperament, A4 = 440 Hz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pitch {
    /// The MIDI note number: 69 is A4.
    pub midi: i32,
}

impl Pitch {
    /// The nearest note to a frequency.
    pub fn of_hz(hz: f64) -> Pitch {
        Pitch { midi: (69.0 + 12.0 * (hz / 440.0).log2()).round() as i32 }
    }

    pub fn hz(self) -> f64 {
        440.0 * 2f64.powf((self.midi - 69) as f64 / 12.0)
    }

    /// "A4", "C#5".
    pub fn name(self) -> String {
        const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
        format!("{}{}", NAMES[self.midi.rem_euclid(12) as usize], self.midi.div_euclid(12) - 1)
    }
}

/// A place on Earth, in degrees north and east.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coordinates {
    pub latitude: f64,
    pub longitude: f64,
}

/// A size in pixels: 1920x1080.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

/// Frames at a frame rate, written as a timecode: 00:00:04:04.
#[derive(Clone, Debug, PartialEq)]
pub struct Timecode {
    pub frames: i64,
    /// Frames a second.
    pub fps: Number,
}

/// Something that happens again and again: every 2 weeks, until a day if it ends.
#[derive(Clone, Debug)]
pub struct Cadence {
    /// The time between two times.
    pub every: Span,
    /// The first time, and the days it happens on up to `until`.
    pub start: civil::Date,
    pub until: Option<civil::Date>,
    /// The first time was named (`from May 1`, `every monday`), so the answer shows it.
    pub named_start: bool,
}

impl Cadence {
    /// The days it happens on, up to `until` (at most a thousand); none without an end.
    pub fn dates(&self) -> Vec<civil::Date> {
        let Some(until) = self.until else { return Vec::new() };
        let mut dates = Vec::new();
        for k in 0..1000i64 {
            let Ok(step) = self.every.checked_mul(k) else { break };
            match self.start.checked_add(step) {
                Ok(day) if day <= until => dates.push(day),
                _ => break,
            }
        }
        dates
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
    /// From one value to another: `5 – 10 kg`.
    Range(Box<Value>, Box<Value>),
    /// Several answers at once: `$10 in EUR, JPY`.
    List(Vec<Value>),
    Pitch(Pitch),
    Coordinates(Coordinates),
    Resolution(Resolution),
    Timecode(Timecode),
    Cadence(Cadence),
}

impl Value {
    pub fn number(n: Number) -> Value {
        Value::Quantity(Quantity::plain(n))
    }

    /// The number of a plain number, as SoulverCore's `decimalValue`.
    pub fn as_number(&self) -> Option<Number> {
        match self {
            Value::Quantity(q) if q.unit.is_none() => Some(q.number),
            _ => None,
        }
    }

    /// An amount with or without a unit: `5 km`, `$20`, `7`.
    pub fn as_quantity(&self) -> Option<&Quantity> {
        match self {
            Value::Quantity(q) => Some(q),
            _ => None,
        }
    }

    /// A percentage, as written: 20 for 20%.
    pub fn as_percent(&self) -> Option<Number> {
        match self {
            Value::Percent(p) => Some(*p),
            _ => None,
        }
    }

    pub fn as_moment(&self) -> Option<&Moment> {
        match self {
            Value::Moment(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_duration(&self) -> Option<&Duration> {
        match self {
            Value::Duration(d) => Some(d),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }

    /// The same value, written the same way: for sheets, to tell which answers changed.
    pub fn same_as(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Quantity(a), Value::Quantity(b)) => a == b,
            (Value::Percent(a), Value::Percent(b)) => a == b,
            (Value::Moment(a), Value::Moment(b)) => {
                a.time == b.time && a.kind == b.kind && a.zoned == b.zoned && a.seconds == b.seconds
            }
            (Value::Duration(a), Value::Duration(b)) => {
                a.span.fieldwise() == b.span.fieldwise() && a.anchor == b.anchor && a.laptime == b.laptime
            }
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Text(a), Value::Text(b)) => a == b,
            (Value::Range(a1, a2), Value::Range(b1, b2)) => a1.same_as(b1) && a2.same_as(b2),
            (Value::List(a), Value::List(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.same_as(b)),
            (Value::Pitch(a), Value::Pitch(b)) => a == b,
            (Value::Coordinates(a), Value::Coordinates(b)) => a == b,
            (Value::Resolution(a), Value::Resolution(b)) => a == b,
            (Value::Timecode(a), Value::Timecode(b)) => a == b,
            (Value::Cadence(a), Value::Cadence(b)) => {
                a.every.fieldwise() == b.every.fieldwise() && a.start == b.start && a.until == b.until
            }
            _ => false,
        }
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
            Value::Range(..) => "a range".into(),
            Value::List(_) => "a list".into(),
            Value::Pitch(_) => "a note".into(),
            Value::Coordinates(_) => "coordinates".into(),
            Value::Resolution(_) => "a resolution".into(),
            Value::Timecode(_) => "a timecode".into(),
            Value::Cadence(_) => "a cadence".into(),
        }
    }
}
