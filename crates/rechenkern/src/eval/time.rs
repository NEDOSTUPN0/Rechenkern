//! Dates, clock times, durations and time zones.

use jiff::civil::{Date, DateTime, Weekday};
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, Unit as Cal, Zoned};

use super::Env;
use crate::ast::{Expr, Holiday, MonthDay, Op, TimeExpr, Which};
use crate::error::{Error, Result, bail};
use crate::number::Number;
use crate::units::{Dim, Unit, registry};
use crate::value::{Duration, Moment, MomentKind, Quantity, Value};

impl From<jiff::Error> for Error {
    fn from(e: jiff::Error) -> Error {
        Error::new(e.to_string())
    }
}

/// Average length of a calendar unit in seconds.
fn unit_seconds(cal: Cal) -> Number {
    match cal {
        Cal::Year => Number::from_i64(31_556_952),
        Cal::Month => Number::from_i64(2_629_746),
        Cal::Week => Number::from_i64(604_800),
        Cal::Day => Number::from_i64(86_400),
        Cal::Hour => Number::from_i64(3600),
        Cal::Minute => Number::from_i64(60),
        Cal::Second => Number::ONE,
        Cal::Millisecond => Number::pow10(-3),
        Cal::Microsecond => Number::pow10(-6),
        Cal::Nanosecond => Number::pow10(-9),
    }
}

const SPAN_UNITS: [Cal; 10] = [
    Cal::Year,
    Cal::Month,
    Cal::Week,
    Cal::Day,
    Cal::Hour,
    Cal::Minute,
    Cal::Second,
    Cal::Millisecond,
    Cal::Microsecond,
    Cal::Nanosecond,
];

/// One field of a span.
pub(crate) fn get(span: &Span, cal: Cal) -> i64 {
    match cal {
        Cal::Year => span.get_years() as i64,
        Cal::Month => span.get_months() as i64,
        Cal::Week => span.get_weeks() as i64,
        Cal::Day => span.get_days() as i64,
        Cal::Hour => span.get_hours() as i64,
        Cal::Minute => span.get_minutes(),
        Cal::Second => span.get_seconds(),
        Cal::Millisecond => span.get_milliseconds(),
        Cal::Microsecond => span.get_microseconds(),
        Cal::Nanosecond => span.get_nanoseconds(),
    }
}

fn set(span: Span, cal: Cal, n: i64) -> Result<Span> {
    Ok(match cal {
        Cal::Year => span.try_years(n)?,
        Cal::Month => span.try_months(n)?,
        Cal::Week => span.try_weeks(n)?,
        Cal::Day => span.try_days(n)?,
        Cal::Hour => span.try_hours(n)?,
        Cal::Minute => span.try_minutes(n)?,
        Cal::Second => span.try_seconds(n)?,
        Cal::Millisecond => span.try_milliseconds(n)?,
        Cal::Microsecond => span.try_microseconds(n)?,
        Cal::Nanosecond => span.try_nanoseconds(n)?,
    })
}

/// Adds spans field by field, keeping "1 month 3 days" as written.
pub(super) fn add_spans(a: Span, b: Span) -> Result<Span> {
    let fieldwise = SPAN_UNITS.iter().try_fold(Span::new(), |s, &cal| set(s, cal, get(&a, cal) + get(&b, cal)));
    match fieldwise {
        Ok(span) => Ok(span),
        // Mixed signs: fall back to exact seconds.
        Err(_) => balance(span_seconds(&a) + span_seconds(&b), Cal::Week),
    }
}

/// Seconds in a span, with average months and years.
fn span_seconds(span: &Span) -> Number {
    SPAN_UNITS.iter().fold(Number::ZERO, |acc, &cal| acc + Number::from_i64(get(span, cal)) * unit_seconds(cal))
}

/// The largest unit a span uses, capped at weeks.
pub(super) fn largest_unit(span: &Span) -> Cal {
    SPAN_UNITS
        .iter()
        .copied()
        .find(|&cal| get(span, cal) != 0)
        .map_or(Cal::Second, |cal| cal.max(Cal::Second).min(Cal::Week))
}

/// Splits seconds into weeks, days, hours, minutes and seconds up to `largest`.
pub(super) fn balance(seconds: Number, largest: Cal) -> Result<Span> {
    let negative = seconds.is_negative();
    let mut rest = seconds.abs();
    let mut span = Span::new();
    for cal in [Cal::Week, Cal::Day, Cal::Hour, Cal::Minute, Cal::Second] {
        if cal > largest {
            continue;
        }
        let size = unit_seconds(cal);
        let count = (rest / size).floor();
        rest = rest - count * size;
        span = set(span, cal, count.to_i64().ok_or_else(|| Error::new("duration is too long"))?)?;
    }
    span = with_fraction(span, rest)?;
    Ok(if negative { span.negate() } else { span })
}

/// Adds part of a second as milli-, micro- and nanoseconds.
fn with_fraction(span: Span, fraction: Number) -> Result<Span> {
    let nanos = (fraction * Number::pow10(9)).round_dp(0).to_i64().unwrap_or(0);
    let span = set(span, Cal::Millisecond, nanos / 1_000_000)?;
    let span = set(span, Cal::Microsecond, nanos / 1000 % 1000)?;
    set(span, Cal::Nanosecond, nanos % 1000)
}

/// Splits seconds over the given units: "in hours and minutes".
pub(super) fn split(seconds: Number, units: &[Unit]) -> Result<Span> {
    let mut parts: Vec<(Cal, Number)> = Vec::new();
    for u in units {
        let Some(def) = u.single().filter(|d| d.dim == Dim::TIME) else { bail!("can only split time into time units") };
        let cal = def.calendar.map_or(Cal::Second, |(c, _)| c);
        parts.push((cal, def.scale));
    }
    parts.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let negative = seconds.is_negative();
    let mut rest = seconds.abs();
    let mut span = Span::new();
    for (i, (cal, size)) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        let count = if last { rest / *size } else { (rest / *size).floor() };
        rest = rest - count.floor() * *size;
        if last && *cal == Cal::Second {
            span = set(span, Cal::Second, count.floor().to_i64().unwrap_or(0))?;
            span = with_fraction(span, count.fract())?;
        } else {
            let whole = if last { count.round_dp(0) } else { count };
            span =
                set(span, *cal, get(&span, *cal) + whole.to_i64().ok_or_else(|| Error::new("duration is too long"))?)?;
        }
    }
    Ok(if negative { span.negate() } else { span })
}

/// A date without a year, maybe moved: "March 10", "March 10 + 1 day", "Christmas", "Q3".
pub(crate) fn yearless(e: &Expr) -> bool {
    match e {
        Expr::Time(TimeExpr::Date { year: None, .. } | TimeExpr::Holiday { year: None, .. }) => true,
        Expr::Binary(Op::Add, date, _) | Expr::Period(date, _) => yearless(date),
        _ => false,
    }
}

fn days(n: i64) -> Span {
    Span::new().days(n)
}

impl Env<'_> {
    pub(super) fn today(&self) -> Date {
        self.now.get().date()
    }

    pub(super) fn local_zone(&self) -> TimeZone {
        self.now.get().time_zone().clone()
    }

    /// A day, at its start or, if the config says so, at noon.
    fn date_moment(&self, date: Date) -> Result<Moment> {
        let hour = if self.config.features.dates.noon_for_single_dates { 12 } else { 0 };
        let time = date.at(hour, 0, 0, 0).to_zoned(self.local_zone())?;
        Ok(Moment { time, kind: MomentKind::Date, zoned: false, seconds: false })
    }

    pub(super) fn moment(&self, t: &TimeExpr) -> Result<Moment> {
        let today = self.today();
        match t {
            TimeExpr::Now => {
                Ok(Moment { time: self.now.get().clone(), kind: MomentKind::Clock, zoned: false, seconds: false })
            }
            TimeExpr::Today(offset) => self.date_moment(today.checked_add(days(*offset))?),
            TimeExpr::Date { year: Some(y), month, day } => self.date_moment(Date::new(*y, *month, *day)?),
            TimeExpr::Date { year: None, month, day } => {
                // This year, unless the date is most of a year away: in December
                // "January 12" is next year's.
                let mut year = today.year();
                let future = self.config.features.dates.prefer_future_for_ambiguous_dates;
                if let Ok(d) = Date::new(year, *month, *day) {
                    let days = today.until(d)?.get_days();
                    year += if days < -270 || (future && days < 0) {
                        1
                    } else if days > 270 && !future {
                        -1
                    } else {
                        0
                    };
                }
                // February 29 is the next one.
                match (year..year + 8).find_map(|y| Date::new(y, *month, *day).ok()) {
                    Some(d) => self.date_moment(d),
                    None => bail!("invalid date {month}/{day}"),
                }
            }
            TimeExpr::InMonth(pick, month) => {
                let pick_in = |d: Date| -> Result<Date> {
                    Ok(match *pick {
                        MonthDay::Nth(n, weekday) => d
                            .first_of_month()
                            .nth_weekday_of_month(n, weekday)
                            .map_err(|_| Error::new(format!("{} has no {n}th {weekday:?}", d.strftime("%B %Y"))))?,
                        MonthDay::Day(-1) => d.last_of_month(),
                        MonthDay::Day(day) => Date::new(d.year(), d.month(), day)
                            .map_err(|_| Error::new(format!("that month has no day {day}")))?,
                    })
                };
                let date = match month {
                    Some(month) => match self.eval(month)? {
                        Value::Moment(m) => pick_in(m.time.date())?,
                        v => bail!("expected a month, not {}", v.kind()),
                    },
                    // "first monday of the month": the next one.
                    None => match pick_in(today) {
                        Ok(d) if d >= today => d,
                        _ => pick_in(today.first_of_month().checked_add(Span::new().months(1))?)?,
                    },
                };
                self.date_moment(date)
            }
            TimeExpr::Weekday(weekday, which) => {
                self.date_moment(weekday_date(today, *weekday, *which, self.config.week_start)?)
            }
            TimeExpr::Period(unit, which) => {
                self.date_moment(today.checked_add(set(Span::new(), *unit, which.step())?)?)
            }
            TimeExpr::PeriodStart(unit, which) => {
                let first = match unit {
                    Cal::Week => today.checked_sub(days(day_in_week(today.weekday(), self.config.week_start)))?,
                    Cal::Month => today.first_of_month(),
                    _ => today.first_of_year(),
                };
                self.date_moment(first.checked_add(set(Span::new(), *unit, which.step())?)?)
            }
            TimeExpr::Quarter(which) => {
                let first = today.first_of_month().checked_sub(Span::new().months((today.month() - 1) % 3))?;
                self.date_moment(first.checked_add(Span::new().months(3 * which.step()))?)
            }
            TimeExpr::Clock { hour, minute, second, on } => {
                let (date, zone) = match on {
                    Some(on) => match self.eval(on)? {
                        Value::Moment(m) => (m.time.date(), m.time.time_zone().clone()),
                        // "Q3 2027 at 3pm": its first day.
                        Value::Duration(d) if d.start().is_some() => {
                            (d.start().unwrap_or_default().date(), self.local_zone())
                        }
                        v => bail!("expected a date, not {}", v.kind()),
                    },
                    None => (today, self.local_zone()),
                };
                let whole = second.trunc().to_i64().unwrap_or(0) as i8;
                let nanos = (second.fract() * Number::pow10(9)).round_dp(0).to_i64().unwrap_or(0) as i32;
                let time = date.at(*hour, *minute, whole, nanos).to_zoned(zone)?;
                let kind = if on.is_some() { MomentKind::DateTime } else { MomentKind::Clock };
                Ok(Moment { time, kind, zoned: false, seconds: !second.is_zero() })
            }
            TimeExpr::Holiday { holiday, year } => {
                let date = match year {
                    Some(y) => holiday_date(*holiday, *y)?,
                    // The next one, counting today.
                    None => holiday_near(*holiday, today, true)?,
                };
                self.date_moment(date)
            }
            TimeExpr::InYear(date, which) => {
                let year = today.year() + which.step() as i16;
                let date = match **date {
                    TimeExpr::Date { month, day, .. } => TimeExpr::Date { year: Some(year), month, day },
                    TimeExpr::Holiday { holiday, .. } => TimeExpr::Holiday { holiday, year: Some(year) },
                    ref other => other.clone(),
                };
                self.moment(&date)
            }
            TimeExpr::Iso(text) => {
                let time = if let Ok(z) = text.parse::<Zoned>() {
                    z
                } else if let Ok(ts) = text.parse::<Timestamp>() {
                    ts.to_zoned(self.local_zone())
                } else {
                    text.parse::<DateTime>()?.to_zoned(self.local_zone())?
                };
                Ok(Moment { time, kind: MomentKind::DateTime, zoned: false, seconds: true })
            }
        }
    }

    /// "3pm Tokyo": the clock time is read in that zone.
    pub(super) fn place_in_zone(&self, value: Value, zone: &TimeZone) -> Result<Moment> {
        match value {
            Value::Moment(m) => Ok(Moment { time: m.time.datetime().to_zoned(zone.clone())?, zoned: true, ..m }),
            v => bail!("only a date or a time can be in a time zone, not {}", v.kind()),
        }
    }

    /// "3pm in Tokyo": the same instant seen in another zone.
    pub(super) fn to_zone(&self, value: Value, zone: &TimeZone) -> Result<Moment> {
        let Value::Moment(m) = value else {
            bail!("only a date or a time can be in a time zone, not {}", value.kind())
        };
        if m.kind == MomentKind::Date {
            // "date in Vancouver" is today's date over there.
            let date = if m.time.date() == self.today() {
                self.now.get().with_time_zone(zone.clone()).date()
            } else {
                m.time.date()
            };
            return Ok(Moment { time: date.to_zoned(zone.clone())?, zoned: true, ..m });
        }
        Ok(Moment { time: m.time.with_time_zone(zone.clone()), zoned: true, ..m })
    }

    /// The two ends of a range. A date without a year is read next to the other end:
    /// "Dec 1 to Jan 31" ends next year, "days since Dec 25" starts last year.
    pub(crate) fn range_ends(&self, a: &Expr, b: &Expr) -> Result<(Value, Value)> {
        // "monday to friday", "friday to friday": the end is the next such day after the start.
        if let Expr::Time(TimeExpr::Weekday(weekday, None)) = b {
            let start = self.endpoint(a)?;
            if let Value::Moment(m) = &start {
                let end = m.time.date().nth_weekday(1, *weekday)?;
                return Ok((start, Value::Moment(self.date_moment(end)?)));
            }
        }
        if yearless(b) {
            let start = self.endpoint(a)?;
            let end = match &start {
                Value::Moment(m) => self.date_near(b, m.time.date(), true)?,
                _ => self.endpoint(b)?,
            };
            return Ok((start, end));
        }
        if yearless(a) {
            let end = self.endpoint(b)?;
            let start = match &end {
                Value::Moment(m) => self.date_near(a, m.time.date(), false)?,
                _ => self.endpoint(a)?,
            };
            return Ok((start, end));
        }
        Ok((self.endpoint(a)?, self.endpoint(b)?))
    }

    /// A date without a year (maybe moved, as in "through March 10") on or after
    /// `anchor`, or on or before it.
    fn date_near(&self, e: &Expr, anchor: Date, after: bool) -> Result<Value> {
        match e {
            Expr::Time(TimeExpr::Date { year: None, month, day }) => {
                // February 29 can be years away.
                let step = if after { 1 } else { -1 };
                let date = (0..=8)
                    .filter_map(|k| Date::new(anchor.year() + step * k, *month, *day).ok())
                    .find(|&d| if after { d >= anchor } else { d <= anchor })
                    .ok_or_else(|| Error::new(format!("invalid date {month}/{day}")))?;
                Ok(Value::Moment(self.date_moment(date)?))
            }
            Expr::Time(TimeExpr::Holiday { holiday, year: None }) => {
                Ok(Value::Moment(self.date_moment(holiday_near(*holiday, anchor, after)?)?))
            }
            Expr::Binary(Op::Add, date, by) => self.add(self.date_near(date, anchor, after)?, self.eval(by)?),
            Expr::Period(first, _) => self.date_near(first, anchor, after),
            e => self.endpoint(e),
        }
    }

    /// A range endpoint; a period there is its first day: `days until Q4`.
    pub(super) fn endpoint(&self, e: &Expr) -> Result<Value> {
        match e {
            Expr::Period(first, _) => self.eval(first),
            e => self.eval(e),
        }
    }

    /// A period as the time from its first day to the next period's.
    pub(super) fn period(&self, first: &Expr, length: &Expr) -> Result<Value> {
        let first = self.eval(first)?;
        let end = self.add(first.clone(), self.eval(length)?)?;
        self.range(first, end)
    }

    /// A range, negative when its end is behind its start: `days until March 5, 2025`.
    pub(crate) fn signed_range(&self, a: Value, b: Value) -> Result<Value> {
        // Clock times alone wrap past midnight instead.
        let clocks = |a: &Moment, b: &Moment| a.kind == MomentKind::Clock && b.kind == MomentKind::Clock;
        let behind = matches!((&a, &b), (Value::Moment(a), Value::Moment(b)) if b.time < a.time && !clocks(a, b));
        Ok(match self.range(a, b)? {
            // Counted back from the later end.
            Value::Duration(d) if behind => {
                let Some(start) = d.anchor else {
                    return Ok(Value::Duration(Duration { span: d.span.negate(), ..d }));
                };
                let end = start.checked_add(d.span)?;
                let days = [Cal::Year, Cal::Month, Cal::Week, Cal::Day].iter().any(|&c| get(&d.span, c) != 0);
                let mut span = end.until((if days { Cal::Year } else { Cal::Hour }, start))?;
                if d.span.get_weeks() != 0 {
                    let n = span.get_days() as i64;
                    span = span.try_days(n % 7)?.try_weeks(n / 7)?;
                }
                Value::Duration(Duration { span, anchor: Some(end), ..d })
            }
            v => v,
        })
    }

    pub(crate) fn range(&self, a: Value, b: Value) -> Result<Value> {
        match (a, b) {
            (Value::Moment(a), Value::Moment(b)) => Ok(Value::Duration(self.between(&a, &b, true)?)),
            // Lap times and timecodes: "from 01:30:00 to 02:45:30" = 01:15:30.
            (a @ Value::Duration(_), b @ Value::Duration(_)) => self.sub(b, a),
            (Value::Timecode(a), Value::Timecode(b)) if a.fps == b.fps => {
                Ok(Value::Timecode(crate::value::Timecode { frames: b.frames - a.frames, fps: a.fps }))
            }
            (Value::Quantity(a), Value::Quantity(b)) if a.unit.is_none() && b.unit.is_none() => {
                self.sub(Value::Quantity(b), Value::Quantity(a))
            }
            (a, b) => bail!("can't make a range from {} to {}", a.kind(), b.kind()),
        }
    }

    /// Difference between two zones' UTC offsets right now.
    pub(super) fn zone_difference(&self, a: &TimeZone, b: &TimeZone) -> Result<Value> {
        let now = self.now.get().timestamp();
        let seconds = b.to_offset(now).seconds() - a.to_offset(now).seconds();
        Ok(Value::Duration(Duration::new(balance(Number::from_i64(seconds as i64), Cal::Hour)?)))
    }

    /// Moves a moment by a duration or a time quantity.
    pub(super) fn shift(&self, m: &Moment, by: &Value, back: bool) -> Result<Moment> {
        if let Value::Quantity(q) = by
            && q.unit.dim() == Dim::WORKDAY
        {
            let Some(n) = q.number.to_i64().filter(|_| q.number.is_integer() && q.unit.single().is_some()) else {
                bail!("only whole workdays can be added to a date");
            };
            let date = add_workdays(m.time.date(), if back { -n } else { n }, &self.config.holidays)?;
            return Ok(Moment { time: m.time.with().date(date).build()?, ..m.clone() });
        }
        let span = self.span_of(by)?;
        let span = if back { span.negate() } else { span };
        let time = m.time.checked_add(span)?;
        let sub_day = [Cal::Hour, Cal::Minute, Cal::Second, Cal::Millisecond, Cal::Microsecond, Cal::Nanosecond]
            .iter()
            .any(|&c| get(&span, c) != 0);
        let kind = if m.kind == MomentKind::Date && sub_day { MomentKind::DateTime } else { m.kind };
        Ok(Moment { time, kind, ..m.clone() })
    }

    /// Time between two moments. Clock times wrap past midnight when `forward`
    /// ("10pm to 2am"); otherwise the difference is always positive.
    pub(super) fn between(&self, a: &Moment, b: &Moment, forward: bool) -> Result<Duration> {
        use MomentKind::*;
        if a.kind == Date && b.kind == Date {
            let (d1, d2) = (a.time.date(), b.time.date());
            let (start, end) = if d1 <= d2 { (d1, d2) } else { (d2, d1) };
            let span = start.until((Cal::Year, end))?;
            // Days become weeks and days: "3 weeks 5 days".
            let d = span.get_days() as i64;
            let span = span.try_days(d % 7)?.try_weeks(d / 7)?;
            return Ok(Duration {
                span,
                anchor: Some(start.to_datetime(jiff::civil::Time::midnight())),
                laptime: false,
            });
        }
        // "now to midnight" shouldn't show the current seconds.
        let minutes = |m: &Moment| -> Result<Zoned> {
            if m.seconds {
                return Ok(m.time.clone());
            }
            Ok(m.time.round(jiff::ZonedRound::new().smallest(Cal::Minute).mode(jiff::RoundMode::Trunc))?)
        };
        let a = &Moment { time: minutes(a)?, ..a.clone() };
        let mut end = minutes(b)?;
        if forward && a.kind == Clock && b.kind == Clock && end < a.time {
            end = end.checked_add(days(1))?;
        }
        let (start, end) = if a.time <= end { (a.time.clone(), end) } else { (end, a.time.clone()) };
        let largest = if a.kind == Clock && b.kind == Clock { Cal::Hour } else { Cal::Year };
        let span = start.until((largest, &end))?;
        Ok(Duration { span, anchor: Some(start.datetime()), laptime: false })
    }

    /// A calendar span for date arithmetic: "1 month" stays one month.
    pub(super) fn span_of(&self, v: &Value) -> Result<Span> {
        match v {
            Value::Duration(d) => Ok(d.span),
            Value::Quantity(q) if q.unit.dim() == Dim::TIME => match q.unit.single().and_then(|d| d.calendar) {
                Some((cal, multiple)) => {
                    let total = q.number * Number::from_i64(multiple);
                    let whole = total.trunc();
                    let span =
                        set(Span::new(), cal, whole.to_i64().ok_or_else(|| Error::new("duration is too long"))?)?;
                    let rest = (total - whole) * unit_seconds(cal);
                    if rest.is_zero() { Ok(span) } else { add_spans(span, balance(rest, Cal::Day)?) }
                }
                None => balance(self.seconds(v)?, Cal::Week),
            },
            v => bail!("can't add {} to a date", v.kind()),
        }
    }

    /// Length in seconds of a duration or a time quantity.
    pub(super) fn seconds(&self, v: &Value) -> Result<Number> {
        match v {
            Value::Duration(d) => {
                let calendar = d.span.get_years() != 0 || d.span.get_months() != 0;
                match (calendar, d.anchor) {
                    (true, Some(anchor)) => Ok(Number::from_f64(d.span.total((Cal::Second, anchor))?)),
                    _ => Ok(span_seconds(&d.span)),
                }
            }
            Value::Quantity(q) if q.unit.dim() == Dim::TIME => {
                Ok(self.convert_quantity(q, &registry().get("s"))?.number)
            }
            v => bail!("expected a duration, not {}", v.kind()),
        }
    }

    /// A duration as a quantity of `unit`, exact for calendar units when anchored.
    pub(super) fn duration_in(&self, d: &Duration, unit: &Unit) -> Result<Quantity> {
        if unit.dim() != Dim::TIME {
            bail!("can't convert a duration to {}", crate::format::unit_text(unit, true));
        }
        // An empty span counts as seconds: jiff panics totalling it against a date.
        if let (Some((cal, multiple)), Some(anchor)) = (unit.single().and_then(|u| u.calendar), d.anchor)
            && cal >= Cal::Day
            && !d.span.is_zero()
        {
            let total = Number::from_f64(d.span.total((cal, anchor))?) / Number::from_i64(multiple);
            return Ok(Quantity::new(total, unit.clone()));
        }
        let seconds = Quantity::new(self.seconds(&Value::Duration(d.clone()))?, registry().get("s"));
        self.convert_quantity(&seconds, unit)
    }

    /// Workdays in a duration: Monday to Friday dates in a span of days or more,
    /// starting today unless anchored; hours of work in a shorter one.
    pub(super) fn workdays(&self, d: &Duration) -> Result<Number> {
        let span = d.span.abs();
        if [Cal::Year, Cal::Month, Cal::Week, Cal::Day].iter().all(|&c| get(&span, c) == 0) {
            let hours = self.seconds(&Value::Duration(d.clone()))?.abs() / Number::from_i64(3600);
            return Ok(hours / self.config.hours_per_workday);
        }
        let anchor = d.anchor.unwrap_or_else(|| self.today().to_datetime(jiff::civil::Time::midnight()));
        // A span counted back ("workdays until" a day behind) ends at its anchor and counts negative.
        let back = d.span.is_negative();
        let start = if back { anchor.checked_add(d.span)? } else { anchor };
        let (start, end) = (start.date(), start.checked_add(span)?.date());
        let days = start.until((Cal::Day, end))?.get_days() as i64;
        let mut count = days / 7 * 5;
        let mut day = start.checked_add(Span::new().weeks(days / 7))?;
        while day < end {
            if !is_weekend(day) {
                count += 1;
            }
            day = day.tomorrow()?;
        }
        let holidays = self.config.holidays.iter().filter(|&&h| start <= h && h < end && !is_weekend(h));
        let count = count - holidays.count() as i64;
        Ok(Number::from_i64(if back { -count } else { count }))
    }

    /// The unit with its workdays written as hours of work, and the factor for
    /// the value: `$800/workday` is $100/h. `None` without workdays.
    pub(super) fn work_hours(&self, unit: &Unit) -> Option<(Unit, Number)> {
        let workday = registry().get("workday");
        let exp = unit.dim().exponent(Dim::WORKDAY);
        if exp == 0 {
            return None;
        }
        let hours = unit.product(&workday.pow(-exp)).product(&registry().get("h").pow(exp));
        Some((hours, self.config.hours_per_workday.powi(exp as i64)))
    }

    /// Workdays and clock time meet as hours of work: `3 workdays in hours`,
    /// `$100/hour in per workday`. Days and longer don't: they count dates.
    pub(super) fn work_time(&self, q: &Quantity, to: &Unit) -> Result<Option<Quantity>> {
        let (from, factor) = self.work_hours(&q.unit).unwrap_or((q.unit.clone(), Number::ONE));
        let (target, target_factor) = self.work_hours(to).unwrap_or((to.clone(), Number::ONE));
        let has_workdays = from != q.unit || target != *to;
        if !has_workdays || from.dim() != target.dim() || !shorter_than_days(&q.unit) || !shorter_than_days(to) {
            return Ok(None);
        }
        let hours = self.convert_quantity(&Quantity::new(q.number * factor, from), &target)?;
        Ok(Some(Quantity::new(hours.number / target_factor, to.clone())))
    }

    /// Seconds (or milliseconds, for big numbers) since 1970 as a date.
    pub(super) fn timestamp_moment(&self, n: Number) -> Result<Moment> {
        let seconds = if n.abs() >= Number::pow10(11) { n / Number::from_i64(1000) } else { n };
        let nanos = (seconds * Number::pow10(9)).round_dp(0);
        let nanos = nanos.to_i128().ok_or_else(|| Error::new("timestamp is out of range"))?;
        let time = Timestamp::from_nanosecond(nanos)?.to_zoned(self.local_zone());
        Ok(Moment { time, kind: MomentKind::DateTime, zoned: false, seconds: true })
    }
}

impl Env<'_> {
    /// How far a moment is from now, as people say it: "tomorrow", "in 3 days", "2 hours ago".
    pub(super) fn relative(&self, m: &Moment) -> Result<String> {
        let now = self.now.get().with_time_zone(m.time.time_zone().clone());
        let plural = |n: i64, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
        let say = |n: i64, unit: &str| match n {
            0 => "now".to_string(),
            n if n > 0 => format!("in {}", plural(n, unit)),
            n => format!("{} ago", plural(-n, unit)),
        };
        let days = i64::from(m.time.date().since(now.date())?.get_days());
        let seconds = m.time.timestamp().as_second() - now.timestamp().as_second();
        Ok(match (m.kind, days) {
            (MomentKind::Date, 0) => "today".to_string(),
            (_, 1) if m.kind == MomentKind::Date || seconds.abs() >= 86_400 => "tomorrow".to_string(),
            (_, -1) if m.kind == MomentKind::Date || seconds.abs() >= 86_400 => "yesterday".to_string(),
            _ if days.abs() >= 2 => match days.abs() {
                d if d < 14 => say(days, "day"),
                d if d < 60 => say((days as f64 / 7.0).round() as i64, "week"),
                d if d < 730 => say((days as f64 / 30.436875).round() as i64, "month"),
                _ => say((days as f64 / 365.2425).round() as i64, "year"),
            },
            _ => match seconds.abs() {
                s if s < 60 => "now".to_string(),
                s if s < 3_600 => say(seconds / 60, "minute"),
                _ => say((seconds as f64 / 3_600.0).round() as i64, "hour"),
            },
        })
    }
}

/// Its time units are all shorter than a day: hours of work fit in workdays, days don't.
pub(super) fn shorter_than_days(unit: &Unit) -> bool {
    let day = Number::from_i64(86_400);
    unit.factors().iter().map(|&(id, _)| registry().def(id)).all(|d| d.dim != Dim::TIME || d.scale < day)
}

fn is_weekend(date: Date) -> bool {
    matches!(date.weekday(), Weekday::Saturday | Weekday::Sunday)
}

/// Moves `n` workdays from `date`, skipping weekends and holidays: Friday + 1 is Monday.
fn add_workdays(date: Date, n: i64, holidays: &[Date]) -> Result<Date> {
    let step = days(n.signum());
    let off = |day: Date| is_weekend(day) || holidays.contains(&day);
    let mut day = date;
    // From a day off, count from the workday behind it: Saturday + 1 is Monday.
    while n != 0 && off(day) {
        day = day.checked_sub(step)?;
    }
    // Whole weeks at once when only weekends are off.
    let rest = if holidays.is_empty() {
        day = day.checked_add(Span::new().try_weeks(n / 5)?)?;
        n % 5
    } else {
        n
    };
    for _ in 0..rest.abs() {
        day = day.checked_add(step)?;
        while off(day) {
            day = day.checked_add(step)?;
        }
    }
    Ok(day)
}

fn weekday_date(today: Date, weekday: Weekday, which: Option<Which>, week_start: Weekday) -> Result<Date> {
    Ok(match which {
        None if today.weekday() == weekday => today,
        None | Some(Which::Next) => today.nth_weekday(1, weekday)?,
        Some(Which::Last) => today.nth_weekday(-1, weekday)?,
        Some(Which::This) => {
            let offset = day_in_week(weekday, week_start) - day_in_week(today.weekday(), week_start);
            today.checked_add(days(offset))?
        }
    })
}

/// Days from the start of the week to `day`: Monday is 0 in weeks from Monday.
fn day_in_week(day: Weekday, week_start: Weekday) -> i64 {
    (day.to_monday_zero_offset() as i64 - week_start.to_monday_zero_offset() as i64).rem_euclid(7)
}

/// The holiday on or after `anchor`, or on or before it.
fn holiday_near(holiday: Holiday, anchor: Date, after: bool) -> Result<Date> {
    let years = [anchor.year() - 1, anchor.year(), anchor.year() + 1];
    let dates = years.iter().filter_map(|&y| holiday_dates(holiday, y).ok()).flatten();
    let found = match after {
        true => dates.filter(|&d| d >= anchor).min(),
        false => dates.filter(|&d| d <= anchor).max(),
    };
    match found {
        Some(date) => Ok(date),
        None => holiday_date(holiday, anchor.year()),
    }
}

/// The holiday in `year`; the first one if it comes twice, as Ramadan can.
fn holiday_date(holiday: Holiday, year: i16) -> Result<Date> {
    match holiday_dates(holiday, year)?.first() {
        Some(&date) => Ok(date),
        None => bail!("no such holiday in {year}"),
    }
}

/// Every day the holiday falls on in `year`.
fn holiday_dates(holiday: Holiday, year: i16) -> Result<Vec<Date>> {
    let ramadan = |end: bool| -> Result<Vec<Date>> {
        let days_in = ramadan_months(year)?.into_iter().map(|(first, length)| match end {
            true => first.checked_add(days(length - 1)),
            false => first.checked_sub(days(1)),
        });
        let dates = days_in.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(dates.into_iter().filter(|d| d.year() == year).collect())
    };
    let d = |m: i8, d: i8| Date::new(year, m, d);
    Ok(vec![match holiday {
        Holiday::NewYear => d(1, 1)?,
        Holiday::NewYearsEve => d(12, 31)?,
        Holiday::Valentines => d(2, 14)?,
        Holiday::Halloween => d(10, 31)?,
        Holiday::ChristmasEve => d(12, 24)?,
        Holiday::Christmas => d(12, 25)?,
        Holiday::BoxingDay => d(12, 26)?,
        Holiday::OrthodoxChristmas => d(1, 7)?,
        Holiday::Easter => easter(year)?,
        Holiday::GoodFriday => easter(year)?.checked_sub(days(2))?,
        Holiday::HolySaturday => easter(year)?.checked_sub(days(1))?,
        Holiday::EasterMonday => easter(year)?.checked_add(days(1))?,
        Holiday::OrthodoxEaster => orthodox_easter(year)?,
        Holiday::OrthodoxGoodFriday => orthodox_easter(year)?.checked_sub(days(2))?,
        Holiday::Thanksgiving => d(11, 1)?.nth_weekday_of_month(4, Weekday::Thursday)?,
        Holiday::BlackFriday => d(11, 1)?.nth_weekday_of_month(4, Weekday::Thursday)?.checked_add(days(1))?,
        Holiday::ChineseNewYear => chinese_new_year(year)?,
        Holiday::ChineseNewYearsEve => chinese_new_year(year)?.checked_sub(days(1))?,
        Holiday::RamadanStart => return ramadan(false),
        Holiday::RamadanEnd => return ramadan(true),
        Holiday::HanukkahStart => kislev_25(year)?.checked_sub(days(1))?,
        Holiday::HanukkahEnd => kislev_25(year)?.checked_add(days(7))?,
    }])
}

/// The first days of the months of Ramadan near `year`, and their lengths, from the
/// Umm al-Qura calendar (`data/ramadan.txt`).
fn ramadan_months(year: i16) -> Result<Vec<(Date, i64)>> {
    static TABLE: std::sync::LazyLock<Vec<(Date, i64)>> = std::sync::LazyLock::new(|| {
        include_str!("../data/ramadan.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let (date, length) = l.split_once(' ')?;
                Some((date.parse().ok()?, length.parse().ok()?))
            })
            .collect()
    });
    let near: Vec<_> = TABLE.iter().copied().filter(|(d, _)| (d.year() - year).abs() <= 1).collect();
    if near.is_empty() {
        let (first, last) = (TABLE[0].0.year(), TABLE[TABLE.len() - 1].0.year());
        bail!("Ramadan is only known from {first} to {last}");
    }
    Ok(near)
}

/// 25 Kislev, the first day of Hanukkah, in the Hebrew year that ends in the autumn of
/// the next Gregorian year (Calendrical Calculations, Reingold and Dershowitz).
fn kislev_25(year: i16) -> Result<Date> {
    let hebrew_year = year as i64 + 3761;
    let new_year = hebrew_new_year(hebrew_year);
    let year_length = hebrew_new_year(hebrew_year + 1) - new_year;
    // Heshvan has 30 days in a complete year (355 or 385 days), Kislev 29 in a deficient one.
    let heshvan = if year_length % 10 == 5 { 30 } else { 29 };
    let rata_die = new_year + 30 + heshvan + 24;
    // Day 719,163 of the common era is January 1, 1970.
    Ok(Date::new(1970, 1, 1)?.checked_add(days(rata_die - 719_163))?)
}

/// The day (counted from January 1 of year 1) that the Hebrew year begins on, 1 Tishri.
fn hebrew_new_year(year: i64) -> i64 {
    const EPOCH: i64 = -1_373_427;
    let elapsed = |y: i64| {
        let months = (235 * y - 234).div_euclid(19);
        let parts = 12_084 + 13_753 * months;
        let day = 29 * months + parts.div_euclid(25_920);
        // The new year never falls on a Sunday, Wednesday or Friday.
        if (3 * (day + 1)).rem_euclid(7) < 3 { day + 1 } else { day }
    };
    let (before, this, next) = (elapsed(year - 1), elapsed(year), elapsed(year + 1));
    // Years must be 353 to 356 or 383 to 386 days long.
    let correction = if next - this == 356 {
        2
    } else if this - before == 382 {
        1
    } else {
        0
    };
    EPOCH + this + correction
}

/// Days after January 21 that Chinese New Year falls on, from 1900 to 2100.
/// From the Chinese calendar in ICU, checked against the new moons.
#[rustfmt::skip]
const CHINESE_NEW_YEAR: [u8; 201] = [
    10, 29, 18, 8, 26, 14, 4, 23, 12, 1, 20, 9, 28, 16, 5, 24, 14, 2, 21, 11,
    30, 18, 7, 26, 15, 3, 23, 12, 2, 20, 9, 27, 16, 5, 24, 14, 3, 21, 10, 29,
    18, 6, 25, 15, 4, 23, 12, 1, 20, 8, 27, 16, 6, 24, 13, 3, 22, 10, 28, 18,
    7, 25, 15, 4, 23, 12, 0, 19, 9, 27, 16, 6, 25, 13, 2, 21, 10, 28, 17, 7,
    26, 15, 4, 23, 12, 30, 19, 8, 27, 16, 6, 25, 14, 2, 20, 10, 29, 17, 7, 26,
    15, 3, 22, 11, 1, 19, 8, 28, 17, 5, 24, 13, 2, 20, 10, 29, 18, 7, 26, 15,
    4, 22, 11, 1, 20, 8, 27, 16, 5, 23, 13, 2, 21, 10, 29, 18, 7, 25, 14, 3,
    22, 11, 1, 20, 9, 27, 16, 5, 24, 12, 2, 21, 11, 29, 18, 7, 25, 14, 3, 22,
    12, 0, 19, 8, 27, 15, 5, 24, 13, 2, 21, 10, 29, 17, 6, 25, 15, 3, 22, 12,
    1, 19, 8, 27, 16, 5, 24, 13, 3, 20, 9, 28, 17, 6, 25, 15, 4, 22, 11, 0,
    19,
];

/// Chinese New Year, which follows the moon, so it comes from a table.
fn chinese_new_year(year: i16) -> Result<Date> {
    let Some(&offset) = usize::try_from(year - 1900).ok().and_then(|i| CHINESE_NEW_YEAR.get(i)) else {
        bail!("Chinese New Year is only known from 1900 to 2100");
    };
    Ok(Date::new(year, 1, 21)?.checked_add(days(offset as i64))?)
}

/// Western Easter (anonymous Gregorian algorithm).
fn easter(year: i16) -> Result<Date> {
    let y = year as i32;
    let (a, b, c) = (y % 19, y / 100, y % 100);
    let (d, e) = (b / 4, b % 4);
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let (i, k) = (c / 4, c % 4);
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let n = h + l - 7 * m + 114;
    Ok(Date::new(year, (n / 31) as i8, (n % 31 + 1) as i8)?)
}

/// Orthodox Easter: the Julian computus shifted to the Gregorian calendar.
fn orthodox_easter(year: i16) -> Result<Date> {
    let y = year as i32;
    let (a, b, c) = (y % 4, y % 7, y % 19);
    let d = (19 * c + 15) % 30;
    let e = (2 * a + 4 * b - d + 34) % 7;
    let n = d + e + 114;
    let julian = Date::new(year, (n / 31) as i8, (n % 31 + 1) as i8)?;
    // The calendars differ by 13 days from 1900 to 2099.
    Ok(julian.checked_add(days(13))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easter_dates() {
        assert_eq!(easter(2024).unwrap(), Date::new(2024, 3, 31).unwrap());
        assert_eq!(easter(2027).unwrap(), Date::new(2027, 3, 28).unwrap());
        assert_eq!(orthodox_easter(2024).unwrap(), Date::new(2024, 5, 5).unwrap());
    }

    #[test]
    fn hanukkah_dates() {
        assert_eq!(kislev_25(2026).unwrap(), Date::new(2026, 12, 5).unwrap());
        assert_eq!(kislev_25(2027).unwrap(), Date::new(2027, 12, 25).unwrap());
        assert_eq!(kislev_25(2030).unwrap(), Date::new(2030, 12, 21).unwrap());
        assert_eq!(kislev_25(2000).unwrap(), Date::new(2000, 12, 22).unwrap());
    }

    #[test]
    fn ramadan_dates() {
        let start = |y| holiday_dates(Holiday::RamadanStart, y).unwrap();
        assert_eq!(start(2027), [Date::new(2027, 2, 7).unwrap()]);
        assert_eq!(start(2030), [Date::new(2030, 1, 4).unwrap(), Date::new(2030, 12, 25).unwrap()]);
        assert_eq!(holiday_date(Holiday::RamadanEnd, 2027).unwrap(), Date::new(2027, 3, 8).unwrap());
        assert!(ramadan_months(2100).is_err());
    }

    #[test]
    fn chinese_new_year_dates() {
        assert_eq!(chinese_new_year(1900).unwrap(), Date::new(1900, 1, 31).unwrap());
        assert_eq!(chinese_new_year(2024).unwrap(), Date::new(2024, 2, 10).unwrap());
        assert_eq!(chinese_new_year(2100).unwrap(), Date::new(2100, 2, 9).unwrap());
        assert!(chinese_new_year(2101).is_err());
    }
}
