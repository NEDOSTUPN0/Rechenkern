//! The calculator: evaluates lines one after another, like a notepad.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;

use crate::alternatives::Alternative;
use crate::answer::Answer;
use crate::ast::{Expr, Func, LineRef, Stmt, UserFunction};
use crate::config::{Config, ConversionRule, Now, ResultConversions};
use crate::custom::{CustomPlace, CustomUnits, PlaceProvider};
use crate::error::{Error, Result};
use crate::eval::{Display, Env, Line, LineKind};
use crate::inflation::InflationProvider;
use crate::parser::Kept;
use crate::parser::highlight::{Highlight, TokenKind};
use crate::parser::{self, Scope};
use crate::rates::{RateProvider, RateTable, Rates};
use crate::units::{Unit, registry};
use crate::value::Quantity;
use crate::value::Value;
use crate::weather::WeatherProvider;
use crate::zones::{self, Location, PlaceInfo};

/// Evaluates natural language math.
///
/// Lines are remembered, so later lines can use variables (`rent = $1500`),
/// earlier answers (`prev`, `line 2`) and totals (`sum`).
///
/// A calculator is `Send` but not `Sync`: for several threads, give each a clone. Clones
/// share the providers and what they fetched (rates, places), and nothing else.
///
/// ```
/// let mut calc = rechenkern::Calculator::new();
/// assert_eq!(calc.calculate("20% of 50 km").unwrap().unwrap().text(), "10 km");
/// ```
pub struct Calculator {
    config: Arc<Config>,
    /// The custom units of the config, looked up once.
    units: OnceCell<std::result::Result<CustomUnits, Error>>,
    pub(crate) names: Names,
    pub(crate) lines: Vec<Line>,
    rates: Rates,
    inflation: Option<Arc<dyn InflationProvider>>,
    places: Option<Arc<dyn PlaceProvider>>,
    weather: Option<Arc<dyn WeatherProvider>>,
    /// What the place provider said for each name asked.
    found_places: RefCell<HashMap<String, Option<CustomPlace>>>,
    /// The line asked the place provider about a place it is still looking up.
    pending_place: Cell<bool>,
}

/// A copy shares the providers and what they gave; variables and lines are its own.
impl Clone for Calculator {
    fn clone(&self) -> Calculator {
        Calculator {
            config: self.config.clone(),
            units: OnceCell::new(),
            names: self.names.clone(),
            lines: self.lines.clone(),
            rates: self.rates.clone(),
            inflation: self.inflation.clone(),
            places: self.places.clone(),
            weather: self.weather.clone(),
            found_places: self.found_places.clone(),
            pending_place: Cell::new(false),
        }
    }
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
            names: Names::default(),
            lines: Vec::new(),
            rates: Rates::default(),
            inflation: None,
            places: None,
            weather: None,
            found_places: RefCell::default(),
            pending_place: Cell::new(false),
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
            .get_or_init(|| {
                let f = &self.config.formatting;
                CustomUnits::build(&self.config.custom_units, &f.symbols, &f.unit_symbols)
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Where exchange rates come from; asked once, on the first conversion.
    pub fn set_rate_provider(&mut self, provider: impl RateProvider + 'static) {
        self.rates = Rates::with_provider(Arc::new(provider));
    }

    /// Where consumer price indexes come from, for currencies besides the US dollar or
    /// newer ones; the bundled US index is used otherwise.
    pub fn set_inflation_provider(&mut self, provider: impl InflationProvider + 'static) {
        self.inflation = Some(Arc::new(provider));
    }

    /// Where the weather comes from: `weather in Tokyo`.
    pub fn set_weather_provider(&mut self, provider: impl WeatherProvider + 'static) {
        self.weather = Some(Arc::new(provider));
    }

    /// Where places the engine doesn't know are looked up: `time in Smallville`.
    pub fn set_place_provider(&mut self, provider: impl PlaceProvider + 'static) {
        self.places = Some(Arc::new(provider));
        self.found_places.borrow_mut().clear();
    }

    /// A place from the provider, asked once per name.
    fn find_place(&self, name: &str) -> Option<CustomPlace> {
        let provider = self.places.as_ref()?;
        let key = name.to_lowercase();
        if let Some(found) = self.found_places.borrow().get(&key) {
            return found.clone();
        }
        if provider.pending(name) {
            self.pending_place.set(true);
            return None;
        }
        let found = provider.place(name);
        self.found_places.borrow_mut().insert(key, found.clone());
        found
    }

    /// What a place name is: the config's places first, then the built-in ones, then the provider's.
    pub fn place(&self, name: &str) -> Option<PlaceInfo> {
        let custom = |p: &CustomPlace| {
            let location =
                p.coordinates.map(|(latitude, longitude)| Location { zone: p.zone.clone(), latitude, longitude });
            let country = p.zone.iana_name().and_then(zones::country_of_zone);
            PlaceInfo { name: p.name.clone(), kind: p.kind, zone: p.zone.clone(), location, country }
        };
        let key = name.trim().to_lowercase();
        if let Some(p) = self.config.custom_places.iter().find(|p| p.is_named(&key)) {
            return Some(custom(p));
        }
        zones::place(name).or_else(|| self.find_place(name.trim()).as_ref().map(custom))
    }

    /// A unit by any of its spellings, custom units of the config first: "km", "miles per hour".
    pub fn unit(&self, name: &str) -> Option<Unit> {
        let custom = self.units().ok().and_then(|units| units.lookup(name));
        custom.or_else(|| registry().lookup(name))
    }

    /// An amount in another unit of the same kind, at the calculator's rates: `5 km` in miles.
    pub fn convert(&self, quantity: &Quantity, to: &Unit) -> Result<Quantity> {
        let (names, lines) = (Names::default(), Vec::new());
        self.env(&names, &lines).convert_quantity(quantity, to)
    }

    pub fn set_rates(&mut self, table: RateTable) {
        self.rates = Rates::with_table(table);
    }

    /// Exchange rates, loading them if needed.
    pub fn rates(&self) -> Result<&RateTable> {
        self.rates.table()
    }

    /// A function the lines defined: `area(w, h) = w * h`.
    pub(crate) fn function_defined(&self, name: &str) -> bool {
        self.names.functions.contains_key(name)
    }

    pub fn variable(&self, name: &str) -> Option<&Value> {
        self.names.vars.get(&name.to_lowercase())
    }

    pub fn set_variable(&mut self, name: &str, value: Value) {
        self.names.vars.insert(name.to_lowercase(), value);
    }

    /// Forgets variables and earlier lines.
    pub fn clear(&mut self) {
        self.names = Names::default();
        self.lines.clear();
    }

    /// Works out one line with these variables over the ones set already, remembering
    /// nothing, as SoulverCore's `calculate(_:with:)`.
    pub fn calculate_with(&self, line: &str, variables: &HashMap<String, Value>) -> Result<Option<Answer>> {
        let mut names = self.names.clone();
        names.vars.extend(variables.iter().map(|(name, value)| (name.to_lowercase(), value.clone())));
        self.run(line, &names, &self.lines, false).0
    }

    /// Calculates one line. `Ok(None)` means there is nothing to calculate,
    /// like a heading, a comment or plain text.
    pub fn calculate(&mut self, line: &str) -> Result<Option<Answer>> {
        let (result, effects) = self.run(line, &self.names, &self.lines, false);
        self.names.apply(&effects);
        self.lines.push(effects.line);
        result
    }

    /// Calculates a line after the `lines` above it, with the variables they left,
    /// without changing the calculator. With `track`, the names the parser asks about
    /// are kept in the effects too.
    pub(crate) fn run(
        &self,
        line: &str,
        names: &Names,
        lines: &[Line],
        track: bool,
    ) -> (Result<Option<Answer>>, Effects) {
        let trimmed = line.trim();
        let kind = line_kind(trimmed);
        let tags = match kind == LineKind::Value && self.config.features.comments.hashtags {
            true => hashtags(trimmed),
            false => Vec::new(),
        };
        let mut effects = Effects {
            line: Line { value: None, kind, tags: tags.clone() },
            assigned: None,
            declared: None,
            defined: None,
            clears_vars: false,
            reads: Reads::default(),
        };
        if kind != LineKind::Value {
            effects.clears_vars = kind == LineKind::Divider && self.config.features.dividers_reset_variable_scope;
            return (Ok(None), effects);
        }
        let asked = RefCell::new(Vec::new());
        let asked = track.then_some(&asked);
        let body = &trimmed[self.label_end(trimmed, names, asked)..];
        let label = &trimmed[..trimmed.len() - body.len()];
        self.pending_place.set(false);
        let result = self.evaluate(body, label, names, lines, asked, &mut effects);
        // "time in Smallville" while the provider looks it up: not unknown, only not yet known.
        let result = match result {
            Err(e) if self.pending_place.get() && e.kind() == crate::error::ErrorKind::Unknown => {
                Err(Error::pending("the place is being looked up"))
            }
            result => result,
        };
        if self.config.features.labels_as_variables
            && effects.assigned.is_none()
            && let Ok(Some((answer, false))) = &result
            && let Some(name) = label_name(&trimmed[..trimmed.len() - body.len()])
        {
            effects.assigned = Some((name, answer.reference(), Kept::default()));
        }
        if let Some(asked) = asked {
            effects.reads.names.extend(asked.take());
        }
        let result = result.map(|answer| {
            answer.map(|(answer, is_total)| {
                effects.line = Line {
                    value: Some(answer.reference()),
                    kind: if is_total { LineKind::Total } else { LineKind::Value },
                    tags,
                };
                answer
            })
        });
        (result, effects)
    }

    /// The answer written other ways: in hex, as a fraction, in other units, a date as ISO 8601.
    pub fn alternatives(&self, answer: &Answer) -> Vec<Alternative> {
        self.env(&self.names, &self.lines).alternatives(answer)
    }

    /// Everything a calculation after `lines` can see.
    pub(crate) fn env<'a>(&'a self, names: &'a Names, lines: &'a [Line]) -> Env<'a> {
        Env {
            config: &self.config,
            now: Now::new(&self.config),
            vars: &names.vars,
            functions: &names.functions,
            depth: Default::default(),
            lines,
            tags: Vec::new(),
            rates: &self.rates,
            food_as: Default::default(),
            rate_date: Default::default(),
            rates_at_hand: Default::default(),
            inflation: self.inflation.as_deref(),
            weather: self.weather.as_deref(),
            reads: Default::default(),
            custom_call: Default::default(),
        }
    }

    /// What each part of a line is, for syntax highlighting; ranges are in bytes.
    /// Variables are the ones known now, as for the next line calculated.
    pub fn highlight(&self, line: &str) -> Vec<Highlight> {
        self.highlight_with(line, &self.names)
    }

    /// Highlights a line that sees these variables.
    pub(crate) fn highlight_with(&self, line: &str, names: &Names) -> Vec<Highlight> {
        let start = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        if line_kind(trimmed) == LineKind::Heading {
            let marks = trimmed.len() - trimmed.trim_start_matches('#').len();
            let text = start + trimmed.len() - trimmed[marks..].trim_start().len();
            let indicator = Highlight { range: start..start + marks, kind: TokenKind::HeadingIndicator };
            let heading = Highlight { range: text..start + trimmed.len(), kind: TokenKind::Heading };
            return [indicator, heading].into_iter().filter(|h| !h.range.is_empty()).collect();
        }
        if line_kind(trimmed) != LineKind::Value {
            return Vec::new();
        }
        let body = &trimmed[self.label_end(trimmed, names, None)..];
        let mut out = Vec::new();
        let label = trimmed[..trimmed.len() - body.len()].trim_end();
        if !label.is_empty() {
            // "1. Rent:": the list marker before the label is its own.
            let marker = list_marker_len(label);
            if marker > 0 {
                out.push(Highlight { range: start..start + marker, kind: TokenKind::ListMarker });
            }
            let label_start = start + marker + label[marker..].len() - label[marker..].trim_start().len();
            out.push(Highlight { range: label_start..start + label.len(), kind: TokenKind::Label });
        }
        let is_var = |name: &str| names.vars.contains_key(name);
        let is_function = |name: &str| names.functions.contains_key(name);
        let empty = CustomUnits::default();
        let places = |name: &str| self.find_place(name);
        let scope = self.scope(&is_var, &is_function, names, &places, self.units().unwrap_or(&empty), true);
        out.extend(parser::highlight::highlight(body, start + trimmed.len() - body.len(), &scope));
        out
    }

    /// The values the placeholders of one of these phrases match in a text, as SoulverCore's
    /// `functionParametersIn`: `"$90 split 3 ways"` and `"__money split __number ways"` give
    /// $90 and 3. `None` if no phrase matches the whole text.
    pub fn function_parameters(&self, text: &str, prototypes: &[&str]) -> Option<Vec<Value>> {
        let mut config = (*self.config).clone();
        config.custom_functions = prototypes
            .iter()
            .map(|p| crate::custom::CustomFunction::new(p, |values| Ok(Value::Text(format!("{}", values.len())))))
            .collect();
        let mut probe = Calculator::with_config(config);
        probe.names = self.names.clone();
        let is_var = |name: &str| probe.names.vars.contains_key(name);
        let is_function = |name: &str| probe.names.functions.contains_key(name);
        let empty = CustomUnits::default();
        let scope = probe.scope(&is_var, &is_function, &probe.names, &|_| None, probe.units().unwrap_or(&empty), true);
        let Ok(Some(Stmt::Expr(Expr::Custom { args, .. }))) = parser::parse(text, &scope) else { return None };
        let env = probe.env(&probe.names, &[]);
        args.iter()
            .map(|arg| match arg {
                crate::ast::CustomArg::Expr(expr, _) => env.eval(expr).ok(),
                crate::ast::CustomArg::Unit(unit) => {
                    Some(Value::Quantity(crate::value::Quantity::new(crate::Number::ONE, unit.clone())))
                }
                crate::ast::CustomArg::Text(text) => Some(Value::Text(text.clone())),
                crate::ast::CustomArg::Value(value) => Some(value.clone()),
            })
            .collect()
    }

    /// The values written in a text, with the variables known now.
    pub(crate) fn values_in(&self, text: &str) -> Vec<(std::ops::Range<usize>, Expr)> {
        let is_var = |name: &str| self.names.vars.contains_key(name);
        let is_function = |name: &str| self.names.functions.contains_key(name);
        let empty = CustomUnits::default();
        let scope = self.scope(&is_var, &is_function, &self.names, &|_| None, self.units().unwrap_or(&empty), false);
        parser::values_in(text, &scope)
    }

    /// Calculates every line of a text from scratch.
    pub fn calculate_sheet(&mut self, text: &str) -> Vec<Result<Option<Answer>>> {
        self.clear();
        text.lines().map(|line| self.calculate(line)).collect()
    }

    /// Where the expression starts after a label: "Rent: $1,500", or "Coffee - $5" if the
    /// config reads dashes as labels.
    fn label_end(&self, line: &str, names: &Names, asked: Option<&RefCell<Vec<String>>>) -> usize {
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
        let is_var = |name: &str| names.asks(asked, name, false);
        let is_function = |name: &str| names.asks(asked, name, true);
        let empty = CustomUnits::default();
        let places = |name: &str| self.find_place(name);
        let scope = self.scope(&is_var, &is_function, names, &places, self.units().unwrap_or(&empty), true);
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
        label: &str,
        names: &Names,
        lines: &[Line],
        asked: Option<&RefCell<Vec<String>>>,
        effects: &mut Effects,
    ) -> Result<Option<(Answer, bool)>> {
        let is_var = |name: &str| names.asks(asked, name, false);
        let is_function = |name: &str| names.asks(asked, name, true);
        let places = |name: &str| self.find_place(name);
        let mut scope = self.scope(&is_var, &is_function, names, &places, self.units()?, true);
        // "flour: 200 g in cups": the label names the food.
        scope.label_food = label_name(label).and_then(|name| crate::units::foods::named(&name));
        let Some(stmt) = parser::parse(line, &scope)? else { return Ok(None) };
        // "area(w, h) = w * h" defines a function and has no answer.
        if let Stmt::Function { name, params, body } = stmt {
            let function = UserFunction { params, body };
            effects.defined = Some((name, Arc::new(function)));
            return Ok(None);
        }
        if let Stmt::Unit(name) = stmt {
            effects.declared = Some(declare_unit(&name)?);
            return Ok(None);
        }
        let mut env = self.env(names, lines);
        if self.config.features.comments.hashtags {
            env.tags = hashtags(line);
        }
        let result = self.evaluate_stmt(&stmt, &env, &scope, &names.vars);
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
    ) -> Result<Option<(Answer, bool, Option<Assignment>)>> {
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
            Stmt::If { .. } | Stmt::Function { .. } | Stmt::Unit(_) => {
                unreachable!("branches, functions and units are dealt with above")
            }
            Stmt::Expr(expr) => {
                let Some(expr) = env.branch(expr)? else { return Ok(None) };
                let (value, display, parts) = env.answer_line(expr)?;
                // "sum in EUR" and "average" are totals too.
                let is_total =
                    expr.contains(&|e| matches!(e, Expr::Line(LineRef::Sum | LineRef::Average | LineRef::Stat(_))));
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
        let mut parts = parts;
        let (value, mut display) = env.convert_result(expr, value, display, &mut parts)?;
        // "0xff + 1" = 0x100, but "0xff in decimal" = 255.
        if let Some(radix) = scope.radix.get().or_else(|| base_call(expr))
            && matches!(display, Display::Auto)
            && !matches!(expr, Expr::Convert(..))
            && let Value::Quantity(q) = &value
            && q.unit.is_none()
            && q.number.is_integer()
            && !q.number.is_negative()
            && q.number.to_i128().is_some()
        {
            display = Display::Radix(radix);
        }
        if scope.scaled.get() && matches!(display, Display::Auto) {
            display = Display::Scaled;
        }
        let (value, mut display) = self.convert_display(env, expr, value, display)?;
        if self.config.formatting.fractions
            && matches!(display, Display::Auto)
            && let Value::Quantity(q) = &value
            && !q.unit.is_money()
            && crate::format::is_simple_fraction(q.number)
        {
            display = Display::Fraction;
        }
        let kept = Kept {
            radix: match display {
                Display::Radix(radix) => Some(radix),
                _ => None,
            },
            food: food_of(expr),
        };
        let mut answer = env.describe(expr, value, display, parts, &self.config);
        if matches!(expr, Expr::Custom { .. })
            && let Some((function, values)) = env.custom_call.take()
        {
            let custom = &self.config.custom_functions[function];
            answer.set_custom_call(crate::answer::CustomCall {
                id: custom.id.clone(),
                prototype: custom.prototype.clone(),
                values,
            });
        }
        let assigned = name.map(|name| (name, answer.reference(), kept));
        Ok(Some((answer, is_total, assigned)))
    }

    /// The answer converted by the config's rules for how it is shown: hex answers in decimal.
    fn convert_display(&self, env: &Env, expr: &Expr, value: Value, display: Display) -> Result<(Value, Display)> {
        let ResultConversions::Rules(rules) = &self.config.formatting.conversions else { return Ok((value, display)) };
        if matches!(expr, Expr::Convert(..)) {
            return Ok((value, display));
        }
        let kind = crate::answer::kind_of(&display);
        let format = rules.iter().find_map(|rule| match rule {
            ConversionRule::Display { from, to } if *from == kind => parser::target::converter_format(*to),
            _ => None,
        });
        match format {
            Some(format) => env.convert(value, &crate::ast::Target::Format(format), Display::Auto),
            None => Ok((value, display)),
        }
    }

    /// The parser's view of the calculator, with closures that answer its questions.
    fn scope<'a>(
        &'a self,
        is_var: &'a dyn Fn(&str) -> bool,
        is_function: &'a dyn Fn(&str) -> bool,
        names: &'a Names,
        places: &'a dyn Fn(&str) -> Option<CustomPlace>,
        units: &'a CustomUnits,
        weather: bool,
    ) -> Scope<'a> {
        Scope {
            config: &self.config,
            is_var,
            is_function,
            units,
            scaled: Cell::new(false),
            radix: Cell::new(None),
            kept: &names.kept,
            label_food: None,
            declared: &names.units,
            places,
            weather,
        }
    }
}

/// The variables and functions lines have set, which later lines see.
#[derive(Clone, Debug, Default)]
pub(crate) struct Names {
    pub vars: HashMap<String, Value>,
    /// What variables keep besides their values: a base, a food.
    pub kept: HashMap<String, Kept>,
    /// Units declared in lines, by their spellings in lowercase.
    pub units: HashMap<String, Unit>,
    pub functions: HashMap<String, Arc<UserFunction>>,
}

impl Names {
    /// Takes in what a line set: a variable, a function, or a clean slate after a divider.
    pub fn apply(&mut self, effects: &Effects) {
        if effects.clears_vars {
            *self = Names::default();
        }
        if let Some((name, value, kept)) = &effects.assigned {
            self.vars.insert(name.clone(), value.clone());
            self.kept.insert(name.clone(), *kept);
        }
        if let Some((spellings, unit)) = &effects.declared {
            for spelling in spellings {
                self.units.insert(spelling.clone(), unit.clone());
            }
        }
        if let Some((name, function)) = &effects.defined {
            self.functions.insert(name.clone(), function.clone());
        }
    }

    /// Whether `name` is a variable (or a function), noting the question in `asked`.
    fn asks(&self, asked: Option<&RefCell<Vec<String>>>, name: &str, function: bool) -> bool {
        if let Some(asked) = asked {
            asked.borrow_mut().push(name.to_string());
        }
        if function { self.functions.contains_key(name) } else { self.vars.contains_key(name) }
    }
}

/// A variable set, and what it keeps besides the value: `mask = 0xFF`.
pub(crate) type Assignment = (String, Value, Kept);

/// What a line did besides answering: what later lines see of it, and what it read.
#[derive(Clone, Debug)]
pub(crate) struct Effects {
    pub line: Line,
    /// A variable set: `rent = $1,500`.
    pub assigned: Option<Assignment>,
    /// A unit declared, with its spellings: `widgets = new unit`.
    pub declared: Option<(Vec<String>, Unit)>,
    /// A function defined: `area(w, h) = w * h`.
    pub defined: Option<(String, Arc<UserFunction>)>,
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

impl Reads {
    /// Takes in what a call read.
    pub fn merge(&mut self, other: Reads) {
        self.names.extend(other.names);
        self.lines.extend(other.lines);
        self.clock |= other.clock;
    }
}

/// What kind of line the text is, before calculating it.
pub(crate) fn line_kind(trimmed: &str) -> LineKind {
    if trimmed.is_empty() {
        LineKind::Blank
    } else if trimmed.starts_with('#') && !tag_statistic(trimmed) {
        LineKind::Heading
    } else if trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-') {
        LineKind::Divider
    } else {
        LineKind::Value
    }
}

/// The length of a list marker starting a text and the space after it: "1." in "1. Rent:", "a)".
fn list_marker_len(text: &str) -> usize {
    let digits = text.len() - text.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let lead = match digits {
        0 if text.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && text[1..].starts_with(')') => 1,
        0 => return 0,
        n => n,
    };
    let rest = &text[lead..];
    let ends = (rest.starts_with(')') || (rest.starts_with('.') && digits > 0)) && rest[1..].starts_with(' ');
    if ends { lead + 1 } else { 0 }
}

/// "#food total", "#food average in EUR": a statistic of tagged lines, not a heading.
fn tag_statistic(trimmed: &str) -> bool {
    const WORDS: &[&str] = &["total", "subtotal", "sum", "average", "avg", "mean", "median", "max", "min", "count"];
    let mut words = trimmed.split_whitespace();
    let tag = words.next().unwrap_or_default();
    tag.len() > 1 && words.next().is_some_and(|w| WORDS.contains(&w.to_lowercase().as_str()))
}

/// The hashtags in a line, lowercase: "food" in "Coffee $5 #food".
fn hashtags(line: &str) -> Vec<String> {
    if !line.contains('#') {
        return Vec::new();
    }
    let mut tags = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find('#') {
        let before = rest[..at].chars().next_back();
        let tag: String = rest[at + 1..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if before.is_none_or(char::is_whitespace) && tag.chars().next().is_some_and(char::is_alphabetic) {
            tags.push(tag.to_lowercase());
        }
        rest = &rest[at + 1..];
    }
    tags
}

/// Removes a leading "Label:" from a line.
/// The food an amount is of: `2 cups of flour`, also converted (`... in grams`).
fn food_of(expr: &Expr) -> Option<crate::units::foods::Food> {
    match expr {
        Expr::Food(_, food) => Some(*food),
        Expr::Counted { food, .. } => Some(*food),
        Expr::Convert(inner, _) => food_of(inner),
        _ => None,
    }
}

/// A unit of its own kind named in the plural or singular: "widgets", "widget".
fn declare_unit(name: &str) -> Result<(Vec<String>, Unit)> {
    let singular = match name.strip_suffix('s') {
        Some(stem) if !stem.ends_with('s') && !stem.is_empty() => stem,
        _ => name,
    };
    let plural = if singular == name { format!("{name}s") } else { name.to_string() };
    if crate::units::registry().lookup(singular).is_some() || crate::units::registry().lookup(&plural).is_some() {
        return Err(crate::Error::new(format!("{name} is a unit already")));
    }
    let unit = crate::units::declare(singular, &plural).ok_or_else(|| crate::Error::new("too many declared units"))?;
    Ok((vec![singular.to_string(), plural], unit))
}

/// The variable a label names: "Rent:" is `rent`, "Phone bill -" `phone bill`. Only words.
fn label_name(label: &str) -> Option<String> {
    let words = label.trim_end().trim_end_matches([':', '-', '–', '—']).split_whitespace().collect::<Vec<_>>();
    let words_only = words.iter().all(|w| w.chars().all(char::is_alphabetic));
    (!words.is_empty() && words.len() <= 6 && words_only).then(|| words.join(" ").to_lowercase())
}

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

/// The base of a call that writes a number in one: `hex(9) + 1` is in base 16.
fn base_call(expr: &Expr) -> Option<u32> {
    [(Func::Hex, 16), (Func::Bin, 2), (Func::Oct, 8)]
        .into_iter()
        .find(|&(func, _)| expr.contains(&|e| matches!(e, Expr::Call(f, _) if *f == func)))
        .map(|(_, radix)| radix)
}
