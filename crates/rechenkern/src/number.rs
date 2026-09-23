//! Numbers: exact decimals with a float fallback for huge or tiny values.

use std::cmp::Ordering;
use std::fmt;
use std::ops::{Add, Div, Mul, Neg, Sub};
use std::str::FromStr;

use rust_decimal::RoundingStrategy;
use rust_decimal::prelude::*;

/// A real number.
///
/// Values stay exact decimals (up to 28 significant digits) whenever they fit,
/// so `0.1 + 0.2` is exactly `0.3`. Anything outside the decimal range falls
/// back to `f64`.
#[derive(Clone, Copy, Debug)]
pub enum Number {
    Exact(Decimal),
    Float(f64),
}

use Number::{Exact, Float};

impl Number {
    pub const ZERO: Number = Exact(Decimal::ZERO);
    pub const ONE: Number = Exact(Decimal::ONE);

    pub fn from_i64(n: i64) -> Number {
        Exact(Decimal::from(n))
    }

    /// Converts a float, preferring an exact decimal when it fits.
    pub fn from_f64(x: f64) -> Number {
        if x == 0.0 {
            return Self::ZERO;
        }
        // Decimals keep only 28 fractional digits, so tiny values lose precision.
        if x.is_finite() && (1e-12..7.9e27).contains(&x.abs()) {
            // `Display` prints the shortest string that round-trips.
            if let Ok(d) = Decimal::from_str_exact(&x.to_string()) {
                return Exact(d);
            }
        }
        Float(x)
    }

    /// Parses a literal such as `1234.5` or `1.5e-3`.
    pub fn parse(s: &str) -> Option<Number> {
        let exact =
            if s.contains(['e', 'E']) { Decimal::from_scientific(s).ok() } else { Decimal::from_str_exact(s).ok() };
        match exact {
            Some(d) => Some(Exact(d)),
            None => s.parse::<f64>().ok().filter(|f| f.is_finite()).map(Float),
        }
    }

    /// `10^exp` as an exact number where possible.
    pub fn pow10(exp: i32) -> Number {
        Number::from_i64(10).powi(exp as i64)
    }

    /// Ratio of two integers, e.g. `ratio(5, 9)`.
    pub fn ratio(a: i64, b: i64) -> Number {
        Number::from_i64(a) / Number::from_i64(b)
    }

    pub fn to_f64(self) -> f64 {
        match self {
            Exact(d) => d.to_f64().unwrap_or(f64::NAN),
            Float(f) => f,
        }
    }

    pub fn as_decimal(self) -> Option<Decimal> {
        match self {
            Exact(d) => Some(d),
            Float(_) => None,
        }
    }

    pub fn is_zero(self) -> bool {
        match self {
            Exact(d) => d.is_zero(),
            Float(f) => f == 0.0,
        }
    }

    pub fn is_negative(self) -> bool {
        match self {
            Exact(d) => d.is_sign_negative() && !d.is_zero(),
            Float(f) => f < 0.0,
        }
    }

    pub fn is_finite(self) -> bool {
        match self {
            Exact(_) => true,
            Float(f) => f.is_finite(),
        }
    }

    pub fn is_integer(self) -> bool {
        match self {
            Exact(d) => d.fract().is_zero(),
            Float(f) => f.fract() == 0.0,
        }
    }

    /// The value as an integer, if it is one and fits.
    pub fn to_i64(self) -> Option<i64> {
        if !self.is_integer() {
            return None;
        }
        match self {
            Exact(d) => d.to_i64(),
            Float(f) if f.abs() < 9.2e18 => Some(f as i64),
            Float(_) => None,
        }
    }

    pub fn to_i128(self) -> Option<i128> {
        if !self.is_integer() {
            return None;
        }
        match self {
            Exact(d) => d.to_i128(),
            Float(f) if f.abs() < 1.7e38 => Some(f as i128),
            Float(_) => None,
        }
    }

    pub fn abs(self) -> Number {
        match self {
            Exact(d) => Exact(d.abs()),
            Float(f) => Float(f.abs()),
        }
    }

    pub fn signum(self) -> i32 {
        match self.partial_cmp(&Self::ZERO) {
            Some(Ordering::Less) => -1,
            Some(Ordering::Greater) => 1,
            _ => 0,
        }
    }

    pub fn floor(self) -> Number {
        self.map(|d| d.floor(), f64::floor)
    }

    pub fn ceil(self) -> Number {
        self.map(|d| d.ceil(), f64::ceil)
    }

    pub fn trunc(self) -> Number {
        self.map(|d| d.trunc(), f64::trunc)
    }

    pub fn fract(self) -> Number {
        self - self.trunc()
    }

    /// Rounds half away from zero to `dp` decimal places.
    pub fn round_dp(self, dp: u32) -> Number {
        match self {
            Exact(d) => Exact(d.round_dp_with_strategy(dp.min(28), RoundingStrategy::MidpointAwayFromZero)),
            Float(f) => {
                let k = 10f64.powi(dp as i32);
                Number::from_f64((f * k).round() / k)
            }
        }
    }

    /// Rounds to `digits` significant digits.
    pub fn round_sig(self, digits: u32) -> Number {
        if self.is_zero() {
            return self;
        }
        let dp = digits as i32 - 1 - self.magnitude();
        if dp >= 0 {
            self.round_dp(dp as u32)
        } else {
            let k = Number::pow10(-dp);
            (self / k).round_dp(0) * k
        }
    }

    /// Decimal exponent: `floor(log10(|x|))`.
    pub fn magnitude(self) -> i32 {
        match self.abs() {
            Exact(d) if !d.is_zero() => {
                // Digits of the unscaled mantissa minus the scale.
                let digits = d.mantissa().unsigned_abs().to_string().len() as i32;
                digits - 1 - d.scale() as i32
            }
            Exact(_) => 0,
            Float(f) => f.log10().floor() as i32,
        }
    }

    /// Integer power, exact when the result fits.
    pub fn powi(self, exp: i64) -> Number {
        if let Exact(base) = self
            && let Some(r) = pow_exact(base, exp.unsigned_abs())
        {
            return if exp < 0 { Self::ONE / Exact(r) } else { Exact(r) };
        }
        Number::from_f64(self.to_f64().powf(exp as f64))
    }

    pub fn pow(self, exp: Number) -> Number {
        match exp.to_i64() {
            Some(e) if e.abs() <= 4096 => self.powi(e),
            _ => Number::from_f64(self.to_f64().powf(exp.to_f64())),
        }
    }

    /// Applies an `f64` function, returning `None` for NaN or infinite results.
    pub fn apply(self, f: impl Fn(f64) -> f64) -> Option<Number> {
        let r = f(self.to_f64());
        r.is_finite().then(|| Number::from_f64(r))
    }

    pub fn checked_div(self, rhs: Number) -> Option<Number> {
        (!rhs.is_zero()).then(|| self / rhs)
    }

    pub fn checked_rem(self, rhs: Number) -> Option<Number> {
        if rhs.is_zero() {
            return None;
        }
        if let (Exact(a), Exact(b)) = (self, rhs)
            && let Some(r) = a.checked_rem(b)
        {
            return Some(Exact(r));
        }
        Some(Number::from_f64(self.to_f64() % rhs.to_f64()))
    }

    fn map(self, exact: impl Fn(Decimal) -> Decimal, float: impl Fn(f64) -> f64) -> Number {
        match self {
            Exact(d) => Exact(exact(d)),
            Float(f) => Number::from_f64(float(f)),
        }
    }

    fn binary(
        self,
        rhs: Number,
        exact: impl Fn(Decimal, Decimal) -> Option<Decimal>,
        float: impl Fn(f64, f64) -> f64,
    ) -> Number {
        if let (Exact(a), Exact(b)) = (self, rhs)
            && let Some(r) = exact(a, b)
        {
            return Exact(r);
        }
        Number::from_f64(float(self.to_f64(), rhs.to_f64()))
    }
}

/// Exponentiation by squaring; `None` on overflow.
fn pow_exact(base: Decimal, mut exp: u64) -> Option<Decimal> {
    let mut result = Decimal::ONE;
    let mut base = base;
    while exp > 0 {
        if exp & 1 == 1 {
            result = result.checked_mul(base)?;
        }
        exp >>= 1;
        if exp > 0 {
            base = base.checked_mul(base)?;
        }
    }
    Some(result)
}

impl Add for Number {
    type Output = Number;
    fn add(self, rhs: Number) -> Number {
        self.binary(rhs, |a, b| a.checked_add(b), |a, b| a + b)
    }
}

impl Sub for Number {
    type Output = Number;
    fn sub(self, rhs: Number) -> Number {
        self.binary(rhs, |a, b| a.checked_sub(b), |a, b| a - b)
    }
}

impl Mul for Number {
    type Output = Number;
    fn mul(self, rhs: Number) -> Number {
        self.binary(rhs, |a, b| a.checked_mul(b), |a, b| a * b)
    }
}

/// Division; dividing by zero yields a non-finite float, so check first.
impl Div for Number {
    type Output = Number;
    fn div(self, rhs: Number) -> Number {
        if rhs.is_zero() {
            return Float(self.to_f64() / 0.0);
        }
        self.binary(rhs, |a, b| a.checked_div(b), |a, b| a / b)
    }
}

impl Neg for Number {
    type Output = Number;
    fn neg(self) -> Number {
        match self {
            Exact(d) => Exact(-d),
            Float(f) => Float(-f),
        }
    }
}

impl From<i64> for Number {
    fn from(n: i64) -> Number {
        Number::from_i64(n)
    }
}

impl From<Decimal> for Number {
    fn from(d: Decimal) -> Number {
        Exact(d)
    }
}

impl FromStr for Number {
    type Err = ();
    fn from_str(s: &str) -> Result<Number, ()> {
        Number::parse(s).ok_or(())
    }
}

impl PartialEq for Number {
    fn eq(&self, other: &Number) -> bool {
        self.partial_cmp(other) == Some(Ordering::Equal)
    }
}

impl PartialOrd for Number {
    fn partial_cmp(&self, other: &Number) -> Option<Ordering> {
        match (self, other) {
            (Exact(a), Exact(b)) => Some(a.cmp(b)),
            _ => self.to_f64().partial_cmp(&other.to_f64()),
        }
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Exact(d) => write!(f, "{}", d.normalize()),
            Float(x) => write!(f, "{x:e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> Number {
        Number::parse(s).unwrap()
    }

    #[test]
    fn exact_arithmetic() {
        assert_eq!((n("0.1") + n("0.2")).to_string(), "0.3");
        assert_eq!(n("2").powi(64).to_string(), "18446744073709551616");
        assert_eq!((n("1") / n("4")).to_string(), "0.25");
    }

    #[test]
    fn float_fallback() {
        let big = n("2").powi(100);
        assert!(matches!(big, Float(_)));
        assert_eq!(n("1e30").magnitude(), 30);
        assert!(matches!(big / big, Exact(_)));
    }

    #[test]
    fn rounding() {
        assert_eq!(n("2.5").round_dp(0).to_string(), "3");
        assert_eq!(n("123456").round_sig(2).to_string(), "120000");
        assert_eq!(n("0.000123456").round_sig(3).to_string(), "0.000123");
        assert_eq!(n("0.05").magnitude(), -2);
    }
}
