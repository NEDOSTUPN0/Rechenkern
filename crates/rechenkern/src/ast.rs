//! Syntax tree of a parsed line.

use jiff::civil::Weekday;
use jiff::tz::TimeZone;

use crate::number::Number;
use crate::units::Unit;

#[derive(Clone, Debug)]
pub enum Stmt {
    Expr(Expr),
    /// `name = expr`, `name += expr`, `name -= expr`.
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
    Binary(Op, Box<Expr>, Box<Expr>),
    /// Adjacent parts added together: `5 ft 3 in`, `1 hour 30 min`.
    Composite(Vec<Expr>),
    Call(Func, Vec<Expr>),
    Var(String),
    Line(LineRef),
    Time(TimeExpr),
    /// Interprets a clock time in a zone: `3pm Tokyo`.
    InZone(Box<Expr>, TimeZone),
    /// Time between two moments: `9am to 5pm`.
    Range(Box<Expr>, Box<Expr>),
    /// Difference between the UTC offsets of two zones.
    ZoneDiff(TimeZone, TimeZone),
    Convert(Box<Expr>, Target),
    Round(Box<Expr>, Rounding),
    /// `a if cond else b`; a missing branch means no answer.
    If {
        cond: Box<Expr>,
        then: Option<Box<Expr>>,
        otherwise: Option<Box<Expr>>,
    },
    /// Compound growth: `$1,000 after 3 years at 7% compounding monthly`.
    Growth {
        principal: Box<Expr>,
        time: Box<Expr>,
        rate: Box<Expr>,
        /// The rate applies once per this time unit (a year by default)...
        period: Unit,
        /// ...compounding this many times within it.
        compounds: i64,
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
}

impl Expr {
    pub fn boxed(self) -> Box<Expr> {
        Box::new(self)
    }

    pub fn binary(op: Op, a: Expr, b: Expr) -> Expr {
        Expr::Binary(op, a.boxed(), b.boxed())
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
    /// ISO 8601 timestamp text.
    Iso(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Holiday {
    NewYear,
    NewYearsEve,
    Valentines,
    Easter,
    GoodFriday,
    EasterMonday,
    OrthodoxEaster,
    Halloween,
    Thanksgiving,
    BlackFriday,
    ChristmasEve,
    Christmas,
    BoxingDay,
    OrthodoxChristmas,
}

/// What `in`, `to` or `as` converts into.
#[derive(Clone, Debug)]
pub enum Target {
    Unit(Unit),
    /// Several units at once: `in feet and inches`.
    Units(Vec<Unit>),
    Zone(TimeZone),
    Format(Format),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Hex,
    Binary,
    Octal,
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
