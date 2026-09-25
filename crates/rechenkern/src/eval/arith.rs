//! Operators on values: units, percentages, dates and plain numbers.

use std::cmp::Ordering;

use super::{Display, Env};
use crate::ast::{Format, Op};
use crate::error::{Error, Result, bail};
use crate::format::unit_text;
use crate::number::Number;
use crate::units::{Dim, Unit, registry};
use crate::value::{Duration, MomentKind, Quantity, Value};

fn hundred() -> Number {
    Number::from_i64(100)
}

impl Env<'_> {
    pub(crate) fn binary(&self, op: Op, a: Value, b: Value) -> Result<Value> {
        match op {
            Op::Add => self.add(a, b),
            Op::Sub => self.sub(a, b),
            Op::Mul => self.mul(a, b),
            Op::Div => self.div(a, b),
            Op::Per => match (a, b) {
                (Value::Quantity(x), Value::Quantity(y)) => {
                    Ok(Value::Quantity(Quantity::new(x.number / y.number, x.unit.product(&y.unit.pow(-1)))))
                }
                (a, b) => self.div(a, b),
            },
            Op::Pow => self.pow(a, b),
            Op::Mod => self.rem(a, b),
            Op::Of => self.of(a, b),
            Op::OutOf => Ok(self.to_format(self.div(a, b)?, Format::Percent, Display::Auto)?.0),
            Op::On | Op::Off => self.on_off(a, b, op == Op::Off),
            Op::At => self.at(a, b),
            Op::BitAnd | Op::BitOr | Op::BitXor | Op::Shl | Op::Shr => bitwise(op, a, b),
            Op::Eq | Op::Ne | Op::Lt | Op::Gt | Op::Le | Op::Ge => self.compare(op, a, b),
            Op::And | Op::Or => match (a, b) {
                (Value::Bool(x), Value::Bool(y)) => Ok(Value::Bool(if op == Op::And { x && y } else { x || y })),
                _ => bail!("\"and\" and \"or\" need true or false on both sides"),
            },
            Op::Perm | Op::Comb => super::functions::choose(a, b, op == Op::Perm),
        }
    }

    pub(super) fn negate(&self, v: Value) -> Result<Value> {
        Ok(match v {
            Value::Quantity(q) => Value::Quantity(Quantity::new(-q.number, q.unit)),
            Value::Percent(p) => Value::Percent(-p),
            Value::Duration(d) => Value::Duration(Duration { span: d.span.negate(), ..d }),
            v => bail!("can't negate {}", v.kind()),
        })
    }

    pub(super) fn add(&self, a: Value, b: Value) -> Result<Value> {
        self.add_sub(a, b, false)
    }

    pub(super) fn sub(&self, a: Value, b: Value) -> Result<Value> {
        self.add_sub(a, b, true)
    }

    fn add_sub(&self, a: Value, b: Value, sub: bool) -> Result<Value> {
        let sign = |n: Number| if sub { -n } else { n };
        Ok(match (a, b) {
            (Value::Quantity(x), Value::Quantity(y)) => self.add_quantities(x, y, sub)?,
            // "$50 + 10%" adds ten percent of $50.
            (Value::Quantity(x), Value::Percent(p)) => {
                Value::Quantity(Quantity::new(x.number * (Number::ONE + sign(p) / hundred()), x.unit))
            }
            (Value::Percent(p), Value::Percent(q)) => Value::Percent(p + sign(q)),
            // "30% + 0.4" treats 0.4 as 40%.
            (Value::Percent(p), Value::Quantity(y)) if y.unit.is_none() => {
                Value::Percent(p + sign(y.number) * hundred())
            }
            (Value::Moment(m), d @ (Value::Quantity(_) | Value::Duration(_))) => {
                Value::Moment(self.shift(&m, &d, sub)?)
            }
            (d @ (Value::Quantity(_) | Value::Duration(_)), Value::Moment(m)) if !sub => {
                Value::Moment(self.shift(&m, &d, false)?)
            }
            (Value::Moment(a), Value::Moment(b)) if sub => Value::Duration(self.between(&a, &b, false)?),
            // "1:30 + 0:45" adds the second clock time as a duration.
            (Value::Moment(a), Value::Moment(b)) if b.kind == MomentKind::Clock => {
                let since_midnight = b.time.start_of_day()?.until(&b.time)?;
                Value::Moment(self.shift(&a, &Value::Duration(Duration::new(since_midnight)), false)?)
            }
            (a @ (Value::Duration(_) | Value::Quantity(_)), b @ (Value::Duration(_) | Value::Quantity(_))) => {
                let d = match (&a, &b) {
                    (Value::Duration(d), _) | (_, Value::Duration(d)) => d.clone(),
                    _ => unreachable!("quantities are handled above"),
                };
                let (x, y) = (self.seconds(&a)?, self.seconds(&b)?);
                let total = if sub { x - y } else { x + y };
                let span = super::time::balance(total, super::time::largest_unit(&d.span))?;
                Value::Duration(Duration { span, anchor: None, laptime: d.laptime })
            }
            (a, b) => bail!("can't {} {} and {}", if sub { "subtract" } else { "add" }, a.kind(), b.kind()),
        })
    }

    /// Adds quantities: plain numbers take the other unit, the larger unit wins,
    /// and for money the last currency wins.
    fn add_quantities(&self, x: Quantity, y: Quantity, sub: bool) -> Result<Value> {
        let combine = |a: Number, b: Number| if sub { a - b } else { a + b };
        if x.unit.is_none() || y.unit.is_none() || x.unit.same_as(&y.unit) {
            let unit = if x.unit.is_none() { y.unit } else { x.unit };
            return Ok(Value::Quantity(Quantity::new(combine(x.number, y.number), unit)));
        }
        if x.unit.dim() != y.unit.dim() {
            bail!("can't combine {} and {}", unit_text(&x.unit, true), unit_text(&y.unit, true));
        }
        if x.unit.dim() == Dim::TIME && !is_calendar(&x.unit) && !is_calendar(&y.unit) {
            let (a, b) = (Value::Quantity(x.clone()), Value::Quantity(y.clone()));
            let seconds =
                if sub { self.seconds(&a)? - self.seconds(&b)? } else { self.seconds(&a)? + self.seconds(&b)? };
            let largest = [&x.unit, &y.unit]
                .iter()
                .filter_map(|u| u.single()?.calendar.map(|(cal, _)| cal))
                .max()
                .unwrap_or(jiff::Unit::Second);
            return Ok(Value::Duration(Duration::new(super::time::balance(seconds, largest)?)));
        }
        // "1 year - 2 months" is 10 months: calendar units meet in the smaller one.
        if x.unit.dim() == Dim::TIME && is_calendar(&x.unit) && is_calendar(&y.unit) {
            let scale = |id| self.scale(id);
            let target = if x.unit.scale(&scale)? < y.unit.scale(&scale)? { &x.unit } else { &y.unit };
            let (a, b) = (self.convert_quantity(&x, target)?.number, self.convert_quantity(&y, target)?.number);
            return Ok(Value::Quantity(Quantity::new(combine(a, b), target.clone())));
        }
        let temperature = x.unit.single().is_some_and(|d| d.is_temperature());
        // Money and rates use the last unit: "$20/day + $300/week" is per week.
        let target = if y.unit.currency().is_some() || y.unit.factors().len() > 1 {
            y.unit.clone()
        } else if temperature {
            x.unit.clone()
        } else {
            let scale = |id| self.scale(id);
            if y.unit.scale(&scale)? > x.unit.scale(&scale)? { y.unit.clone() } else { x.unit.clone() }
        };
        let to_target = |q: Quantity| -> Result<Number> {
            if temperature && !q.unit.same_as(&target) {
                // A temperature on the right is a difference, not a reading.
                let scale = |id| self.scale(id);
                return Ok(q.number * q.unit.scale(&scale)? / target.scale(&scale)?);
            }
            Ok(self.convert_quantity(&q, &target)?.number)
        };
        let (a, b) = (to_target(x)?, to_target(y)?);
        Ok(Value::Quantity(Quantity::new(combine(a, b), target)))
    }

    pub(super) fn mul(&self, a: Value, b: Value) -> Result<Value> {
        Ok(match (a, b) {
            (Value::Quantity(x), Value::Quantity(y)) => {
                let units = (x.unit.clone(), y.unit.clone());
                self.tidy_time(self.mul_quantities(x, y, false)?, units)?
            }
            (Value::Quantity(q), Value::Percent(p)) | (Value::Percent(p), Value::Quantity(q)) => {
                Value::Quantity(Quantity::new(q.number * p / hundred(), q.unit))
            }
            (Value::Percent(p), Value::Percent(q)) => Value::Percent(p * q / hundred()),
            (Value::Duration(d), Value::Quantity(n)) | (Value::Quantity(n), Value::Duration(d)) if n.unit.is_none() => {
                let seconds = self.seconds(&Value::Duration(d.clone()))? * n.number;
                Value::Duration(Duration {
                    span: super::time::balance(seconds, super::time::largest_unit(&d.span))?,
                    anchor: None,
                    ..d
                })
            }
            (a, b) => bail!("can't multiply {} by {}", a.kind(), b.kind()),
        })
    }

    pub(super) fn div(&self, a: Value, b: Value) -> Result<Value> {
        let zero = |n: Number| if n.is_zero() { Err(Error::new("division by zero")) } else { Ok(()) };
        Ok(match (a, b) {
            (Value::Quantity(x), Value::Quantity(y)) => {
                zero(y.number)?;
                let units = (x.unit.clone(), y.unit.clone());
                self.tidy_time(self.mul_quantities(x, y, true)?, units)?
            }
            // "$50 / 20%" is the whole that $50 is 20% of.
            (Value::Quantity(x), Value::Percent(p)) => {
                zero(p)?;
                Value::Quantity(Quantity::new(x.number * hundred() / p, x.unit))
            }
            (Value::Percent(p), Value::Quantity(y)) if y.unit.is_none() => {
                zero(y.number)?;
                Value::Percent(p / y.number)
            }
            (Value::Percent(p), Value::Percent(q)) => {
                zero(q)?;
                Value::number(p / q)
            }
            (Value::Duration(d), Value::Quantity(n)) if n.unit.is_none() => {
                zero(n.number)?;
                let seconds = self.seconds(&Value::Duration(d.clone()))? / n.number;
                Value::Duration(Duration {
                    span: super::time::balance(seconds, super::time::largest_unit(&d.span))?,
                    anchor: None,
                    ..d
                })
            }
            (a @ (Value::Duration(_) | Value::Quantity(_)), b @ (Value::Duration(_) | Value::Quantity(_))) => {
                let (x, y) = (self.seconds(&a)?, self.seconds(&b)?);
                zero(y)?;
                Value::number(x / y)
            }
            (a, b) => bail!("can't divide {} by {}", a.kind(), b.kind()),
        })
    }

    /// Seconds that come out of unit algebra read better as a time span:
    /// "3 GB / 10 MB/s" is 5 minutes, not 300 seconds.
    fn tidy_time(&self, q: Quantity, (a, b): (Unit, Unit)) -> Result<Value> {
        let second = registry().get("s");
        let derived = q.unit.same_as(&second) && !a.same_as(&second) && !b.same_as(&second);
        if derived && q.number.abs() >= Number::from_i64(60) {
            return Ok(Value::Duration(Duration::new(super::time::balance(q.number, jiff::Unit::Hour)?)));
        }
        Ok(Value::Quantity(q))
    }

    /// Multiplies or divides quantities, merging units of the same kind.
    fn mul_quantities(&self, x: Quantity, y: Quantity, divide: bool) -> Result<Quantity> {
        let y_unit = if divide { y.unit.pow(-1) } else { y.unit.clone() };
        let number = if divide { x.number / y.number } else { x.number * y.number };
        // "$30 × 4 days" means $30 a day: money times a non-money unit stays money.
        if !divide
            && (x.unit.is_money() || y.unit.is_money())
            && x.unit.currency().is_some() != y.unit.currency().is_some()
        {
            let other = if x.unit.is_money() { &y.unit } else { &x.unit };
            if !other.is_none() && other.factors().iter().all(|&(_, e)| e > 0) {
                let money = if x.unit.is_money() { x.unit } else { y.unit };
                return Ok(Quantity::new(number, money));
            }
        }
        // "$500/month / 30 days" spreads the money over those days: $16.67/day.
        if divide && x.unit.dim() == Dim::MONEY.div(Dim::TIME) && y.unit.dim() == Dim::TIME {
            return Ok(Quantity::new(number, x.unit.numerator().product(&y_unit)));
        }
        let (unit, factor) = x.unit.mul(&y_unit, &|id| self.scale(id))?;
        Ok(Quantity::new(number * factor, unit))
    }

    fn pow(&self, a: Value, b: Value) -> Result<Value> {
        let exp = match b {
            Value::Quantity(q) if q.unit.is_none() => q.number,
            Value::Percent(p) => p / hundred(),
            b => bail!("can't raise to {}", b.kind()),
        };
        let q = match a {
            Value::Quantity(q) => q,
            Value::Percent(p) => Quantity::plain(p / hundred()),
            a => bail!("can't raise {} to a power", a.kind()),
        };
        let unit = if q.unit.is_none() {
            Unit::none()
        } else {
            match exp.to_i64() {
                Some(e) if (-9..=9).contains(&e) => q.unit.pow(e as i8),
                _ => match (exp * Number::from_i64(2)).to_i64() {
                    // Square roots of units: (4 m²)^0.5
                    Some(1) => q.unit.root(2).ok_or_else(|| Error::new("can't take that root of the unit"))?,
                    _ => bail!("units can only be raised to whole powers"),
                },
            }
        };
        let n = q.number.pow(exp);
        if n.to_f64().is_nan() {
            bail!("result is not a real number");
        }
        if !n.is_finite() {
            bail!("result is too large");
        }
        Ok(Value::Quantity(Quantity::new(n, unit)))
    }

    fn rem(&self, a: Value, b: Value) -> Result<Value> {
        let (Value::Quantity(x), Value::Quantity(y)) = (a, b) else { bail!("remainder needs two numbers") };
        let y = if y.unit.is_none() { y } else { self.convert_quantity(&y, &x.unit)? };
        let r = x.number.checked_rem(y.number).ok_or_else(|| Error::new("division by zero"))?;
        Ok(Value::Quantity(Quantity::new(r, x.unit)))
    }

    /// "20% of 50", "half of 10", "2/3 of $600".
    fn of(&self, a: Value, b: Value) -> Result<Value> {
        match a {
            Value::Percent(_) => self.mul(a, b),
            Value::Quantity(q) if q.unit.is_none() => self.mul(Value::Quantity(q), b),
            a => bail!("can't take {} of something", a.kind()),
        }
    }

    /// "10% on 200" adds, "10% off 200" subtracts.
    fn on_off(&self, a: Value, b: Value, off: bool) -> Result<Value> {
        match a {
            Value::Percent(p) => self.add_sub(b, Value::Percent(p), off),
            // "$5 off $20"
            a => self.add_sub(b, a, off),
        }
    }

    /// "30 hours at $30/hour" multiplies, "$500 at $20/hour" divides.
    fn at(&self, a: Value, b: Value) -> Result<Value> {
        // Playback speed: "1 hour at 1.5x".
        let is_time = matches!(&a, Value::Duration(_)) || matches!(&a, Value::Quantity(q) if q.unit.dim() == Dim::TIME);
        if is_time && matches!(&b, Value::Quantity(q) if q.unit.is_none()) {
            return self.div(a, b);
        }
        let product = self.mul(a.clone(), b.clone());
        let quotient = self.div(a, b);
        let factors = |v: &Result<Value>| match v {
            Ok(Value::Quantity(q)) => q.unit.factors().len(),
            Ok(_) => 1,
            Err(_) => usize::MAX,
        };
        if factors(&quotient) < factors(&product) { quotient } else { product }
    }

    fn compare(&self, op: Op, a: Value, b: Value) -> Result<Value> {
        let ordering = match (&a, &b) {
            (Value::Quantity(x), Value::Quantity(y)) => {
                let y =
                    if x.unit.is_none() || y.unit.is_none() { y.clone() } else { self.convert_quantity(y, &x.unit)? };
                x.number.partial_cmp(&y.number)
            }
            (Value::Percent(p), Value::Percent(q)) => p.partial_cmp(q),
            (Value::Moment(x), Value::Moment(y)) => Some(x.time.cmp(&y.time)),
            (Value::Bool(x), Value::Bool(y)) => Some(x.cmp(y)),
            (Value::Duration(_) | Value::Quantity(_), Value::Duration(_) | Value::Quantity(_)) => {
                self.seconds(&a)?.partial_cmp(&self.seconds(&b)?)
            }
            _ => bail!("can't compare {} with {}", a.kind(), b.kind()),
        };
        let Some(ord) = ordering else { bail!("can't compare these numbers") };
        Ok(Value::Bool(match op {
            Op::Eq => ord == Ordering::Equal,
            Op::Ne => ord != Ordering::Equal,
            Op::Lt => ord == Ordering::Less,
            Op::Gt => ord == Ordering::Greater,
            Op::Le => ord != Ordering::Greater,
            _ => ord != Ordering::Less,
        }))
    }
}

/// Months and longer have no fixed length.
fn is_calendar(unit: &Unit) -> bool {
    unit.single().and_then(|d| d.calendar).is_none_or(|(cal, _)| cal >= jiff::Unit::Month)
}

fn bitwise(op: Op, a: Value, b: Value) -> Result<Value> {
    let int = |v: &Value| match v {
        Value::Quantity(q) if q.unit.is_none() => q.number.to_i128(),
        _ => None,
    };
    let (Some(x), Some(y)) = (int(&a), int(&b)) else { bail!("bitwise operators need whole numbers") };
    let shift = |y: i128| u32::try_from(y).ok().filter(|s| *s < 127).ok_or_else(|| Error::new("shift is too large"));
    let r = match op {
        Op::BitAnd => x & y,
        Op::BitOr => x | y,
        Op::BitXor => x ^ y,
        Op::Shl => x.checked_shl(shift(y)?).ok_or_else(|| Error::new("shift is too large"))?,
        _ => x >> shift(y)?,
    };
    let n = i64::try_from(r).map(Number::from_i64).unwrap_or_else(|_| Number::from_f64(r as f64));
    Ok(Value::number(n))
}
