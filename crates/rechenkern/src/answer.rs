//! The answer to a line, and what the line asked: a conversion has a source and
//! a target side, as an answer card in a launcher shows them.
//!
//! Lines keep only the values the sides are made of; their texts and captions
//! are written the first time they are asked for.

use std::cell::OnceCell;
use std::fmt;
use std::sync::Arc;

use jiff::Zoned;

use crate::ast::{Expr, Format, Op, Target, TimeExpr};
use crate::config::{Config, ConversionRule, Now, ResultConversions};
use crate::custom;
use crate::error::Result;
use crate::eval::{Display, Env, time, weight_or_volume};
use crate::format;
use crate::number::Number;
use crate::units::{Dim, Unit, registry};
use crate::value::{Moment, MomentKind, Value};
use crate::zones::Place;

/// What a line asks, as SoulverCore's `TokenListForm` tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// A value in another unit or format: `500 km to mi`, `255 in hex`.
    Conversion,
    /// The time now somewhere else: `time in Paris`.
    TimeAtPlace,
    /// A time moved to another zone: `3pm Berlin in Tokyo`.
    TimeConversion,
    /// Between the clocks of two places: `time diff Tokyo London`.
    TimeDifference,
    /// A date moved by a length of time: `today + 30 days`.
    DateShift,
    /// The time between two dates: `days until christmas`.
    DateInterval,
    /// One of the host's phrases: `tip on $50`.
    CustomFunction,
    /// Anything else with an answer: `2+2*3`, `20% of 150`.
    Calculation,
    /// A lone value read back: `5 km`, `today`.
    Value,
}

impl Form {
    /// The name in snake case: "time_at_place".
    pub fn name(self) -> &'static str {
        match self {
            Form::Conversion => "conversion",
            Form::TimeAtPlace => "time_at_place",
            Form::TimeConversion => "time_conversion",
            Form::TimeDifference => "time_difference",
            Form::DateShift => "date_shift",
            Form::DateInterval => "date_interval",
            Form::CustomFunction => "custom_function",
            Form::Calculation => "calculation",
            Form::Value => "value",
        }
    }
}

/// One side of a line with two: what is converted, or what it becomes.
#[derive(Clone, Debug)]
pub struct Side {
    pub value: Value,
    /// Formatted like answers, without a time zone: "500 km", "23:53".
    pub text: String,
    /// The long name of the unit or place: "Kilometers", "Paris".
    pub label: Option<String>,
}

/// The values a line's sides are made of, kept while it is calculated.
#[derive(Clone, Debug, Default)]
pub(crate) struct Parts {
    /// What is converted or moved, or where a range starts.
    source: Option<(Value, Display)>,
    /// Where a range ends.
    end: Option<Value>,
    target: Option<Target>,
    /// The places of a time conversion or difference.
    places: Option<(Option<Place>, Place)>,
}

/// The result of a line.
#[derive(Clone, Debug)]
pub struct Answer {
    value: Value,
    display: Display,
    text: String,
    form: Form,
    parts: Parts,
    config: Arc<Config>,
    now: Option<Zoned>,
    sides: OnceCell<Option<(Side, Side, Option<String>)>>,
}

impl Answer {
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// The formatted answer, e.g. "€9.20".
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn form(&self) -> Form {
        self.form
    }

    /// What later lines get for this answer: the value, or the value as shown if the
    /// config says references take the formatted value.
    pub(crate) fn reference(&self) -> Value {
        match self.config.formatting.references_take_formatted_value {
            true => format::shown(&self.value, &self.display, &self.config),
            false => self.value.clone(),
        }
    }

    /// What is converted, for lines with two sides.
    pub fn source(&self) -> Option<&Side> {
        self.sides().map(|(source, _, _)| source)
    }

    /// What it becomes; its value is the answer.
    pub fn target(&self) -> Option<&Side> {
        self.sides().map(|(_, target, _)| target)
    }

    /// A remark kept out of the answer: "+1 day" when a time lands on another date.
    pub fn note(&self) -> Option<&str> {
        self.sides()?.2.as_deref()
    }

    /// The answer with every digit, no thousands separators and currency codes, as
    /// SoulverCore's unformatted results.
    pub fn unformatted_text(&self) -> String {
        let config = (*self.config).clone().unformatted();
        let now = match &self.now {
            Some(time) => Now::at(&config, time.clone()),
            None => Now::new(&config),
        };
        format::render(&self.value, &Display::Auto, &config, &now)
    }

    /// The answer for the clipboard, without a time zone or a remark: "21:53" for "21:53 CEST".
    pub fn copy_text(&self) -> String {
        match (&self.value, &self.display) {
            (Value::Moment(m), _) => self.moment_text(m, self.note().is_some()),
            (_, Display::Note(_)) => self.render(&self.value, &Display::Auto),
            _ => self.text.clone(),
        }
    }

    fn sides(&self) -> Option<&(Side, Side, Option<String>)> {
        self.sides.get_or_init(|| self.make_sides()).as_ref()
    }

    fn make_sides(&self) -> Option<(Side, Side, Option<String>)> {
        let parts = &self.parts;
        match self.form {
            Form::Conversion => {
                let (from, display) = parts.source.clone()?;
                let label = match parts.target.as_ref()? {
                    Target::Format(format) => format_caption(*format),
                    Target::Units(units) => {
                        let names = units.iter().map(unit_caption).collect::<Option<Vec<_>>>()?;
                        Some(names.join(if units.iter().any(Unit::is_money) { ", " } else { " and " }))
                    }
                    _ => caption(&self.value),
                };
                let target = Side { label, ..self.side(self.value.clone(), &self.display) };
                Some((self.side(from, &display), target, None))
            }
            Form::TimeAtPlace | Form::TimeConversion => {
                let (Some((Value::Moment(from), _)), Value::Moment(to)) = (&parts.source, &self.value) else {
                    return None;
                };
                let (from_place, to_place) = parts.places.clone()?;
                let from_place = from_place.unwrap_or_else(|| Place::of_zone(&self.config.local_zone()));
                Some(self.time_sides(from.clone(), &from_place, to.clone(), &to_place))
            }
            Form::TimeDifference => {
                let (Some(a), b) = parts.places.clone()? else { return None };
                let now = self.now.clone().unwrap_or_else(|| self.config.now());
                let at = |place: &Place| Moment {
                    time: now.with_time_zone(place.zone.clone()),
                    kind: MomentKind::Clock,
                    zoned: true,
                    seconds: false,
                };
                Some(self.time_sides(at(&a), &a, at(&b), &b))
            }
            Form::DateShift => {
                let (from, display) = parts.source.clone()?;
                Some((self.side(from, &display), self.side(self.value.clone(), &self.display), None))
            }
            Form::DateInterval => {
                let (start, _) = parts.source.clone()?;
                let end = parts.end.clone()?;
                Some((self.side(start, &Display::Auto), self.side(end, &Display::Auto), None))
            }
            Form::Calculation | Form::CustomFunction | Form::Value => None,
        }
    }

    /// Two clocks and how many days apart their dates are.
    fn time_sides(
        &self,
        from: Moment,
        from_place: &Place,
        to: Moment,
        to_place: &Place,
    ) -> (Side, Side, Option<String>) {
        let days = days_between(&from.time, &to.time);
        let note = (days != 0).then(|| format!("{days:+} day{}", if days.abs() == 1 { "" } else { "s" }));
        let side = |m: Moment, place: &Place| Side {
            text: self.moment_text(&m, note.is_some()),
            label: Some(place.name.clone()),
            value: Value::Moment(m),
        };
        (side(from, from_place), side(to, to_place), note)
    }

    fn side(&self, value: Value, display: &Display) -> Side {
        let text = match &value {
            Value::Moment(m) => self.moment_text(m, false),
            _ => self.render(&value, display),
        };
        Side { label: caption(&value), value, text }
    }

    fn render(&self, value: &Value, display: &Display) -> String {
        let now = match &self.now {
            Some(time) => Now::at(&self.config, time.clone()),
            None => Now::new(&self.config),
        };
        format::render(value, display, &self.config, &now)
    }

    /// A date or time without its zone; `clock_only` leaves out the date of a clock time.
    fn moment_text(&self, m: &Moment, clock_only: bool) -> String {
        let today = || self.now.as_ref().map_or_else(|| self.config.now().date(), Zoned::date);
        let kind = match m.kind {
            MomentKind::Clock if !clock_only && m.time.date() != today() => MomentKind::DateTime,
            kind => kind,
        };
        format::moment_text(m, kind, &self.config)
    }
}

impl fmt::Display for Answer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// Metric units in the order of size, for `ResultConversions::Automatic`.
const METRIC_FAMILIES: &[&[&str]] = &[
    &["km", "m", "cm", "mm", "µm", "nm"],
    &["t", "kg", "g", "mg", "µg"],
    &["L", "mL"],
    &["TB", "GB", "MB", "kB", "B"],
    &["Tbit", "Gbit", "Mbit", "kbit", "bit"],
    &["GJ", "MJ", "kJ", "J"],
    &["GW", "MW", "kW", "W"],
    &["GHz", "MHz", "kHz", "Hz"],
];

impl Env<'_> {
    /// The answer converted as the config asks, when the line asks for no conversion itself.
    pub(crate) fn convert_result(&self, expr: &Expr, value: Value, display: Display) -> Result<(Value, Display)> {
        if matches!(expr, Expr::Convert(..)) || !matches!(display, Display::Auto) {
            return Ok((value, display));
        }
        let converted = match (&self.config.formatting.conversions, &value) {
            (ResultConversions::None, _) => None,
            (ResultConversions::Automatic, Value::Quantity(q)) => {
                let symbol = q.unit.single().map(|d| d.symbol.as_str());
                let family = METRIC_FAMILIES.iter().find(|f| symbol.is_some_and(|s| f.contains(&s)));
                match family {
                    Some(family) => {
                        let units = family.iter().map(|s| registry().get(s)).collect::<Vec<_>>();
                        let fits = |u: &Unit| self.convert_quantity(q, u).is_ok_and(|c| c.number.abs() >= Number::ONE);
                        let unit = units.iter().find(|u| fits(u)).or(units.last());
                        unit.map(|u| Target::Unit(u.clone()))
                    }
                    None => None,
                }
            }
            (ResultConversions::Automatic, _) => None,
            (ResultConversions::Rules(rules), _) => rules.iter().find_map(|rule| match (rule, &value) {
                (ConversionRule::Unit { from, to }, Value::Quantity(q)) => {
                    let from = custom::parse_unit(from)?;
                    q.unit.same_as(&from).then(|| Some(Target::Unit(custom::parse_unit(to)?)))?
                }
                (ConversionRule::Kind { like, to }, Value::Quantity(q)) => {
                    let like = custom::parse_unit(like)?;
                    (!q.unit.is_none() && q.unit.dim() == like.dim())
                        .then(|| Some(Target::Unit(custom::parse_unit(to)?)))?
                }
                (ConversionRule::TimeZone(zone), Value::Moment(m)) if m.kind != MomentKind::Date => {
                    Some(Target::Zone(Place::of_zone(zone)))
                }
                _ => None,
            }),
        };
        match converted {
            Some(target) => self.convert(value, &target, display),
            None => Ok((value, display)),
        }
    }

    /// Evaluates a whole line, keeping the values its sides are made of.
    pub(crate) fn answer_line(&self, expr: &Expr) -> Result<(Value, Display, Parts)> {
        if let Expr::RatesOn(inner, date) = expr {
            return self.on_rates_of(date, || self.answer_line(inner));
        }
        let mut parts = Parts::default();
        let (value, display) = match expr {
            Expr::Convert(inner, target) if !matches!(**inner, Expr::Range(..)) => {
                let saved = self.food_as.replace(weight_or_volume(target));
                let source = self.answer(inner);
                self.food_as.set(saved);
                let (from, display) = source?;
                parts.source = Some((from.clone(), display.clone()));
                parts.target = Some(target.clone());
                if let Target::Zone(place) = target {
                    parts.places = Some((placed(inner).cloned(), place.clone()));
                }
                self.convert(from, target, display)?
            }
            _ if let Some((a, b)) = interval(expr) => {
                let (a, b) = self.range_ends(a, b)?;
                parts.source = Some((a.clone(), Display::Auto));
                parts.end = Some(b.clone());
                let span = self.range(a, b)?;
                match expr {
                    Expr::Convert(_, target) => self.convert(span, target, Display::Auto)?,
                    _ => (span, Display::Auto),
                }
            }
            Expr::Binary(op @ (Op::Add | Op::Sub), a, b) => {
                let (a, b) = (self.eval(a)?, self.eval(b)?);
                if let Some(moment) = [&a, &b].into_iter().find(|v| matches!(v, Value::Moment(_))) {
                    parts.source = Some((moment.clone(), Display::Auto));
                }
                (self.binary(*op, a, b)?, Display::Auto)
            }
            Expr::ZoneDiff(a, b) => {
                parts.places = Some((Some(a.clone()), b.clone()));
                self.answer(expr)?
            }
            _ => self.answer(expr)?,
        };
        Ok((value, display, parts))
    }

    /// The answer to a line from its value and the parts kept while working it out.
    pub(crate) fn describe(
        &self,
        expr: &Expr,
        value: Value,
        display: Display,
        parts: Parts,
        config: &Arc<Config>,
    ) -> Answer {
        let text = format::render(&value, &display, self.config, &self.now);
        let form = form(expr, &value);
        Answer {
            value,
            display,
            text,
            form,
            parts,
            config: config.clone(),
            now: self.now.peek().cloned(),
            sides: OnceCell::new(),
        }
    }
}

/// The form of a line from its top-level expression.
fn form(expr: &Expr, value: &Value) -> Form {
    match expr {
        Expr::RatesOn(inner, _) => form(inner, value),
        Expr::Custom { .. } => Form::CustomFunction,
        Expr::Convert(inner, Target::Zone(_)) => match &**inner {
            Expr::Time(TimeExpr::Now | TimeExpr::Today(0)) => Form::TimeAtPlace,
            _ => Form::TimeConversion,
        },
        _ if interval(expr).is_some() => Form::DateInterval,
        Expr::Convert(..) => Form::Conversion,
        Expr::ZoneDiff(..) => Form::TimeDifference,
        Expr::Binary(Op::Add | Op::Sub, ..) if matches!(value, Value::Moment(_)) => Form::DateShift,
        _ if is_lone_value(expr) => Form::Value,
        _ => Form::Calculation,
    }
}

/// The ends of a range of dates or times, also inside a conversion: `days until christmas`,
/// `Jan 1 - Mar 1`.
fn interval(expr: &Expr) -> Option<(&Expr, &Expr)> {
    match expr {
        Expr::Convert(inner, _) => interval(inner),
        Expr::Range(a, b) if is_moment(a) && is_moment(b) => Some((a, b)),
        Expr::Binary(Op::Sub, a, b) if time::yearless(a) && time::yearless(b) => Some((a, b)),
        _ => None,
    }
}

fn is_moment(expr: &Expr) -> bool {
    match expr {
        Expr::Time(_) | Expr::InZone(..) => true,
        Expr::Binary(Op::Add | Op::Sub, a, _) => is_moment(a),
        Expr::Convert(inner, Target::Zone(_)) => is_moment(inner),
        _ => false,
    }
}

/// A value written out, with nothing to work out: `5 km`, `20%`, `today`, `3pm Tokyo`.
fn is_lone_value(expr: &Expr) -> bool {
    match expr {
        Expr::Number(_) | Expr::Bool(_) | Expr::BareUnit(_) | Expr::Time(_) | Expr::Var(_) | Expr::Line(_) => true,
        Expr::WithUnit(e, _) | Expr::Percent(e) | Expr::Neg(e) | Expr::Noted(e, _) | Expr::Food(e, _) => {
            is_lone_value(e)
        }
        Expr::InZone(e, _) => matches!(**e, Expr::Time(_)),
        Expr::Composite(parts) => parts.iter().all(is_lone_value),
        _ => false,
    }
}

/// The place a moment is written in: Berlin in `3pm Berlin`.
fn placed(expr: &Expr) -> Option<&Place> {
    match expr {
        Expr::InZone(_, place) | Expr::Convert(_, Target::Zone(place)) => Some(place),
        Expr::Binary(_, a, _) => placed(a),
        _ => None,
    }
}

/// Days from the date of `a` to the date of `b`, each in its own zone.
fn days_between(a: &Zoned, b: &Zoned) -> i64 {
    let (a, b) = (a.date(), b.date());
    b.since(a).map_or(0, |span| span.get_days() as i64)
}

/// The long name of a value's unit: "Kilometers", "US Dollars".
fn caption(value: &Value) -> Option<String> {
    match value {
        Value::Quantity(q) if !q.unit.is_none() => unit_caption(&q.unit),
        _ => None,
    }
}

/// "Miles", "Fahrenheit", "Kilometers per Hour", "US Dollars", "Square Meters".
pub(crate) fn unit_caption(unit: &Unit) -> Option<String> {
    let reg = registry();
    if let Some(def) = unit.single() {
        let name = match def.currency {
            Some(currency) if def.scale == crate::Number::ONE => currency.plural,
            _ if def.dim == Dim::TEMPERATURE => def.plural.strip_prefix("degrees ").unwrap_or(&def.plural),
            _ => &def.plural,
        };
        return Some(title(name));
    }
    let (num, den): (Vec<_>, Vec<_>) = unit.factors().iter().copied().partition(|&(_, e)| e > 0);
    // Singular names but the last of the numerator: "kilowatt hours", "newton meters".
    let name = |id, e: i8, plural: bool| {
        let def = reg.def(id);
        let base = match (def.currency, plural) {
            (Some(c), true) => c.plural,
            (Some(c), false) => c.name,
            (None, true) => &def.plural,
            (None, false) => &def.name,
        };
        match e.abs() {
            1 => base.to_string(),
            2 if def.dim == Dim::LENGTH => format!("square {base}"),
            3 if def.dim == Dim::LENGTH => format!("cubic {base}"),
            2 => format!("{base} squared"),
            3 => format!("{base} cubed"),
            e => format!("{base} to the power of {e}"),
        }
    };
    let mut words: Vec<String> = num.iter().enumerate().map(|(i, &(id, e))| name(id, e, i + 1 == num.len())).collect();
    for &(id, e) in &den {
        words.push(format!("per {}", name(id, e, false)));
    }
    Some(title(&words.join(" ")))
}

/// Title case, leaving short joining words alone: "US Dollars per Hour".
fn title(text: &str) -> String {
    const SMALL: &[&str] = &["per", "of", "and", "to", "the", "in"];
    text.split(' ')
        .enumerate()
        .map(|(i, word)| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) if i == 0 || !SMALL.contains(&word) => c.to_uppercase().chain(chars).collect(),
                _ => word.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_caption(format: Format) -> Option<String> {
    let name = match format {
        Format::Hex => "Hexadecimal",
        Format::Binary => "Binary",
        Format::Octal => "Octal",
        Format::Base(base) => return Some(format!("Base {base}")),
        Format::Decimal => "Decimal",
        Format::Scientific => "Scientific Notation",
        Format::Fraction => "Fraction",
        Format::Number => "Number",
        Format::Percent => "Percentage",
        Format::Multiplier => "Multiplier",
        Format::Timespan => "Timespan",
        Format::Laptime => "Laptime",
        Format::Timestamp => "Unix Timestamp",
        Format::Date => "Date",
        Format::Iso => "ISO 8601",
        Format::Weekday => "Weekday",
        Format::WeekNumber => "Week Number",
        Format::DayOfYear => "Day of the Year",
        Format::DayOfMonth => "Day of the Month",
        Format::Pitch => "Pitch",
    };
    Some(name.to_string())
}
