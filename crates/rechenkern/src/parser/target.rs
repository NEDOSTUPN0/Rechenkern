//! Conversions (`in km`, `as hex`, `to Tokyo`) and rounding (`to 2 dp`).

use super::{Parser, words};
use crate::ast::{Direction, Expr, Format, Func, Op, Rounding, Sky, Target, TimeExpr};
use crate::config::Converter;
use crate::error::{Result, bail};
use crate::lexer::{Tok, Token};
use crate::number::Number;
use crate::units::{Dim, Unit, registry};
use crate::zones::{self, Location, Place, PlaceKind};

impl Parser<'_> {
    /// One conversion step after `expr`, or `None` if there is none.
    pub(super) fn conversion(&mut self, expr: &Expr) -> Result<Option<Expr>> {
        let start = self.pos;
        // "2 hours to 1.5x", "2 hours to 1.5x speed": played at that speed.
        if self.cur().is_some_and(|t| t.is_word("to"))
            && let Some(Tok::Num(speed)) = self.toks.get(self.pos + 1).map(|t| &t.tok)
            && self.toks.get(self.pos + 2).is_some_and(|t| t.word() == Some("x") && !t.space_before)
            && let speed_word = usize::from(self.toks.get(self.pos + 3).is_some_and(|t| t.is_word("speed")))
            && self.toks.len() == self.pos + 3 + speed_word
        {
            self.pos += 3 + speed_word;
            return Ok(Some(Expr::binary(Op::At, expr.clone(), Expr::Number(*speed))));
        }
        if self.converter_allowed(Converter::Rounding) && self.eat_rounding_word() {
            let dir = self.direction();
            return self.rounded_to(expr, dir).map(Some);
        }
        // "time in Tokyo 3 hours ago": that much earlier, still shown in Tokyo.
        if is_moment(expr)
            && let Some((amount, op)) = self.time_shift()
        {
            return Ok(Some(shifted(expr, op, amount)));
        }
        // "time in Tokyo after 3 weeks"
        if is_moment(expr) && self.words_at(self.pos, &["after"]) {
            let mut p = self.clone();
            p.pos += 1;
            if let Some(amount) = p.time_amount() {
                *self = p;
                return Ok(Some(shifted(expr, Op::Add, amount)));
            }
        }
        // "March 9, 2024 day of the week"
        if is_moment(expr)
            && self.converter_allowed(Converter::DatePart)
            && let Some((format, n)) = self.phrase_at(self.pos, &words::DATE_PARTS)
        {
            self.pos += n;
            return Ok(Some(Expr::Convert(expr.clone().boxed(), Target::Format(format))));
        }
        // "March 3 through March 10", "Jan 1 until Mar 1"
        let until: &[&str] = if is_moment(expr) { &["until", "till"] } else { &[] };
        if let Some(through) = self.range_word(until) {
            return Ok(Some(Expr::Range(expr.clone().boxed(), self.range_end(through)?.boxed())));
        }
        // "5 km cast to miles": the number kept, the unit changed.
        if self.words_at(self.pos, &["cast"])
            && ["to", "as", "into", "in"].iter().any(|w| self.words_at(self.pos + 1, &[w]))
        {
            let mut p = self.clone();
            p.pos += 2;
            if let Some(Target::Unit(unit)) = p.target() {
                *self = p;
                return Ok(Some(Expr::Convert(expr.clone().boxed(), Target::Cast(unit))));
            }
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
        // "5:30/km in /mile": a pace in minutes per another length.
        if self.cur().is_some_and(|t| t.is_sym("/"))
            && expr.edge_unit(true).is_some_and(|u| u.dim() == Dim::TIME.div(Dim::LENGTH))
            && let Some((length, n)) = self.unit_at(self.pos + 1, true).filter(|(u, _)| u.dim() == Dim::LENGTH)
        {
            self.pos += 1 + n;
            let pace = registry().get("min").product(&length.pow(-1));
            return Ok(Some(Expr::Convert(expr.clone().boxed(), Target::Unit(pace))));
        }
        // "3pm in 24 hour", "15:45 to 12-hour time": the clock it is shown on; "in 24 hours" moves it.
        if is_moment(expr)
            && let Some((h24, n)) = self.clock_style_at(self.pos)
        {
            self.pos += n;
            return Ok(Some(Parser::formatted(expr.clone(), Format::Clock(h24))));
        }
        // "today as yyyy-MM-dd", "now as HH:mm": the rest of the line is the template.
        if is_moment(expr)
            && let Some(first) = self.toks.get(self.pos)
        {
            let rest = self.src[first.start..self.toks[self.toks.len() - 1].end].trim();
            if is_date_pattern(rest) {
                self.pos = self.toks.len();
                return Ok(Some(Expr::Convert(expr.clone().boxed(), Target::Pattern(rest.to_string()))));
            }
        }

        // "meters in 10 km", "seconds in a day": how many of the unit fit.
        if let (Expr::BareUnit(unit), "in") = (expr, keyword.as_str())
            && !unit.is_money()
            && self.operand_follows(0)
        {
            // "square meters in sq ft": one of a unit, which may have a power.
            let amount = match self.unit_expr(false) {
                Some(of) => Expr::BareUnit(of),
                None => self.bit_or()?,
            };
            return Ok(Some(Expr::Convert(amount.boxed(), Target::Unit(unit.clone()))));
        }
        if keyword == "to"
            && self.converter_allowed(Converter::Rounding)
            && let Some((rounding, format)) = self.rounding(Direction::Nearest)?
        {
            return Ok(Some(rounded(expr, rounding, format)));
        }
        // "10 km in 50 minutes": a pace.
        if keyword == "in"
            && expr.edge_unit(true).is_some_and(|u| u.dim() == Dim::LENGTH)
            && let Some(amount) = self.time_amount()
        {
            return Ok(Some(Expr::Pace(expr.clone().boxed(), amount.boxed())));
        }
        // "time in Tokyo in 3 hours", "time in 3 hours": the time that much later.
        if keyword == "in"
            && is_moment(expr)
            && let Some(amount) = self.time_amount()
        {
            return Ok(Some(shifted(expr, Op::Add, amount)));
        }
        // "Q3 2027 in Tokyo": a period is the same days anywhere.
        if matches!(expr, Expr::Period(..))
            && let Some((_, n)) = self.zone_at(self.pos)
        {
            self.pos += n;
            return Ok(Some(expr.clone()));
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
        let target = match target {
            Some(target) if !self.converter_allowed(converter(&target)) => {
                self.pos = start + 1;
                None
            }
            target => target,
        };
        if let Some(target) = target {
            // "5 minutes 20 seconds as frames @ 24 fps": that many seconds of frames.
            if let Target::Format(Format::Frames) = &target
                && let Some(fps) = self.fps_after()
            {
                let seconds = Expr::Convert(expr.clone().boxed(), Target::Unit(registry().get("s")));
                let frames = Expr::binary(Op::Mul, Parser::formatted(seconds, Format::Number), Expr::Number(fps));
                let whole = Expr::Round(frames.boxed(), Rounding::Places(0, Direction::Nearest));
                return Ok(Some(Parser::formatted(whole, Format::Frames)));
            }
            // "27 inch in px with 326 ppi": pixels at that density.
            if let Target::Unit(unit) = &target
                && unit.same_as(&registry().get("px"))
                && let Some(density) = self.density_after()?
            {
                let inches = Expr::Convert(expr.clone().boxed(), Target::Unit(registry().get("in")));
                let dots = Parser::formatted(
                    Expr::Convert(density.boxed(), Target::Unit(registry().get("ppi"))),
                    Format::Number,
                );
                let pixels = Expr::binary(Op::Mul, Parser::formatted(inches, Format::Number), dots);
                return Ok(Some(Expr::WithUnit(pixels.boxed(), unit.clone())));
            }
            // "9am in New York to Tokyo": the first zone says where the clock is.
            if let Target::Zone(zone) = &target
                && matches!(expr, Expr::Time(TimeExpr::Clock { .. }))
                && self.zone_conversion_follows()
            {
                return Ok(Some(Expr::InZone(expr.clone().boxed(), zone.clone())));
            }
            if let Target::Unit(unit) = &target
                && let Some(converted) = self.at_density(expr, unit)
            {
                return Ok(Some(converted));
            }
            if let Target::Unit(unit) = &target
                && unit.is_money()
                && let Some(converted) = self.at_rate(expr, unit)?
            {
                return Ok(Some(converted));
            }
            // "$100 in EUR on Jan 1, 2020", "$100 in EUR 2020-01-01": that day's rate.
            if let Target::Unit(unit) = &target
                && unit.is_money()
                && let Some(date) = self.rates_date()?
            {
                return Ok(Some(Expr::RatesOn(Expr::Convert(expr.clone().boxed(), target).boxed(), date.boxed())));
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
        if asks_zone && let Some((place, _, n)) = self.found_place(self.pos) {
            self.pos += n;
            return Ok(Some(Expr::Convert(expr.clone().boxed(), Target::Zone(place))));
        }
        if asks_zone {
            self.unknown_place()?;
        }
        self.pos = start;
        Ok(None)
    }

    /// "5 cm to px at 300 ppi", "1000 px in inches at 72 dpi": pixels at a density given.
    fn at_density(&mut self, expr: &Expr, to: &Unit) -> Option<Expr> {
        let reg = registry();
        let (px, ppi, inch) = (reg.get("px"), reg.get("ppi"), reg.get("in"));
        let mut p = self.clone();
        if !(p.eat_word("at") || p.eat_sym("@")) || !p.peek().is_some_and(|t| matches!(t.tok, Tok::Num(_))) {
            return None;
        }
        let density = p.unary().ok().filter(|d| d.edge_unit(true) == Some(&ppi))?;
        let number = |e: Expr, unit: &Unit| {
            Self::formatted(Expr::Convert(e.boxed(), Target::Unit(unit.clone())), Format::Number)
        };
        let dots = number(density, &ppi);
        let from_px = expr.edge_unit(true) == Some(&px);
        let converted = if *to == px && !from_px {
            Expr::WithUnit(Expr::binary(Op::Mul, number(expr.clone(), &inch), dots).boxed(), px)
        } else if from_px && *to != px && to.dim() == Dim::LENGTH {
            let inches = Expr::WithUnit(Expr::binary(Op::Div, number(expr.clone(), &px), dots).boxed(), inch);
            Expr::Convert(inches.boxed(), Target::Unit(to.clone()))
        } else {
            return None;
        };
        *self = p;
        Some(converted)
    }

    /// "$100 in EUR at 0.9", "100 AUD as USD at 1.50 AUD/USD": converted at the rate given.
    fn at_rate(&mut self, expr: &Expr, to: &Unit) -> Result<Option<Expr>> {
        let mut p = self.clone();
        if !(p.eat_word("at") || p.eat_sym("@")) || !p.peek().is_some_and(|t| matches!(t.tok, Tok::Num(_))) {
            return Ok(None);
        }
        let (rate, top, bottom) = match p.unary()? {
            rate @ Expr::Number(_) => (rate, None, None),
            Expr::Binary(Op::Per, a, b) => match (*a, *b) {
                (Expr::WithUnit(rate, top), Expr::BareUnit(bottom)) => (*rate, Some(top), Some(bottom)),
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        // A rate with units says which way it goes: 1.50 AUD/USD is AUD per USD.
        let from = expr.edge_unit(true).filter(|u| u.is_money());
        let divide = match (top, bottom) {
            (Some(top), Some(bottom)) if top == *to && from.is_none_or(|f| *f == bottom) => false,
            (Some(top), Some(bottom)) if bottom == *to && from.is_none_or(|f| *f == top) => true,
            (None, None) => false,
            _ => bail!("the rate doesn't convert between these currencies"),
        };
        *self = p;
        let amount = Expr::Convert(expr.clone().boxed(), Target::Format(Format::Number));
        let op = if divide { Op::Div } else { Op::Mul };
        Ok(Some(Expr::WithUnit(Expr::binary(op, amount, rate).boxed(), to.clone())))
    }

    /// Fails with a helpful message if words that aren't a known place follow.
    /// A place the host's provider finds for the words at `i` (after an article), longest
    /// first, with where it is and its length in tokens: "Smallville" in "time in Smallville".
    pub(super) fn found_place(&self, i: usize) -> Option<(Place, Option<Location>, usize)> {
        if !self.config().features.dynamic_place_queries {
            return None;
        }
        let mut start = i;
        while self.lower(start).is_some_and(|w| matches!(w.as_str(), "a" | "an" | "the")) {
            start += 1;
        }
        let (words, ends) = self.name_words(start);
        let count = (0..words.len()).take_while(|&k| !self.significant(start + k)).count().min(3);
        (1..=count).rev().find_map(|n| {
            let end = ends[n - 1];
            let name = &self.src[self.toks[start].start..self.toks[end - 1].end];
            let found = (self.scope.places)(name)?;
            let location = found.coordinates.map(|(latitude, longitude)| Location {
                zone: found.zone.clone(),
                latitude,
                longitude,
            });
            Some((Place { name: found.name, zone: found.zone, kind: PlaceKind::City }, location, end - i))
        })
    }

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
        // A number or an article right here: "time in Tokyo in 3 hours" is not
        // "in 3 hours" after the word "Tokyo".
        let starts = self.cur().is_some_and(|t| match &t.tok {
            Tok::Num(_) => true,
            Tok::Word(w) => {
                let w = w.to_lowercase();
                matches!(w.as_str(), "a" | "an") || words::number_word(&w).is_some()
            }
            _ => false,
        });
        if !starts {
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

    /// An amount of time and `ago` or `later` at the cursor, read past: "3 hours ago".
    fn time_shift(&mut self) -> Option<(Expr, Op)> {
        let end = (self.pos..self.toks.len()).find(|&i| self.words_at(i, &["ago"]) || self.words_at(i, &["later"]))?;
        let mut amount = self.sub(self.pos, end);
        let shift = amount.time_amount()?;
        if amount.pos != end - self.pos {
            return None;
        }
        let op = if self.toks[end].is_word("later") { Op::Add } else { Op::Sub };
        self.pos = end + 1;
        Some((shift, op))
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
    pub(super) fn rounding(&mut self, dir: Direction) -> Result<Option<(Rounding, Option<Format>)>> {
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
                    // "nearest 1/4"
                    if self.cur().is_some_and(|t| t.is_sym("/"))
                        && let Some(Tok::Num(d)) = self.toks.get(self.pos + 1).map(|t| &t.tok)
                        && !d.is_zero()
                    {
                        self.pos += 2;
                        return Ok(Some((Rounding::Multiple(*n / *d, dir), None)));
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
        let n = *n;
        self.pos += 1;
        // "1 million digits"
        let Some(count) = (n * self.scale_words()).to_i64().and_then(|n| u32::try_from(n).ok()) else {
            self.pos = start;
            return Ok(None);
        };
        let w = self.lower(self.pos).unwrap_or_default();
        let significant = match w.as_str() {
            "dp" | "decimals" | "places" | "place" | "digits" | "digit" => false,
            "decimal" => {
                self.pos += 1;
                for w in ["places", "place", "points", "point"] {
                    self.eat_word(w);
                }
                return Ok(Some((Rounding::Places(count, dir), None)));
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
            return Ok(Some((Rounding::Significant(count), None)));
        }
        Ok(Some((Rounding::Places(count, dir), None)))
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
        // "in monthly", "as yearly": per that period.
        if let Some(period) = self.lower(self.pos).and_then(|w| words::period_word(&w)) {
            self.pos += 1;
            return Some(Target::Per(registry().get(period)));
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
        // "in base 3", "to base 36"
        if self.words_at(self.pos, &["base"])
            && let Some(Tok::Num(n)) = self.toks.get(self.pos + 1).map(|t| &t.tok)
            && let Some(base) = n.to_i64().filter(|b| (2..=36).contains(b) && n.is_integer())
        {
            self.pos += 2;
            return Some(Target::Format(Format::Base(base as u8)));
        }
        // "440 Hz to 1/s"
        if self.cur().is_some_and(|t| matches!(t.tok, Tok::Num(n) if n == crate::number::Number::ONE))
            && self.toks.get(self.pos + 1).is_some_and(|t| t.is_sym("/"))
        {
            let mut p = self.clone();
            p.pos += 2;
            if let Some(unit) = p.unit_expr(false) {
                *self = p;
                return Some(Target::Unit(unit.pow(-1)));
            }
        }
        if let Some(unit) = self.unit_expr(true) {
            // "$10 to $20": an amount of money, not a currency to convert to.
            if unit.is_money() && self.cur().is_some_and(|t| matches!(t.tok, Tok::Num(_))) {
                self.pos = start;
                return None;
            }
            let mut units = vec![unit];
            loop {
                let mut p = self.clone();
                // "in feet and inches", "in years & months", or just "in lb oz".
                let joined = p.eat_word("and") || p.eat_sym(",") || p.eat_sym("&");
                let Some(unit) = p.unit_expr(true) else { break };
                if !joined && unit.dim() != units[0].dim() {
                    break;
                }
                *self = p;
                units.push(unit);
            }
            return Some(if units.len() == 1 { Target::Unit(units.pop().unwrap()) } else { Target::Units(units) });
        }
        if matches!(self.lower(self.pos).as_deref(), Some("local" | "here")) {
            self.pos += 1;
            self.eat_word("time");
            return Some(Target::Zone(Place::of_zone(&self.config().local_zone())));
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

impl Parser<'_> {
    /// The day of the exchange rates after a currency conversion: "on Jan 1, 2020".
    fn rates_date(&mut self) -> Result<Option<Expr>> {
        let mut p = self.clone();
        p.eat_word("on");
        if !p.date_starts(p.pos) {
            return Ok(None);
        }
        if !self.config().features.historical_currency_queries {
            bail!("exchange rates of other days are turned off");
        }
        let date = p.unary()?;
        *self = p;
        Ok(Some(date))
    }

    /// The config lets lines use this kind of conversion.
    pub(super) fn converter_allowed(&self, converter: Converter) -> bool {
        let features = &self.config().features;
        features.converters && !features.disabled_converters.contains(&converter)
    }
}

/// The kind of conversion to a target, for `Features::disabled_converters`.
fn converter(target: &Target) -> Converter {
    match target {
        Target::Unit(_) | Target::Cast(_) => Converter::Unit,
        Target::Pattern(_) => Converter::DatePart,
        Target::Units(units) if units.iter().any(Unit::is_money) => Converter::Unit,
        Target::Units(_) => Converter::Parts,
        Target::Per(_) => Converter::Rate,
        Target::Zone(_) => Converter::TimeZone,
        Target::Format(format) => match format {
            Format::Hex => Converter::Hexadecimal,
            Format::Binary => Converter::Binary,
            Format::Octal => Converter::Octal,
            Format::Base(_) => Converter::Base,
            Format::Decimal => Converter::Decimal,
            Format::Scientific => Converter::Scientific,
            Format::Fraction => Converter::Fraction,
            Format::Number => Converter::Number,
            Format::Percent => Converter::Percentage,
            Format::Multiplier => Converter::Multiplier,
            Format::Timespan => Converter::Timespan,
            Format::Laptime | Format::Hms => Converter::Laptime,
            Format::Timestamp => Converter::Timestamp,
            Format::Date => Converter::Date,
            Format::Iso => Converter::Iso8601,
            Format::Weekday | Format::WeekNumber | Format::DayOfYear | Format::DayOfMonth | Format::WeekOfMonth => {
                Converter::DatePart
            }
            Format::Pitch | Format::Midi => Converter::Pitch,
            Format::Dms => Converter::Dms,
            Format::Megapixels | Format::Frames => Converter::Number,
            Format::Clock(_) => Converter::Date,
        },
    }
}

/// A date or time, or a time zone conversion of one.
/// A date template: only pattern letters and separators, with a year, month, day or hour
/// written as people write them in templates ("yyyy", "MM", "dd", "HH").
fn is_date_pattern(text: &str) -> bool {
    let allowed = |c: char| "yMdHhmsaEZ".contains(c) || " -/:.,'T".contains(c);
    let marked = ["yy", "MM", "dd", "HH", "hh", "mm", "EEE"].iter().any(|p| text.contains(p));
    // "yyyy" alone, or parts with separators; not a word like "add".
    let separated = text.chars().any(|c| " -/:.,".contains(c));
    let repeated = text.chars().all(|c| text.starts_with(c));
    marked && text.chars().all(allowed) && (separated || repeated)
}

impl Parser<'_> {
    /// "@ 24 fps", "at 30 fps" at the cursor, read past: the frame rate.
    fn fps_after(&mut self) -> Option<Number> {
        let at = self.cur().is_some_and(|t| t.is_sym("@") || t.is_word("at"));
        let Some(Tok::Num(fps)) = self.toks.get(self.pos + 1).map(|t| &t.tok) else { return None };
        let unit = self.toks.get(self.pos + 2).is_some_and(|t| t.is_word("fps"));
        (at && unit).then(|| {
            self.pos += 3;
            *fps
        })
    }

    /// "with 326 ppi", "at 300 dpi" at the cursor, read past: a pixel density.
    fn density_after(&mut self) -> Result<Option<Expr>> {
        if !self.cur().is_some_and(|t| t.is_word("with") || t.is_word("at")) {
            return Ok(None);
        }
        let mut p = self.clone();
        p.pos += 1;
        let Ok(density) = p.unary() else { return Ok(None) };
        let ppi = registry().get("ppi");
        if !density.edge_unit(true).is_some_and(|u| u.dim() == ppi.dim()) {
            return Ok(None);
        }
        *self = p;
        Ok(Some(density))
    }

    /// "24 hour", "24-hour time", "12h clock", "military time" at token `i`: whether the clock
    /// has 24 hours, and the length in tokens.
    fn clock_style_at(&self, i: usize) -> Option<(bool, usize)> {
        if self.words_at(i, &["military", "time"]) {
            return Some((true, 2));
        }
        let Tok::Num(hours) = self.toks.get(i)?.tok else { return None };
        let h24 = match hours.to_i64()? {
            24 => true,
            12 => false,
            _ => return None,
        };
        let hyphen = usize::from(self.toks.get(i + 1).is_some_and(|t| t.is_sym("-")));
        let word = self.lower(i + 1 + hyphen)?;
        if !matches!(word.as_str(), "hour" | "h" | "hr") {
            return None;
        }
        let mut n = 2 + hyphen;
        if self.lower(i + n).is_some_and(|w| matches!(w.as_str(), "time" | "clock" | "format")) {
            n += 1;
        }
        // Nothing else follows: "in 24 hour increments" is something else.
        (i + n == self.toks.len()).then_some((h24, n))
    }
}

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
fn shifted(expr: &Expr, op: Op, amount: Expr) -> Expr {
    match expr {
        Expr::Convert(inner, Target::Zone(zone)) => {
            Expr::Convert(shifted(inner, op, amount).boxed(), Target::Zone(zone.clone()))
        }
        other => Expr::binary(op, other.clone(), amount),
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
    // Pi has every digit: "pi to 1000 dp".
    if matches!(inner, Expr::Number(n) if Some(n) == words::constant("pi")) {
        match rounding {
            Rounding::Places(dp, dir) => return Expr::Pi(dp, dir),
            Rounding::Significant(digits @ 1..) => return Expr::Pi(digits - 1, Direction::Nearest),
            _ => {}
        }
    }
    let expr = Expr::Round(inner.boxed(), rounding);
    match format {
        Some(f) => Expr::Convert(expr.boxed(), Target::Format(f)),
        None => expr,
    }
}
