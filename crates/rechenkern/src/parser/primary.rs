//! Primary expressions: numbers with units, functions, variables and words.

use super::{Parser, words};
use crate::ast::{Expr, Format, Func, LineRef, Op, Target, TimeExpr};
use crate::error::{Result, bail};
use crate::lexer::Tok;
use crate::number::Number;
use crate::units::{Dim, Unit, registry};

impl Parser<'_> {
    pub(super) fn primary(&mut self) -> Result<Expr> {
        let Some(tok) = self.peek() else { bail!("expected a value at the end") };
        match &tok.tok {
            Tok::Num(n) => {
                self.pos += 1;
                self.number(*n)
            }
            Tok::Clock { h, m, s } => {
                self.pos += 1;
                self.clock_literal(*h, *m, *s)
            }
            Tok::Date { parts, sep } => {
                self.pos += 1;
                let date = self.numeric_date(*parts, *sep)?;
                self.date_tail(date)
            }
            Tok::IsoDateTime(text) => {
                self.pos += 1;
                Ok(Expr::Time(TimeExpr::Iso(text.clone())))
            }
            Tok::Sym("(") => {
                self.pos += 1;
                let saved = std::mem::replace(&mut self.in_list, false);
                let expr = self.expr()?;
                self.in_list = saved;
                self.eat_sym(")");
                Ok(expr)
            }
            Tok::Sym("°") if self.unit_at(self.pos, false).is_some_and(|(u, _)| u.dim() == Dim::TEMPERATURE) => {
                let (unit, n) = self.unit_at(self.pos, false).unwrap();
                self.pos += n;
                Ok(Expr::BareUnit(unit))
            }
            Tok::Word(_) => match self.word()? {
                Some(expr) => Ok(expr),
                // The word turned out to be a comment: try what follows.
                None => {
                    self.pos += 1;
                    self.primary()
                }
            },
            _ => bail!("unexpected \"{}\"", &self.src[tok.start..tok.end]),
        }
    }

    fn number(&mut self, n: Number) -> Result<Expr> {
        if let Some(date) = self.day_first_date(n)? {
            return self.date_tail(date);
        }
        if let Some(clock) = self.clock_from_hour(n)? {
            return Ok(clock);
        }
        let value = self.multipliers(n, false);
        // "1.5x" is a plain multiplier.
        if self.cur().is_some_and(|t| t.word() == Some("x") && !t.space_before) && !self.operand_follows(1) {
            self.pos += 1;
            return Ok(Expr::Number(value));
        }
        self.quantity(Expr::Number(value))
    }

    /// Applies `5k`, `2.5 million`, `3 dozen`; money allows `$5m`, `$2B`.
    fn multipliers(&mut self, mut n: Number, money: bool) -> Number {
        while let Some(tok) = self.cur() {
            let Some(w) = tok.word() else { break };
            let glued = if !tok.space_before {
                if money { words::money_multiplier(w) } else { words::suffix_multiplier(w) }
            } else {
                None
            };
            match glued.or_else(|| words::scale_word(&w.to_lowercase())) {
                Some(m) => {
                    n = n * m;
                    self.pos += 1;
                }
                None => break,
            }
        }
        n
    }

    /// Attaches a unit after a value, then further parts: `5 ft 3 in`.
    fn quantity(&mut self, value: Expr) -> Result<Expr> {
        let Some(unit) = self.number_unit() else { return Ok(value) };
        let mut last = unit.clone();
        let mut parts = vec![Expr::WithUnit(value.boxed(), unit)];
        while let Some((part, unit)) = self.composite_part(&last) {
            parts.push(part);
            last = unit;
        }
        Ok(if parts.len() == 1 { parts.pop().unwrap() } else { Expr::Composite(parts) })
    }

    /// The unit written right after a number.
    fn number_unit(&mut self) -> Option<Unit> {
        let tok = self.cur()?;
        let symbol = match &tok.tok {
            Tok::Sym("'") => Some("ft"),
            Tok::Sym("\"") => Some("in"),
            Tok::Word(w) if w.eq_ignore_ascii_case("in") && self.inch_here() => Some("in"),
            _ => None,
        };
        if let Some(symbol) = symbol {
            self.pos += 1;
            return Some(registry().get(symbol));
        }
        self.unit_phrase(true)
    }

    /// After a number, "in" means inches unless a conversion target follows.
    fn inch_here(&self) -> bool {
        match self.toks.get(self.pos + 1) {
            None => true,
            Some(t) => match &t.tok {
                Tok::Sym(s) => *s != "(",
                Tok::Word(w) => {
                    matches!(
                        w.to_lowercase().as_str(),
                        "to" | "as" | "into" | "in" | "and" | "plus" | "minus" | "times" | "x" | "per"
                    )
                }
                _ => false,
            },
        }
    }

    /// A unit with optional `square`/`cubic` and powers: `sq ft`, `m²`, `m^3`.
    pub(super) fn unit_phrase(&mut self, after_number: bool) -> Option<Unit> {
        let start = self.pos;
        let power = match self.lower(self.pos).as_deref() {
            Some("square" | "sq") => 2,
            Some("cubic" | "cu") => 3,
            _ => 1,
        };
        if power > 1 {
            self.pos += 1;
        }
        // "sq in" is square inches.
        let inch = power > 1 && self.toks.get(self.pos).is_some_and(|t| t.is_word("in"));
        let found = if inch { Some((registry().get("in"), 1)) } else { self.unit_at(self.pos, after_number) };
        let Some((unit, n)) = found else {
            self.pos = start;
            return None;
        };
        if power > 1 && unit.dim() != Dim::LENGTH {
            self.pos = start;
            return None;
        }
        self.pos += n;
        let mut unit = unit.pow(power);
        let exp = match self.cur().map(|t| &t.tok) {
            Some(Tok::Sym("²")) => Some((2, 1)),
            Some(Tok::Sym("³")) => Some((3, 1)),
            Some(Tok::Sym("^")) => match self.toks.get(self.pos + 1).map(|t| &t.tok) {
                Some(Tok::Num(e)) => e.to_i64().filter(|e| (1..=9).contains(e)).map(|e| (e as i8, 2)),
                _ => None,
            },
            Some(Tok::Word(w)) if w.eq_ignore_ascii_case("squared") => Some((2, 1)),
            Some(Tok::Word(w)) if w.eq_ignore_ascii_case("cubed") => Some((3, 1)),
            _ => None,
        };
        if let Some((e, len)) = exp {
            unit = unit.pow(e);
            self.pos += len;
        }
        Some(unit)
    }

    /// Another part of a composite quantity with the same dimension.
    fn composite_part(&mut self, last: &Unit) -> Option<(Expr, Unit)> {
        if last.is_money() {
            return None;
        }
        let mut p = self.clone();
        if !p.in_list {
            p.eat_word("and");
        }
        let Some(Tok::Num(n)) = p.cur().map(|t| &t.tok) else { return None };
        p.pos += 1;
        // "1h 30m": after hours, "m" is minutes.
        let unit = if last.dim() == Dim::TIME && p.cur().is_some_and(|t| t.word() == Some("m")) {
            p.pos += 1;
            registry().get("min")
        } else {
            p.number_unit()?
        };
        if unit.dim() != last.dim() {
            return None;
        }
        self.pos = p.pos;
        Some((Expr::WithUnit(Expr::Number(*n).boxed(), unit.clone()), unit))
    }

    /// A word in value position. `None` means the word is a comment.
    fn word(&mut self) -> Result<Option<Expr>> {
        let i = self.pos;
        let lower = self.lower(i).unwrap_or_default();
        let w = lower.as_str();

        if let Some((name, n)) = self.var_at(i) {
            self.pos += n;
            return Ok(Some(Expr::Var(name)));
        }
        if let Some((zone, n, date_only)) = self.place_time_at(i) {
            self.pos += n;
            let now = Expr::Time(if date_only { TimeExpr::Today(0) } else { TimeExpr::Now });
            return Ok(Some(Expr::Convert(now.boxed(), Target::Zone(zone))));
        }
        if let Some(expr) = self.line_ref()? {
            return Ok(Some(expr));
        }
        if let Some(expr) = self.date_part()? {
            return Ok(Some(expr));
        }
        if let Some(expr) = self.time_word()? {
            return Ok(Some(expr));
        }
        // "remainder of 21 divided by 5"
        if w == "remainder" && self.toks.get(i + 1).is_some_and(|t| t.is_word("of")) {
            self.pos += 2;
            let a = self.unary()?;
            if !self.eat_sym("/") && !(self.eat_word("divided") && self.eat_word("by")) && !self.eat_word("mod") {
                bail!("expected \"divided by\"");
            }
            let b = self.unary()?;
            return Ok(Some(Expr::binary(Op::Mod, a, b)));
        }
        if let Some((func, n)) = self.phrase_at(i, words::FUNCTIONS) {
            return self.call(func, n);
        }
        if matches!(w, "true" | "false") {
            self.pos += 1;
            return Ok(Some(Expr::Bool(w == "true")));
        }
        if let Some((expr, n)) = self.physical_constant(i) {
            self.pos += n;
            return Ok(Some(expr));
        }
        if let Some(c) = words::constant(w) {
            self.pos += 1;
            return Ok(Some(Expr::Number(c)));
        }
        if let Some(n) = self.number_words() {
            return self.number(n).map(Some);
        }
        if let Some(n) = words::scale_word(w).or_else(|| words::fraction_word(w)) {
            self.pos += 1;
            return Ok(Some(Expr::Number(n)));
        }
        if let Some((unit, n)) = self.unit_at(i, false) {
            return self.bare_unit(unit, n).map(Some);
        }
        if words::KEYWORDS.contains(&w) || w == "x" {
            bail!("unexpected \"{}\"", self.toks[i].word().unwrap_or(w));
        }
        Ok(None)
    }

    /// `prev`, `line 3`, `sum`, `average`.
    fn line_ref(&mut self) -> Result<Option<Expr>> {
        let w = self.lower(self.pos).unwrap_or_default();
        let followed_by_args = self.toks.get(self.pos + 1).is_some_and(|t| t.is_word("of") || t.is_sym("("));
        let line = match w.as_str() {
            "prev" | "previous" | "ans" | "answer" | "above" => LineRef::Previous,
            "sum" | "total" | "subtotal" if !followed_by_args => LineRef::Sum,
            "average" | "avg" | "mean" if !followed_by_args => LineRef::Average,
            "line" => match self.toks.get(self.pos + 1).map(|t| &t.tok) {
                Some(Tok::Num(n)) => {
                    let n =
                        n.to_i64().filter(|n| *n > 0).ok_or_else(|| crate::Error::new("line numbers start at 1"))?;
                    self.pos += 1;
                    LineRef::Line(n as usize)
                }
                _ => return Ok(None),
            },
            _ => match w.strip_prefix("line").and_then(|n| n.parse::<usize>().ok()) {
                Some(n) if n > 0 => LineRef::Line(n),
                _ => return Ok(None),
            },
        };
        self.pos += 1;
        Ok(Some(Expr::Line(line)))
    }

    /// "twenty five", "seven".
    fn number_words(&mut self) -> Option<Number> {
        let first = words::number_word(&self.lower(self.pos)?)?;
        self.pos += 1;
        let mut total = first;
        if first >= 20
            && first % 10 == 0
            && let Some(ones) =
                self.lower(self.pos).and_then(|w| words::number_word(&w)).filter(|n| (1..10).contains(n))
        {
            total += ones;
            self.pos += 1;
        }
        Some(Number::from_i64(total))
    }

    fn bare_unit(&mut self, unit: Unit, n: usize) -> Result<Expr> {
        self.pos += n;
        if (unit.dim() == Dim::TIME || unit.dim() == Dim::WORKDAY)
            && let Some(expr) = self.time_unit_phrase(&unit)?
        {
            return Ok(expr);
        }
        // Currency before the amount: "$5", "€10", "USD 20".
        if unit.is_money()
            && let Some(Tok::Num(v)) = self.cur().map(|t| &t.tok)
        {
            self.pos += 1;
            let value = self.multipliers(*v, true);
            return Ok(Expr::WithUnit(Expr::Number(value).boxed(), unit));
        }
        Ok(Expr::BareUnit(unit))
    }

    fn call(&mut self, func: Func, n: usize) -> Result<Option<Expr>> {
        let start = self.pos;
        self.pos += n;
        // "random" alone, "root 3 of 27", "log 20 base 4"
        let has_args = self.at_sym("(") || self.at_word("of") || self.at_word("between") || self.operand_follows(0);
        if !has_args {
            if func == Func::Random {
                return Ok(Some(Expr::Call(func, vec![])));
            }
            self.pos = start;
            return Ok(None);
        }
        if func == Func::Root && !self.at_sym("(") {
            let degree = self.unary()?;
            self.eat_word("of");
            let x = self.unary()?;
            return Ok(Some(Expr::Call(func, vec![degree, x])));
        }
        let mut args = if self.eat_sym("(") {
            let mut args = Vec::new();
            while !self.at_sym(")") && self.peek().is_some() {
                args.push(self.expr()?);
                if !self.eat_sym(",") {
                    break;
                }
            }
            self.eat_sym(")");
            args
        } else if self.eat_word("of") || self.eat_word("between") {
            self.list()?
        } else {
            vec![self.unary()?]
        };
        if func == Func::Log && self.eat_word("base") {
            args.push(self.unary()?);
        }
        // "clamp 26 between 5 and 25", "clamp 4 from 5 to 25"
        if func == Func::Clamp && args.len() == 1 {
            if self.eat_word("between") {
                args.extend(self.list()?);
            } else if self.eat_word("from") {
                args.push(self.bit_or()?);
                self.eat_word("to");
                args.push(self.bit_or()?);
            }
        }
        Ok(Some(Expr::Call(func, args)))
    }

    /// Items separated by commas and "and": `3, 4 and 5`.
    pub(super) fn list(&mut self) -> Result<Vec<Expr>> {
        let saved = std::mem::replace(&mut self.in_list, true);
        let mut items = vec![self.bit_or()?];
        while self.eat_sym(",") || self.eat_word("and") {
            items.push(self.bit_or()?);
        }
        self.in_list = saved;
        Ok(items)
    }

    /// "speed of light", "standard gravity".
    fn physical_constant(&self, i: usize) -> Option<(Expr, usize)> {
        let (index, n) = self.phrase_at(i, words::PHYSICAL_CONSTANTS)?;
        let (value, per_second) = match index {
            0 => ("299792458", 1),
            1 => ("343", 1),
            2 => ("9.80665", 2),
            _ => ("6.02214076e23", 0),
        };
        let value = Expr::Number(Number::parse(value)?);
        let expr = match per_second {
            0 => value,
            p => Expr::WithUnit(value.boxed(), registry().get("m").product(&registry().get("s").pow(-p))),
        };
        Some((expr, n))
    }

    /// "week of year", "day of the week on March 9", "weekday on 2024-03-09".
    fn date_part(&mut self) -> Result<Option<Expr>> {
        let Some((format, n)) = self.phrase_at(self.pos, words::DATE_PARTS) else { return Ok(None) };
        self.pos += n;
        let date = if self.eat_word("on") || self.eat_word("of") || self.eat_word("for") {
            self.additive()?
        } else {
            Expr::Time(TimeExpr::Today(0))
        };
        Ok(Some(Self::formatted(date, format)))
    }

    /// Wraps a value in a format conversion.
    pub(super) fn formatted(expr: Expr, format: Format) -> Expr {
        Expr::Convert(expr.boxed(), Target::Format(format))
    }
}
