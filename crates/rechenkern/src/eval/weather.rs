//! The weather, from the host's provider.

use std::borrow::Cow;

use super::{Display, Env};
use crate::ast::Expr;
use crate::error::{Error, Result, bail};
use crate::number::Number;
use crate::units::registry;
use crate::value::{Quantity, Value};
use crate::weather::{WeatherQuery, WeatherRequest};
use crate::zones::{self, Location, Place};

impl Env<'_> {
    /// The weather a line asks for, with the sky as a note when it asks for the weather itself.
    pub(super) fn weather(
        &self,
        what: WeatherQuery,
        place: Option<&(String, Location)>,
        on: Option<&Expr>,
    ) -> Result<(Value, Display)> {
        let Some(provider) = self.weather else { bail!("the weather needs a weather provider") };
        let (name, place) = match place {
            Some((name, location)) => (name.clone(), location.clone()),
            None => {
                let local = self.local_zone();
                let location = zones::zone_location(&local)
                    .ok_or_else(|| Error::new("don't know where you are: name a city, as in weather in Paris"))?;
                (Place::of_zone(&local).name, location)
            }
        };
        let today = self.now.get().with_time_zone(place.zone.clone()).date();
        let date = self.sky_date(on, &place)?;
        if date != today && !self.config.features.historical_weather_queries {
            bail!("the weather of other days is turned off");
        }
        let request = WeatherRequest {
            place: name,
            latitude: place.latitude,
            longitude: place.longitude,
            zone: place.zone.clone(),
            date: (date != today).then_some(date),
            query: what,
        };
        if provider.pending(&request) {
            return Err(Error::pending("the weather is on its way"));
        }
        let weather = provider.weather(&request).map_err(Error::new)?;
        let missing = || Error::new("the weather provider doesn't know that");
        let celsius = |t: Option<f64>| -> Result<Value> {
            let t = t.ok_or_else(missing)?;
            Ok(Value::Quantity(Quantity::new(Number::from_f64(t), registry().get("°C"))))
        };
        let percent =
            |p: Option<f64>| -> Result<Value> { Ok(Value::Percent(Number::from_f64(p.ok_or_else(missing)?))) };
        let amount = |v: Option<f64>, unit: &str| -> Result<Value> {
            Ok(Value::Quantity(Quantity::new(Number::from_f64(v.ok_or_else(missing)?), registry().get(unit))))
        };
        let value = match what {
            WeatherQuery::Summary | WeatherQuery::Temperature => celsius(weather.temperature)?,
            WeatherQuery::FeelsLike => celsius(weather.feels_like)?,
            WeatherQuery::DewPoint => celsius(weather.dew_point)?,
            WeatherQuery::Precipitation => amount(weather.precipitation, "mm")?,
            WeatherQuery::Rain => amount(weather.rain, "mm")?,
            WeatherQuery::Snowfall => amount(weather.snowfall, "cm")?,
            WeatherQuery::CloudCover => percent(weather.cloud_cover)?,
            WeatherQuery::Visibility => amount(weather.visibility, "km")?,
            WeatherQuery::Pressure => amount(weather.pressure.map(f64::round), "hPa")?,
            WeatherQuery::Conditions => Value::Text(weather.condition.clone().ok_or_else(missing)?),
            WeatherQuery::High => celsius(weather.high)?,
            WeatherQuery::Low => celsius(weather.low)?,
            WeatherQuery::Humidity => percent(weather.humidity)?,
            WeatherQuery::ChanceOfRain => percent(weather.chance_of_rain)?,
            WeatherQuery::WindSpeed => {
                let speed = Number::from_f64(weather.wind_speed.ok_or_else(missing)?);
                Value::Quantity(Quantity::new(speed, registry().lookup("kph").expect("kilometers per hour")))
            }
            WeatherQuery::WindDirection => {
                let degrees = Number::from_f64(weather.wind_direction.ok_or_else(missing)?.round());
                Value::Quantity(Quantity::new(degrees, registry().get("°")))
            }
            WeatherQuery::UvIndex => Value::number(Number::from_f64(weather.uv_index.ok_or_else(missing)?)),
        };
        let display = match (what, weather.condition) {
            (WeatherQuery::Summary, Some(sky)) => Display::Note(Cow::Owned(sky)),
            (WeatherQuery::Pressure, _) => match weather.pressure_change {
                Some(change) if change >= 1.0 => Display::Note(Cow::Borrowed("rising")),
                Some(change) if change <= -1.0 => Display::Note(Cow::Borrowed("falling")),
                Some(_) => Display::Note(Cow::Borrowed("steady")),
                None => Display::Auto,
            },
            (WeatherQuery::WindDirection, _) => {
                Display::Note(Cow::Borrowed(compass_point(weather.wind_direction.unwrap_or_default())))
            }
            _ => Display::Auto,
        };
        Ok((value, display))
    }
}

/// The compass point of a direction in degrees from north: "NW".
fn compass_point(degrees: f64) -> &'static str {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    POINTS[((degrees.rem_euclid(360.0) + 22.5) / 45.0) as usize % 8]
}
