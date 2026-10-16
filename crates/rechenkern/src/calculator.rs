//! The calculator: evaluates lines one after another, like a notepad.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;

use crate::alternatives::Alternative;
use crate::answer::Answer;
use crate::ast::{Expr, LineRef, Stmt};
use crate::config::{Config, Now};
use crate::custom::{CustomPlace, CustomUnits, PlaceProvider};
use crate::error::{Error, Result};
use crate::eval::{Display, Env, Line, LineKind};
use crate::inflation::InflationProvider;
use crate::parser::highlight::{Highlight, TokenKind};
use crate::parser::{self, Scope};
use crate::rates::{RateProvider, RateTable, Rates};
use crate::value::Value;
use crate::weather::WeatherProvider;

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
    places: Option<Box<dyn PlaceProvider>>,
    weather: Option<Box<dyn WeatherProvider>>,
    /// What the place provider said for each name asked.
    found_places: RefCell<HashMap<String, Option<CustomPlace>>>,
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
            places: None,
            weather: None,
            found_places: RefCell::default(),
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

    /// Where the weather comes from: `weather in Tokyo`.
    pub fn set_weather_provider(&mut self, provider: impl WeatherProvider + 'static) {
        self.weather = Some(Box::new(provider));
    }

    /// Where places the engine doesn't know are looked up: `time in Smallville`.
    pub fn set_place_provider(&mut self, provider: impl PlaceProvider + 'static) {
        self.places = Some(Box::new(provider));
        self.found_places.borrow_mut().clear();
    }

    /// A place from the provider, asked once per name.
    fn find_place(&self, name: &str) -> Option<CustomPlace> {
        let provider = self.places.as_ref()?;
        let key = name.to_lowercase();
        if let Some(found) = self.found_places.borrow().get(&key) {
            return found.clone();
        }
        let found = provider.place(name);
        self.found_places.borrow_mut().insert(key, found.clone());
        found
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
        let (result, effects) = self.run(line, &self.vars, &self.lines, false);
        if effects.clears_vars {
            self.vars.clear();
        }
        if let Some((name, value)) = effects.assigned {
            self.vars.insert(name, value);
        }
        self.lines.push(effects.line);
        result
    }

    /// Calculates a line after the `lines` above it, with the variables they left,
    /// without changing the calculator. With `track`, the names the parser asks about
    /// are kept in the effects too.
    pub(crate) fn run(
        &self,
        line: &str,
        vars: &HashMap<String, Value>,
        lines: &[Line],
        track: bool,
    ) -> (Result<Option<Answer>>, Effects) {
        let trimmed = line.trim();
        let kind = line_kind(trimmed);
        let mut effects =
            Effects { line: Line { value: None, kind }, assigned: None, clears_vars: false, reads: Reads::default() };
        if kind != LineKind::Value {
            effects.clears_vars = kind == LineKind::Divider && self.config.features.dividers_reset_variable_scope;
            return (Ok(None), effects);
        }
        let names = RefCell::new(Vec::new());
        let names = track.then_some(&names);
        let body = &trimmed[self.label_end(trimmed, vars, names)..];
        let result = self.evaluate(body, vars, lines, names, &mut effects);
        if let Some(names) = names {
            effects.reads.names.extend(names.take());
        }
        let result = result.map(|answer| {
            answer.map(|(answer, is_total)| {
                effects.line = Line {
                    value: Some(answer.reference()),
                    kind: if is_total { LineKind::Total } else { LineKind::Value },
                };
                answer
            })
        });
        (result, effects)
    }

    /// The answer written other ways: in hex, as a fraction, in other units, a date as ISO 8601.
    pub fn alternatives(&self, answer: &Answer) -> Vec<Alternative> {
        self.env(&self.vars, &self.lines).alternatives(answer)
    }

    /// Everything a calculation after `lines` can see.
    pub(crate) fn env<'a>(&'a self, vars: &'a HashMap<String, Value>, lines: &'a [Line]) -> Env<'a> {
        Env {
            config: &self.config,
            now: Now::new(&self.config),
            vars,
            lines,
            rates: &self.rates,
            food_as: Default::default(),
            rate_date: Default::default(),
            inflation: self.inflation.as_deref(),
            weather: self.weather.as_deref(),
            reads: Default::default(),
        }
    }

    /// What each part of a line is, for syntax highlighting; ranges are in bytes.
    /// Variables are the ones known now, as for the next line calculated.
    pub fn highlight(&self, line: &str) -> Vec<Highlight> {
        self.highlight_with(line, &self.vars)
    }

    /// Highlights a line that sees these variables.
    pub(crate) fn highlight_with(&self, line: &str, vars: &HashMap<String, Value>) -> Vec<Highlight> {
        let start = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        if line_kind(trimmed) == LineKind::Heading {
            return vec![Highlight { range: start..start + trimmed.len(), kind: TokenKind::Heading }];
        }
        if line_kind(trimmed) != LineKind::Value {
            return Vec::new();
        }
        let body = &trimmed[self.label_end(trimmed, vars, None)..];
        let mut out = Vec::new();
        let label = trimmed[..trimmed.len() - body.len()].trim_end();
        if !label.is_empty() {
            out.push(Highlight { range: start..start + label.len(), kind: TokenKind::Label });
        }
        let is_var = |name: &str| vars.contains_key(name);
        let empty = CustomUnits::default();
        let scope = Scope {
            config: &self.config,
            is_var: &is_var,
            units: self.units().unwrap_or(&empty),
            scaled: Cell::new(false),
            places: &|name| self.find_place(name),
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
    fn label_end(&self, line: &str, vars: &HashMap<String, Value>, names: Option<&RefCell<Vec<String>>>) -> usize {
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
        let is_var = |name: &str| {
            if let Some(names) = names {
                names.borrow_mut().push(name.to_string());
            }
            vars.contains_key(name)
        };
        let empty = CustomUnits::default();
        let scope = Scope {
            config: &self.config,
            is_var: &is_var,
            units: self.units().unwrap_or(&empty),
            scaled: Cell::new(false),
            places: &|name| self.find_place(name),
        };
        match parser::parse(&line[..at], &scope) {
            Ok(None) => at + dash,
            _ => 0,
        }
    }

    /// Parses and evaluates; the flag marks `sum`/`total` lines. Names the parser asks
    /// about go to `names`, a variable set and what the line read to `effects`.
    fn evaluate(
        &self,
        line: &str,
        vars: &HashMap<String, Value>,
        lines: &[Line],
        names: Option<&RefCell<Vec<String>>>,
        effects: &mut Effects,
    ) -> Result<Option<(Answer, bool)>> {
        let is_var = |name: &str| {
            if let Some(names) = names {
                names.borrow_mut().push(name.to_string());
            }
            vars.contains_key(name)
        };
        let find_place = |name: &str| self.find_place(name);
        let scope = Scope {
            config: &self.config,
            is_var: &is_var,
            units: self.units()?,
            scaled: Cell::new(false),
            places: &find_place,
        };
        let Some(stmt) = parser::parse(line, &scope)? else { return Ok(None) };
        let env = self.env(vars, lines);
        let result = self.evaluate_stmt(&stmt, &env, &scope, vars);
        let mut reads = env.reads.take();
        reads.clock = env.now.peek().is_some();
        effects.reads = reads;
        let Some((answer, is_total, assigned)) = result? else { return Ok(None) };
        effects.assigned = assigned;
        Ok(Some((answer, is_total)))
    }

    #[allow(clippy::type_complexity)]
    fn evaluate_stmt(
        &self,
        stmt: &Stmt,
        env: &Env,
        scope: &Scope,
        vars: &HashMap<String, Value>,
    ) -> Result<Option<(Answer, bool, Option<(String, Value)>)>> {
        // Pick the branch of "if ... then ... else ...".
        let mut stmt = stmt;
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
                    env.reads.borrow_mut().names.push(name.clone());
                    let old =
                        vars.get(name).cloned().ok_or_else(|| crate::Error::new(format!("unknown variable {name}")))?;
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
        let assigned = name.map(|name| (name, answer.reference()));
        Ok(Some((answer, is_total, assigned)))
    }
}

/// What a line did besides answering: what later lines see of it, and what it read.
#[derive(Clone, Debug)]
pub(crate) struct Effects {
    pub line: Line,
    /// A variable set: `rent = $1,500`.
    pub assigned: Option<(String, Value)>,
    /// A divider that forgets the variables above it.
    pub clears_vars: bool,
    pub reads: Reads,
}

/// What a line looked at while it was calculated: what it depends on.
#[derive(Clone, Debug, Default)]
pub(crate) struct Reads {
    /// Variables read, and the names the parser asked about: a new variable may change
    /// how a line reads.
    pub names: Vec<String>,
    /// Earlier lines read: by number, as `prev` or in a total.
    pub lines: Vec<usize>,
    /// The current time.
    pub clock: bool,
}

/// What kind of line the text is, before calculating it.
pub(crate) fn line_kind(trimmed: &str) -> LineKind {
    if trimmed.is_empty() {
        LineKind::Blank
    } else if trimmed.starts_with('#') {
        LineKind::Heading
    } else if trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-') {
        LineKind::Divider
    } else {
        LineKind::Value
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
