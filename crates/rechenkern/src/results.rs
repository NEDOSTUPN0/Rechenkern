//! Arithmetic on answers outside a sheet, as SoulverCore's `ResultCollection`,
//! `ResultOperations` and `ResultPercentageCalculator`.

use crate::answer::{Answer, Parts};
use crate::ast::{Expr, Format, Func, LineRef, Op, Target};
use crate::calculator::{Calculator, Names};
use crate::error::Result;
use crate::eval::{Display, Env};
use crate::sheet::Statistic;
use crate::value::Value;

impl Calculator {
    /// A statistic of values, as a sheet works one out over its lines; `None` without values.
    pub fn statistic(&self, statistic: Statistic, values: &[Value]) -> Option<Result<Answer>> {
        let (names, lines) = (Names::default(), Vec::new());
        let env = self.env(&names, &lines);
        statistic_of(&env, statistic, values.to_vec())
    }

    /// The statistic of these answers' values.
    pub fn statistic_of_answers(&self, statistic: Statistic, answers: &[Answer]) -> Option<Result<Answer>> {
        let values: Vec<Value> = answers.iter().map(|a| a.value().clone()).collect();
        self.statistic(statistic, &values)
    }

    /// Two values put together: `a + b`, `a × b`...
    pub fn combine(&self, a: &Value, op: char, b: &Value) -> Result<Answer> {
        let op = match op {
            '+' => Op::Add,
            '-' | '−' => Op::Sub,
            '*' | '×' => Op::Mul,
            '/' | '÷' => Op::Div,
            '^' => Op::Pow,
            _ => return Err(crate::Error::new(format!("unknown operator {op}"))),
        };
        self.worked_out(|env| env.binary(op, a.clone(), b.clone()))
    }

    /// The part as a percentage of the whole: 20 of 200 is 10%.
    pub fn percentage_of(&self, part: &Value, whole: &Value) -> Result<Answer> {
        self.worked_out(|env| env.binary(Op::OutOf, part.clone(), whole.clone()))
    }

    /// How much `to` changed from `from`, as a percentage: 50 to 75 is 50%.
    pub fn percent_change(&self, from: &Value, to: &Value) -> Result<Answer> {
        self.worked_out(|env| {
            let ratio = env.binary(Op::Div, env.binary(Op::Sub, to.clone(), from.clone())?, from.clone())?;
            Ok(env.convert(ratio, &Target::Format(Format::Percent), Display::Auto)?.0)
        })
    }

    fn worked_out(&self, work: impl FnOnce(&Env) -> Result<Value>) -> Result<Answer> {
        let (names, lines) = (Names::default(), Vec::new());
        let env = self.env(&names, &lines);
        let value = work(&env)?;
        Ok(answer_of(&env, value))
    }
}

/// A statistic of values; the sheet's totals come here too.
pub(crate) fn statistic_of(env: &Env, statistic: Statistic, values: Vec<Value>) -> Option<Result<Answer>> {
    if values.is_empty() {
        return None;
    }
    let func = match statistic {
        Statistic::Total | Statistic::Reduction => Func::Sum,
        Statistic::Average => Func::Average,
        Statistic::Count => Func::Count,
        Statistic::Median => Func::Median,
        Statistic::Min => Func::Min,
        Statistic::Max => Func::Max,
        Statistic::Lcm => Func::Lcm,
        Statistic::Gcd => Func::Gcd,
        Statistic::StandardDeviation => Func::StdDev,
    };
    // "100, 20, 5": 100 - 20 - 5.
    if statistic == Statistic::Reduction {
        let mut values = values.into_iter();
        let first = values.next()?;
        let result = values.try_fold(first, |acc, v| env.sub(acc, v));
        return Some(result.map(|value| answer_of(env, value)));
    }
    Some(env.call(func, values).map(|value| answer_of(env, value)))
}

/// An answer for a value worked out outside a line.
pub(crate) fn answer_of(env: &Env, value: Value) -> Answer {
    let config = std::sync::Arc::new(env.config.clone());
    env.describe(&Expr::Line(LineRef::Sum), value, Display::Auto, Parts::default(), &config)
}
