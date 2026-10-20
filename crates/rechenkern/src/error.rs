//! Error type for failed calculations.

use std::fmt;

/// What went wrong, as SoulverCore's `EvaluationError` kinds tell it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The words make no calculation: "unexpected …", "expected a value".
    Syntax,
    /// Values that don't go together: "can't convert km to kg", "can't add money and a date".
    UnitMismatch,
    DivisionByZero,
    /// The square root of a negative number, `asin(2)`.
    NotReal,
    /// Too large to write: `1e400`, `10^400`.
    TooLarge,
    /// A number out of the range a function takes: `factorial(200)`, `round to 40 dp`.
    OutOfRange,
    /// The wrong number of values for a function.
    Arity,
    /// A name nobody defined: a place, a variable, a function, a unit, a currency.
    Unknown,
    /// Data that isn't at hand: no exchange rate, no weather provider, an unknown place's sun.
    MissingData,
    /// The config turns this part of the language off.
    TurnedOff,
    /// A provider is still fetching what the line needs: calculate it again later.
    Pending,
    /// Anything else.
    Other,
}

/// Why a line could not be calculated, as a human readable message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    message: String,
    /// The words make no calculation, so the line may be text with numbers in it.
    unclear: bool,
    kind: Option<ErrorKind>,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Error {
        Error { message: message.into(), unclear: false, kind: None }
    }

    /// An error of a given kind, for hosts: `Error::of_kind(ErrorKind::Pending, "rates are on the way")`.
    pub fn of_kind(kind: ErrorKind, message: impl Into<String>) -> Error {
        Error { message: message.into(), unclear: false, kind: Some(kind) }
    }

    /// What a provider says while it fetches in the background; the line answers again later.
    pub fn pending(message: impl Into<String>) -> Error {
        Error::of_kind(ErrorKind::Pending, message)
    }

    pub(crate) fn unclear(message: impl Into<String>) -> Error {
        Error { message: message.into(), unclear: true, kind: Some(ErrorKind::Syntax) }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// What went wrong; most errors know it from their message.
    pub fn kind(&self) -> ErrorKind {
        self.kind.unwrap_or_else(|| kind_of(&self.message))
    }

    pub fn is_pending(&self) -> bool {
        self.kind() == ErrorKind::Pending
    }

    pub(crate) fn is_unclear(&self) -> bool {
        self.unclear
    }
}

/// The kind an error's message tells.
fn kind_of(message: &str) -> ErrorKind {
    let has = |words: &[&str]| words.iter().any(|w| message.contains(w));
    if has(&["division by zero"]) {
        ErrorKind::DivisionByZero
    } else if has(&["not a real number", "even root of a negative"]) {
        ErrorKind::NotReal
    } else if has(&["too large", "too long", "too big", "too many", "out of range"]) {
        ErrorKind::TooLarge
    } else if has(&["turned off"]) {
        ErrorKind::TurnedOff
    } else if has(&[" takes ", "needs at least"]) {
        ErrorKind::Arity
    } else if has(&["unknown "]) {
        ErrorKind::Unknown
    } else if has(&["no exchange rate", "needs a weather provider", "doesn't know", "don't know where", "aren't"]) {
        ErrorKind::MissingData
    } else if has(&["can't convert", "can't add", "can't combine", "can't multiply", "can't divide", "can't compare"])
        || has(&["can't subtract", "can't raise", "needs a plain number", "needs an angle", "can't show", "can't make"])
        || has(&["only a date", "expected a date", "expected a length", "expected a month, not", "is not a"])
    {
        ErrorKind::UnitMismatch
    } else if has(&["needs a whole number", " from 0 to ", "0 to 28", "has no ", "no day ", "invalid date"]) {
        ErrorKind::OutOfRange
    } else if has(&["unexpected", "expected", "missing a value", "nothing to calculate", "didn't understand"]) {
        ErrorKind::Syntax
    } else {
        ErrorKind::Other
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Returns early with a formatted [`Error`].
macro_rules! bail {
    ($($arg:tt)*) => {
        return Err($crate::error::Error::new(format!($($arg)*)))
    };
}

pub(crate) use bail;
