//! Evaluates syntax trees into values.

mod arith;
mod functions;
mod time;

pub(crate) use time::get as span_field;

use std::collections::HashMap;

use jiff::Zoned;

use crate::ast::{Direction, Expr, Format, GrowthResult, LineRef, Rounding, Target};
use crate::config::Config;
use crate::error::{Error, Result, bail};
use crate::format::unit_text;
use crate::number::Number;
use crate::rates::Rates;
use crate::units::{Dim, Unit, UnitId, registry};
use crate::value::{Duration, Moment, MomentKind, Quantity, Value};

/// How an answer is written, beyond its value.
#[derive(Clone, Debug, Default)]
pub enum Display {
    #[default]
    Auto,
    /// Base 2, 8 or 16.
    Radix(u32),
    Scientific,
    Fraction,
    Multiplier,
    /// Split over several units: "5 ft 6 in".
    Parts(Vec<Unit>),
    /// No thousands separators, e.g. for timestamps.
    Plain,
    /// Several conversions of one value: "€9.00, ¥1,500".
    Each(Vec<Value>),
}

/// What kind of line an earlier line was, for totals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineKind {
    Value,
    Blank,
    Heading,
    Divider,
    /// A `sum` or `total` line; the next total starts after it.
    Total,
}

#[derive(Clone, Debug)]
pub(crate) struct Line {
    pub value: Option<Value>,
    pub kind: LineKind,
}

/// Everything a calculation can see.
pub(crate) struct Env<'a> {
    pub config: &'a Config,
    pub now: Zoned,
    pub vars: &'a HashMap<String, Value>,
    pub lines: &'a [Line],
    pub rates: &'a Rates,
}

impl Env<'_> {
    /// Evaluates a line, keeping how the answer should be shown.
    pub fn answer(&self, expr: &Expr) -> Result<(Value, Display)> {
        match expr {
            Expr::Convert(inner, target) => {
                let (value, display) = self.answer(inner)?;
                self.convert(value, target, display)
            }
            Expr::Composite(parts) => {
                let value = self.composite(parts)?;
                let display = match &value {
                    Value::Quantity(_) => Display::Parts(parts.iter().filter_map(part_unit).collect()),
                    _ => Display::Auto,
                };
                Ok((value, display))
            }
            _ => Ok((self.eval(expr)?, Display::Auto)),
        }
    }

    pub fn eval(&self, expr: &Expr) -> Result<Value> {
        Ok(match expr {
            Expr::Number(n) => Value::number(*n),
            Expr::Bool(b) => Value::Bool(*b),
            Expr::WithUnit(inner, unit) => match self.eval(inner)? {
                Value::Quantity(q) if q.unit.is_none() => Value::Quantity(Quantity::new(q.number, unit.clone())),
                v => bail!("can't give {} a unit", v.kind()),
            },
            Expr::BareUnit(unit) => Value::Quantity(Quantity::new(Number::ONE, unit.clone())),
            Expr::Percent(inner) => match self.eval(inner)? {
                Value::Quantity(q) if q.unit.is_none() => Value::Percent(q.number),
                v => bail!("can't make {} a percentage", v.kind()),
            },
            Expr::Neg(inner) => self.negate(self.eval(inner)?)?,
            Expr::Factorial(inner) => functions::factorial(self.eval(inner)?)?,
            Expr::Binary(op, a, b) => self.binary(*op, self.eval(a)?, self.eval(b)?)?,
            Expr::Composite(parts) => self.composite(parts)?,
            Expr::Call(func, args) => {
                let args = args.iter().map(|a| self.eval(a)).collect::<Result<Vec<_>>>()?;
                self.call(*func, args)?
            }
            Expr::Var(name) => {
                self.vars.get(name).cloned().ok_or_else(|| Error::new(format!("unknown variable {name}")))?
            }
            Expr::Line(line) => self.line(*line)?,
            Expr::Time(t) => Value::Moment(self.moment(t)?),
            Expr::InZone(inner, zone) => Value::Moment(self.place_in_zone(self.eval(inner)?, zone)?),
            Expr::Range(a, b) => self.range(self.eval(a)?, self.eval(b)?)?,
            Expr::ZoneDiff(a, b) => self.zone_difference(a, b)?,
            Expr::Convert(..) => self.answer(expr)?.0,
            Expr::Round(inner, rounding) => self.round(self.eval(inner)?, *rounding)?,
            Expr::If { .. } => match self.branch(expr)? {
                Some(e) => self.eval(e)?,
                None => bail!("the condition isn't met"),
            },
            Expr::Growth { principal, time, rate, period, compounds, result } => {
                let (principal, time, rate) = (self.eval(principal)?, self.eval(time)?, self.eval(rate)?);
                self.growth(principal, time, rate, period, *compounds, *result)?
            }
        })
    }

    /// The branch of `a if cond else b` to evaluate; `None` if there is none.
    pub fn branch<'e>(&self, expr: &'e Expr) -> Result<Option<&'e Expr>> {
        let Expr::If { cond, then, otherwise } = expr else { return Ok(Some(expr)) };
        let chosen = if self.truthy(&self.eval(cond)?)? { then } else { otherwise };
        Ok(chosen.as_deref())
    }

    pub fn truthy(&self, value: &Value) -> Result<bool> {
        Ok(match value {
            Value::Bool(b) => *b,
            Value::Quantity(q) => !q.number.is_zero(),
            Value::Percent(p) => !p.is_zero(),
            v => bail!("{} is neither true nor false", v.kind()),
        })
    }

    /// Compound growth of `principal` over `time` at yearly `rate`.
    fn growth(
        &self,
        principal: Value,
        time: Value,
        rate: Value,
        period: &Unit,
        compounds: i64,
        result: GrowthResult,
    ) -> Result<Value> {
        let periods = match &time {
            Value::Duration(d) => self.duration_in(d, period)?.number,
            Value::Quantity(q) if q.unit.dim() == Dim::TIME => self.convert_quantity(q, period)?.number,
            v => bail!("expected a length of time, not {}", v.kind()),
        };
        let rate = match rate {
            Value::Percent(p) => p / Number::from_i64(100),
            v => bail!("expected a percentage rate, not {}", v.kind()),
        };
        let n = Number::from_i64(compounds);
        let factor = (Number::ONE + rate / n).pow(periods * n);
        let grown = self.mul(principal.clone(), Value::number(factor))?;
        match result {
            GrowthResult::Future => Ok(grown),
            GrowthResult::Interest => self.sub(grown, principal),
            GrowthResult::Present => self.div(principal, Value::number(factor)),
        }
    }

    /// Size of one unit in base units; currencies use exchange rates.
    pub fn scale(&self, id: UnitId) -> Result<Number> {
        let def = registry().def(id);
        match def.currency {
            Some(c) => self.rates.usd_value(c.code),
            None => Ok(def.scale),
        }
    }

    pub fn convert_quantity(&self, q: &Quantity, to: &Unit) -> Result<Quantity> {
        if q.unit.same_as(to) || q.unit.is_none() {
            return Ok(Quantity::new(q.number, to.clone()));
        }
        if q.unit.dim() != to.dim() {
            bail!("can't convert {} to {}", unit_text(&q.unit, true), unit_text(to, true));
        }
        if let (Some(a), Some(b)) = (q.unit.single(), to.single())
            && a.is_temperature()
            && b.is_temperature()
        {
            let kelvin = q.number * a.scale + a.offset;
            return Ok(Quantity::new((kelvin - b.offset) / b.scale, to.clone()));
        }
        let scale = |id| self.scale(id);
        let factor = q.unit.scale(&scale)? / to.scale(&scale)?;
        Ok(Quantity::new(q.number * factor, to.clone()))
    }

    /// Adjacent parts like `5 ft 3 in` or `3 months 2 weeks`.
    fn composite(&self, parts: &[Expr]) -> Result<Value> {
        let values = parts.iter().map(|p| self.eval(p)).collect::<Result<Vec<_>>>()?;
        let all_time = values.iter().all(|v| matches!(v, Value::Quantity(q) if q.unit.dim() == Dim::TIME));
        if all_time {
            let mut span = jiff::Span::new();
            for v in &values {
                span = time::add_spans(span, self.span_of(v)?)?;
            }
            return Ok(Value::Duration(Duration::new(span)));
        }
        let mut total = values[0].clone();
        for v in &values[1..] {
            total = self.add(total, v.clone())?;
        }
        Ok(total)
    }

    fn line(&self, line: LineRef) -> Result<Value> {
        match line {
            LineRef::Previous => {
                self.lines.iter().rev().find_map(|l| l.value.clone()).ok_or_else(|| Error::new("no previous answer"))
            }
            LineRef::Line(n) => self
                .lines
                .get(n - 1)
                .and_then(|l| l.value.clone())
                .ok_or_else(|| Error::new(format!("line {n} has no answer"))),
            LineRef::Sum | LineRef::Average => {
                let mut block: Vec<Value> = self
                    .lines
                    .iter()
                    .rev()
                    .take_while(|l| l.kind == LineKind::Value)
                    .filter_map(|l| l.value.clone())
                    .filter(|v| matches!(v, Value::Quantity(_) | Value::Percent(_) | Value::Duration(_)))
                    .collect();
                block.reverse();
                let count = block.len();
                let mut values = block.into_iter();
                let Some(first) = values.next() else {
                    return match line {
                        LineRef::Sum => Ok(Value::number(Number::ZERO)),
                        _ => bail!("nothing to average"),
                    };
                };
                let sum = values.try_fold(first, |acc, v| self.add(acc, v))?;
                match line {
                    LineRef::Sum => Ok(sum),
                    _ => self.div(sum, Value::number(Number::from_i64(count as i64))),
                }
            }
        }
    }

    fn convert(&self, value: Value, target: &Target, display: Display) -> Result<(Value, Display)> {
        Ok(match target {
            Target::Unit(unit) => (self.to_unit(value, unit)?, Display::Auto),
            Target::Units(units) => self.to_parts(value, units)?,
            Target::Zone(zone) => (Value::Moment(self.to_zone(value, zone)?), Display::Auto),
            Target::Format(format) => self.to_format(value, *format, display)?,
        })
    }

    fn to_unit(&self, value: Value, unit: &Unit) -> Result<Value> {
        if unit.dim() == Dim::WORKDAY && !matches!(&value, Value::Quantity(q) if q.unit.dim() == Dim::WORKDAY) {
            let duration = match value {
                Value::Duration(d) => d,
                v => Duration::new(self.span_of(&v)?),
            };
            return Ok(Value::Quantity(Quantity::new(self.workdays(&duration)?, unit.clone())));
        }
        match value {
            Value::Quantity(q) => Ok(Value::Quantity(self.convert_quantity(&q, unit)?)),
            Value::Duration(d) => Ok(Value::Quantity(self.duration_in(&d, unit)?)),
            Value::Percent(p) if unit.is_none() => Ok(Value::number(p / Number::from_i64(100))),
            v => bail!("can't convert {} to {}", v.kind(), unit_text(unit, true)),
        }
    }

    /// `in feet and inches`, `in hours and minutes`.
    fn to_parts(&self, value: Value, units: &[Unit]) -> Result<(Value, Display)> {
        // Currencies don't split: "in EUR, JPY" converts to each.
        if units.iter().any(Unit::is_money) {
            let values: Vec<Value> = units.iter().map(|u| self.to_unit(value.clone(), u)).collect::<Result<_>>()?;
            return Ok((values[0].clone(), Display::Each(values)));
        }
        let time = units.iter().all(|u| u.dim() == Dim::TIME);
        let q = match value {
            Value::Duration(d) if time => self.duration_in(&d, &registry().get("s"))?,
            Value::Quantity(q) => q,
            v => bail!("can't split {} into parts", v.kind()),
        };
        let q = self.convert_quantity(&q, &units[0])?;
        if time {
            let seconds = self.convert_quantity(&q, &registry().get("s"))?.number;
            return Ok((Value::Duration(Duration::new(time::split(seconds, units)?)), Display::Auto));
        }
        Ok((Value::Quantity(q), Display::Parts(units.to_vec())))
    }

    fn to_format(&self, value: Value, format: Format, display: Display) -> Result<(Value, Display)> {
        let number_of = |v: &Value| match v {
            Value::Quantity(q) => Some(q.number),
            Value::Percent(p) => Some(*p / Number::from_i64(100)),
            _ => None,
        };
        Ok(match format {
            Format::Hex | Format::Binary | Format::Octal => {
                let radix = match format {
                    Format::Hex => 16,
                    Format::Binary => 2,
                    _ => 8,
                };
                match &value {
                    Value::Quantity(q) if q.number.is_integer() => (value, Display::Radix(radix)),
                    _ => bail!("only whole numbers can be shown in base {radix}"),
                }
            }
            Format::Decimal => match value {
                Value::Percent(p) => (Value::number(p / Number::from_i64(100)), Display::Auto),
                v => (v, if matches!(display, Display::Plain) { display } else { Display::Auto }),
            },
            Format::Scientific => (value, Display::Scientific),
            Format::Fraction => match value {
                Value::Percent(p) => (Value::number(p / Number::from_i64(100)), Display::Fraction),
                v => (v, Display::Fraction),
            },
            Format::Multiplier => match number_of(&value) {
                Some(n) => (Value::number(n), Display::Multiplier),
                None => bail!("can't show {} as a multiplier", value.kind()),
            },
            Format::Number => match &value {
                Value::Duration(d) => (Value::number(self.duration_in(d, &registry().get("s"))?.number), Display::Auto),
                _ => match number_of(&value) {
                    Some(n) => (Value::number(n), Display::Auto),
                    None => bail!("can't make {} a plain number", value.kind()),
                },
            },
            Format::Percent => match value {
                Value::Percent(_) => (value, Display::Auto),
                Value::Quantity(q) if q.unit.is_none() => {
                    (Value::Percent(q.number * Number::from_i64(100)), Display::Auto)
                }
                v => bail!("can't show {} as a percentage", v.kind()),
            },
            Format::Timespan | Format::Laptime => {
                let mut d = match value {
                    Value::Duration(d) => d,
                    Value::Quantity(q) if q.unit.dim() == Dim::TIME => {
                        let seconds = self.convert_quantity(&q, &registry().get("s"))?.number;
                        let largest = if format == Format::Laptime { jiff::Unit::Hour } else { jiff::Unit::Week };
                        Duration::new(time::balance(seconds, largest)?)
                    }
                    v => bail!("can't show {} as a time span", v.kind()),
                };
                if format == Format::Laptime {
                    let seconds = self.duration_in(&d, &registry().get("s"))?.number;
                    d = Duration { span: time::balance(seconds, jiff::Unit::Hour)?, anchor: None, laptime: true };
                } else {
                    d.laptime = false;
                }
                (Value::Duration(d), Display::Auto)
            }
            Format::Timestamp => match value {
                Value::Moment(m) => {
                    let ts = m.time.timestamp();
                    let n = Number::from_i64(ts.as_second()) + Number::ratio(ts.subsec_millisecond() as i64, 1000);
                    (Value::number(n), Display::Plain)
                }
                v => bail!("can't make a timestamp from {}", v.kind()),
            },
            Format::Date => match value {
                Value::Moment(m) => (Value::Moment(Moment { kind: MomentKind::DateTime, ..m }), Display::Auto),
                Value::Quantity(q) if q.unit.is_none() => {
                    (Value::Moment(self.timestamp_moment(q.number)?), Display::Auto)
                }
                v => bail!("can't make a date from {}", v.kind()),
            },
            Format::Iso => match value {
                Value::Moment(m) => (Value::Text(m.time.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string()), Display::Auto),
                v => bail!("can't write {} as ISO 8601", v.kind()),
            },
            Format::Weekday => match value {
                Value::Moment(m) => (Value::Text(m.time.strftime("%A").to_string()), Display::Auto),
                v => bail!("{} has no weekday", v.kind()),
            },
            Format::WeekNumber | Format::DayOfYear | Format::DayOfMonth => match value {
                Value::Moment(m) => {
                    let n = match format {
                        Format::WeekNumber => m.time.date().iso_week_date().week() as i64,
                        Format::DayOfYear => m.time.day_of_year() as i64,
                        _ => m.time.day() as i64,
                    };
                    (Value::number(Number::from_i64(n)), Display::Auto)
                }
                v => bail!("{} is not a date", v.kind()),
            },
        })
    }

    fn round(&self, value: Value, rounding: Rounding) -> Result<Value> {
        let round = |n: Number| match rounding {
            Rounding::Places(dp, dir) => round_places(n, dp, dir),
            Rounding::Significant(digits) => n.round_sig(digits),
            Rounding::Multiple(m, dir) => round_places(n / m, 0, dir) * m,
        };
        Ok(match value {
            Value::Quantity(q) => Value::Quantity(Quantity::new(round(q.number), q.unit)),
            Value::Percent(p) => Value::Percent(round(p)),
            Value::Duration(d) => {
                let seconds = self.duration_in(&d, &registry().get("s"))?;
                Value::Quantity(Quantity::new(round(seconds.number), seconds.unit))
            }
            v => bail!("can't round {}", v.kind()),
        })
    }
}

fn round_places(n: Number, dp: u32, dir: Direction) -> Number {
    match dir {
        Direction::Nearest => n.round_dp(dp),
        Direction::Up | Direction::Down => {
            let k = Number::pow10(dp as i32);
            let scaled = n * k;
            let r = if dir == Direction::Up { scaled.ceil() } else { scaled.floor() };
            r / k
        }
    }
}

fn part_unit(expr: &Expr) -> Option<Unit> {
    match expr {
        Expr::WithUnit(_, u) => Some(u.clone()),
        _ => None,
    }
}
