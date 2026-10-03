//! Conversions (`in km`, `as hex`, `to Tokyo`) and rounding (`to 2 dp`).

use super::{Parser, words};
use crate::ast::{Direction, Expr, Format, Func, Op, Rounding, Sky, Target, TimeExpr};
use crate::error::{Result, bail};
use crate::lexer::{Tok, Token};
use crate::number::Number;
use crate::units::{Dim, Unit, registry};
use crate::zones;

impl Parser<'_> {
    /// One conversion step after `expr`, or `None` if there is none.
    pub(super) fn conversion(&mut self, expr: &Expr) -> Result<Option<Expr>> {
        let start = self.pos;
        if self.eat_rounding_word() {
            let dir = self.direction();
            return self.rounded_to(expr, dir).map(Some);
        }
        // "March 3 through March 10"
        if self.range_word(&[]).is_some() {
            return Ok(Some(Expr::Range(expr.clone().boxed(), self.range_end(true)?.boxed())));
        }
        let Some(tok) = self.peek() else { return Ok(None) };
        let keyword = match &tok.tok {
            Tok::Sym("->") => "->".to_string(),
            Tok::Word(w) => w.to_lowercase(),
            _ => return Ok(None),
        };
        if !matches!(keyword.as_str(), "in" | "to" | "as" | "into" | "->") {
            return Ok(None);
        }
        self.pos += 1;

        // "meters in 10 km", "seconds in a day": how many of the unit fit.
        if let (Expr::BareUnit(unit), "in") = (expr, keyword.as_str())
            && !unit.is_money()
            && self.operand_follows(0)
        {
            let amount = self.bit_or()?;
            return Ok(Some(Expr::Convert(amount.boxed(), Target::Unit(unit.clone()))));
        }
        if keyword == "to"
            && let Some((rounding, format)) = self.rounding(Direction::Nearest)?
        {
            return Ok(Some(rounded(expr, rounding, format)));
        }
        // "time in Tokyo in 3 hours", "time in 3 hours": the time that much later.
        if keyword == "in"
            && is_moment(expr)
            && let Some(amount) = self.time_amount()
        {
            return Ok(Some(later(expr, amount)));
        }
        // A date or time goes to a place, even one named like a unit: "time in Cordoba".
        let zone = match is_moment(expr) {
            true => self.zone_at(self.pos).filter(|&(_, n)| self.unit_at(self.pos, false).is_none_or(|(_, m)| m <= n)),
            false => None,
        };
        let target = match zone {
            Some((zone, n)) => {
                self.pos += n;
                Some(Target::Zone(zone))
            }
            None => self.target(),
        };
        if let Some(target) = target {
            // "9am in New York to Tokyo": the first zone says where the clock is.
            if let Target::Zone(zone) = &target
                && matches!(expr, Expr::Time(TimeExpr::Clock { .. }))
                && self.zone_conversion_follows()
            {
                return Ok(Some(Expr::InZone(expr.clone().boxed(), zone.clone())));
            }
            return Ok(Some(Expr::Convert(expr.clone().boxed(), target)));
        }
        // "9am to 5pm", "March 1 to June 1".
        if keyword == "to" && self.operand_follows(0) {
            let end = self.bit_or()?;
            return Ok(Some(Expr::Range(expr.clone().boxed(), end.boxed())));
        }
        // "time in Qwertyville": a time zone was asked for, but the place is unknown.
        // After "to" only a clock time asks for a zone: "time to go" does not.
        let asks_zone = if keyword == "to" { is_clock(expr) } else { is_moment(expr) };
        if asks_zone {
            self.unknown_place()?;
        }
        self.pos = start;
        Ok(None)
    }

    /// Fails with a helpful message if words that aren't a known place follow.
    pub(super) fn unknown_place(&self) -> Result<()> {
        let mut i = self.pos;
        while self.lower(i).is_some_and(|w| matches!(w.as_str(), "a" | "an" | "the")) {
            i += 1;
        }
        let (words, _) = self.name_words(i);
        let count = (0..words.len()).take_while(|&k| !self.significant(i + k)).count().min(3);
        if count == 0 {
            return Ok(());
        }
        let typed: Vec<&str> = self.toks[i..].iter().filter_map(Token::word).take(count).collect();
        let typed = typed.join(" ");
        match zones::suggest(&words[..count].join(" ")) {
            Some(hint) => bail!("unknown place \"{typed}\", did you mean \"{hint}\"?"),
            None => bail!("unknown place \"{typed}\""),
        }
    }

    /// An amount of time at the cursor, read past: "3 hours", "an hour", "1 hour 30 min".
    fn time_amount(&mut self) -> Option<Expr> {
        // Right here: "time in Tokyo in 3 hours" is not "in 3 hours" after the word "Tokyo".
        if !self.cur().is_some_and(|t| matches!(t.tok, Tok::Num(_)) || self.significant(self.pos)) {
            return None;
        }
        let mut p = self.clone();
        let amount = p.multiplicative().ok()?;
        let unit = match &amount {
            Expr::BareUnit(unit) | Expr::WithUnit(_, unit) => Some(unit),
            Expr::Composite(parts) => parts.last().and_then(|e| e.edge_unit(true)),
            _ => None,
        };
        unit.filter(|u| u.dim() == Dim::TIME)?;
        *self = p;
        Some(amount)
    }

    fn zone_conversion_follows(&self) -> bool {
        let mut p = self.clone();
        p.skip_noise();
        let is_keyword = p.cur().is_some_and(|t| t.is_sym("->") || ["in", "to", "into"].iter().any(|w| t.is_word(w)));
        is_keyword && p.zone_at(p.pos + 1).is_some()
    }

    /// "rounded", or "round" before a direction or "to": "37 round up to nearest 10".
    fn eat_rounding_word(&mut self) -> bool {
        if self.eat_word("rounded") {
            return true;
        }
        let round = self.at_word("round") && matches!(self.lower(self.pos + 1).as_deref(), Some("up" | "down" | "to"));
        if round {
            self.pos += 1;
        }
        round
    }

    pub(super) fn direction(&mut self) -> Direction {
        if self.eat_word("up") {
            Direction::Up
        } else if self.eat_word("down") {
            Direction::Down
        } else {
            Direction::Nearest
        }
    }

    /// Rounds `expr` in a direction, then to what follows: "rounded down to 2 dp",
    /// "rounded 3 dp", "rounded down to 10".
    pub(super) fn rounded_to(&mut self, expr: &Expr, dir: Direction) -> Result<Expr> {
        let start = self.pos;
        let to = self.eat_word("to");
        let (rounding, format) = match self.rounding(dir)? {
            Some(rounding) => rounding,
            None => match self.cur().map(|t| &t.tok) {
                Some(Tok::Num(n)) if to => {
                    let n = *n;
                    self.pos += 1;
                    (Rounding::Multiple(n, dir), None)
                }
                // "rounded to EUR" converts after rounding.
                _ if to && self.clone().target().is_some() => {
                    self.pos = start;
                    (Rounding::Places(0, dir), None)
                }
                _ => (Rounding::Places(0, dir), None),
            },
        };
        Ok(rounded(expr, rounding, format))
    }

    /// `2 dp`, `3 decimal places`, `4 sf`, `nearest 10`, `nearest thousand`, `nearest 16th`.
    fn rounding(&mut self, dir: Direction) -> Result<Option<(Rounding, Option<Format>)>> {
        let start = self.pos;
        self.eat_word("the");
        // "up to next 10", "down to last 10"; "to next 10" rounds up.
        let dir = match self.lower(self.pos).as_deref() {
            Some("next") if dir == Direction::Nearest => Direction::Up,
            Some("last") if dir == Direction::Nearest => Direction::Down,
            _ => dir,
        };
        if self.eat_word("nearest") || self.eat_word("next") || self.eat_word("last") {
            let Some(tok) = self.cur() else {
                self.pos = start;
                return Ok(None);
            };
            let multiple = match &tok.tok {
                Tok::Num(n) => {
                    self.pos += 1;
                    // "nearest 16th" rounds to sixteenths.
                    if self.eat_ordinal() {
                        return Ok(Some((Rounding::Multiple(Number::ONE / *n, dir), Some(Format::Fraction))));
                    }
                    *n * self.scale_words()
                }
                Tok::Word(w) => {
                    let w = w.to_lowercase();
                    match words::number_word(&w).map(Number::from_i64).or_else(|| words::scale_word(&w)) {
                        Some(n) => {
                            self.pos += 1;
                            n * self.scale_words()
                        }
                        None if matches!(w.as_str(), "whole" | "integer") => {
                            self.pos += 1;
                            self.eat_word("number");
                            Number::ONE
                        }
                        None => {
                            self.pos = start;
                            return Ok(None);
                        }
                    }
                }
                _ => {
                    self.pos = start;
                    return Ok(None);
                }
            };
            return Ok(Some((Rounding::Multiple(multiple, dir), None)));
        }
        let Some(Tok::Num(n)) = self.cur().map(|t| &t.tok) else {
            self.pos = start;
            return Ok(None);
        };
        let Some(count) = n.to_i64().filter(|n| (0..=28).contains(n)) else {
            self.pos = start;
            return Ok(None);
        };
        self.pos += 1;
        let w = self.lower(self.pos).unwrap_or_default();
        let significant = match w.as_str() {
            "dp" | "decimals" | "places" | "digits" => false,
            "decimal" => {
                self.pos += 1;
                self.eat_word("places");
                self.eat_word("place");
                return Ok(Some((Rounding::Places(count as u32, dir), None)));
            }
            "sf" | "sig" | "significant" => true,
            _ => {
                self.pos = start;
                return Ok(None);
            }
        };
        self.pos += 1;
        if significant {
            for w in ["figs", "figures", "figure", "digits"] {
                self.eat_word(w);
            }
            return Ok(Some((Rounding::Significant(count as u32), None)));
        }
        Ok(Some((Rounding::Places(count as u32, dir), None)))
    }

    /// Scale words after a number: "nearest 5 thousand".
    fn scale_words(&mut self) -> Number {
        let mut n = Number::ONE;
        while let Some(s) = self.lower(self.pos).and_then(|w| words::scale_word(&w)) {
            n = n * s;
            self.pos += 1;
        }
        n
    }

    /// What to convert into: a unit, several units, a format or a time zone.
    pub(super) fn target(&mut self) -> Option<Target> {
        let start = self.pos;
        // "in a day"; but "in A" alone is amperes.
        while matches!(self.lower(self.pos).as_deref(), Some("a" | "an" | "the"))
            && self.toks.get(self.pos + 1).is_some_and(|t| t.word().is_some())
        {
            self.pos += 1;
        }
        if self.cur().is_some_and(|t| t.is_sym("%")) {
            self.pos += 1;
            return Some(Target::Format(Format::Percent));
        }
        if self.cur().is_some_and(|t| t.is_word("x")) && !(self.scope.is_var)("x") {
            self.pos += 1;
            return Some(Target::Format(Format::Multiplier));
        }
        if self.cur().is_some_and(|t| t.is_word("per") || t.is_sym("/")) {
            let mut p = self.clone();
            p.pos += 1;
            if let Some(period) = p.unit_expr(false) {
                *self = p;
                return Some(Target::Per(period));
            }
        }
        if let Some((format, n)) = self.phrase_at(self.pos, &words::FORMATS) {
            // "in dec" is decimal, but "in days" stays a unit.
            if self.unit_at(self.pos, false).is_none_or(|(_, m)| m < n) {
                self.pos += n;
                return Some(Target::Format(format));
            }
        }
        if let Some(unit) = self.unit_expr(true) {
            let mut units = vec![unit];
            loop {
                let mut p = self.clone();
                if !(p.eat_word("and") || p.eat_sym(",")) {
                    break;
                }
                let Some(unit) = p.unit_expr(true) else { break };
                *self = p;
                units.push(unit);
            }
            return Some(if units.len() == 1 { Target::Unit(units.pop().unwrap()) } else { Target::Units(units) });
        }
        if matches!(self.lower(self.pos).as_deref(), Some("local" | "here")) {
            self.pos += 1;
            self.eat_word("time");
            return Some(Target::Zone(self.config().local_zone()));
        }
        if let Some((zone, n)) = self.zone_at(self.pos) {
            self.pos += n;
            return Some(Target::Zone(zone));
        }
        self.pos = start;
        None
    }

    /// A unit expression such as `km/h`, `$/hour`, `kg*m/s^2` or `cubic feet`.
    fn unit_expr(&mut self, allow_inch: bool) -> Option<Unit> {
        let mut unit = self.single_target_unit(allow_inch)?;
        loop {
            let divide = match self.cur().map(|t| &t.tok) {
                Some(Tok::Sym("/")) => true,
                Some(Tok::Sym("*")) => false,
                Some(Tok::Word(w)) if w.eq_ignore_ascii_case("per") => true,
                _ => break,
            };
            let mut p = self.clone();
            p.pos += 1;
            let Some(next) = p.single_target_unit(false) else { break };
            *self = p;
            unit = unit.product(&if divide { next.pow(-1) } else { next });
        }
        Some(unit)
    }

    fn single_target_unit(&mut self, allow_inch: bool) -> Option<Unit> {
        if allow_inch && self.cur().is_some_and(|t| t.is_word("in")) {
            self.pos += 1;
            return Some(registry().get("in"));
        }
        // Articles are skipped already: "A" here is amperes.
        self.unit_phrase(true)
    }
}

/// A date or time, or a time zone conversion of one.
fn is_moment(expr: &Expr) -> bool {
    match expr {
        Expr::Time(_) | Expr::InZone(..) => true,
        // A moment moved by a time: "now + 3 hours".
        Expr::Binary(Op::Add, a, b) => is_moment(a) != is_moment(b),
        Expr::Binary(Op::Sub, a, b) => is_moment(a) && !is_moment(b),
        Expr::Sky { what, .. } => matches!(what, Sky::Sunrise | Sky::Sunset),
        Expr::Convert(inner, Target::Zone(_)) => is_moment(inner),
        _ => false,
    }
}

/// `expr` moved by `amount`, still shown in its zone: "time in Tokyo in 3 hours".
fn later(expr: &Expr, amount: Expr) -> Expr {
    match expr {
        Expr::Convert(inner, Target::Zone(zone)) => {
            Expr::Convert(later(inner, amount).boxed(), Target::Zone(zone.clone()))
        }
        other => Expr::binary(Op::Add, other.clone(), amount),
    }
}

/// A written clock time like `3pm`, possibly placed in a zone.
pub(super) fn is_clock(expr: &Expr) -> bool {
    match expr {
        Expr::Time(TimeExpr::Clock { .. }) => true,
        Expr::InZone(inner, _) | Expr::Convert(inner, Target::Zone(_)) => is_clock(inner),
        _ => false,
    }
}

/// Applies rounding; `round x to 2 dp` rounds `x` only once.
fn rounded(expr: &Expr, rounding: Rounding, format: Option<Format>) -> Expr {
    let inner = match expr {
        Expr::Call(Func::Round, args) if args.len() == 1 => args[0].clone(),
        other => other.clone(),
    };
    let expr = Expr::Round(inner.boxed(), rounding);
    match format {
        Some(f) => Expr::Convert(expr.boxed(), Target::Format(f)),
        None => expr,
    }
}
