//! The sun and the moon: sunrise, sunset, hours of daylight and the moon's phase.

use jiff::civil::{Date, Time};
use jiff::{RoundMode, Span, Timestamp, Unit as Cal, Zoned, ZonedRound};

use super::Env;
use crate::ast::{Expr, Sky, TimeExpr};
use crate::astro::{self, SunDay};
use crate::error::{Error, Result, bail};
use crate::number::Number;
use crate::units::registry;
use crate::value::{Duration, Moment, MomentKind, Quantity, Value};
use crate::zones::{self, Location};

impl Env<'_> {
    pub(super) fn sky(&self, what: Sky, place: Option<&Location>, on: Option<&Expr>) -> Result<Value> {
        if !what.is_sun() {
            let moon = astro::moon(self.sky_moment(on)?);
            return Ok(match what {
                Sky::MoonPhase => Value::Text(moon.phase.name().into()),
                _ => Value::number(Number::from_i64(moon.age.floor() as i64 + 1)),
            });
        }
        let zoned = place.is_some();
        let place = match place {
            Some(place) => place.clone(),
            None => zones::zone_location(&self.local_zone())
                .ok_or_else(|| Error::new("don't know where you are: name a city, as in sunrise in Paris"))?,
        };
        let date = self.sky_date(on, &place)?;
        let noon = date.to_datetime(Time::constant(12, 0, 0, 0)).to_zoned(place.zone.clone())?.timestamp();
        let day =
            astro::sun_day(noon, place.latitude, place.longitude).ok_or_else(|| Error::new("date out of range"))?;
        let (rise, set) = match day {
            SunDay::Rises { rise, set } => (to_minute(rise, &place)?, to_minute(set, &place)?),
            SunDay::AlwaysUp if what == Sky::Daylight => return Ok(hours(24)),
            SunDay::AlwaysDown if what == Sky::Daylight => return Ok(hours(0)),
            SunDay::AlwaysUp => bail!("polar day: the sun doesn't set there on {}", date.strftime("%a, %-d %b %Y")),
            SunDay::AlwaysDown => {
                bail!("polar night: the sun doesn't rise there on {}", date.strftime("%a, %-d %b %Y"))
            }
        };
        // Minutes, as the sun's times are shown: daylight is sunset minus sunrise.
        let kind = if on.is_some() { MomentKind::DateTime } else { MomentKind::Clock };
        let moment = |time: Zoned| Value::Moment(Moment { time, kind, zoned, seconds: false });
        Ok(match what {
            Sky::Sunrise => moment(rise),
            Sky::Sunset => moment(set),
            _ => {
                let span = rise.until((Cal::Hour, &set))?;
                Value::Duration(Duration { span, anchor: Some(rise.datetime()), laptime: false })
            }
        })
    }

    /// The day at the place: today there, a day counted from it ("tomorrow",
    /// "in 2 days"), or the date asked for.
    pub(super) fn sky_date(&self, on: Option<&Expr>, place: &Location) -> Result<Date> {
        let today = self.now.get().with_time_zone(place.zone.clone()).date();
        let Some(on) = on else { return Ok(today) };
        let date = match self.eval(on)? {
            Value::Moment(m) => m.time.date(),
            v => bail!("expected a date, not {}", v.kind()),
        };
        let from_today = on.contains(&|e| matches!(e, Expr::Time(TimeExpr::Today(_) | TimeExpr::Now)))
            && !on.contains(&|e| matches!(e, Expr::Time(t) if !matches!(t, TimeExpr::Today(_) | TimeExpr::Now)));
        if !from_today {
            return Ok(date);
        }
        let days = self.today().until((Cal::Day, date))?.get_days();
        Ok(today.checked_add(Span::new().days(days))?)
    }

    /// The moment the moon is asked for: now, a time, or noon on a date.
    fn sky_moment(&self, on: Option<&Expr>) -> Result<Timestamp> {
        let Some(on) = on else { return Ok(self.now.get().timestamp()) };
        match self.eval(on)? {
            Value::Moment(m) if m.kind == MomentKind::Date => {
                Ok(m.time.with().time(Time::constant(12, 0, 0, 0)).build()?.timestamp())
            }
            Value::Moment(m) => Ok(m.time.timestamp()),
            v => bail!("expected a date, not {}", v.kind()),
        }
    }
}

/// A time of the sun at the place, to the nearest minute.
fn to_minute(t: Timestamp, place: &Location) -> Result<Zoned> {
    let round = ZonedRound::new().smallest(Cal::Minute).mode(RoundMode::HalfExpand);
    Ok(t.to_zoned(place.zone.clone()).round(round)?)
}

fn hours(n: i64) -> Value {
    Value::Quantity(Quantity::new(Number::from_i64(n), registry().get("h")))
}
