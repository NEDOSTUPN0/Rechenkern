//! The calculator: evaluates lines one after another, like a notepad.

use std::cell::{Cell, OnceCell};
use std::collections::HashMap;
use std::sync::Arc;

use crate::alternatives::Alternative;
use crate::answer::Answer;
use crate::ast::{Expr, LineRef, Stmt};
use crate::config::{Config, Now};
use crate::custom::CustomUnits;
use crate::error::{Error, Result};
use crate::eval::{Display, Env, Line, LineKind};
use crate::inflation::InflationProvider;
use crate::parser::highlight::{Highlight, TokenKind};
use crate::parser::{self, Scope};
use crate::rates::{RateProvider, RateTable, Rates};
use crate::value::Value;

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
    config: Arc<Config>,
    /// The custom units of the config, looked up once.
    units: OnceCell<std::result::Result<CustomUnits, Error>>,
    vars: HashMap<String, Value>,
    lines: Vec<Line>,
    rates: Rates,
    inflation: Option<Box<dyn InflationProvider>>,
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
        Calculator {
            config: Arc::new(config),
            units: OnceCell::new(),
            vars: HashMap::new(),
            lines: Vec::new(),
            rates: Rates::default(),
            inflation: None,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut Config {
        self.units = OnceCell::new();
        Arc::make_mut(&mut self.config)
    }

    /// The custom units of the config.
    fn units(&self) -> Result<&CustomUnits> {
        self.units
            .get_or_init(|| CustomUnits::build(&self.config.custom_units, &self.config.formatting.symbols))
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Where exchange rates come from; asked once, on the first conversion.
    pub fn set_rate_provider(&mut self, provider: impl RateProvider + 'static) {
        self.rates = Rates::with_provider(Box::new(provider));
    }

    /// Where consumer price indexes come from, for currencies besides the US dollar or
    /// newer ones; the bundled US index is used otherwise.
    pub fn set_inflation_provider(&mut self, provider: impl InflationProvider + 'static) {
        self.inflation = Some(Box::new(provider));
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
            if kind == LineKind::Divider && self.config.features.dividers_reset_variable_scope {
                self.vars.clear();
            }
            self.lines.push(Line { value: None, kind });
            return Ok(None);
        }
        let result = self.evaluate(&trimmed[self.label_end(trimmed)..]);
        let (value, kind) = match &result {
            Ok(Some((answer, is_total))) => {
                (Some(answer.reference()), if *is_total { LineKind::Total } else { LineKind::Value })
            }
            _ => (None, LineKind::Value),
        };
        self.lines.push(Line { value, kind });
        result.map(|r| r.map(|(answer, _)| answer))
    }

    /// The answer written other ways: in hex, as a fraction, in other units, a date as ISO 8601.
    pub fn alternatives(&self, answer: &Answer) -> Vec<Alternative> {
        let env = Env {
            config: &self.config,
            now: Now::new(&self.config),
            vars: &self.vars,
            lines: &self.lines,
            rates: &self.rates,
            food_as: Default::default(),
            rate_date: Default::default(),
            inflation: self.inflation.as_deref(),
        };
        env.alternatives(answer)
    }

    /// What each part of a line is, for syntax highlighting; ranges are in bytes.
    /// Variables are the ones known now, as for the next line calculated.
    pub fn highlight(&self, line: &str) -> Vec<Highlight> {
        let start = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            return vec![Highlight { range: start..start + trimmed.len(), kind: TokenKind::Heading }];
        }
        if trimmed.is_empty() || (trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-')) {
            return Vec::new();
        }
        let body = &trimmed[self.label_end(trimmed)..];
        let mut out = Vec::new();
        let label = trimmed[..trimmed.len() - body.len()].trim_end();
        if !label.is_empty() {
            out.push(Highlight { range: start..start + label.len(), kind: TokenKind::Label });
        }
        let is_var = |name: &str| self.vars.contains_key(name);
        let empty = CustomUnits::default();
        let scope = Scope {
            config: &self.config,
            is_var: &is_var,
            units: self.units().unwrap_or(&empty),
            scaled: Cell::new(false),
        };
        out.extend(parser::highlight::highlight(body, start + trimmed.len() - body.len(), &scope));
        out
    }

    /// Calculates every line of a text from scratch.
    pub fn calculate_sheet(&mut self, text: &str) -> Vec<Result<Option<Answer>>> {
        self.clear();
        text.lines().map(|line| self.calculate(line)).collect()
    }

    /// Where the expression starts after a label: "Rent: $1,500", or "Coffee - $5" if the
    /// config reads dashes as labels.
    fn label_end(&self, line: &str) -> usize {
        let features = &self.config.features;
        let colon = if features.comments.labels { line.len() - strip_label(line).len() } else { 0 };
        if colon > 0 || !features.dash_separated_labels {
            return colon;
        }
        // "Coffee - 12.50": words before a dash between spaces, which make no value.
        let Some((at, dash)) = [" - ", " – ", " — "].iter().filter_map(|d| Some((line.find(d)?, d.len()))).min()
        else {
            return 0;
        };
        let is_var = |name: &str| self.vars.contains_key(name);
        let empty = CustomUnits::default();
        let scope = Scope {
            config: &self.config,
            is_var: &is_var,
            units: self.units().unwrap_or(&empty),
            scaled: Cell::new(false),
        };
        match parser::parse(&line[..at], &scope) {
            Ok(None) => at + dash,
            _ => 0,
        }
    }

    /// Parses and evaluates; the flag marks `sum`/`total` lines.
    fn evaluate(&mut self, line: &str) -> Result<Option<(Answer, bool)>> {
        let is_var = |name: &str| self.vars.contains_key(name);
        let scope = Scope { config: &self.config, is_var: &is_var, units: self.units()?, scaled: Cell::new(false) };
        let Some(stmt) = parser::parse(line, &scope)? else { return Ok(None) };
        let env = Env {
            config: &self.config,
            now: Now::new(&self.config),
            vars: &self.vars,
            lines: &self.lines,
            rates: &self.rates,
            food_as: Default::default(),
            rate_date: Default::default(),
            inflation: self.inflation.as_deref(),
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
        let (expr, value, display, parts, is_total, name) = match stmt {
            Stmt::If { .. } => unreachable!("branches are resolved above"),
            Stmt::Expr(expr) => {
                let Some(expr) = env.branch(expr)? else { return Ok(None) };
                let (value, display, parts) = env.answer_line(expr)?;
                // "sum in EUR" and "average" are totals too.
                let is_total = expr.contains(&|e| matches!(e, Expr::Line(LineRef::Sum | LineRef::Average)));
                (expr, value, display, parts, is_total, None)
            }
            Stmt::Assign { name, op, expr } => {
                let Some(expr) = env.branch(expr)? else { return Ok(None) };
                let (mut value, display, parts) = env.answer_line(expr)?;
                if let Some(op) = op {
                    let old = self
                        .vars
                        .get(name)
                        .cloned()
                        .ok_or_else(|| crate::Error::new(format!("unknown variable {name}")))?;
                    value = env.binary(*op, old, value)?;
                }
                (expr, value, display, parts, false, Some(name.clone()))
            }
        };
        if let Value::Quantity(crate::value::Quantity { number, .. }) | Value::Percent(number) = &value
            && !number.is_finite()
        {
            return Err(crate::Error::new("result is too large"));
        }
        let (value, mut display) = env.convert_result(expr, value, display)?;
        if scope.scaled.get() && matches!(display, Display::Auto) {
            display = Display::Scaled;
        }
        let answer = env.describe(expr, value, display, parts, &self.config);
        drop(env);
        if let Some(name) = name {
            self.vars.insert(name, answer.reference());
        }
        Ok(Some((answer, is_total)))
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
