//! Dates, clock times, durations and time zones.

use jiff::civil::{Date, DateTime, Weekday};
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, Unit as Cal, Zoned};

use super::Env;
use crate::ast::{Holiday, TimeExpr, Which};
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
    let millis = (rest * Number::from_i64(1000)).round_dp(0).to_i64().unwrap_or(0);
    span = set(span, Cal::Millisecond, millis)?;
    Ok(if negative { span.negate() } else { span })
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
            let millis = (count.fract() * Number::from_i64(1000)).round_dp(0).to_i64().unwrap_or(0);
            span = set(span, Cal::Millisecond, millis)?;
        } else {
            let whole = if last { count.round_dp(0) } else { count };
            span =
                set(span, *cal, get(&span, *cal) + whole.to_i64().ok_or_else(|| Error::new("duration is too long"))?)?;
        }
    }
    Ok(if negative { span.negate() } else { span })
}

fn days(n: i64) -> Span {
    Span::new().days(n)
}

impl Env<'_> {
    fn today(&self) -> Date {
        self.now.get().date()
    }

    fn local_zone(&self) -> TimeZone {
        self.now.get().time_zone().clone()
    }

    fn date_moment(&self, date: Date) -> Result<Moment> {
        Ok(Moment { time: date.to_zoned(self.local_zone())?, kind: MomentKind::Date, zoned: false, seconds: false })
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
                if let Ok(d) = Date::new(year, *month, *day) {
                    let days = today.until(d)?.get_days();
                    year += if days < -270 {
                        1
                    } else if days > 270 {
                        -1
                    } else {
                        0
                    };
                }
                match Date::new(year, *month, *day) {
                    Ok(d) => self.date_moment(d),
                    Err(_) => bail!("invalid date {month}/{day}"),
                }
            }
            TimeExpr::Weekday(weekday, which) => self.date_moment(weekday_date(today, *weekday, *which)?),
            TimeExpr::Period(unit, which) => {
                let step = match which {
                    Which::Next => 1,
                    Which::Last => -1,
                    Which::This => 0,
                };
                self.date_moment(today.checked_add(set(Span::new(), *unit, step)?)?)
            }
            TimeExpr::Clock { hour, minute, second, on } => {
                let (date, zone) = match on {
                    Some(on) => match self.eval(on)? {
                        Value::Moment(m) => (m.time.date(), m.time.time_zone().clone()),
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
                    None => {
                        let this_year = holiday_date(*holiday, today.year())?;
                        if this_year >= today { this_year } else { holiday_date(*holiday, today.year() + 1)? }
                    }
                };
                self.date_moment(date)
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
            v => bail!("{} has no time zone", v.kind()),
        }
    }

    /// "3pm in Tokyo": the same instant seen in another zone.
    pub(super) fn to_zone(&self, value: Value, zone: &TimeZone) -> Result<Moment> {
        let Value::Moment(m) = value else { bail!("{} has no time zone", value.kind()) };
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

    pub(super) fn range(&self, a: Value, b: Value) -> Result<Value> {
        match (a, b) {
            (Value::Moment(a), Value::Moment(b)) => Ok(Value::Duration(self.between(&a, &b, true)?)),
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
        let span = self.span_of(by)?;
        let span = if back { span.negate() } else { span };
        let time = m.time.checked_add(span)?;
        let sub_day = [Cal::Hour, Cal::Minute, Cal::Second, Cal::Millisecond].iter().any(|&c| get(&span, c) != 0);
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

    /// Monday to Friday days in a duration, starting today unless anchored.
    pub(super) fn workdays(&self, d: &Duration) -> Result<Number> {
        let span = d.span.abs();
        let start = d.anchor.map_or(self.today(), |a| a.date());
        let end = start.checked_add(span)?;
        let days = start.until((Cal::Day, end))?.get_days() as i64;
        let mut count = days / 7 * 5;
        let mut day = start.checked_add(Span::new().weeks(days / 7))?;
        while day < end {
            if !matches!(day.weekday(), Weekday::Saturday | Weekday::Sunday) {
                count += 1;
            }
            day = day.tomorrow()?;
        }
        Ok(Number::from_i64(count))
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

fn weekday_date(today: Date, weekday: Weekday, which: Option<Which>) -> Result<Date> {
    Ok(match which {
        None if today.weekday() == weekday => today,
        None | Some(Which::Next) => today.nth_weekday(1, weekday)?,
        Some(Which::Last) => today.nth_weekday(-1, weekday)?,
        Some(Which::This) => {
            let offset = weekday.to_monday_zero_offset() - today.weekday().to_monday_zero_offset();
            today.checked_add(days(offset as i64))?
        }
    })
}

fn holiday_date(holiday: Holiday, year: i16) -> Result<Date> {
    let d = |m: i8, d: i8| Date::new(year, m, d);
    Ok(match holiday {
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
    })
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
    fn chinese_new_year_dates() {
        assert_eq!(chinese_new_year(1900).unwrap(), Date::new(1900, 1, 31).unwrap());
        assert_eq!(chinese_new_year(2024).unwrap(), Date::new(2024, 2, 10).unwrap());
        assert_eq!(chinese_new_year(2100).unwrap(), Date::new(2100, 2, 9).unwrap());
        assert!(chinese_new_year(2101).is_err());
    }
}
