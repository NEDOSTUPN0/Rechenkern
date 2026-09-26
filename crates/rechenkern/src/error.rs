//! Error type for failed calculations.

use std::fmt;

/// Why a line could not be calculated, as a human readable message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    message: String,
    /// The words make no calculation, so the line may be text with numbers in it.
    unclear: bool,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Error {
        Error { message: message.into(), unclear: false }
    }

    pub(crate) fn unclear(message: impl Into<String>) -> Error {
        Error { message: message.into(), unclear: true }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn is_unclear(&self) -> bool {
        self.unclear
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
