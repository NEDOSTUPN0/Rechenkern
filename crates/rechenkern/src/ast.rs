//! Syntax tree of a parsed line.

use jiff::civil::Weekday;

use crate::number::Number;
use crate::units::Unit;
use crate::units::foods::Food;
use crate::weather::WeatherQuery;
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
    /// `area(w, h) = w * h`: a function for later lines.
    Function {
        name: String,
        params: Vec<String>,
        body: Expr,
    },
    /// `if cond then stmt else stmt`.
    If {
        cond: Expr,
        then: Box<Stmt>,
        otherwise: Option<Box<Stmt>>,
    },
}

/// A function defined in a line: its parameters and what it works out.
#[derive(Clone, Debug)]
pub struct UserFunction {
    pub params: Vec<String>,
    pub body: Expr,
}

impl Stmt {
    /// Some part of it is a date or a time: `today`, `3pm`, `time in Tokyo`.
    pub fn has_dates(&self) -> bool {
        let dated = |e: &Expr| matches!(e, Expr::Time(_) | Expr::InZone(..) | Expr::ZoneDiff(..) | Expr::Sky { .. });
        match self {
            Stmt::Expr(e) | Stmt::Assign { expr: e, .. } | Stmt::Function { body: e, .. } => e.contains(&dated),
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
    /// A count of a food: `1 stick of butter` weighs 113 g, `2 eggs` are 2 but weigh 100 g
    /// in a conversion. `piece` is a word for the piece ("stick"), which makes it a weight.
    Counted {
        count: Box<Expr>,
        grams: Number,
        food: Food,
        piece: bool,
    },
    Call(Func, Vec<Expr>),
    /// A call of a function defined in a line: `area(3, 4)`.
    Apply(String, Vec<Expr>),
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
    /// The weather from the host's provider: `weather in Tokyo`, `humidity in London tomorrow`.
    /// Without a place it is the local zone's city's; without a date it is now.
    Weather {
        what: WeatherQuery,
        place: Option<(String, Location)>,
        on: Option<Box<Expr>>,
    },
    /// The sun or the moon: `sunrise in Tokyo`, `moon phase on May 1`. Without
    /// a place the sun is the local zone's city's; without a date it is today.
    Sky {
        what: Sky,
        place: Option<Location>,
        on: Option<Box<Expr>>,
    },
    /// A time that repeats: `every 2 weeks until Dec 31`.
    Cadence {
        every: Box<Expr>,
        start: Option<Box<Expr>>,
        until: Option<Box<Expr>>,
    },
    /// A price and the sales tax in it: `8% vat in $108`, `$108 before 8% tax`.
    SalesTax {
        amount: Box<Expr>,
        rate: Box<Expr>,
        part: TaxPart,
    },
    /// Minutes for each unit of a distance: `10 km in 50 minutes` is 5:00 min/km.
    Pace(Box<Expr>, Box<Expr>),
    /// A place on Earth: `coordinates of Tokyo`.
    Coordinates(f64, f64),
    /// A size in pixels: `1920x1080`.
    Resolution(u32, u32),
    Convert(Box<Expr>, Target),
    /// A conversion with the exchange rates of a day: `$100 in EUR on Jan 1, 2020`.
    RatesOn(Box<Expr>, Box<Expr>),
    Round(Box<Expr>, Rounding),
    /// Pi to any number of decimal places: `pi to 1000 dp`, `100 digits of pi`.
    Pi(u32, Direction),
    /// `a if cond else b`; a missing branch means no answer.
    If {
        cond: Box<Expr>,
        then: Option<Box<Expr>>,
        otherwise: Option<Box<Expr>>,
    },
    /// An amount of money in prices of another year: `$1,000 in 1990 dollars`. A year
    /// that is `None` is now; with a rate, prices grow by it every year.
    Inflation {
        amount: Box<Expr>,
        from: Option<i16>,
        to: Option<i16>,
        rate: Option<Box<Expr>>,
    },
    /// One of the host's phrases, by its index in `Config::custom_functions`.
    Custom {
        function: usize,
        args: Vec<CustomArg>,
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

/// What a sales tax phrase asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaxPart {
    /// The tax in a price.
    Tax,
    /// The price without the tax.
    Net,
    /// The price with the tax.
    Gross,
}

/// What a placeholder of a host's phrase matched.
#[derive(Clone, Debug)]
pub enum CustomArg {
    /// A value, and the kind the placeholder asks for.
    Expr(Expr, crate::custom::SlotKind),
    Unit(Unit),
    Text(String),
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
                | Expr::Round(e, _)
                | Expr::Counted { count: e, .. } => e.contains(f),
                Expr::RatesOn(e, date) => e.contains(f) || date.contains(f),
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
    /// The MIDI number of a note: `A4 as midi` = 69.
    Midi,
    /// Degrees, minutes and seconds of an angle or coordinates: 40° 26′ 46″.
    Dms,
    /// Millions of pixels of a resolution.
    Megapixels,
    /// Frames of a timecode.
    Frames,
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
