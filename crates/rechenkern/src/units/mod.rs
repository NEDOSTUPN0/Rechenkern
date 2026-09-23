//! Units of measurement: definitions, compound units and lookup by name.

mod dim;
mod table;

use std::collections::HashMap;
use std::sync::LazyLock;

pub use dim::Dim;

use crate::currency::Currency;
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

/// All known units and their spellings.
pub struct Registry {
    defs: Vec<UnitDef>,
    /// Case-sensitive symbols like "MB" or "°C".
    exact: HashMap<String, Unit>,
    /// Lowercase names like "kilometers" or "light year".
    folded: HashMap<String, Unit>,
    /// Preferred symbols for compound units, e.g. `mi/h` is shown as "mph".
    compound_symbols: Vec<(Vec<(UnitId, i8)>, String)>,
    /// Longest multi-word name, in words.
    max_words: usize,
}

static REGISTRY: LazyLock<Registry> = LazyLock::new(table::build);

pub fn registry() -> &'static Registry {
    &REGISTRY
}

impl Registry {
    pub fn def(&self, id: UnitId) -> &UnitDef {
        &self.defs[id as usize]
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

    pub fn max_words(&self) -> usize {
        self.max_words
    }

    /// Preferred symbol for a compound unit ("mph", "kWh").
    pub fn compound_symbol(&self, unit: &Unit) -> Option<&str> {
        let key = unit.key();
        self.compound_symbols.iter().find(|(k, _)| *k == key).map(|(_, s)| s.as_str())
    }
}
