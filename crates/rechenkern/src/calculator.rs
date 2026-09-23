//! The calculator: evaluates lines one after another, like a notepad.

use std::collections::HashMap;
use std::fmt;

use crate::ast::{Expr, LineRef, Stmt};
use crate::config::Config;
use crate::error::Result;
use crate::eval::{Env, Line, LineKind};
use crate::format;
use crate::parser::{self, Scope};
use crate::rates::{RateProvider, RateTable, Rates};
use crate::value::Value;

/// The result of a line.
#[derive(Clone, Debug)]
pub struct Answer {
    value: Value,
    text: String,
}

impl Answer {
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// The formatted answer, e.g. "€9.20".
    pub fn text(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for Answer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// Evaluates natural language math.
///
/// Lines are remembered, so later lines can use variables (`rent = $1500`),
/// earlier answers (`prev`, `line 2`) and totals (`sum`).
///
/// ```
/// let mut calc = rechenkern::Calculator::new();
/// assert_eq!(calc.calculate("20% of 50 km").unwrap().unwrap().text(), "10 km");
/// ```
pub struct Calculator {
    config: Config,
    vars: HashMap<String, Value>,
    lines: Vec<Line>,
    rates: Rates,
}

impl Default for Calculator {
    fn default() -> Calculator {
        Calculator::new()
    }
}

impl Calculator {
    pub fn new() -> Calculator {
        Calculator::with_config(Config::default())
    }

    pub fn with_config(config: Config) -> Calculator {
        Calculator { config, vars: HashMap::new(), lines: Vec::new(), rates: Rates::default() }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    /// Where exchange rates come from; asked once, on the first conversion.
    pub fn set_rate_provider(&mut self, provider: impl RateProvider + 'static) {
        self.rates = Rates::with_provider(Box::new(provider));
    }

    pub fn set_rates(&mut self, table: RateTable) {
        self.rates = Rates::with_table(table);
    }

    /// Exchange rates, loading them if needed.
    pub fn rates(&self) -> Result<&RateTable> {
        self.rates.table()
    }

    pub fn variable(&self, name: &str) -> Option<&Value> {
        self.vars.get(&name.to_lowercase())
    }

    pub fn set_variable(&mut self, name: &str, value: Value) {
        self.vars.insert(name.to_lowercase(), value);
    }

    /// Forgets variables and earlier lines.
    pub fn clear(&mut self) {
        self.vars.clear();
        self.lines.clear();
    }

    /// Calculates one line. `Ok(None)` means there is nothing to calculate,
    /// like a heading, a comment or plain text.
    pub fn calculate(&mut self, line: &str) -> Result<Option<Answer>> {
        let trimmed = line.trim();
        let kind = if trimmed.is_empty() {
            LineKind::Blank
        } else if trimmed.starts_with('#') {
            LineKind::Heading
        } else if trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-') {
            LineKind::Divider
        } else {
            LineKind::Value
        };
        if kind != LineKind::Value {
            if kind == LineKind::Divider {
                self.vars.clear();
            }
            self.lines.push(Line { value: None, kind });
            return Ok(None);
        }
        let result = self.evaluate(strip_label(trimmed));
        let (value, kind) = match &result {
            Ok(Some((answer, is_total))) => {
                (Some(answer.value.clone()), if *is_total { LineKind::Total } else { LineKind::Value })
            }
            _ => (None, LineKind::Value),
        };
        self.lines.push(Line { value, kind });
        result.map(|r| r.map(|(answer, _)| answer))
    }

    /// Calculates every line of a text from scratch.
    pub fn calculate_sheet(&mut self, text: &str) -> Vec<Result<Option<Answer>>> {
        self.clear();
        text.lines().map(|line| self.calculate(line)).collect()
    }

    /// Parses and evaluates; the flag marks `sum`/`total` lines.
    fn evaluate(&mut self, line: &str) -> Result<Option<(Answer, bool)>> {
        let is_var = |name: &str| self.vars.contains_key(name);
        let scope = Scope { config: &self.config, is_var: &is_var };
        let Some(stmt) = parser::parse(line, &scope)? else { return Ok(None) };
        let env = Env {
            config: &self.config,
            now: self.config.now(),
            vars: &self.vars,
            lines: &self.lines,
            rates: &self.rates,
        };
        // Pick the branch of "if ... then ... else ...".
        let mut stmt = &stmt;
        while let Stmt::If { cond, then, otherwise } = stmt {
            let chosen = if env.truthy(&env.eval(cond)?)? { Some(then) } else { otherwise.as_ref() };
            match chosen {
                Some(s) => stmt = s,
                None => return Ok(None),
            }
        }
        let (value, display, is_total, name) = match stmt {
            Stmt::If { .. } => unreachable!("branches are resolved above"),
            Stmt::Expr(expr) => {
                let Some(expr) = env.branch(expr)? else { return Ok(None) };
                let (value, display) = env.answer(expr)?;
                (value, display, matches!(expr, Expr::Line(LineRef::Sum)), None)
            }
            Stmt::Assign { name, op, expr } => {
                let Some(expr) = env.branch(expr)? else { return Ok(None) };
                let (mut value, display) = env.answer(expr)?;
                if let Some(op) = op {
                    let old = self
                        .vars
                        .get(name)
                        .cloned()
                        .ok_or_else(|| crate::Error::new(format!("unknown variable {name}")))?;
                    value = env.binary(*op, old, value)?;
                }
                (value, display, false, Some(name.clone()))
            }
        };
        let text = format::render(&value, &display, &self.config, &env.now);
        drop(env);
        if let Some(name) = name {
            self.vars.insert(name, value.clone());
        }
        Ok(Some((Answer { value, text }, is_total)))
    }
}

/// Removes a leading "Label:" from a line.
fn strip_label(line: &str) -> &str {
    let bytes = line.as_bytes();
    for (i, c) in line.char_indices() {
        if c == '"' {
            break;
        }
        if c == ':'
            && bytes.get(i + 1).is_none_or(|b| b.is_ascii_whitespace())
            && line[..i].chars().any(char::is_alphabetic)
        {
            return &line[i + 1..];
        }
    }
    line
}
