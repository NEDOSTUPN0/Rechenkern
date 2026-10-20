//! What a host adds to the engine or changes in it: its own units and places,
//! and which units generic names mean (`Config::unit_system`).

use jiff::tz::TimeZone;

use crate::config::{CurrencySymbol, UnitSymbol};
use crate::currency::Currency;
use crate::error::{Error, Result};
use crate::hash::TableMap;
use crate::number::Number;
use crate::units::{self, Dim, Unit, UnitDef, registry};
use crate::value::Value;
use crate::zones::PlaceKind;

/// A unit of the host's own, worth an amount of a known one: a smoot is 1.7018 m.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomUnit {
    /// Singular name: "smoot".
    pub name: String,
    /// Plural name; the name with an "s" if `None`.
    pub plural: Option<String>,
    /// Short form for answers ("smt"); answers spell the name out without one.
    pub symbol: Option<String>,
    /// Other spellings that mean the unit.
    pub aliases: Vec<String>,
    /// How many of `unit` one of it is.
    pub size: Number,
    /// The unit it is measured in, as written in a line: "m", "kg", "USD", "m/s".
    pub unit: String,
}

impl CustomUnit {
    pub fn new(name: &str, size: Number, unit: &str) -> CustomUnit {
        CustomUnit { name: name.into(), plural: None, symbol: None, aliases: Vec::new(), size, unit: unit.into() }
    }
}

/// A place of the host's own, for time zones and the sun: an office, a home town.
#[derive(Clone, Debug)]
pub struct CustomPlace {
    pub name: String,
    pub zone: TimeZone,
    /// Latitude (north) and longitude (east) in degrees, for sunrise and distances.
    pub coordinates: Option<(f64, f64)>,
    /// Other names lines may use: "HQ" for "Head Office".
    pub aliases: Vec<String>,
    /// What the place is, for highlighting and captions; a city unless the host says.
    pub kind: PlaceKind,
}

impl CustomPlace {
    pub fn new(name: &str, zone: TimeZone) -> CustomPlace {
        CustomPlace { name: name.into(), zone, coordinates: None, aliases: Vec::new(), kind: PlaceKind::City }
    }

    /// The place goes by this lowercase name or alias.
    pub(crate) fn is_named(&self, lowercase: &str) -> bool {
        self.name.to_lowercase() == lowercase || self.aliases.iter().any(|a| a.to_lowercase() == lowercase)
    }
}

/// Variables the host keeps up to date, as SoulverCore's `VariableProvider`: asked for names
/// lines use that no line set (`stock price * 10`). Keep it quick: it is asked while parsing.
pub trait VariableProvider: Send + Sync {
    fn variable(&self, name: &str) -> Option<Value>;
}

/// Distances of the host's own between places, as SoulverCore's `distanceBetween`: by road,
/// say. Kilometers, or `None` for the great-circle distance.
pub trait DistanceProvider: Send + Sync {
    fn distance(&self, from: &crate::zones::Location, to: &crate::zones::Location) -> Option<f64>;
}

/// Places a host finds by name, as SoulverCore's `PlaceDataProvider`: asked where a line
/// expects a place the engine doesn't know (`time in Smallville`, `sunrise in Smallville`),
/// once per name.
pub trait PlaceProvider: Send + Sync {
    /// The place `name` (as written in the line) stands for, if there is one.
    fn place(&self, name: &str) -> Option<CustomPlace>;

    /// The place is being looked up in the background: the line answers `ErrorKind::Pending`
    /// and is asked again later (`Sheet::recalculate_pending`).
    fn pending(&self, name: &str) -> bool {
        let _ = name;
        false
    }
}

/// What generic names like "gallon", "cup" and "ton" mean, as in SoulverCore.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UnitSystem {
    /// US gallons and cups, short tons.
    #[default]
    CustomaryUs,
    /// Imperial gallons, pints and fluid ounces, long tons; metric cups and spoons.
    Imperial,
    /// Metric cups (250 mL) and spoons (15 and 5 mL), tonnes.
    Metric,
}

impl UnitSystem {
    /// The unit `text` means in this system: in Britain a "gallon" is the imperial gallon.
    /// Names that say whose unit it is keep it: "us gallon".
    pub(crate) fn adapt(self, text: &str, unit: Unit) -> Unit {
        if self == UnitSystem::CustomaryUs || text.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("us ")) {
            return unit;
        }
        let Some(def) = unit.single() else { return unit };
        let symbol = match (self, def.symbol.as_str()) {
            (UnitSystem::Imperial, "gal") => "imp gal",
            (UnitSystem::Imperial, "qt") => "imp qt",
            (UnitSystem::Imperial, "pint") => "imp pt",
            (UnitSystem::Imperial, "fl oz") => "imp fl oz",
            (UnitSystem::Imperial, "ton") => "long ton",
            (UnitSystem::Imperial, "cwt") => "long cwt",
            (UnitSystem::Imperial, "pk") => "imp pk",
            (UnitSystem::Imperial, "gi") => "imp gi",
            (UnitSystem::Metric, "ton") => "t",
            (_, "cup") => "metric cup",
            (_, "tbsp") => "metric tbsp",
            (_, "tsp") => "metric tsp",
            _ => return unit,
        };
        registry().get(symbol)
    }
}

/// The custom units of a calculator, by spelling.
#[derive(Debug, Default)]
pub(crate) struct CustomUnits {
    exact: TableMap<String, Unit>,
    folded: TableMap<String, Unit>,
    /// Longest name in words.
    pub max_words: usize,
}

impl CustomUnits {
    pub fn build(units: &[CustomUnit], symbols: &[CurrencySymbol], unit_symbols: &[UnitSymbol]) -> Result<CustomUnits> {
        let mut table = CustomUnits::default();
        // The host's currency symbols read as their currencies: "100 kr".
        for symbol in symbols {
            let unit = registry()
                .currency(&symbol.code)
                .ok_or_else(|| Error::new(format!("unknown currency {}", symbol.code)))?;
            table.exact.insert(symbol.symbol.clone(), unit);
        }
        for symbol in unit_symbols {
            let unit = parse_unit(&symbol.unit).ok_or_else(|| Error::new(format!("unknown unit {}", symbol.unit)))?;
            table.max_words = table.max_words.max(symbol.symbol.split(' ').count());
            table.exact.insert(symbol.symbol.clone(), unit);
        }
        for custom in units {
            let unit = Unit::of(define(custom)?);
            let plural = custom.plural.clone().unwrap_or_else(|| format!("{}s", custom.name));
            let spellings = [Some(&custom.name), Some(&plural), custom.symbol.as_ref()].into_iter().flatten();
            for text in spellings.chain(&custom.aliases) {
                table.max_words = table.max_words.max(text.split(' ').count());
                // Names in lowercase match any case: "Smoots"; others only as written: "SMT".
                match text.chars().any(char::is_uppercase) {
                    true => table.exact.insert(text.clone(), unit.clone()),
                    false => table.folded.insert(text.clone(), unit.clone()),
                };
            }
        }
        Ok(table)
    }

    pub fn lookup(&self, text: &str) -> Option<Unit> {
        if self.exact.is_empty() && self.folded.is_empty() {
            return None;
        }
        self.exact.get(text).or_else(|| self.folded.get(&text.to_lowercase())).cloned()
    }
}

/// Registers a custom unit after the built-in ones.
fn define(custom: &CustomUnit) -> Result<units::UnitId> {
    let fail = |why: &str| Error::new(format!("custom unit {}: {why}", custom.name));
    let unit = parse_unit(&custom.unit).ok_or_else(|| fail(&format!("unknown unit {}", custom.unit)))?;
    if unit.dim() == Dim::TEMPERATURE {
        return Err(fail("temperatures have a zero of their own"));
    }
    // Money keeps its currency, so it follows the exchange rates, as cents do.
    let (scale, currency): (Number, Option<&'static Currency>) = match unit.currency() {
        None => {
            let scale =
                unit.factors().iter().fold(Number::ONE, |s, &(id, e)| s * registry().def(id).scale.powi(e as i64));
            (scale, None)
        }
        Some((currency, 1)) if unit.factors().len() == 1 => (registry().def(unit.factors()[0].0).scale, Some(currency)),
        Some(_) => return Err(fail("only a plain amount of money can be its size")),
    };
    let symbol = custom.symbol.clone();
    let def = UnitDef {
        spelled: symbol.is_none(),
        symbol: symbol.unwrap_or_else(|| custom.name.clone()),
        name: custom.name.clone(),
        plural: custom.plural.clone().unwrap_or_else(|| format!("{}s", custom.name)),
        dim: unit.dim(),
        scale: custom.size * scale,
        offset: Number::ZERO,
        tight: false,
        calendar: None,
        currency,
    };
    units::define(def).ok_or_else(|| fail("too many units"))
}

/// A unit written as in a line or as a product of symbols: "km", "m/s", "kg*m^2".
pub(crate) fn parse_unit(text: &str) -> Option<Unit> {
    if let Some(unit) = registry().lookup(text) {
        return Some(unit);
    }
    let (num, den) = text.split_once('/').unwrap_or((text, ""));
    let mut unit = Unit::none();
    for (part, sign) in [(num, 1), (den, -1)] {
        for factor in part.split(['*', '·']).map(str::trim).filter(|f| !f.is_empty()) {
            let (symbol, exp) = factor.split_once('^').map_or((factor, Some(1)), |(s, e)| (s, e.parse::<i8>().ok()));
            unit = unit.product(&registry().lookup(symbol)?.pow(exp? * sign));
        }
    }
    (!unit.is_none()).then_some(unit)
}

/// Works out a custom function from the values its placeholders matched.
pub type FunctionHandler = std::sync::Arc<dyn Fn(&[Value]) -> std::result::Result<Value, String> + Send + Sync>;

/// A phrase of the host's own, as SoulverCore's `CustomFunction`: `tip on __money`,
/// `__money split __number ways`. A line that is the whole phrase calls the handler with
/// the values of its placeholders, in order:
///
/// - `__number`: a plain number; `__percentage`; `__money`; `__date`; `__duration`
/// - `__amount` (or `__unit_expression`): an amount with any unit
/// - `__unit`: a unit's name, as one of it: "in __unit"
/// - `__place`: a place's name, as text
/// - `__value`: anything
/// - a `CustomType`'s placeholder: one of its words, as text
#[derive(Clone)]
pub struct CustomFunction {
    pub prototype: String,
    /// Other phrases for the same function.
    pub aliases: Vec<String>,
    pub handler: FunctionHandler,
    /// The host's name for the function, given back in `Answer::custom_call`.
    pub id: Option<String>,
}

impl CustomFunction {
    pub fn new(
        prototype: &str,
        handler: impl Fn(&[Value]) -> std::result::Result<Value, String> + Send + Sync + 'static,
    ) -> CustomFunction {
        CustomFunction {
            prototype: prototype.into(),
            aliases: Vec::new(),
            handler: std::sync::Arc::new(handler),
            id: None,
        }
    }

    /// The same function with the host's name for it.
    pub fn with_id(self, id: &str) -> CustomFunction {
        CustomFunction { id: Some(id.into()), ..self }
    }
}

impl std::fmt::Debug for CustomFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("CustomFunction").field("prototype", &self.prototype).field("aliases", &self.aliases).finish()
    }
}

/// Words a custom placeholder matches, as SoulverCore's `CustomType`: `__airline` for
/// "QF", "BA" and "Qantas".
#[derive(Clone, Debug, Default)]
pub struct CustomType {
    /// "__airline".
    pub placeholder: String,
    pub words: Vec<String>,
    /// What each word stands for, by position, as SoulverCore's `associatedObject`; a word
    /// without one is handed over as text.
    pub values: Vec<Value>,
}

impl PartialEq for CustomType {
    fn eq(&self, other: &CustomType) -> bool {
        self.placeholder == other.placeholder
            && self.words == other.words
            && self.values.len() == other.values.len()
            && self.values.iter().zip(&other.values).all(|(a, b)| a.same_as(b))
    }
}

/// One part of a prototype phrase.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Slot {
    /// A word, number or symbol written as is.
    Literal(crate::lexer::Tok),
    Placeholder(SlotKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlotKind {
    Number,
    Amount,
    Money,
    Percentage,
    Date,
    Duration,
    Unit,
    Place,
    Value,
    /// A `CustomType`, by index.
    Type(usize),
}

/// The parts of a prototype phrase, or `None` for an unknown placeholder.
pub(crate) fn slots(prototype: &str, types: &[CustomType]) -> Option<Vec<Slot>> {
    let config = crate::Config::default();
    crate::lexer::lex(prototype, &config)
        .into_iter()
        .map(|t| {
            let text = &prototype[t.start..t.end];
            let Some(name) = text.strip_prefix("__") else { return Some(Slot::Literal(t.tok)) };
            let kind = match name {
                "number" => SlotKind::Number,
                "amount" | "unit_expression" => SlotKind::Amount,
                "money" | "currency_expression" => SlotKind::Money,
                "percentage" | "percent" => SlotKind::Percentage,
                "date" => SlotKind::Date,
                "duration" | "timespan" => SlotKind::Duration,
                "unit" => SlotKind::Unit,
                "place" => SlotKind::Place,
                "value" | "any" => SlotKind::Value,
                _ => SlotKind::Type(types.iter().position(|t| t.placeholder == text)?),
            };
            Some(Slot::Placeholder(kind))
        })
        .collect()
}
