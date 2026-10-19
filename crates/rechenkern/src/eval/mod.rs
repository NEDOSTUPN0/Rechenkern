//! Evaluates syntax trees into values.

mod arith;
mod functions;
mod sky;
pub(crate) mod time;
mod weather;

pub(crate) use time::get as span_field;

use std::cell::Cell;
use std::collections::HashMap;

use crate::ast::{Direction, Expr, Format, Func, GrowthResult, LineRef, Op, Rounding, Target};
use crate::config::{Config, Now};
use crate::error::{Error, Result, bail};
use crate::format::unit_text;
use crate::inflation::InflationProvider;
use crate::number::Number;
use crate::rates::Rates;
use crate::units::foods::Food;
use crate::units::{Dim, Unit, UnitId, registry};
use crate::value::{Duration, Moment, MomentKind, Quantity, Value};
use crate::weather::WeatherProvider;

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
    /// A remark after the value: "Thu, 19 Nov 2026 (probably)".
    Note(std::borrow::Cow<'static, str>),
    /// A duration as its first and last day: "1 Jul – 30 Sep 2026".
    Period,
    /// Rounded to decimal places, all of them shown: "0.333333333333333".
    Places(u32),
    /// Rounded to significant digits, all of them shown.
    Significant(u32),
    /// Digits beyond what a number holds: pi to 1000 places.
    Digits(String),
    /// Coordinates in degrees, minutes and seconds.
    Dms,
    /// With the scale the line wrote, while the answer stays exact with it: "$2.5M".
    Scaled,
}

/// What kind of line an earlier line was, for totals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineKind {
    Value,
    Blank,
    Heading,
    Divider,
    /// A `sum`, `total` or `average` line; the next total starts after it.
    Total,
}

#[derive(Clone, Debug)]
pub(crate) struct Line {
    pub value: Option<Value>,
    pub kind: LineKind,
    /// Hashtags in the line, lowercase: "food" in "Coffee $5 #food".
    pub tags: Vec<String>,
}

/// Everything a calculation can see.
pub(crate) struct Env<'a> {
    pub config: &'a Config,
    pub now: Now<'a>,
    pub vars: &'a HashMap<String, Value>,
    /// Functions defined in earlier lines.
    pub functions: &'a HashMap<String, std::sync::Arc<crate::ast::UserFunction>>,
    /// How deep calls of those functions go, to stop endless recursion.
    pub depth: Cell<u32>,
    pub lines: &'a [Line],
    /// Hashtags of the line being worked out: "sum #food" adds up the lines tagged so.
    pub tags: Vec<String>,
    pub rates: &'a Rates,
    /// Inside a conversion to a weight or a volume, amounts of food become that kind:
    /// "1 cup of flour + 1 cup of sugar in grams".
    pub food_as: Cell<Option<Dim>>,
    /// Inside `... on Jan 1, 2020`, exchange rates are that day's.
    pub rate_date: Cell<Option<jiff::civil::Date>>,
    /// Money converts at rates at hand, never downloaded: a conversion nobody asked for.
    pub rates_at_hand: Cell<bool>,
    pub inflation: Option<&'a dyn InflationProvider>,
    pub weather: Option<&'a dyn WeatherProvider>,
    /// What the line read, for sheets that recalculate only what changed.
    pub reads: std::cell::RefCell<crate::calculator::Reads>,
}

impl Env<'_> {
    /// Evaluates a line, keeping how the answer should be shown.
    pub fn answer(&self, expr: &Expr) -> Result<(Value, Display)> {
        match expr {
            // "20-30 minutes in hours": both ends of a range of amounts.
            Expr::Convert(inner, target)
                if let Expr::Range(a, b) = &**inner
                    && let Some(ends) = self.amount_range(a, b)? =>
            {
                let [a, b] = ends.map(|q| self.convert(Value::Quantity(q), target, Display::Auto));
                Ok((Value::Range(a?.0.into(), b?.0.into()), Display::Auto))
            }
            Expr::Range(a, b) if let Some([a, b]) = self.amount_range(a, b)? => {
                Ok((Value::Range(Value::Quantity(a).into(), Value::Quantity(b).into()), Display::Auto))
            }
            Expr::Convert(inner, target) => {
                let saved = self.food_as.replace(weight_or_volume(target));
                let answer = self.answer(inner);
                self.food_as.set(saved);
                let (value, display) = answer?;
                self.convert(value, target, display)
            }
            Expr::Food(inner, _) if self.food_as.get().is_none() => self.answer(inner),
            Expr::Composite(parts) => {
                let value = self.composite(parts)?;
                let display = match &value {
                    Value::Quantity(_) => Display::Parts(parts.iter().filter_map(part_unit).collect()),
                    _ => Display::Auto,
                };
                Ok((value, display))
            }
            Expr::Noted(inner, note) => Ok((self.eval(inner)?, Display::Note((*note).into()))),
            Expr::Weather { what, place, on } => self.weather(*what, place.as_ref(), on.as_deref()),
            Expr::RatesOn(inner, date) => self.on_rates_of(date, || self.answer(inner)),
            Expr::Period(..) => Ok((self.eval(expr)?, Display::Period)),
            Expr::Round(_, Rounding::Places(dp, _)) => Ok((self.eval(expr)?, Display::Places(*dp))),
            Expr::Round(_, Rounding::Significant(digits)) => Ok((self.eval(expr)?, Display::Significant(*digits))),
            Expr::Pi(places, dir) => {
                let (value, text) = pi(*places, *dir)?;
                Ok((value, Display::Digits(text)))
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
            Expr::Not(inner) => match self.eval(inner)? {
                Value::Bool(b) => Value::Bool(!b),
                v => bail!("can't take \"not\" of {}", v.kind()),
            },
            // "Jan 1 - Mar 1" is a range, as "Jan 1 to Mar 1".
            Expr::Binary(Op::Sub, a, b) if time::yearless(a) && time::yearless(b) => {
                let (a, b) = self.range_ends(a, b)?;
                self.range(a, b)?
            }
            Expr::Binary(op, a, b) => self.binary(*op, self.eval(a)?, self.eval(b)?)?,
            Expr::Composite(parts) => self.composite(parts)?,
            Expr::Noted(inner, _) => self.eval(inner)?,
            Expr::Food(inner, food) => match self.food_as.get() {
                Some(dim) => self.weigh_or_measure(self.eval(inner)?, *food, dim)?,
                None => self.eval(inner)?,
            },
            Expr::Counted { count, grams, food, piece } => {
                let count = self.eval(count)?;
                let to = self.food_as.get();
                if to.is_none() && !piece {
                    return Ok(count);
                }
                let weight = self.mul(count, Value::Quantity(Quantity::new(*grams, registry().get("g"))))?;
                match to {
                    Some(dim) => self.weigh_or_measure(weight, *food, dim)?,
                    None => weight,
                }
            }
            // "halfway between Jan 1 and March 1": dates without a year are read as a range.
            Expr::Call(Func::Midpoint, args) if args.len() == 2 && args.iter().any(time::yearless) => {
                let (a, b) = self.range_ends(&args[0], &args[1])?;
                self.call(Func::Midpoint, vec![a, b])?
            }
            Expr::Call(func, args) => {
                let args = args.iter().map(|a| self.eval(a)).collect::<Result<Vec<_>>>()?;
                self.call(*func, args)?
            }
            Expr::Var(name) => {
                #[cfg(test)]
                crate::tests::SHEET_READS.with(|reads| reads.set(reads.get() + 1));
                self.reads.borrow_mut().names.push(name.clone());
                self.vars.get(name).cloned().ok_or_else(|| Error::new(format!("unknown variable {name}")))?
            }
            Expr::Line(line) => self.line(*line)?,
            Expr::Cadence { every, start, until } => {
                let every = self.span_of(&self.eval(every)?)?;
                let day = |e: &Expr| match self.eval(e)? {
                    Value::Moment(m) => Ok(m.time.date()),
                    v => bail!("expected a date, not {}", v.kind()),
                };
                let start = match start {
                    Some(e) => day(e)?,
                    None => self.today(),
                };
                let until = until.as_deref().map(day).transpose()?;
                Value::Cadence(crate::value::Cadence { every, start, until })
            }
            Expr::SalesTax { amount, rate, part } => {
                let rate = match self.eval(rate)? {
                    Value::Percent(p) => p / Number::from_i64(100),
                    v => bail!("expected a tax rate in percent, not {}", v.kind()),
                };
                let factor = match part {
                    crate::ast::TaxPart::Tax => rate / (Number::ONE + rate),
                    crate::ast::TaxPart::Net => Number::ONE / (Number::ONE + rate),
                    crate::ast::TaxPart::Gross => Number::ONE + rate,
                };
                self.mul(self.eval(amount)?, Value::number(factor))?
            }
            Expr::Pace(distance, time) => {
                let Value::Quantity(distance) = self.eval(distance)? else { bail!("a pace needs a distance") };
                let minutes = self.seconds(&self.eval(time)?)? / Number::from_i64(60);
                let unit = registry().get("min").product(&distance.unit.pow(-1));
                Value::Quantity(Quantity::new(minutes / distance.number, unit))
            }
            Expr::Resolution(width, height) => {
                Value::Resolution(crate::value::Resolution { width: *width, height: *height })
            }
            Expr::Coordinates(latitude, longitude) => {
                Value::Coordinates(crate::value::Coordinates { latitude: *latitude, longitude: *longitude })
            }
            Expr::Apply(name, args) => self.apply(name, args)?,
            Expr::Time(t) => Value::Moment(self.moment(t)?),
            Expr::InZone(inner, zone) => Value::Moment(self.place_in_zone(self.eval(inner)?, &zone.zone)?),
            Expr::Range(a, b) => {
                let (a, b) = self.range_ends(a, b)?;
                self.range(a, b)?
            }
            Expr::Period(first, length) => self.period(first, length)?,
            Expr::ZoneDiff(a, b) => self.zone_difference(&a.zone, &b.zone)?,
            Expr::Sky { what, place, on } => self.sky(*what, place.as_ref(), on.as_deref())?,
            Expr::Weather { what, place, on } => self.weather(*what, place.as_ref(), on.as_deref())?.0,
            Expr::Convert(..) | Expr::RatesOn(..) => self.answer(expr)?.0,
            Expr::Round(inner, rounding) => self.round(self.eval(inner)?, *rounding)?,
            Expr::Pi(places, dir) => pi(*places, *dir)?.0,
            Expr::If { .. } => match self.branch(expr)? {
                Some(e) => self.eval(e)?,
                None => bail!("the condition isn't met"),
            },
            Expr::Custom { function, args } => self.custom_function(*function, args)?,
            Expr::Inflation { amount, from, to, rate } => {
                self.inflation(self.eval(amount)?, *from, *to, rate.as_deref())?
            }
            Expr::Growth { principal, time, rate, period, periods, compounds, result } => {
                let (principal, time, rate) = (self.eval(principal)?, self.eval(time)?, self.eval(rate)?);
                self.growth(principal, time, rate, (period, *periods), *compounds, *result)?
            }
        })
    }

    /// Works out `f` with the exchange rates of the day `date` stands for.
    pub(crate) fn on_rates_of<T>(&self, date: &Expr, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let day = match self.eval(date)? {
            Value::Moment(m) => m.time.date(),
            v => bail!("expected a date for the exchange rates, not {}", v.kind()),
        };
        let saved = self.rate_date.replace(Some(day));
        let result = f();
        self.rate_date.set(saved);
        result
    }

    /// The ends of a range of amounts in one unit, as in "5-10 kg": a plain number
    /// takes the other end's unit. `None` for anything else, such as dates.
    fn amount_range(&self, a: &Expr, b: &Expr) -> Result<Option<[Quantity; 2]>> {
        let Value::Quantity(mut a) = self.eval(a)? else { return Ok(None) };
        let Value::Quantity(mut b) = self.eval(b)? else { return Ok(None) };
        if a.unit.is_none() {
            a.unit = b.unit.clone();
        } else if b.unit.is_none() {
            b.unit = a.unit.clone();
        }
        Ok((a.unit == b.unit).then_some([a, b]))
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

    /// A function defined in an earlier line, with its parameters set to the arguments.
    fn apply(&self, name: &str, args: &[Expr]) -> Result<Value> {
        self.reads.borrow_mut().names.push(name.to_string());
        let function = self.functions.get(name).ok_or_else(|| Error::new(format!("unknown function \"{name}\"")))?;
        if args.len() != function.params.len() {
            bail!("{name} takes {} values, not {}", function.params.len(), args.len());
        }
        if self.depth.get() >= 64 {
            bail!("{name} calls itself too deep");
        }
        let mut vars = self.vars.clone();
        for (param, arg) in function.params.iter().zip(args) {
            vars.insert(param.clone(), self.eval(arg)?);
        }
        let inner = Env {
            config: self.config,
            now: crate::config::Now::new(self.config),
            vars: &vars,
            functions: self.functions,
            depth: Cell::new(self.depth.get() + 1),
            lines: self.lines,
            tags: self.tags.clone(),
            rates: self.rates,
            food_as: Cell::new(None),
            rate_date: Cell::new(self.rate_date.get()),
            rates_at_hand: Cell::new(self.rates_at_hand.get()),
            inflation: self.inflation,
            weather: self.weather,
            reads: Default::default(),
        };
        let result = inner.eval(&function.body);
        self.reads.borrow_mut().merge(inner.reads.take());
        result
    }

    /// One of the host's phrases, worked out by its handler.
    fn custom_function(&self, function: usize, args: &[crate::ast::CustomArg]) -> Result<Value> {
        use crate::ast::CustomArg;
        use crate::custom::SlotKind;
        let custom = &self.config.custom_functions[function];
        let values = args
            .iter()
            .map(|arg| match arg {
                CustomArg::Unit(unit) => Ok(Value::Quantity(Quantity::new(Number::ONE, unit.clone()))),
                CustomArg::Text(text) => Ok(Value::Text(text.clone())),
                CustomArg::Expr(expr, kind) => {
                    let value = self.eval(expr)?;
                    let fits = match (kind, &value) {
                        (SlotKind::Number, Value::Quantity(q)) => q.unit.is_none(),
                        (SlotKind::Amount, Value::Quantity(q)) => !q.unit.is_none(),
                        (SlotKind::Money, Value::Quantity(q)) => q.unit.is_money(),
                        (SlotKind::Percentage, Value::Percent(_)) => true,
                        (SlotKind::Date, Value::Moment(_)) => true,
                        (SlotKind::Duration, Value::Duration(_)) => true,
                        (SlotKind::Duration, Value::Quantity(q)) => q.unit.dim() == Dim::TIME,
                        (SlotKind::Value, _) => true,
                        _ => false,
                    };
                    match fits {
                        true => Ok(value),
                        false => bail!("\"{}\" needs another kind of value than {}", custom.prototype, value.kind()),
                    }
                }
            })
            .collect::<Result<Vec<_>>>()?;
        (custom.handler)(&values).map_err(Error::new)
    }

    /// `amount` in prices of year `to` instead of year `from` (`None` is now): by the consumer
    /// price index, or by a yearly rate of inflation.
    fn inflation(&self, amount: Value, from: Option<i16>, to: Option<i16>, rate: Option<&Expr>) -> Result<Value> {
        let currency = match &amount {
            Value::Quantity(q) if q.unit.is_none() => self.config.dollar.clone(),
            Value::Quantity(q) => match q.unit.currency() {
                Some((currency, 1)) => currency.code.to_string(),
                _ => bail!("inflation changes amounts of money, not {}", amount.kind()),
            },
            v => bail!("inflation changes amounts of money, not {}", v.kind()),
        };
        let factor = match rate {
            Some(rate) => {
                let rate = match self.eval(rate)? {
                    Value::Percent(p) => p / Number::from_i64(100),
                    v => bail!("expected a percentage of inflation, not {}", v.kind()),
                };
                let this_year = self.today().year();
                let years = to.unwrap_or(this_year) - from.unwrap_or(this_year);
                Number::ONE / (Number::ONE + rate).pow(Number::from_i64(years as i64))
            }
            None => {
                let [from, to] = self.price_levels(&currency, [from, to])?;
                to / from
            }
        };
        self.mul(amount, Value::number(factor))
    }

    /// Consumer price indexes of the years (`None` is now), all from the host's provider or
    /// all from the bundled US index, so they share a base.
    fn price_levels(&self, currency: &str, years: [Option<i16>; 2]) -> Result<[Number; 2]> {
        let today = self.today();
        let asked = |year: Option<i16>| match year {
            Some(y) => self.inflation?.cpi(currency, y, None),
            None => self.inflation?.cpi(currency, today.year(), Some(today.month())),
        };
        if let [Some(a), Some(b)] = years.map(asked) {
            return Ok([a, b]);
        }
        if currency != "USD" {
            bail!("no inflation data for {currency}");
        }
        let bundled = |year: Option<i16>| match year {
            Some(y) => crate::inflation::us_cpi(y).ok_or_else(|| Error::new(format!("no inflation data for {y}"))),
            None => Ok(crate::inflation::us_cpi_now()),
        };
        Ok([bundled(years[0])?, bundled(years[1])?])
    }

    /// Compound growth of `principal` over `time` at `rate` per `period`, or a loan.
    fn growth(
        &self,
        principal: Value,
        time: Value,
        rate: Value,
        (period, periods): (&Unit, Number),
        compounds: Option<i64>,
        result: GrowthResult,
    ) -> Result<Value> {
        if periods.is_zero() {
            bail!("division by zero");
        }
        let length_in = |unit: &Unit| match &time {
            Value::Duration(d) => Ok(self.duration_in(d, unit)?.number),
            Value::Quantity(q) if q.unit.dim() == Dim::TIME => Ok(self.convert_quantity(q, unit)?.number),
            v => bail!("expected a length of time, not {}", v.kind()),
        };
        // How many periods the time is: "5 years" are 2.5 periods of 2 years.
        let periods_in_time = || Ok::<_, Error>(length_in(period)? / periods);
        let rate = match rate {
            Value::Percent(p) => p / Number::from_i64(100),
            v => bail!("expected a percentage rate, not {}", v.kind()),
        };
        let loan = matches!(result, GrowthResult::Repayment | GrowthResult::LoanInterest);
        // Number of compounding steps and the rate of each; a loan is paid back monthly.
        let (steps, rate) = match compounds.map(Number::from_i64) {
            Some(n) => (periods_in_time()? * n, rate / n),
            None if loan => {
                let month = registry().get("mo");
                let months_per_period =
                    self.convert_quantity(&Quantity::new(Number::ONE, month.clone()), period)?.number;
                (length_in(&month)?, rate * months_per_period / periods)
            }
            None => (periods_in_time()?, rate),
        };
        let factor = (Number::ONE + rate).pow(steps);
        // What the principal turns into: grown, or paid back in equal payments, one per step.
        let multiple = match loan {
            true if rate.is_zero() => Number::ONE,
            true => steps * rate * factor / (factor - Number::ONE),
            false => factor,
        };
        let total = self.mul(principal.clone(), Value::number(multiple))?;
        match result {
            GrowthResult::Future | GrowthResult::Repayment => Ok(total),
            GrowthResult::Interest | GrowthResult::LoanInterest => self.sub(total, principal),
            GrowthResult::Present => self.div(principal, Value::number(factor)),
        }
    }

    /// Size of one unit in base units; currencies use exchange rates.
    pub fn scale(&self, id: UnitId) -> Result<Number> {
        let def = registry().def(id);
        match def.currency {
            // A cent's scale is its part of the dollar.
            Some(c) => {
                let usd = match self.rate_date.get() {
                    Some(date) => self.rates.usd_value_on(c.code, date)?,
                    None if self.rates_at_hand.get() => self.rates.usd_value_at_hand(c.code)?,
                    None => {
                        self.rates.usd_value(c.code, self.config.features.use_default_rates_for_unhandled_currencies)?
                    }
                };
                Ok(usd * def.scale)
            }
            // An em is the font size, 16 px unless the config says otherwise.
            None if matches!(def.symbol.as_str(), "em" | "rem") => {
                Ok(def.scale * self.config.base_font_size / Number::from_i64(16))
            }
            None => Ok(def.scale),
        }
    }

    pub fn convert_quantity(&self, q: &Quantity, to: &Unit) -> Result<Quantity> {
        if q.unit.same_as(to) || q.unit.is_none() {
            return Ok(Quantity::new(q.number, to.clone()));
        }
        if q.unit.dim() != to.dim() {
            if let Some(q) = self.work_time(q, to)? {
                return Ok(q);
            }
            // A turn is a cycle: "60 rpm in Hz", "1 Hz in rad/s".
            let turn = registry().get("rev");
            if q.unit.dim() == Dim::ANGULAR_VELOCITY && to.dim() == Dim::FREQUENCY {
                let cycles = self.convert_quantity(q, &to.product(&turn))?;
                return Ok(Quantity::new(cycles.number, to.clone()));
            }
            if q.unit.dim() == Dim::FREQUENCY && to.dim() == Dim::ANGULAR_VELOCITY {
                return self.convert_quantity(&Quantity::new(q.number, q.unit.product(&turn)), to);
            }
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
        #[cfg(test)]
        crate::tests::SHEET_READS.with(|reads| reads.set(reads.get() + 1));
        match line {
            LineRef::Previous => {
                let found = self.lines.iter().enumerate().rev().find_map(|(i, l)| Some((i, l.value.clone()?)));
                let mut reads = self.reads.borrow_mut();
                // Every line passed on the way up: one of them may get an answer.
                reads.lines.extend(found.as_ref().map_or(0, |&(i, _)| i)..self.lines.len());
                found.map(|(_, v)| v).ok_or_else(|| Error::new("no previous answer"))
            }
            LineRef::Line(n) => {
                self.reads.borrow_mut().lines.push(n - 1);
                self.lines
                    .get(n - 1)
                    .and_then(|l| l.value.clone())
                    .ok_or_else(|| Error::new(format!("line {n} has no answer")))
            }
            LineRef::Sum | LineRef::Average | LineRef::Stat(_) => {
                // Totals right above take the same lines: "sum", then "sum in EUR" or "average".
                let first = self.lines.len()
                    - self
                        .lines
                        .iter()
                        .rev()
                        .skip_while(|l| l.kind == LineKind::Total)
                        .take_while(|l| l.kind == LineKind::Value)
                        .count()
                    - self.lines.iter().rev().take_while(|l| l.kind == LineKind::Total).count();
                // The line that ends the block counts too: it may stop being a heading.
                self.reads.borrow_mut().lines.extend(first.saturating_sub(1)..self.lines.len());
                let tagged = |l: &&Line| self.tags.is_empty() || l.tags.iter().any(|t| self.tags.contains(t));
                let mut block: Vec<Value> = self
                    .lines
                    .iter()
                    .rev()
                    .skip_while(|l| l.kind == LineKind::Total)
                    .take_while(|l| l.kind == LineKind::Value)
                    .filter(tagged)
                    .filter_map(|l| l.value.clone())
                    .filter(|v| matches!(v, Value::Quantity(_) | Value::Percent(_) | Value::Duration(_)))
                    .collect();
                block.reverse();
                if let LineRef::Stat(func) = line {
                    return self.call(func, block);
                }
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

    pub(crate) fn convert(&self, value: Value, target: &Target, display: Display) -> Result<(Value, Display)> {
        Ok(match target {
            Target::Unit(unit) => (self.to_unit(value, unit)?, Display::Auto),
            Target::Units(units) => self.to_parts(value, units)?,
            Target::Per(period) => {
                let unit = match &value {
                    Value::Quantity(q) => q.unit.numerator().product(&period.pow(-1)),
                    v => bail!("{} is not a rate", v.kind()),
                };
                (self.to_unit(value, &unit)?, Display::Auto)
            }
            Target::Zone(zone) => (Value::Moment(self.to_zone(value, &zone.zone)?), Display::Auto),
            Target::Format(format) => self.to_format(value, *format, display)?,
            Target::Cast(unit) => match value {
                Value::Quantity(q) => (Value::Quantity(Quantity::new(q.number, unit.clone())), Display::Auto),
                Value::Percent(p) => (Value::Quantity(Quantity::new(p, unit.clone())), Display::Auto),
                v => bail!("can't cast {} to a unit", v.kind()),
            },
        })
    }

    /// An amount of food as a weight or a volume: its weight in grams or its volume in cups.
    fn weigh_or_measure(&self, value: Value, food: Food, to: Dim) -> Result<Value> {
        let Value::Quantity(q) = &value else { return Ok(value) };
        let (gram, cup) = (registry().get("g"), registry().get("cup"));
        Ok(match (q.unit.dim(), to) {
            (Dim::MASS, Dim::VOLUME) => {
                let grams = self.convert_quantity(q, &gram)?.number;
                Value::Quantity(Quantity::new(grams / food.grams_per_cup, cup))
            }
            (Dim::VOLUME, Dim::MASS) => {
                let cups = self.convert_quantity(q, &cup)?.number;
                Value::Quantity(Quantity::new(cups * food.grams_per_cup, gram))
            }
            _ => value,
        })
    }

    pub(crate) fn to_unit(&self, value: Value, unit: &Unit) -> Result<Value> {
        // Days and longer count Monday to Friday dates; hours are hours of work.
        if unit.dim() == Dim::WORKDAY {
            let dates = match &value {
                Value::Duration(d) => Some(d.clone()),
                Value::Quantity(q) if q.unit.dim() == Dim::TIME && !time::shorter_than_days(&q.unit) => {
                    Some(Duration::new(self.span_of(&value)?))
                }
                _ => None,
            };
            if let Some(d) = dates {
                return Ok(Value::Quantity(Quantity::new(self.workdays(&d)?, unit.clone())));
            }
        }
        match value {
            // "30 mpg in L/100km", "2 seconds in Hz": a unit of the inverse kind takes the reciprocal,
            // but a count per a unit other than time changes that unit: "3 per kg in lb" = 1.36/lb.
            Value::Quantity(q) if !unit.dim().is_none() && q.unit.dim() == unit.dim().pow(-1) => {
                if q.unit.dim() != Dim::FREQUENCY && q.unit.factors().iter().all(|&(_, e)| e < 0) {
                    return self.to_unit(Value::Quantity(q), &unit.pow(-1));
                }
                self.to_unit(self.div(Value::number(Number::ONE), Value::Quantity(q))?, unit)
            }
            // An exchange rate the other way round: "A$1.50/$ in USD/AUD" = $0.67/A$.
            Value::Quantity(q) if is_exchange_rate(&q.unit) && q.unit.pow(-1).same_as(unit) => {
                if q.number.is_zero() {
                    bail!("division by zero");
                }
                Ok(Value::Quantity(Quantity::new(Number::ONE / q.number, unit.clone())))
            }
            Value::Quantity(q) => Ok(Value::Quantity(self.convert_quantity(&q, unit)?)),
            Value::Duration(d) => Ok(Value::Quantity(self.duration_in(&d, unit)?)),
            // "100 frames at 24 fps in seconds"
            Value::Timecode(t) if unit.dim() == Dim::TIME => {
                let seconds = Quantity::new(Number::from_i64(t.frames) / t.fps, registry().get("s"));
                Ok(Value::Quantity(self.convert_quantity(&seconds, unit)?))
            }
            // "A4 in Hz"
            Value::Pitch(p) if unit.dim() == Dim::FREQUENCY => {
                let hz = Quantity::new(Number::from_f64(p.hz()), registry().get("Hz"));
                Ok(Value::Quantity(self.convert_quantity(&hz, unit)?))
            }
            Value::Percent(p) if unit.is_none() => Ok(Value::number(p / Number::from_i64(100))),
            v => bail!("can't convert {} to {}", v.kind(), unit_text(unit, true)),
        }
    }

    /// `in feet and inches`, `in hours and minutes`.
    fn to_parts(&self, value: Value, units: &[Unit]) -> Result<(Value, Display)> {
        // Currencies don't split: "in EUR, JPY" converts to each.
        if units.iter().any(Unit::is_money) {
            let values: Vec<Value> = units.iter().map(|u| self.to_unit(value.clone(), u)).collect::<Result<_>>()?;
            return Ok((Value::List(values), Display::Auto));
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
            // Other bases have no prefix to read back, so they are text.
            Format::Base(base) => match &value {
                Value::Quantity(q) if q.number.is_integer() && q.unit.is_none() => {
                    (Value::Text(crate::format::digits_in_base(q.number, base.into())?), Display::Auto)
                }
                _ => bail!("only whole numbers can be shown in base {base}"),
            },
            Format::Midi => match &value {
                Value::Pitch(p) => (Value::number(Number::from_i64(p.midi as i64)), Display::Auto),
                Value::Quantity(q) if q.unit.dim() == Dim::FREQUENCY => {
                    let hz = self.convert_quantity(q, &registry().get("Hz"))?.number.to_f64();
                    (Value::number(Number::from_i64(crate::value::Pitch::of_hz(hz).midi as i64)), Display::Auto)
                }
                v => bail!("only a note or a frequency has a MIDI number, not {}", v.kind()),
            },
            Format::Dms => match value {
                Value::Coordinates(c) => (Value::Coordinates(c), Display::Dms),
                Value::Quantity(q) if q.unit.is_none() || q.unit.dim() == Dim::ANGLE => {
                    let degrees = self.convert_quantity(&q, &registry().get("°"))?;
                    let parts = ["°", "arcmin", "arcsec"].map(|u| registry().get(u)).to_vec();
                    (Value::Quantity(degrees), Display::Parts(parts))
                }
                v => bail!("only an angle or coordinates have degrees, minutes and seconds, not {}", v.kind()),
            },
            Format::Megapixels => match value {
                Value::Resolution(r) => {
                    let pixels = Number::from_i64(r.width as i64 * r.height as i64);
                    (
                        Value::Text(format!("{} MP", crate::format::number(pixels / Number::pow10(6), self.config))),
                        Display::Auto,
                    )
                }
                v => bail!("only a resolution has megapixels, not {}", v.kind()),
            },
            Format::Frames => match value {
                Value::Timecode(t) => (Value::number(Number::from_i64(t.frames)), Display::Auto),
                v => bail!("only a timecode has frames, not {}", v.kind()),
            },
            Format::Pitch => match &value {
                Value::Pitch(_) => (value, Display::Auto),
                Value::Quantity(q) if q.unit.dim() == Dim::FREQUENCY => {
                    let hz = self.convert_quantity(q, &registry().get("Hz"))?.number.to_f64();
                    let pitch = crate::value::Pitch::of_hz(hz);
                    if !(0..=131).contains(&pitch.midi) {
                        bail!("no musical note is that high or low");
                    }
                    (Value::Pitch(pitch), Display::Auto)
                }
                v => bail!("only a frequency has a pitch, not {}", v.kind()),
            },
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
            Format::Number if let Value::Resolution(r) = &value => {
                (Value::number(Number::from_i64(r.width as i64 * r.height as i64)), Display::Auto)
            }
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
        match rounding {
            Rounding::Places(dp, _) if dp > MAX_DIGITS => bail!("can't round to more than {MAX_DIGITS} decimal places"),
            Rounding::Significant(n) if n > MAX_DIGITS => {
                bail!("can't round to more than {MAX_DIGITS} significant digits")
            }
            _ => {}
        }
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

/// Digits a number holds; only pi goes further.
const MAX_DIGITS: u32 = 28;

/// Pi to some places, as a number and as text with every digit.
fn pi(places: u32, dir: Direction) -> Result<(Value, String)> {
    if places > crate::pi::MAX_PLACES {
        bail!("pi goes to at most {} decimal places", crate::pi::MAX_PLACES);
    }
    let text = crate::pi::text(places, dir);
    // "3." and the 28 decimals a number holds.
    let number = Number::parse(&text[..text.len().min(30)]).expect("digits of pi");
    Ok((Value::number(number), text))
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

/// The weight or volume a conversion is to, which amounts of food take.
pub(crate) fn weight_or_volume(target: &Target) -> Option<Dim> {
    let dim = match target {
        Target::Unit(unit) => unit.dim(),
        Target::Units(units) => units.first()?.dim(),
        _ => return None,
    };
    matches!(dim, Dim::MASS | Dim::VOLUME).then_some(dim)
}

/// A currency per another: "A$1.50/$".
pub(crate) fn is_exchange_rate(unit: &Unit) -> bool {
    let money = |id: UnitId| registry().def(id).currency.is_some();
    matches!(unit.factors(), &[(a, x), (b, y)] if x * y == -1 && x.abs() == 1 && money(a) && money(b))
}
