//! Primary expressions: numbers with units, functions, variables and words.

use super::{Parser, words};
use crate::ast::{Direction, Expr, Format, Func, LineRef, Op, Rounding, Target, TimeExpr};
use crate::error::{Error, Result, bail};
use crate::lexer::{Tok, Token};
use crate::number::Number;
use crate::units::foods::Food;
use crate::units::{Dim, Unit, registry};

impl Parser<'_> {
    pub(super) fn primary(&mut self) -> Result<Expr> {
        let Some(tok) = self.peek() else { return Err(Error::unclear("expected a value at the end")) };
        match &tok.tok {
            Tok::Num(n) => {
                self.pos += 1;
                if let Some(date) = self.year_first_date(*n) {
                    return Ok(date);
                }
                // "2nd tuesday of March"
                self.pos -= 1;
                if let Some(day) = self.day_in_month()? {
                    return Ok(day);
                }
                self.pos += 1;
                // "the 15th", "on the 15th of next month": a day of a month.
                let the = self.pos >= 2 && self.toks[self.pos - 2].is_word("the");
                let starts = self.pos == 2 || (self.pos == 3 && self.toks[0].is_word("on"));
                if the
                    && starts
                    && let Some(day) = n.to_i64().filter(|d| (1..=31).contains(d))
                    && self.eat_ordinal()
                {
                    let month = if self.eat_word("of") { self.month_of()? } else { None };
                    return Ok(Expr::Time(TimeExpr::InMonth(crate::ast::MonthDay::Day(day as i8), month)));
                }
                // "0xff + 1": the answer is in the base of the last number written in one.
                let radix = match self.src.as_bytes()[tok.start..].get(..2) {
                    Some([b'0', b'x' | b'X']) => Some(16),
                    Some([b'0', b'b' | b'B']) => Some(2),
                    Some([b'0', b'o' | b'O']) => Some(8),
                    _ => None,
                };
                if radix.is_some() && tok.end > tok.start + 2 {
                    self.scope.radix.set(radix);
                }
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
            Tok::Timecode(parts) => {
                self.pos += 1;
                Ok(Expr::Timecode(*parts, self.frame_rate().unwrap_or(Number::from_i64(24))))
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
            Tok::Sym("°") if let Some((unit, n)) = self.unit_at(self.pos, false) => {
                self.pos += n;
                Ok(Expr::BareUnit(unit))
            }
            Tok::Word(w) => match self.word()? {
                Some(expr) => Ok(expr),
                // The word turned out to be a comment: try what follows.
                None => {
                    let article = w.eq_ignore_ascii_case("a") || w.eq_ignore_ascii_case("an");
                    self.pos += 1;
                    // "a day in hours", "a week from now", "a cup of flour": one of the unit,
                    // but "a quarter of an hour" is a fraction.
                    if article
                        && self.fraction_at(self.pos).is_none()
                        && let Some(unit) = self.unit_phrase(false)
                    {
                        return Ok(Expr::WithUnit(Expr::Number(Number::ONE).boxed(), unit));
                    }
                    self.primary()
                }
            },
            _ => Err(Error::unclear(format!("unexpected \"{}\"", &self.src[tok.start..tok.end]))),
        }
    }

    fn number(&mut self, n: Number) -> Result<Expr> {
        if let Some(resolution) = self.resolution(n) {
            return Ok(resolution);
        }
        if let Some(date) = self.day_first_date(n)? {
            return self.date_tail(date);
        }
        if let Some(clock) = self.clock_from_hour(n)? {
            return Ok(clock);
        }
        let value = self.multipliers(n, false);
        if let Some(root) = self.root_of_degree(value)? {
            return Ok(root);
        }
        // "1st", "21st": an ordinal, not stones.
        if self.cur().is_some_and(|t| t.word() == Some("st") && !t.space_before)
            && value.to_i64().is_some_and(|v| value.is_integer() && v % 10 == 1 && v % 100 != 11)
        {
            self.pos += 1;
            return Ok(Expr::Number(value));
        }
        // "1.5x" is a plain multiplier.
        if self.cur().is_some_and(|t| t.word() == Some("x") && !t.space_before) && !self.operand_follows(1) {
            self.pos += 1;
            return Ok(Expr::Number(value));
        }
        self.quantity(Expr::Number(value))
    }

    /// "1920x1080" right after the width: a resolution. Both sides are 100 or more, so
    /// "3x4" stays a multiplication.
    fn resolution(&mut self, width: Number) -> Option<Expr> {
        let side =
            |n: Number| n.to_i64().filter(|&v| n.is_integer() && (100..=1_000_000).contains(&v)).map(|v| v as u32);
        let width = side(width)?;
        let tok = self.cur().filter(|t| !t.space_before)?;
        let height = match &tok.tok {
            Tok::Word(w) => {
                w.strip_prefix(['x', 'X']).filter(|d| d.bytes().all(|b| b.is_ascii_digit()))?.parse().ok()?
            }
            // "1920×1080"
            Tok::Sym("*") if self.src[tok.start..].starts_with('×') => match self.toks.get(self.pos + 1) {
                Some(Token { tok: Tok::Num(h), space_before: false, .. }) => side(*h)?,
                _ => return None,
            },
            _ => return None,
        };
        side(Number::from_i64(height as i64))?;
        self.pos += if matches!(tok.tok, Tok::Word(_)) { 1 } else { 2 };
        Some(Expr::Resolution(width, height))
    }

    /// Applies `5k`, `2.5 million`, `3 dozen`, `two thirds`; money allows `$5m`, `$2B`.
    fn multipliers(&mut self, mut n: Number, money: bool) -> Number {
        while let Some(tok) = self.cur() {
            let Some(w) = tok.word() else { break };
            let glued = if !tok.space_before {
                if money { words::money_multiplier(w) } else { words::suffix_multiplier(w) }
            } else {
                None
            };
            match glued.or_else(|| words::scale_word(w)).or_else(|| self.fraction_at(self.pos)) {
                Some(m) => {
                    // "5M", "2 billion", "5 млн": answers may keep the scale, as in SoulverCore.
                    // Not "3 dozen", nor letters glued in Cyrillic or Chinese: "5к" is 5,000.
                    if m >= Number::from_i64(1000) && (w.is_ascii() || tok.space_before) {
                        self.scope.scaled.set(true);
                    }
                    n = n * m;
                    self.pos += 1;
                }
                None => break,
            }
        }
        n
    }

    /// A fraction word at token `i`: "half", "two thirds". A quarter is a fraction
    /// only before "of": "3 quarters of an hour", but "1 quarter in months".
    fn fraction_at(&self, i: usize) -> Option<Number> {
        let w = self.lower(i)?;
        words::fraction_word(&w).filter(|_| !w.starts_with("quarter") || self.words_at(i + 1, &["of"]))
    }

    /// Attaches a unit after a value, then further parts: `5 ft 3 in`.
    pub(super) fn quantity(&mut self, value: Expr) -> Result<Expr> {
        let start = self.pos;
        let Some(unit) = self.number_unit() else { return Ok(value) };
        let mut last = unit.clone();
        let mut parts = vec![Expr::WithUnit(value.boxed(), unit)];
        while let Some((part, unit)) = self.composite_part(&last) {
            parts.push(part);
            last = unit;
        }
        // "3 nights in Paris": where the time is spent only describes it; but "time in
        // 3 hours in Tokyo" is the time there.
        let before = self.toks[..start].iter().rev().find(|t| !matches!(t.tok, Tok::Num(_)));
        if last.dim() == Dim::TIME
            && !before.is_some_and(|t| t.is_word("in") || t.is_word("after"))
            && self.words_at(self.pos, &["in"])
            && let Some((_, n)) = self.zone_at(self.pos + 1)
            && self.unit_at(self.pos + 1, false).is_none()
            && self.phrase_at(self.pos + 1, &words::FORMATS).is_none()
        {
            self.pos += 1 + n;
        }
        let amount = if parts.len() == 1 { parts.pop().unwrap() } else { Expr::Composite(parts) };
        if last.dim() == Dim::ANGLE
            && let Some(coordinates) = self.coordinates_literal(&amount)
        {
            return Ok(coordinates);
        }
        Ok(amount)
    }

    /// "at 30 fps" after a timecode: its frame rate.
    fn frame_rate(&mut self) -> Option<Number> {
        let Tok::Num(fps) = self.toks.get(self.pos + 1)?.tok else { return None };
        let fps_unit = self.toks.get(self.pos + 2).is_some_and(|t| t.is_word("fps"));
        (self.words_at(self.pos, &["at"]) && fps_unit).then(|| {
            self.pos += 3;
            fps
        })
    }

    /// "um" or "us" ends the amount: nothing after it but a conversion.
    fn micro_here(&self) -> bool {
        match self.toks.get(self.pos + 1) {
            None => true,
            Some(t) => ["in", "to", "as", "into"].iter().any(|w| t.is_word(w)) || t.word().is_none(),
        }
    }

    /// The unit written right after a number.
    fn number_unit(&mut self) -> Option<Unit> {
        let tok = self.cur()?;
        let symbol = match &tok.tok {
            Tok::Sym("'") => Some("ft"),
            Tok::Sym("\"") => Some("in"),
            // Primes are minutes and seconds of arc, as in SoulverCore.
            Tok::Sym("′") => Some("arcmin"),
            Tok::Sym("″") => Some("arcsec"),
            // "5 um", "10 us in ms": micro typed with a "u", but not "5 us dollars".
            Tok::Word(w) if matches!(w.as_str(), "um" | "us") && self.micro_here() => {
                Some(if w == "um" { "µm" } else { "µs" })
            }
            Tok::Word(w) if w.eq_ignore_ascii_case("in") && self.inch_here() => Some("in"),
            _ => None,
        };
        if let Some(symbol) = symbol {
            self.pos += 1;
            // "2 in²" is square inches, not (2 in)².
            return Some(self.unit_exponent(registry().get(symbol)));
        }
        if let Some(unit) = self.date_shorthand() {
            self.pos += 1;
            return Some(unit);
        }
        let start = self.pos;
        let unit = self.unit_phrase(true)?;
        // "Model 3 $40k": a currency before a number is that number's.
        if unit.is_money() && self.cur().is_some_and(|t| matches!(t.tok, Tok::Num(_))) {
            self.pos = start;
            return None;
        }
        Some(unit)
    }

    /// "w" and "y" glued to a number are weeks and years, as in "3w" and "1y 2mo";
    /// "100w in kW" is watts.
    fn date_shorthand(&self) -> Option<Unit> {
        let tok = self.cur().filter(|t| !t.space_before)?;
        let mut p = self.clone();
        p.pos += 1;
        let power = ["in", "to", "as", "into"].iter().any(|w| p.eat_word(w))
            && p.unit_phrase(true).is_some_and(|u| u.dim() == Dim::POWER);
        let symbol = match tok.word()? {
            "w" if power => "W",
            "w" => "wk",
            "y" => "yr",
            _ => return None,
        };
        Some(registry().get(symbol))
    }

    /// After a number, "in" means inches unless a conversion target follows.
    fn inch_here(&self) -> bool {
        match self.toks.get(self.pos + 1) {
            None => true,
            Some(t) => match &t.tok {
                Tok::Sym(s) => !matches!(*s, "(" | "%"),
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
        // "cubic meters per second": the power is the meters'.
        let per = self.per_unit_in(self.pos, n);
        let base = per.as_ref().map_or(unit.clone(), |per| unit.product(per));
        if power > 1 && base.dim() != Dim::LENGTH {
            self.pos = start;
            return None;
        }
        self.pos += n;
        let unit = match &per {
            Some(per) if power > 1 => base.pow(power).product(&per.pow(-1)),
            _ => unit.pow(power),
        };
        let unit = self.per_unit_power(unit, per.as_ref());
        Some(self.per_amount(&unit).unwrap_or(unit))
    }

    /// A power after a unit; "meters per second squared" gives it to the second.
    fn per_unit_power(&mut self, unit: Unit, per: Option<&Unit>) -> Unit {
        match (self.exponent(), per) {
            (None, _) => unit,
            (Some(e), Some(per)) => unit.product(&per.pow(1 - e)),
            (Some(e), None) => unit.pow(e),
        }
    }

    /// The unit after "per" in a unit name read at `start`, `n` tokens long: "meters per second".
    fn per_unit_in(&self, start: usize, n: usize) -> Option<Unit> {
        let per = (start..start + n).find(|&k| self.toks[k].is_word("per"))?;
        let (unit, len) = self.unit_at(per + 1, true)?;
        (per + 1 + len == start + n).then_some(unit)
    }

    /// A power written after a unit: `m²`, `m^3`, `m squared`.
    pub(super) fn unit_exponent(&mut self, unit: Unit) -> Unit {
        match self.exponent() {
            Some(e) => unit.pow(e),
            None => unit,
        }
    }

    /// A power at the cursor, read past: `²`, `^3`, `squared`.
    fn exponent(&mut self) -> Option<i8> {
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
        let (e, len) = exp?;
        self.pos += len;
        Some(e)
    }

    /// "2026 March 5": a year before a date, if the config reads it so.
    fn year_first_date(&mut self, year: Number) -> Option<Expr> {
        let year = year.to_i64().filter(|y| (1000..=9999).contains(y) && year.is_integer())?;
        let month = self.lower(self.pos).is_some_and(|w| words::month(&w).is_some());
        if !self.config().features.dates.leading_year_parsing || !month {
            return None;
        }
        let mut p = self.clone();
        let Ok(Some(Expr::Time(TimeExpr::Date { year: None, month, day }))) = p.date_word() else { return None };
        *self = p;
        Some(Expr::Time(TimeExpr::Date { year: Some(year as i16), month, day }))
    }

    /// A unit per an amount of another unit, known as a unit of its own: "L/100km", "liters per 100 km".
    fn per_amount(&mut self, unit: &Unit) -> Option<Unit> {
        if !self.cur().is_some_and(|t| t.is_sym("/") || t.is_word("per")) {
            return None;
        }
        let Tok::Num(amount) = self.toks.get(self.pos + 1)?.tok else { return None };
        let (per, n) = self.unit_at(self.pos + 2, true)?;
        let symbol = format!("{}/{amount}{}", unit.single()?.symbol, per.single()?.symbol);
        let unit = self.lookup_unit(&symbol)?;
        self.pos += 2 + n;
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
        // "40° 26′ 46″": after degrees, minutes and seconds of arc.
        } else if last.dim() == Dim::ANGLE
            && p.cur().is_some_and(|t| t.is_sym("'") || t.is_sym("\"") || t.is_sym("′") || t.is_sym("″"))
        {
            let arc = if p.cur().is_some_and(|t| t.is_sym("'") || t.is_sym("′")) { "arcmin" } else { "arcsec" };
            p.pos += 1;
            registry().get(arc)
        } else {
            p.number_unit()?
        };
        if unit.dim() != last.dim() {
            return None;
        }
        self.pos = p.pos;
        Some((Expr::WithUnit(Expr::Number(*n).boxed(), unit.clone()), unit))
    }

    /// The food an amount of weight or volume is of: "of flour" in "1 cup of flour",
    /// "melted butter" in "200 g melted butter".
    pub(super) fn food_after(&mut self, amount: &Expr) -> Option<Food> {
        let dim = amount.edge_unit(true)?.dim();
        if dim != Dim::MASS && dim != Dim::VOLUME {
            return None;
        }
        let mut i = self.pos + usize::from(self.words_at(self.pos, &["of"]));
        let (food, n) = loop {
            // A variable named like a food is the variable.
            if self.var_at(i).is_some() {
                return None;
            }
            if let Some(found) = self.food_at(i) {
                break found;
            }
            if !self.lower(i).is_some_and(|w| words::FOOD_STATES.contains(&w.as_str())) {
                return None;
            }
            i += 1;
        };
        self.pos = i + n;
        Some(food)
    }

    /// A food counted after a number, read past: "stick of butter", "cloves garlic", "garlic
    /// cloves", "large eggs". The grams in a piece, the food and whether a word named the piece.
    pub(super) fn counted_food(&mut self) -> Option<(Number, Food, bool)> {
        // Most numbers are followed by an operator or nothing.
        self.cur()?.word()?;
        let piece_at = |p: &Self, i: usize| {
            let word = p.lower(i)?;
            let word = word.strip_suffix('s').unwrap_or(&word).to_string();
            words::FOOD_PIECES.contains(&word.as_str()).then_some(word)
        };
        let food_at = |p: &Self, mut i: usize| {
            while p.lower(i).is_some_and(|w| words::FOOD_STATES.contains(&w.as_str())) {
                i += 1;
            }
            // A variable named like a food is the variable.
            if p.var_at(i).is_some() {
                return None;
            }
            p.food_at(i).map(|(food, n)| (food, i + n))
        };
        let size = ["extra large", "small", "medium", "large", "jumbo"]
            .into_iter()
            .find_map(|s| self.phrase_len_at(self.pos, s).map(|n| (s, n)));
        let i = self.pos + size.map_or(0, |(_, n)| n);
        let found = if let Some(piece) = piece_at(self, i) {
            // "stick of butter", "cloves garlic"
            let of = usize::from(self.words_at(i + 1, &["of"]));
            food_at(self, i + 1 + of).and_then(|(food, end)| Some((food.piece(&piece)?, food, true, end)))
        } else {
            let (food, end) = food_at(self, i)?;
            match piece_at(self, end) {
                // "garlic cloves"
                Some(piece) => food.piece(&piece).map(|grams| (grams, food, true, end + 1)),
                None => food.piece(size.map_or("each", |(s, _)| s)).map(|grams| (grams, food, false, end)),
            }
        };
        let (grams, food, piece, end) = found?;
        self.pos = end;
        Some((grams, food, piece))
    }

    /// A word in value position. `None` means the word is a comment.
    fn word(&mut self) -> Result<Option<Expr>> {
        let i = self.pos;
        let lower = self.lower(i).unwrap_or_default();
        let w = lower.as_str();

        if let Some((name, n)) = self.var_at(i) {
            self.pos += n;
            let kept = self.scope.kept.get(&name).copied().unwrap_or_default();
            if kept.radix.is_some() {
                self.scope.radix.set(kept.radix);
            }
            return Ok(Some(match kept.food {
                Some(food) => Expr::Food(Expr::Var(name).boxed(), food),
                None => Expr::Var(name),
            }));
        }
        if let Some((month, day)) = self.glued_date(i) {
            self.pos += 1;
            let date = Expr::Time(TimeExpr::Date { year: None, month, day });
            return self.date_tail(date).map(Some);
        }
        if let Some((unit, amount)) = self.glued_money(i) {
            self.pos += 1;
            return Ok(Some(Expr::WithUnit(Expr::Number(amount).boxed(), unit)));
        }
        // "flour 1 cup in grams": the food before its amount.
        if matches!(self.toks.get(i + 1).map(|t| &t.tok), Some(Tok::Num(_)))
            && let Some((food, n)) = self.food_at(i)
            && n == 1
        {
            let mut p = self.clone();
            p.pos += n;
            let amount = p.primary()?;
            if matches!(&amount, Expr::WithUnit(_, unit) if matches!(unit.dim(), Dim::MASS | Dim::VOLUME)) {
                *self = p;
                return Ok(Some(Expr::Food(amount.boxed(), food)));
            }
        }
        // "Mach 2": the number comes after.
        if w == "mach"
            && let Some(Tok::Num(n)) = self.toks.get(i + 1).map(|t| &t.tok)
        {
            self.pos += 2;
            return Ok(Some(Expr::WithUnit(Expr::Number(*n).boxed(), registry().get("mach"))));
        }
        if let Some(expr) = self.weather()? {
            return Ok(Some(expr));
        }
        if let Some(expr) = self.sky()? {
            return Ok(Some(expr));
        }
        if let Some(expr) = self.coordinates()? {
            return Ok(Some(expr));
        }
        if let Some(expr) = self.distance()? {
            return Ok(Some(expr));
        }
        if let Some((hz, n)) = self.note_at(i) {
            self.pos += n;
            return Ok(Some(Expr::WithUnit(Expr::Number(Number::from_f64(hz)).boxed(), registry().get("Hz"))));
        }
        if let Some((zone, n, date_only)) = self.place_time_at(i) {
            self.pos += n;
            let now = Expr::Time(if date_only { TimeExpr::Today(0) } else { TimeExpr::Now });
            return Ok(Some(Expr::Convert(now.boxed(), Target::Zone(zone))));
        }
        if let Some((zone, n)) = self.place_converted_at(i) {
            self.pos += n;
            return Ok(Some(Expr::Convert(Expr::Time(TimeExpr::Now).boxed(), Target::Zone(zone))));
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
        if let Some(degree) = words::ordinal_word(w)
            && self.words_at(i + 1, &["root"])
        {
            self.pos += 1;
            return self.root_of_degree(Number::from_i64(degree));
        }
        // Without arguments "min" may be minutes: "1 BTU·min".
        // A function defined in a line: "area(3, 4)".
        if self.toks.get(i + 1).is_some_and(|t| t.is_sym("(") && !t.space_before) && (self.scope.is_function)(w) {
            self.pos += 2;
            let saved = std::mem::replace(&mut self.in_list, true);
            let mut args = Vec::new();
            while !self.at_sym(")") {
                args.push(self.expr()?);
                if !self.eat_sym(",") {
                    break;
                }
            }
            self.in_list = saved;
            self.eat_sym(")");
            return Ok(Some(Expr::Apply(w.to_string(), args)));
        }
        if let Some((func, n)) = self.function_at(i)
            && let Some(call) = self.call(func, n)?
        {
            return Ok(Some(call));
        }
        if matches!(w, "true" | "false") {
            self.pos += 1;
            return Ok(Some(Expr::Bool(w == "true")));
        }
        if let Some((expr, n)) = self.physical_constant(i) {
            self.pos += n;
            return Ok(Some(expr));
        }
        if let Some((food, n)) = self.density_at(i) {
            self.pos += n;
            return Ok(Some(density(food)));
        }
        if let Some(c) = words::constant(w) {
            self.pos += 1;
            return Ok(Some(Expr::Number(c)));
        }
        if let Some(n) = self.number_words() {
            return self.number(n).map(Some);
        }
        if let Some(n) = words::scale_word(self.toks[i].word().unwrap_or(w)).or_else(|| self.fraction_at(i)) {
            self.pos += 1;
            // "half a dozen", "half a million", "half an hour"
            let article = usize::from(matches!(self.lower(self.pos).as_deref(), Some("a" | "an")));
            if self.fraction_at(i).is_some() {
                if self
                    .toks
                    .get(self.pos + article)
                    .and_then(|t| t.word())
                    .is_some_and(|w| words::scale_word(w).is_some())
                {
                    self.pos += article;
                    return Ok(Some(Expr::Number(self.multipliers(n, false))));
                }
                if article == 1 && self.unit_at(self.pos + 1, true).is_some() {
                    self.pos += 1;
                    return self.quantity(Expr::Number(n)).map(Some);
                }
            }
            return Ok(Some(Expr::Number(n)));
        }
        // "square meter in sq ft"
        if matches!(w, "square" | "sq" | "cubic" | "cu")
            && let Some(unit) = self.unit_phrase(false)
        {
            return Ok(Some(Expr::BareUnit(unit)));
        }
        if let Some((unit, n)) = self.unit_at(i, false) {
            return self.bare_unit(unit, n).map(Some);
        }
        if words::KEYWORDS.contains(&w) || w == "x" {
            return Err(Error::unclear(format!("unexpected \"{}\"", self.toks[i].word().unwrap_or(w))));
        }
        if self.toks.get(i + 1).is_some_and(|t| t.is_sym("(") && !t.space_before) {
            bail!("unknown function \"{}\"", self.toks[i].word().unwrap_or(w));
        }
        Ok(None)
    }

    /// A musical note converted to a frequency at token `i`: "C#4 in Hz", "Bb3 to Hz".
    /// Its frequency in equal temperament from A4 = 440 Hz, and its length in tokens.
    pub(super) fn note_at(&self, i: usize) -> Option<(f64, usize)> {
        let tok = self.toks.get(i)?;
        let word = tok.word()?;
        let semitone = match word.chars().next()? {
            'C' => 0,
            'D' => 2,
            'E' => 4,
            'F' => 5,
            'G' => 7,
            'A' => 9,
            'B' => 11,
            _ => return None,
        };
        let rest = &word[1..];
        // "C#4" and "B♭3" are a letter, a sign and an octave; "Bb3" and "A4" are one word.
        let (shift, octave, n) = if rest.is_empty() {
            let sign = self.src[tok.end..].chars().next()?;
            let shift = match sign {
                '#' | '♯' => 1,
                '♭' => -1,
                _ => return None,
            };
            let octave = self.toks.get(i + 1).filter(|t| t.start == tok.end + sign.len_utf8())?;
            let Tok::Num(octave) = octave.tok else { return None };
            (shift, octave.to_i64().filter(|_| octave.is_integer())?, 2)
        } else {
            let (shift, digits) = match rest.strip_prefix('b') {
                Some(digits) => (-1, digits),
                None => (0, rest),
            };
            (shift, digits.parse::<i64>().ok().filter(|_| digits.bytes().all(|b| b.is_ascii_digit()))?, 1)
        };
        if !(0..=10).contains(&octave) {
            return None;
        }
        // Only before a frequency: "A4 paper" is a paper size.
        let keyword = ["in", "to", "as", "into"].iter().any(|w| self.words_at(i + n, &[w]));
        let hz = registry().get("Hz");
        let frequency = self.unit_at(i + n + 1, true).is_some_and(|(u, _)| u.dim() == hz.dim());
        let note =
            self.phrase_at(i + n + 1, &words::FORMATS).is_some_and(|(f, _)| matches!(f, Format::Midi | Format::Pitch));
        if !keyword || !(frequency || note) {
            return None;
        }
        let midi = (octave + 1) * 12 + semitone + shift;
        Some((440.0 * 2f64.powf((midi - 69) as f64 / 12.0), n))
    }

    /// `prev`, `line 3`, `sum`, `average`.
    fn line_ref(&mut self) -> Result<Option<Expr>> {
        let w = self.lower(self.pos).unwrap_or_default();
        // "sum of above", "total of the above", "average above": the lines above, as "sum" alone.
        if matches!(w.as_str(), "sum" | "total" | "subtotal" | "average" | "avg" | "mean") {
            let mut at = self.pos + 1 + usize::from(self.words_at(self.pos + 1, &["of"]));
            at += usize::from(self.words_at(at, &["the"]));
            if self.words_at(at, &["above"]) && self.toks.get(at + 1).is_none_or(|t| !t.is_word("line")) {
                self.pos = at + 1;
                let average = matches!(w.as_str(), "average" | "avg" | "mean");
                return Ok(Some(Expr::Line(if average { LineRef::Average } else { LineRef::Sum })));
            }
        }
        // "sum of 1, 2", "average(1, 2)", "average 36, 42", "average between 10 and 20" are functions.
        let followed_by_args =
            self.toks.get(self.pos + 1).is_some_and(|t| t.is_word("of") || t.is_word("between") || t.is_sym("("))
                || self.amount_follows(1);
        // A statistic of the lines above, as the whole line: "median", "std dev in EUR".
        if self.pos == 0
            && !followed_by_args
            && let Some((func, n)) = self.line_statistic()
        {
            self.pos += n;
            return Ok(Some(Expr::Line(LineRef::Stat(func))));
        }
        let line = match w.as_str() {
            // "previous line", "previous answer"
            "prev" | "previous"
                if self.toks.get(self.pos + 1).is_some_and(|t| t.is_word("line") || t.is_word("answer")) =>
            {
                self.pos += 1;
                LineRef::Previous
            }
            // "previous monday", "previous month" are dates.
            "previous" if self.date_follows_at(self.pos + 1) => return Ok(None),
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

    /// `median`, `max`, `min`, `count`, `std dev` alone, or before a conversion or a hashtag.
    fn line_statistic(&self) -> Option<(Func, usize)> {
        let (func, n) = [
            ("median", Func::Median),
            ("max", Func::Max),
            ("maximum", Func::Max),
            ("min", Func::Min),
            ("minimum", Func::Min),
            ("count", Func::Count),
        ]
        .iter()
        .find(|(w, _)| self.words_at(self.pos, &[w]))
        .map(|&(_, f)| (f, 1))
        .or_else(|| {
            [&["std", "dev"][..], &["stdev"], &["stddev"], &["standard", "deviation"]]
                .iter()
                .find(|ws| self.words_at(self.pos, ws))
                .map(|ws| (Func::StdDev, ws.len()))
        })?;
        let ends = match self.toks.get(self.pos + n) {
            None => true,
            Some(t) => ["in", "to", "as"].iter().any(|w| t.is_word(w)) || t.is_sym("#"),
        };
        ends.then_some((func, n))
    }

    /// "seven", "twenty five", "one hundred and twenty", "two thousand and five".
    /// A scale word that ends the number is left to `multipliers` ("two thousand" = 2k).
    fn number_words(&mut self) -> Option<Number> {
        let (mut group, mut at) = self.hundreds_at(self.pos)?;
        let mut total = 0;
        loop {
            let scale = match self.lower(at).as_deref() {
                Some("thousand") => 1_000,
                Some("million") => 1_000_000,
                Some("billion") => 1_000_000_000,
                Some("trillion") => 1_000_000_000_000,
                _ => break,
            };
            let Some((next, end)) = self.number_word_after_and(at + 1) else {
                // "three million two hundred thousand"; a scale ending the number alone stays.
                if total > 0 {
                    total += group * scale;
                    group = 0;
                    at += 1;
                }
                break;
            };
            total += group * scale;
            group = next;
            at = end;
        }
        self.pos = at;
        Some(Number::from_i64(total + group))
    }

    /// A number below a thousand in words at token `i`: "seven", "twenty-five",
    /// "one hundred and twenty". Returns it and the token after it.
    fn hundreds_at(&self, i: usize) -> Option<(i64, usize)> {
        let (mut n, mut at) = self.tens_at(i)?;
        if n < 10 && self.words_at(at, &["hundred"]) {
            n *= 100;
            at += 1;
            if let Some((rest, end)) = self.number_word_after_and(at).filter(|(rest, _)| *rest < 100) {
                n += rest;
                at = end;
            }
        }
        Some((n, at))
    }

    /// "and five" or "five" at token `i`, as a number below a thousand.
    fn number_word_after_and(&self, i: usize) -> Option<(i64, usize)> {
        let i = if self.words_at(i, &["and"]) { i + 1 } else { i };
        self.hundreds_at(i)
    }

    /// "seven", "twenty five", "twenty-five" at token `i`.
    fn tens_at(&self, i: usize) -> Option<(i64, usize)> {
        let first = words::number_word(&self.lower(i)?)?;
        let next = i + 1;
        let hyphen = self.toks.get(next).is_some_and(|t| t.is_sym("-") && !t.space_before)
            && self.toks.get(next + 1).is_some_and(|t| !t.space_before);
        let at = next + usize::from(hyphen);
        if first >= 20
            && first % 10 == 0
            && let Some(ones) = self.lower(at).and_then(|w| words::number_word(&w)).filter(|n| (1..10).contains(n))
        {
            return Some((first + ones, at + 1));
        }
        Some((first, next))
    }

    fn bare_unit(&mut self, unit: Unit, n: usize) -> Result<Expr> {
        // "whole weeks between March 3 and May 30" counts complete weeks only.
        let whole = self.pos > 0 && self.toks[self.pos - 1].is_word("whole");
        let per = self.per_unit_in(self.pos, n);
        self.pos += n;
        let unit = match per {
            Some(per) => self.per_unit_power(unit, Some(&per)),
            None => unit,
        };
        if (unit.dim() == Dim::TIME || unit.dim() == Dim::WORKDAY)
            && let Some(expr) = self.time_unit_phrase(&unit)?
        {
            return Ok(if whole { Expr::Round(expr.boxed(), Rounding::Places(0, Direction::Down)) } else { expr });
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
        // "random number from 1 to 5", "random number 1 through 5"
        if func == Func::Random
            && let Some(range) = self.random_range()
        {
            return Ok(Some(Expr::Call(func, range)));
        }
        // "random" alone, "root 3 of 27", "log 20 base 4"
        let has_args = self.at_sym("(") || self.at_word("of") || self.at_word("between") || self.operand_follows(0);
        if !has_args {
            if func == Func::Random {
                return Ok(Some(Expr::Call(func, vec![])));
            }
            self.pos = start;
            return Ok(None);
        }
        // "round up 2.5", "round 56 down to nearest 10"
        if func == Func::Round && !self.at_sym("(") {
            let dir = self.direction();
            let x = self.unary()?;
            let dir = if dir == Direction::Nearest { self.direction() } else { dir };
            if dir != Direction::Nearest || self.at_word("to") {
                return self.rounded_to(&x, dir).map(Some);
            }
            return Ok(Some(Expr::Call(func, vec![x])));
        }
        if func == Func::Root && !self.at_sym("(") {
            // "root of 16" is the square root.
            if self.eat_word("of") {
                return Ok(Some(Expr::Call(Func::Sqrt, vec![self.unary()?])));
            }
            let degree = self.unary()?;
            self.eat_word("of");
            let x = self.unary()?;
            return Ok(Some(Expr::Call(func, vec![degree, x])));
        }
        // "log base 2 of 8"
        if func == Func::Log && self.eat_word("base") {
            let base = self.unary()?;
            self.eat_word("of");
            return Ok(Some(Expr::Call(func, vec![self.unary()?, base])));
        }
        let mut args = if self.eat_sym("(") {
            let saved = std::mem::replace(&mut self.in_list, true);
            let mut args = Vec::new();
            while !self.at_sym(")") && self.peek().is_some() {
                args.push(self.expr()?);
                if !self.eat_sym(",") {
                    break;
                }
            }
            self.in_list = saved;
            self.eat_sym(")");
            args
        } else if self.eat_word("of") || self.eat_word("between") {
            self.list()?
        } else if func.takes_list() && self.list_follows() {
            // "average 36, 42, 19, 81"
            self.list()?
        } else {
            let first = self.unary()?;
            // "log 4 of 20": the base comes first, like the degree of a root.
            if func == Func::Log && self.eat_word("of") { vec![self.unary()?, first] } else { vec![first] }
        };
        // "log 20 base 4", "logarithm of 20 to base 4"
        if func == Func::Log && self.at_word("to") && self.words_at(self.pos + 1, &["base"]) {
            self.pos += 1;
        }
        if func == Func::Log && self.eat_word("base") {
            args.push(self.unary()?);
        }
        // "clamp 26 between 5 and 25", "clamp 4 from 5 to 25", "clamp 20 5 25"
        if func == Func::Clamp && args.len() == 1 {
            if self.eat_word("between") {
                args.extend(self.list()?);
            } else if self.eat_word("from") {
                args.push(self.bit_or()?);
                self.eat_word("to");
                args.push(self.bit_or()?);
            } else if self.operand_follows(0) {
                args.push(self.unary()?);
                args.push(self.unary()?);
            }
        }
        Ok(Some(Expr::Call(func, args)))
    }

    /// The ends of a range after "random": "from 1 to 5", "1 through 5", "1-5".
    fn random_range(&mut self) -> Option<Vec<Expr>> {
        let at = self.pos + usize::from(self.toks.get(self.pos).is_some_and(|t| t.is_word("from")));
        let number = |i: usize| match self.toks.get(i)?.tok {
            Tok::Num(n) => Some(Expr::Number(n)),
            _ => None,
        };
        let joined = self
            .toks
            .get(at + 1)
            .is_some_and(|t| ["to", "through", "thru"].iter().any(|w| t.is_word(w)) || t.is_sym("-"));
        let (low, high) = (number(at)?, number(at + 2)?);
        if !joined {
            return None;
        }
        self.pos = at + 3;
        Some(vec![low, high])
    }

    /// A root after its degree: "4th root of 16", "fourth root of 16", "5 root of 100".
    fn root_of_degree(&mut self, degree: Number) -> Result<Option<Expr>> {
        let mut p = self.clone();
        p.eat_ordinal();
        if !p.cur().is_some_and(|t| t.is_word("root")) || degree.to_i64().is_none_or(|d| d < 2) {
            return Ok(None);
        }
        p.pos += 1;
        p.eat_word("of");
        if !p.operand_follows(0) {
            return Ok(None);
        }
        let x = p.unary()?;
        *self = p;
        Ok(Some(Expr::Call(Func::Root, vec![Expr::Number(degree), x])))
    }

    /// A list of items starts at the cursor: `36, 42` or `3 and 7`.
    fn list_follows(&self) -> bool {
        let mut p = self.clone();
        p.in_list = true;
        p.bit_or().is_ok() && (p.at_sym(",") || p.at_word("and"))
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
        let (index, n) = self.phrase_at(i, &words::PHYSICAL_CONSTANTS)?;
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
        let Some((format, n)) = self.phrase_at(self.pos, &words::DATE_PARTS) else { return Ok(None) };
        self.pos += n;
        let date = if self.eat_word("on") || self.eat_word("of") || self.eat_word("for") || self.date_starts(self.pos) {
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

/// A food's weight per volume, shown in g/cm³.
fn density(food: Food) -> Expr {
    let reg = registry();
    let per_cup = reg.get("g").product(&reg.get("cup").pow(-1));
    let per_cm3 = reg.get("g").product(&reg.get("cm").pow(-3));
    let amount = Expr::WithUnit(Expr::Number(food.grams_per_cup).boxed(), per_cup);
    Expr::Convert(amount.boxed(), Target::Unit(per_cm3))
}
