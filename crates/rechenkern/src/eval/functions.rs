//! Built-in functions: roots, logarithms, trigonometry, statistics...

use std::cmp::Ordering;

use super::Env;
use crate::ast::Func;
use crate::config::AngleUnit;
use crate::error::{Error, Result, bail};
use crate::number::Number;
use crate::units::{Dim, Unit, registry};
use crate::value::{Quantity, Value};

/// A plain number from a value; percentages count as fractions.
fn plain(v: &Value, what: &str) -> Result<Number> {
    match v {
        Value::Quantity(q) if q.unit.is_none() => Ok(q.number),
        Value::Percent(p) => Ok(*p / Number::from_i64(100)),
        v => bail!("{what} needs a plain number, not {}", v.kind()),
    }
}

fn integer(v: &Value, what: &str) -> Result<i64> {
    plain(v, what)?.to_i64().ok_or_else(|| Error::new(format!("{what} needs a whole number")))
}

fn real(n: Option<Number>) -> Result<Value> {
    n.map(Value::number).ok_or_else(|| Error::new("result is not a real number"))
}

pub(super) fn factorial(v: Value) -> Result<Value> {
    let n = integer(&v, "factorial")?;
    if !(0..=170).contains(&n) {
        bail!("factorial needs a whole number from 0 to 170");
    }
    Ok(Value::number((2..=n).fold(Number::ONE, |acc, k| acc * Number::from_i64(k))))
}

/// Permutations (`perm`) or combinations of `k` items out of `n`.
pub(super) fn choose(n: Value, k: Value, perm: bool) -> Result<Value> {
    let (n, k) = (integer(&n, "combinations")?, integer(&k, "combinations")?);
    if k < 0 || n < 0 || k > n {
        bail!("combinations need 0 ≤ k ≤ n");
    }
    let mut result = Number::ONE;
    for i in 0..k {
        result = result * Number::from_i64(n - i);
        if !perm {
            result = result / Number::from_i64(i + 1);
        }
    }
    Ok(Value::number(result.round_dp(0)))
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a.abs() } else { gcd(b, a % b) }
}

/// Small xorshift generator seeded from the clock; good enough for dice.
fn random() -> f64 {
    use std::hash::{BuildHasher, RandomState};
    let mut x = RandomState::new().hash_one(std::time::SystemTime::now()) | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

impl Env<'_> {
    pub(super) fn call(&self, func: Func, args: Vec<Value>) -> Result<Value> {
        use Func::*;
        let arity = |n: usize| -> Result<()> {
            if args.len() != n {
                bail!("{func:?} takes {n} argument{}", if n == 1 { "" } else { "s" });
            }
            Ok(())
        };
        match func {
            Sum | Average | Median | Count | StdDev | Min | Max | Gcd | Lcm => return self.aggregate(func, args),
            Random => return self.random(&args),
            _ => {}
        }
        match func {
            Log | Clamp | Root | Midpoint | Perm | Comb => {}
            _ => arity(1)?,
        }
        let Some(x) = args.first() else { bail!("missing a value") };
        match func {
            Sqrt => self.root(x, 2),
            Cbrt => self.root(x, 3),
            Root => {
                arity(2)?;
                let n = integer(&args[0], "root")?;
                self.root(&args[1], n)
            }
            Abs | Round | Ceil | Floor | Trunc | Int => {
                let f = |n: Number| match func {
                    Abs => n.abs(),
                    Round => n.round_dp(0),
                    Ceil => n.ceil(),
                    Floor => n.floor(),
                    _ => n.trunc(),
                };
                Ok(match x {
                    Value::Quantity(q) => Value::Quantity(Quantity::new(f(q.number), q.unit.clone())),
                    Value::Percent(p) => Value::Percent(f(*p)),
                    v => bail!("can't round {}", v.kind()),
                })
            }
            Exp => real(plain(x, "exp")?.apply(f64::exp)),
            Ln => real(plain(x, "ln")?.apply(f64::ln)),
            Log2 => real(plain(x, "log2")?.apply(f64::log2)),
            Log10 => real(plain(x, "log")?.apply(f64::log10)),
            Log => {
                let n = plain(x, "log")?;
                let base = match args.get(1) {
                    Some(b) => plain(b, "log")?.to_f64(),
                    None => 10.0,
                };
                // Exact for powers: log(1000) is 3.
                let r = n.to_f64().log(base);
                real(
                    Some(Number::from_f64(if (r - r.round()).abs() < 1e-12 { r.round() } else { r }))
                        .filter(|n| n.is_finite()),
                )
            }
            Fact => factorial(x.clone()),
            Sin | Cos | Tan | SinD | CosD | TanD => {
                let degrees = matches!(func, SinD | CosD | TanD);
                let rad = self.radians(x, degrees)?;
                let r = match func {
                    Sin | SinD => rad.sin(),
                    Cos | CosD => rad.cos(),
                    _ => rad.tan(),
                };
                // sin(pi) is 0, not 1.2e-16.
                real(Some(Number::from_f64(if r.abs() < 1e-15 { 0.0 } else { r })))
            }
            Asin | Acos | Atan | AsinD | AcosD | AtanD => {
                let n = plain(x, "inverse trigonometry")?.to_f64();
                let r = match func {
                    Asin | AsinD => n.asin(),
                    Acos | AcosD => n.acos(),
                    _ => n.atan(),
                };
                if !r.is_finite() {
                    bail!("result is not a real number");
                }
                let degrees = matches!(func, AsinD | AcosD | AtanD) || self.config.angle_unit == AngleUnit::Degrees;
                Ok(if degrees {
                    Value::Quantity(Quantity::new(Number::from_f64(r.to_degrees()), registry().get("°")))
                } else {
                    Value::number(Number::from_f64(r))
                })
            }
            Sinh | Cosh | Tanh | Asinh | Acosh | Atanh => {
                let n = plain(x, "hyperbolic functions")?;
                let f = match func {
                    Sinh => f64::sinh,
                    Cosh => f64::cosh,
                    Tanh => f64::tanh,
                    Asinh => f64::asinh,
                    Acosh => f64::acosh,
                    _ => f64::atanh,
                };
                real(n.apply(f))
            }
            Midpoint => {
                arity(2)?;
                match (&args[0], &args[1]) {
                    (Value::Moment(a), Value::Moment(_)) => {
                        let half =
                            self.div(self.sub(args[1].clone(), args[0].clone())?, Value::number(Number::from_i64(2)))?;
                        Ok(Value::Moment(self.shift(a, &half, false)?))
                    }
                    _ => self.div(self.add(args[0].clone(), args[1].clone())?, Value::number(Number::from_i64(2))),
                }
            }
            Clamp => {
                arity(3)?;
                let (lo, hi) = (&args[1], &args[2]);
                if self.less(x, lo)? {
                    Ok(lo.clone())
                } else if self.less(hi, x)? {
                    Ok(hi.clone())
                } else {
                    Ok(x.clone())
                }
            }
            Perm | Comb => {
                arity(2)?;
                choose(args[0].clone(), args[1].clone(), func == Perm)
            }
            Hex | Bin | Oct => {
                let n = plain(x, "base conversion")?;
                let radix = match func {
                    Hex => 16,
                    Bin => 2,
                    _ => 8,
                };
                Ok(Value::Text(crate::format::radix(n, radix)?))
            }
            _ => unreachable!("handled above"),
        }
    }

    fn root(&self, x: &Value, n: i64) -> Result<Value> {
        let q = match x {
            Value::Quantity(q) => q.clone(),
            Value::Percent(p) => Quantity::plain(*p / Number::from_i64(100)),
            v => bail!("can't take the root of {}", v.kind()),
        };
        if n == 0 {
            bail!("root of degree 0");
        }
        let unit = if q.unit.is_none() {
            Unit::none()
        } else {
            let e = i8::try_from(n).map_err(|_| Error::new("root degree is too big"))?;
            q.unit.root(e).ok_or_else(|| Error::new("can't take that root of the unit"))?
        };
        let negative = q.number.is_negative();
        if negative && n % 2 == 0 {
            bail!("even root of a negative number");
        }
        let r = q.number.abs().to_f64().powf(1.0 / n as f64);
        // Snap near-integers so sqrt(16) is exactly 4.
        let snapped = if (r - r.round()).abs() < 1e-9 && Number::from_f64(r.round()).powi(n) == q.number.abs() {
            r.round()
        } else {
            r
        };
        let r = if negative { -snapped } else { snapped };
        Ok(Value::Quantity(Quantity::new(Number::from_f64(r), unit)))
    }

    /// Angle in radians: angle quantities convert, plain numbers use the setting.
    fn radians(&self, x: &Value, degrees: bool) -> Result<f64> {
        match x {
            Value::Quantity(q) if q.unit.dim() == Dim::ANGLE => {
                Ok(self.convert_quantity(q, &registry().get("rad"))?.number.to_f64())
            }
            Value::Quantity(q) if q.unit.is_none() => {
                let n = q.number.to_f64();
                Ok(if degrees || self.config.angle_unit == AngleUnit::Degrees { n.to_radians() } else { n })
            }
            v => bail!("trigonometry needs an angle, not {}", v.kind()),
        }
    }

    fn less(&self, a: &Value, b: &Value) -> Result<bool> {
        match self.binary(crate::ast::Op::Lt, a.clone(), b.clone())? {
            Value::Bool(r) => Ok(r),
            _ => unreachable!(),
        }
    }

    fn aggregate(&self, func: Func, args: Vec<Value>) -> Result<Value> {
        use Func::*;
        if args.is_empty() {
            bail!("{func:?} needs at least one value");
        }
        let n = Value::number(Number::from_i64(args.len() as i64));
        match func {
            Count => Ok(n),
            Sum | Average => {
                let mut values = args.into_iter();
                let first = values.next().unwrap();
                let sum = values.try_fold(first, |acc, v| self.add(acc, v))?;
                if func == Sum { Ok(sum) } else { self.div(sum, n) }
            }
            Min | Max => {
                let mut best = args[0].clone();
                for v in &args[1..] {
                    let replace = if func == Min { self.less(v, &best)? } else { self.less(&best, v)? };
                    if replace {
                        best = v.clone();
                    }
                }
                Ok(best)
            }
            Median | StdDev => {
                // Work in the first value's unit.
                let unit = match &args[0] {
                    Value::Quantity(q) => q.unit.clone(),
                    _ => Unit::none(),
                };
                let mut numbers = Vec::new();
                for v in &args {
                    match v {
                        Value::Quantity(q) => numbers.push(self.convert_quantity(q, &unit)?.number),
                        v => bail!("{func:?} needs numbers, not {}", v.kind()),
                    }
                }
                let result = if func == Median {
                    numbers.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
                    let mid = numbers.len() / 2;
                    if numbers.len() % 2 == 1 {
                        numbers[mid]
                    } else {
                        (numbers[mid - 1] + numbers[mid]) / Number::from_i64(2)
                    }
                } else {
                    if numbers.len() < 2 {
                        bail!("standard deviation needs at least two values");
                    }
                    let count = Number::from_i64(numbers.len() as i64);
                    let mean = numbers.iter().fold(Number::ZERO, |a, &b| a + b) / count;
                    let var =
                        numbers.iter().fold(Number::ZERO, |a, &b| a + (b - mean) * (b - mean)) / (count - Number::ONE);
                    Number::from_f64(var.to_f64().sqrt())
                };
                Ok(Value::Quantity(Quantity::new(result, unit)))
            }
            Gcd | Lcm => {
                let mut acc = integer(&args[0], "gcd")?;
                for v in &args[1..] {
                    let x = integer(v, "gcd")?;
                    acc = if func == Gcd {
                        gcd(acc, x)
                    } else {
                        let g = gcd(acc, x);
                        if g == 0 { 0 } else { (acc / g * x).abs() }
                    };
                }
                Ok(Value::number(Number::from_i64(acc)))
            }
            _ => unreachable!(),
        }
    }

    /// `random`, `random number between 1 and 10`.
    fn random(&self, args: &[Value]) -> Result<Value> {
        let r = random();
        match args {
            [] => Ok(Value::number(Number::from_f64(r))),
            [a, b] => {
                let (lo, hi) = (plain(a, "random")?, plain(b, "random")?);
                if lo.is_integer() && hi.is_integer() && !(lo.is_zero() && hi == Number::ONE) {
                    let span = (hi - lo).to_f64() + 1.0;
                    Ok(Value::number(lo + Number::from_f64((r * span).floor())))
                } else {
                    Ok(Value::number(lo + (hi - lo) * Number::from_f64(r)))
                }
            }
            _ => bail!("random takes no arguments or a range"),
        }
    }
}
