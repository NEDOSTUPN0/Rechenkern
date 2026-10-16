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
mod format;
mod hash;
mod inflation;
mod lexer;
mod number;
#[cfg(feature = "online")]
pub mod online;
mod parser;
mod pi;
mod rates;
#[cfg(test)]
mod tests;
pub mod units;
mod value;
mod zones;

pub use alternatives::{Alternative, AlternativeKind};
pub use answer::{Answer, Form, Side};
pub use calculator::Calculator;
pub use config::{
    AngleUnit, CommentOptions, Config, ConversionRule, Converter, CurrencySymbol, DateOptions, DateOrder, Features,
    Formatting, MisplacedSeparator, Notation, ResultConversions, ScalarSystem, Scale, SymbolPosition, TimeZoneOptions,
    TimespanStyle, ZoneCodeCorrection,
};
pub use custom::{CustomPlace, CustomUnit, PlaceProvider, UnitSystem};
pub use error::{Error, Result};
pub use inflation::InflationProvider;
pub use number::Number;
pub use parser::highlight::{Highlight, TokenKind};
pub use rates::{RateProvider, RateTable};
pub use value::{Duration, Moment, MomentKind, Quantity, Value};
pub use zones::{Place, PlaceKind};
