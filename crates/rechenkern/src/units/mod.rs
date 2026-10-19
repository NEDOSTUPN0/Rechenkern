//! Units of measurement: definitions, compound units and lookup by name.

mod dim;
pub(crate) mod foods;
mod table;

use std::sync::{LazyLock, PoisonError, RwLock};

pub use dim::Dim;

use crate::currency::Currency;
use crate::hash::TableMap;
use crate::number::Number;

/// Index of a [`UnitDef`] in the registry.
pub type UnitId = u16;

/// A single named unit such as `km` or `USD`.
#[derive(Debug)]
pub struct UnitDef {
    /// Short form used in answers, e.g. "km".
    pub symbol: String,
    pub name: String,
    pub plural: String,
    pub dim: Dim,
    /// Size of one unit in base units (meters, kilograms, seconds, USD...).
    pub scale: Number,
    /// Zero point offset in base units; only temperatures have one.
    pub offset: Number,
    /// Answers spell the name out ("3 hours") instead of the symbol.
    pub spelled: bool,
    /// No space between number and symbol ("90°").
    pub tight: bool,
    /// Matching calendar unit and multiplier, for date arithmetic.
    pub calendar: Option<(jiff::Unit, i64)>,
    pub currency: Option<&'static Currency>,
}

impl UnitDef {
    pub fn is_temperature(&self) -> bool {
        self.dim == Dim::TEMPERATURE
    }
}

/// A product of unit powers, e.g. `km·h⁻¹`. Empty means a plain number.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Unit {
    factors: Vec<(UnitId, i8)>,
}

impl Unit {
    pub fn none() -> Unit {
        Unit::default()
    }

    pub fn of(id: UnitId) -> Unit {
        Unit { factors: vec![(id, 1)] }
    }

    pub fn factors(&self) -> &[(UnitId, i8)] {
        &self.factors
    }

    pub fn is_none(&self) -> bool {
        self.factors.is_empty()
    }

    /// The unit's definition if it is a single unit to the first power.
    pub fn single(&self) -> Option<&'static UnitDef> {
        match self.factors.as_slice() {
            [(id, 1)] => Some(registry().def(*id)),
            _ => None,
        }
    }

    pub fn dim(&self) -> Dim {
        let reg = registry();
        self.factors.iter().fold(Dim::NONE, |d, &(id, e)| d.mul(reg.def(id).dim.pow(e)))
    }

    /// The only currency in this unit, with its exponent.
    pub fn currency(&self) -> Option<(&'static Currency, i8)> {
        let mut found = self.factors.iter().filter_map(|&(id, e)| Some((registry().def(id).currency?, e)));
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    pub fn is_money(&self) -> bool {
        matches!(self.factors.as_slice(), [(id, 1)] if registry().def(*id).currency.is_some())
    }

    pub fn pow(&self, exp: i8) -> Unit {
        let factors = self.factors.iter().map(|&(id, e)| (id, e * exp)).filter(|f| f.1 != 0).collect();
        Unit { factors }
    }

    /// Product without merging different units of the same kind: `km/h` stays `km/h`.
    pub fn product(&self, other: &Unit) -> Unit {
        let mut factors = self.factors.clone();
        for &(id, e) in &other.factors {
            match factors.iter_mut().find(|f| f.0 == id) {
                Some(f) => f.1 += e,
                None => factors.push((id, e)),
            }
        }
        factors.retain(|f| f.1 != 0);
        Unit { factors }
    }

    /// The factors with positive powers: `$` of `$/month`.
    pub fn numerator(&self) -> Unit {
        Unit { factors: self.factors.iter().copied().filter(|f| f.1 > 0).collect() }
    }

    /// Root of the unit if every exponent is divisible by `n`.
    pub fn root(&self, n: i8) -> Option<Unit> {
        let factors = self.factors.iter().map(|&(id, e)| (e % n == 0).then_some((id, e / n))).collect::<Option<_>>()?;
        Some(Unit { factors })
    }

    /// Product of two units. Factors of the same kind merge into the left
    /// one (`m × cm` gives `m²`), so the returned multiplier must be applied
    /// to the value.
    pub fn mul(&self, other: &Unit, scale: &impl Fn(UnitId) -> crate::Result<Number>) -> crate::Result<(Unit, Number)> {
        let reg = registry();
        let mut factors = self.factors.clone();
        let mut multiplier = Number::ONE;
        for &(id, e) in &other.factors {
            if let Some(f) = factors.iter_mut().find(|f| f.0 == id) {
                f.1 += e;
                continue;
            }
            let def = reg.def(id);
            let same_kind = factors.iter_mut().find(|f| {
                let other = reg.def(f.0);
                other.dim == def.dim && !def.is_temperature()
            });
            match same_kind {
                Some(f) => {
                    multiplier = multiplier * (scale(id)? / scale(f.0)?).powi(e as i64);
                    f.1 += e;
                }
                None => factors.push((id, e)),
            }
        }
        factors.retain(|f| f.1 != 0);
        Ok((Unit { factors }, multiplier))
    }

    /// Size of the unit in base units.
    pub fn scale(&self, scale: &impl Fn(UnitId) -> crate::Result<Number>) -> crate::Result<Number> {
        self.factors.iter().try_fold(Number::ONE, |acc, &(id, e)| Ok(acc * scale(id)?.powi(e as i64)))
    }

    /// The unit with each unit per an amount written as a plain rate, and the
    /// multiplier that goes with it: `L/100km` is 0.01 `L/km`.
    pub fn plain_rates(&self) -> (Unit, Number) {
        let reg = registry();
        let mut unit = Unit::none();
        let mut multiplier = Number::ONE;
        for &(id, e) in &self.factors {
            match reg.per_amount(id) {
                Some((rate, amount)) => {
                    unit = unit.product(&rate.pow(e));
                    multiplier = multiplier * amount.powi(e as i64);
                }
                None => unit = unit.product(&Unit { factors: vec![(id, e)] }),
            }
        }
        (unit, multiplier)
    }

    /// Factors sorted by id, for order independent comparison.
    fn key(&self) -> Vec<(UnitId, i8)> {
        let mut key = self.factors.clone();
        key.sort_unstable();
        key
    }

    pub fn same_as(&self, other: &Unit) -> bool {
        self.key() == other.key()
    }
}

impl std::fmt::Display for Unit {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&crate::format::unit_text(self, false))
    }
}

/// Kinds of quantities with a coherent SI unit that products of other units turn into.
const DERIVED: &[(Dim, &str)] = &[
    (Dim::FORCE, "N"),
    (Dim::ENERGY, "J"),
    (Dim::POWER, "W"),
    (Dim::PRESSURE, "Pa"),
    (Dim::CURRENT, "A"),
    (Dim::VOLTAGE, "V"),
    (Dim::RESISTANCE, "Ω"),
    (Dim::CONDUCTANCE, "siemens"),
    (Dim::CHARGE, "coulomb"),
    (Dim::CAPACITANCE, "farad"),
    (Dim::MAGNETIC_FLUX, "Wb"),
    (Dim::FLUX_DENSITY, "tesla"),
    (Dim::INDUCTANCE, "henry"),
];

/// All known units and their spellings.
pub struct Registry {
    defs: Vec<UnitDef>,
    /// Case-sensitive symbols like "MB" or "°C".
    exact: TableMap<String, Unit>,
    /// Lowercase names like "kilometers" or "light year".
    folded: TableMap<String, Unit>,
    /// Preferred symbols for compound units, e.g. `mi/h` is shown as "mph".
    compound_symbols: Vec<(Vec<(UnitId, i8)>, String)>,
    /// Units per an amount of another unit, as a plain rate and its size: `L/100km` is 0.01 `L/km`.
    per_amount: Vec<(UnitId, Unit, Number)>,
    /// Longest multi-word name, in words.
    max_words: usize,
}

static REGISTRY: LazyLock<Registry> = LazyLock::new(table::build);

/// Units hosts defined, numbered after the built-in ones. They are never removed,
/// so their definitions live as long as the program.
static CUSTOM: RwLock<Vec<&'static UnitDef>> = RwLock::new(Vec::new());

/// Names of the units declared in lines, by their quantity's slot.
static DECLARED: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// A unit of its own kind declared in a line: `widgets = new unit`. The same name is the
/// same unit everywhere; there is room for `dim::DECLARED` names.
pub(crate) fn declare(singular: &str, plural: &str) -> Option<Unit> {
    let mut declared = DECLARED.write().unwrap_or_else(PoisonError::into_inner);
    let slot = match declared.iter().position(|n| n == singular) {
        Some(slot) => slot,
        None if declared.len() < dim::DECLARED => {
            declared.push(singular.to_string());
            declared.len() - 1
        }
        None => return None,
    };
    let def = UnitDef {
        symbol: singular.to_string(),
        name: singular.to_string(),
        plural: plural.to_string(),
        dim: Dim::declared(slot),
        scale: Number::ONE,
        offset: Number::ZERO,
        spelled: true,
        tight: false,
        calendar: None,
        currency: None,
    };
    define(def).map(Unit::of)
}

/// Adds a unit after the built-in ones, or finds the same one added before.
pub(crate) fn define(def: UnitDef) -> Option<UnitId> {
    let builtin = registry().defs.len();
    let mut custom = CUSTOM.write().unwrap_or_else(PoisonError::into_inner);
    let same = |d: &&UnitDef| {
        (&d.symbol, &d.name, &d.plural, d.dim, d.scale, d.spelled, d.currency.map(|c| c.code))
            == (&def.symbol, &def.name, &def.plural, def.dim, def.scale, def.spelled, def.currency.map(|c| c.code))
    };
    let index = match custom.iter().position(same) {
        Some(i) => i,
        None => {
            custom.push(Box::leak(Box::new(def)));
            custom.len() - 1
        }
    };
    UnitId::try_from(builtin + index).ok()
}

pub fn registry() -> &'static Registry {
    &REGISTRY
}

impl Registry {
    pub fn def(&self, id: UnitId) -> &UnitDef {
        match self.defs.get(id as usize) {
            Some(def) => def,
            None => CUSTOM.read().unwrap_or_else(PoisonError::into_inner)[id as usize - self.defs.len()],
        }
    }

    /// Looks up a unit spelling such as "km", "Kilometres", "m2" or "light years".
    pub fn lookup(&self, text: &str) -> Option<Unit> {
        if let Some(u) = self.exact.get(text) {
            return Some(u.clone());
        }
        if let Some(u) = self.folded.get(&text.to_lowercase()) {
            return Some(u.clone());
        }
        // Trailing exponent: "m2", "cm3".
        let exp = match text.chars().last()? {
            '2' => 2,
            '3' => 3,
            _ => return None,
        };
        let unit = self.exact.get(&text[..text.len() - 1])?;
        (unit.dim() == Dim::LENGTH).then(|| unit.pow(exp))
    }

    /// A unit by its exact symbol; panics if missing, so use only for built-in units.
    pub fn get(&self, symbol: &str) -> Unit {
        self.exact.get(symbol).cloned().unwrap_or_else(|| panic!("unknown built-in unit {symbol}"))
    }

    pub fn currency(&self, code: &str) -> Option<Unit> {
        let code = code.to_uppercase();
        self.exact.get(&code).filter(|u| u.is_money()).cloned()
    }

    /// Every spelling of every unit, sorted.
    pub(crate) fn spellings(&self) -> Vec<&str> {
        let mut all: Vec<&str> = self.exact.keys().chain(self.folded.keys()).map(String::as_str).collect();
        all.sort_unstable();
        all.dedup();
        all
    }

    pub fn max_words(&self) -> usize {
        self.max_words
    }

    fn per_amount(&self, id: UnitId) -> Option<(&Unit, Number)> {
        self.per_amount.iter().find(|p| p.0 == id).map(|(_, rate, amount)| (rate, *amount))
    }

    /// Whether a product of this kind can have a unit of its own: a force, a voltage...
    pub fn may_be_named(&self, unit: &Unit) -> bool {
        let dim = unit.dim();
        dim == Dim::ACCELERATION || DERIVED.iter().any(|(d, _)| *d == dim)
    }

    /// A product of units as a unit of its own: `V·A` is a watt, `lb·ft/s²` a poundal,
    /// `kN/m²` a kilopascal, `mAh·V` joules. None when the product reads well as it is:
    /// a single unit, a named compound (`kWh`), a torque (`N·m/s` too) or a length per a time
    /// squared (`km/h²`, but not `mi/(h·s)`).
    pub fn named_product(&self, unit: &Unit) -> Option<Unit> {
        let factors = unit.factors();
        if factors.len() < 2 {
            return None;
        }
        let dim = unit.dim();
        let derived = DERIVED.iter().find(|(d, _)| *d == dim);
        if (derived.is_none() && dim != Dim::ACCELERATION) || self.compound_symbol(unit).is_some() {
            return None;
        }
        let si = match derived {
            Some((_, name)) => self.lookup(name)?,
            None => self.get("m").product(&self.get("s").pow(-2)),
        };
        let kinds: Vec<(Dim, i8)> = factors.iter().map(|&(id, e)| (self.def(id).dim, e)).collect();
        let torque = kinds.contains(&(Dim::FORCE, 1)) && kinds.contains(&(Dim::LENGTH, 1));
        let per_time_squared = kinds.len() == 2 && kinds.contains(&(Dim::TIME, -2));
        if torque || per_time_squared {
            return None;
        }
        // Power for hours is energy in watt hours: "1 hp × 1 h" = 745.7 Wh.
        let hour = factors.iter().any(|&(id, e)| self.def(id).symbol == "h" && e == 1);
        if dim == Dim::ENERGY && hour && kinds.contains(&(Dim::POWER, 1)) && factors.len() == 2 {
            return self.lookup("Wh");
        }
        let scale = factors.iter().fold(Number::ONE, |acc, &(id, e)| acc * self.def(id).scale.powi(e as i64));
        let same_size = |def: &UnitDef| {
            let (a, b) = (def.scale.to_f64(), scale.to_f64());
            def.dim == dim && def.currency.is_none() && def.offset.is_zero() && ((a - b) / b).abs() < 1e-12
        };
        match self.defs.iter().position(same_size) {
            Some(id) => Some(Unit::of(id as UnitId)),
            None => Some(si),
        }
    }

    /// Preferred symbol for a compound unit ("mph", "kWh").
    pub fn compound_symbol(&self, unit: &Unit) -> Option<&str> {
        let key = unit.key();
        self.compound_symbols.iter().find(|(k, _)| *k == key).map(|(_, s)| s.as_str())
    }
}
