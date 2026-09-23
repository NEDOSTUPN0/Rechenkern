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

mod ast;
mod calculator;
mod config;
pub mod currency;
mod error;
mod eval;
mod format;
mod lexer;
mod number;
#[cfg(feature = "online")]
pub mod online;
mod parser;
mod rates;
pub mod units;
mod value;
mod zones;

pub use calculator::{Answer, Calculator};
pub use config::{AngleUnit, Config, DateOrder};
pub use error::{Error, Result};
pub use number::Number;
pub use rates::{RateProvider, RateTable};
pub use value::{Duration, Moment, MomentKind, Quantity, Value};
