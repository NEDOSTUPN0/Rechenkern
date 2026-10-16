//! Syntax tree of a parsed line.

use jiff::civil::Weekday;

use crate::number::Number;
use crate::units::Unit;
use crate::units::foods::Food;
use crate::zones::{Location, Place};

#[derive(Clone, Debug)]
pub enum Stmt {
    Expr(Expr),
    /// `name = expr`, `name += expr`, `name -= expr`, `name *= expr`, `name /= expr`.
    Assign {
        name: String,
        op: Option<Op>,
        expr: Expr,
    },
    /// `if cond then stmt else stmt`.
    If {
        cond: Expr,
        then: Box<Stmt>,
        otherwise: Option<Box<Stmt>>,
    },
}

impl Stmt {
    /// Some part of it is a date or a time: `today`, `3pm`, `time in Tokyo`.
    pub fn has_dates(&self) -> bool {
        let dated = |e: &Expr| matches!(e, Expr::Time(_) | Expr::InZone(..) | Expr::ZoneDiff(..) | Expr::Sky { .. });
        match self {
            Stmt::Expr(e) | Stmt::Assign { expr: e, .. } => e.contains(&dated),
            Stmt::If { cond, then, otherwise } => {
                cond.contains(&dated) || then.has_dates() || otherwise.as_ref().is_some_and(|s| s.has_dates())
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum Expr {
    Number(Number),
    Bool(bool),
    /// A value with a unit attached: `5 km`.
    WithUnit(Box<Expr>, Unit),
    /// A unit on its own, worth one unit: `km` in `km in miles`.
    BareUnit(Unit),
    Percent(Box<Expr>),
    Neg(Box<Expr>),
    Factorial(Box<Expr>),
    /// `!true`: the opposite of a boolean.
    Not(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    /// Adjacent parts added together: `5 ft 3 in`, `1 hour 30 min`.
    Composite(Vec<Expr>),
    /// A value with a remark shown only when it is the whole answer.
    Noted(Box<Expr>, &'static str),
    /// An amount of a food, which converts between weight and volume: `1 cup of flour`.
    Food(Box<Expr>, Food),
    Call(Func, Vec<Expr>),
    Var(String),
    Line(LineRef),
    Time(TimeExpr),
    /// Interprets a clock time in a zone: `3pm Tokyo`.
    InZone(Box<Expr>, Place),
    /// Time between two moments: `9am to 5pm`.
    Range(Box<Expr>, Box<Expr>),
    /// A calendar period, its first day and length: `Q3 2026`.
    Period(Box<Expr>, Box<Expr>),
    /// Difference between the UTC offsets of two zones.
    ZoneDiff(Place, Place),
    /// The sun or the moon: `sunrise in Tokyo`, `moon phase on May 1`. Without
    /// a place the sun is the local zone's city's; without a date it is today.
    Sky {
        what: Sky,
        place: Option<Location>,
        on: Option<Box<Expr>>,
    },
    Convert(Box<Expr>, Target),
    Round(Box<Expr>, Rounding),
    /// Pi to any number of decimal places: `pi to 1000 dp`, `100 digits of pi`.
    Pi(u32, Direction),
    /// `a if cond else b`; a missing branch means no answer.
    If {
        cond: Box<Expr>,
        then: Option<Box<Expr>>,
        otherwise: Option<Box<Expr>>,
    },
    /// Compound growth: `$1,000 after 3 years at 7% compounding monthly`,
    /// or a loan paid back monthly: `total repayment on $10,000 for 5 years at 5%`.
    Growth {
        principal: Box<Expr>,
        time: Box<Expr>,
        rate: Box<Expr>,
        /// The rate applies once per this time unit (a year by default)...
        period: Unit,
        /// ...compounding this many times within it: once if `None`, monthly for a loan.
        compounds: Option<i64>,
        result: GrowthResult,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrowthResult {
    /// The final amount.
    Future,
    /// Only the interest earned.
    Interest,
    /// What a future amount is worth today.
    Present,
    /// Everything paid back on a loan.
    Repayment,
    /// The interest paid on a loan.
    LoanInterest,
}

impl Expr {
    pub fn boxed(self) -> Box<Expr> {
        Box::new(self)
    }

    pub fn binary(op: Op, a: Expr, b: Expr) -> Expr {
        Expr::Binary(op, a.boxed(), b.boxed())
    }

    /// This expression or one inside it (not inside a call) matches `f`.
    pub fn contains(&self, f: &impl Fn(&Expr) -> bool) -> bool {
        f(self)
            || match self {
                Expr::Binary(_, a, b) | Expr::Range(a, b) | Expr::Period(a, b) => a.contains(f) || b.contains(f),
                Expr::Composite(parts) => parts.iter().any(|p| p.contains(f)),
                Expr::WithUnit(e, _)
                | Expr::Percent(e)
                | Expr::Neg(e)
                | Expr::Factorial(e)
                | Expr::Not(e)
                | Expr::Noted(e, _)
                | Expr::Food(e, _)
                | Expr::InZone(e, _)
                | Expr::Convert(e, _)
                | Expr::Round(e, _) => e.contains(f),
                _ => false,
            }
    }

    /// The unit of the first or last value written in it: `$` in `2 x $3`, none in the rate `$5/hour`.
    pub fn edge_unit(&self, last: bool) -> Option<&Unit> {
        match self {
            Expr::WithUnit(_, unit) => Some(unit),
            Expr::Binary(Op::Per, ..) => None,
            Expr::Binary(_, a, b) => if last { b } else { a }.edge_unit(last),
            Expr::Composite(parts) => if last { parts.last() } else { parts.first() }?.edge_unit(last),
            Expr::Neg(e) | Expr::Food(e, _) => e.edge_unit(last),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    /// Division by a bare unit that keeps a rate: `3 hours / day`.
    Per,
    Pow,
    Mod,
    /// `20% of 50`, `half of 10`.
    Of,
    /// `4 out of 10`: a share as a percentage.
    OutOf,
    /// `10% on 200`: add the percentage.
    On,
    /// `10% off 200`: subtract the percentage.
    Off,
    /// `30 hours at $30/hour`: apply a rate.
    At,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    /// `10 permutation 3`.
    Perm,
    /// `25 combination 3`.
    Comb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Func {
    Sqrt,
    Cbrt,
    Root,
    Exp,
    Ln,
    Log,
    Log2,
    Log10,
    Abs,
    Fact,
    Round,
    Ceil,
    Floor,
    Trunc,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Asinh,
    Acosh,
    Atanh,
    /// Trigonometry in degrees: `sind(90)`.
    SinD,
    CosD,
    TanD,
    AsinD,
    AcosD,
    AtanD,
    Min,
    Max,
    Gcd,
    Lcm,
    Sum,
    Average,
    Median,
    Count,
    StdDev,
    Midpoint,
    Random,
    Clamp,
    Perm,
    Comb,
    Hex,
    Bin,
    Oct,
    Int,
    /// `is 17 prime`: true or false.
    IsPrime,
}

impl Func {
    /// Takes a list of values, also without "of": `average 36, 42, 19`.
    pub fn takes_list(self) -> bool {
        use Func::*;
        matches!(self, Sum | Average | Median | Count | StdDev | Min | Max | Gcd | Lcm | Midpoint)
    }
}

/// Reference to answers of earlier lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineRef {
    /// The previous answer: `prev`, `ans`.
    Previous,
    /// `line 3` (1-based).
    Line(usize),
    /// Sum of the block of lines above.
    Sum,
    Average,
}

/// Which occurrence of a weekday or period: `next friday`, `last month`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Next,
    Last,
    This,
}

impl Which {
    /// Periods to move from the current one.
    pub fn step(self) -> i64 {
        match self {
            Which::Next => 1,
            Which::Last => -1,
            Which::This => 0,
        }
    }
}

#[derive(Clone, Debug)]
pub enum TimeExpr {
    /// Current date and time: `now`, `time`.
    Now,
    /// `today` (with a day offset for `tomorrow` and `yesterday`).
    Today(i64),
    /// A calendar date; without a year the nearest one is used.
    Date {
        year: Option<i16>,
        month: i8,
        day: i8,
    },
    Weekday(Weekday, Option<Which>),
    /// `next week`, `last month`: today moved by one period.
    Period(jiff::Unit, Which),
    /// First day of this, next or last quarter.
    Quarter(Which),
    /// First day of this, next or last week (from Monday), month or year.
    PeriodStart(jiff::Unit, Which),
    /// A time of day, optionally on a given date.
    Clock {
        hour: i8,
        minute: i8,
        second: Number,
        on: Option<Box<Expr>>,
    },
    Holiday {
        holiday: Holiday,
        year: Option<i16>,
    },
    /// A date or holiday in this, next or last year: `April 21 next year`.
    InYear(Box<TimeExpr>, Which),
    /// ISO 8601 timestamp text.
    Iso(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sky {
    Sunrise,
    Sunset,
    /// Hours from sunrise to sunset.
    Daylight,
    MoonPhase,
    /// Day of the lunar month, 1 on the day of the new moon.
    LunarDay,
}

impl Sky {
    pub fn is_sun(self) -> bool {
        matches!(self, Sky::Sunrise | Sky::Sunset | Sky::Daylight)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Holiday {
    NewYear,
    NewYearsEve,
    Valentines,
    Easter,
    GoodFriday,
    HolySaturday,
    EasterMonday,
    OrthodoxEaster,
    OrthodoxGoodFriday,
    Halloween,
    Thanksgiving,
    BlackFriday,
    ChristmasEve,
    Christmas,
    BoxingDay,
    OrthodoxChristmas,
    ChineseNewYear,
    ChineseNewYearsEve,
}

/// What `in`, `to` or `as` converts into.
#[derive(Clone, Debug)]
pub enum Target {
    Unit(Unit),
    /// Several units at once: `in feet and inches`.
    Units(Vec<Unit>),
    /// The same rate over another period: `$1,000/month in per week`.
    Per(Unit),
    Zone(Place),
    Format(Format),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Hex,
    Binary,
    Octal,
    /// Digits in another base: `255 in base 3`.
    Base(u8),
    Decimal,
    Scientific,
    Fraction,
    /// Strip the unit: `$100 as number`.
    Number,
    Percent,
    Multiplier,
    Timespan,
    Laptime,
    Timestamp,
    Date,
    Iso,
    Weekday,
    WeekNumber,
    DayOfYear,
    DayOfMonth,
    /// The musical note nearest a frequency: `440 Hz as pitch` = A4.
    Pitch,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rounding {
    /// Decimal places.
    Places(u32, Direction),
    /// Significant figures.
    Significant(u32),
    /// Nearest multiple: `to nearest 10`.
    Multiple(Number, Direction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Nearest,
    Up,
    Down,
}
