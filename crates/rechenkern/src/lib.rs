//! Rechenkern is a natural language calculator engine.
//!
//! It reads lines like `$20 for lunch + 15% tip`, `5 km in miles`,
//! `time in Tokyo` or `days until christmas` and answers them.
//!
//! ```
//! use rechenkern::Calculator;
//!
//! let mut calc = Calculator::new();
//! let answer = calc.calculate("3 hours 30 minutes in minutes").unwrap().unwrap();
//! assert_eq!(answer.to_string(), "210 minutes");
//! ```

mod alternatives;
mod answer;
mod ast;
mod astro;
mod calculator;
mod config;
pub mod currency;
mod custom;
mod error;
mod eval;
mod find;
mod format;
mod hash;
mod income_tax;
mod inflation;
mod json;
mod lexer;
mod number;
#[cfg(feature = "online")]
pub mod online;
mod parser;
mod pi;
mod rates;
mod sheet;
#[cfg(test)]
mod tests;
pub mod units;
mod value;
mod weather;
mod zones;

pub use alternatives::{Alternative, AlternativeKind};
pub use answer::{Answer, CustomCall, DisplayKind, Form, Side};
pub use calculator::Calculator;
pub use config::{
    AngleUnit, CommentOptions, Config, ConversionRule, Converter, CurrencySymbol, DateOptions, DateOrder, Features,
    Formatting, MisplacedSeparator, Notation, ResultConversions, ScalarSystem, Scale, SymbolPosition, TimeZoneOptions,
    TimespanStyle, ZoneCodeCorrection,
};
pub use custom::{CustomFunction, CustomPlace, CustomType, CustomUnit, FunctionHandler, PlaceProvider, UnitSystem};
pub use error::{Error, ErrorKind, Result};
pub use find::{DataKind, Found, Scanner};
pub use inflation::InflationProvider;
pub use number::Number;
pub use parser::highlight::{Highlight, TokenKind};
pub use rates::{RateProvider, RateTable, bundled_rates};
pub use sheet::{
    Alignment, Behaviour, Checkpoint, Color, Definitions, Delta, HtmlColors, HtmlOptions, LineType, ObserverId,
    Portion, Sheet, SheetLine, Statistic, StatisticOptions, TextOptions, TextStyle,
};
pub use value::{Cadence, Coordinates, Duration, Moment, MomentKind, Pitch, Quantity, Resolution, Timecode, Value};
pub use weather::{Weather, WeatherProvider, WeatherQuery, WeatherRequest, condition as weather_condition};
pub use zones::{
    Airport, CityEntry as City, Country, Location, Place, PlaceInfo, PlaceKind, airports, airports_near,
    all_cities as cities, countries, country, country_of_zone,
};
