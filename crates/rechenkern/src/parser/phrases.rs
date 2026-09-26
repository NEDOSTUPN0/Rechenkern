//! Whole-line phrases: percentage questions and proportions.

use super::Parser;
use crate::ast::{Expr, Format, GrowthResult, Op, Target, TimeExpr};
use crate::error::Result;
use crate::lexer::Tok;
use crate::number::Number;
use crate::units::{Dim, registry};

impl Parser<'_> {
    /// Tries phrases such as "20 is what % of 200" on the whole line.
    pub(super) fn phrase(&mut self) -> Result<Option<Expr>> {
        if let Some(expr) = self.growth()? {
            return Ok(Some(expr));
        }
        if let Some(expr) = self.when_it_is()? {
            return Ok(Some(expr));
        }
        if let Some(expr) = self.percent_change()? {
            return Ok(Some(expr));
        }
        // Every phrase below has one of these words.
        if !self.toks.iter().any(|t| ["is", "as", "what"].iter().any(|w| t.is_word(w))) {
            return Ok(None);
        }
        let end = self.toks.len();
        let one = || Expr::Number(Number::ONE);
        let pct = |e: Expr| Self::formatted(e, Format::Percent);

        // "6 is to 60 as 8 is to what", "5 is to 10 as what is to 80"
        if let Some(is_to) = self.find(0, &["is", "to"])
            && let Some(as_) = self.find(is_to + 2, &["as"])
        {
            let a = self.part(0, is_to)?;
            let b = self.part(is_to + 2, as_)?;
            if self.ends_with(&["is", "to", "what"]) {
                let c = self.part(as_ + 1, end - 3)?;
                return Ok(Some(Expr::binary(Op::Div, Expr::binary(Op::Mul, c, b), a)));
            }
            if self.matches_at(as_ + 1, &["what", "is", "to"]) {
                let d = self.part(as_ + 4, end)?;
                return Ok(Some(Expr::binary(Op::Div, Expr::binary(Op::Mul, a, d), b)));
            }
        }

        // "what % of 200 is 20"
        if self.matches_at(0, &["what", "%", "of"])
            && let Some(is) = self.find(3, &["is"])
        {
            let whole = self.part(3, is)?;
            let part = self.part(is + 1, end)?;
            return Ok(Some(pct(Expr::binary(Op::Div, part, whole))));
        }

        // "20 is what % of 200", "180 is what % off 200", "5 is what multiplier on 1"
        for seq in [&["is", "what"][..], &["as", "a"][..], &["as"][..]] {
            let Some(at) = self.find(0, seq) else { continue };
            let k = at + seq.len();
            let format = if self.matches_at(k, &["%"]) {
                Format::Percent
            } else if self.matches_at(k, &["multiplier"]) || self.matches_at(k, &["x"]) {
                Format::Multiplier
            } else {
                continue;
            };
            // "10 is what % 20" means "of", but "10 is what % with 20" doesn't.
            let kind = self.lower(k + 1).filter(|w| matches!(w.as_str(), "of" | "on" | "off"));
            let from = k + 1 + usize::from(kind.is_some());
            let comment = self.toks.get(from).is_some_and(|t| t.word().is_some() && !self.significant(from));
            if from >= end || comment {
                continue;
            }
            let a = self.part(0, at)?;
            let b = self.part(from, end)?;
            let ratio = match kind.as_deref() {
                Some("on") => Expr::binary(Op::Div, Expr::binary(Op::Sub, a, b.clone()), b),
                Some("off") => Expr::binary(Op::Div, Expr::binary(Op::Sub, b.clone(), a), b),
                _ => Expr::binary(Op::Div, a, b),
            };
            return Ok(Some(Self::formatted(ratio, format)));
        }

        // "20 is 10% of what", "180 is 10% off what", "10% on what is 220"
        for kind in ["of", "on", "off"] {
            let (a, p) = if self.ends_with(&[kind, "what"])
                && let Some(is) = self.find(0, &["is"]).filter(|&i| i < end - 2)
            {
                (self.part(0, is)?, self.part(is + 1, end - 2)?)
            } else if let Some(at) = self.find(0, &[kind, "what", "is"]) {
                (self.part(at + 3, end)?, self.part(0, at)?)
            } else {
                continue;
            };
            let divisor = match kind {
                "of" => p,
                "on" => Expr::binary(Op::Add, one(), p),
                _ => Expr::binary(Op::Sub, one(), p),
            };
            return Ok(Some(Expr::binary(Op::Div, a, divisor)));
        }

        // "50 to 75 is what %", "40 to 90 as %", "50 to 75 is what x"
        for (seq, skip) in [(&["is", "what"][..], 2), (&["as", "a"][..], 2), (&["as"][..], 1)] {
            let Some(at) = self.find(0, seq) else { continue };
            let Some(to) = self.find(0, &["to"]).filter(|&t| t < at) else { continue };
            let tail = at + skip;
            let format = if self.matches_at(tail, &["%"]) {
                Format::Percent
            } else if self.matches_at(tail, &["x"]) || self.matches_at(tail, &["multiplier"]) {
                Format::Multiplier
            } else {
                continue;
            };
            if tail + 1 != end {
                continue;
            }
            let a = self.part(0, to)?;
            let b = self.part(to + 1, at)?;
            let change = match format {
                Format::Percent => change(a, b),
                _ => Expr::binary(Op::Div, b, a),
            };
            return Ok(Some(Self::formatted(change, format)));
        }

        // "81 is 9 to what power"
        for tail in [&["to", "what", "power"][..], &["to", "the", "what", "power"][..]] {
            if self.ends_with(tail)
                && let Some(is) = self.find(0, &["is"])
            {
                let (a, b) = (self.part(0, is)?, self.part(is + 1, end - tail.len())?);
                return Ok(Some(Expr::Call(crate::ast::Func::Log, vec![a, b])));
            }
        }

        // "3/20 is what %"
        if self.ends_with(&["is", "what", "%"]) {
            let a = self.part(0, end - 3)?;
            return Ok(Some(pct(a)));
        }

        // "$1,000/month is what per week", "5 km is how much in miles"
        for seq in [&["is", "what"][..], &["is", "how", "much"][..]] {
            let Some(at) = self.find(0, seq) else { continue };
            let mut p = self.sub(at + seq.len(), end);
            p.eat_word("in");
            if let Some(target) = p.target()
                && p.pos == p.toks.len()
            {
                return Ok(Some(Expr::Convert(self.part(0, at)?.boxed(), target)));
            }
        }
        Ok(None)
    }

    /// "$1,000 after 3 years at 7%", "interest on $500 for 2 years @ 5% compounding monthly",
    /// "present value of $1,000 after 20 years at 10%", "monthly repayment on $300k at 6% for 30 years".
    fn growth(&self) -> Result<Option<Expr>> {
        // A rate is always given in percent.
        if !(0..self.toks.len()).any(|i| self.matches_at(i, &["%"])) {
            return Ok(None);
        }
        // "total" and "monthly" ask about a loan: "total interest on", "monthly repayment on".
        let every = self.lower(0).and_then(|w| payment_period(&w));
        let loan = every.is_some() || self.matches_at(0, &["total"]);
        let first = usize::from(loan);
        let Some(&(words, result)) = GROWTH_PHRASES.iter().find(|(words, _)| self.matches_at(first, words)) else {
            return Ok(None);
        };
        let result = match result {
            GrowthResult::Interest if loan => GrowthResult::LoanInterest,
            GrowthResult::Future | GrowthResult::Present if loan => return Ok(None),
            r => r,
        };
        let start = first + words.len();
        let Some(during) = ["after", "for", "over", "in"].iter().filter_map(|w| self.find(start + 1, &[w])).min()
        else {
            return Ok(None);
        };
        let Some(at) = self.find(start + 1, &["at"]).or_else(|| self.find_sym(start + 1, "@")) else {
            return Ok(None);
        };
        let compounding =
            ["compounding", "compounded", "compound"].iter().filter_map(|w| self.find(at + 1, &[w])).min();
        let end = compounding.unwrap_or(self.toks.len());
        // "for 3 years at 7%" or "at 7% for 3 years".
        let (principal_end, time, mut rate_end) = match during < at {
            true => (during, (during + 1, at), end),
            false => (at, (during + 1, end), during),
        };
        // "10% per month" grows every month; plain rates are yearly.
        let mut period = registry().get("yr");
        let per = self
            .toks
            .get(rate_end.wrapping_sub(2))
            .is_some_and(|t| t.is_sym("/") || ["per", "a", "an", "every", "each"].iter().any(|w| t.is_word(w)));
        if per
            && self.matches_at(rate_end.wrapping_sub(3), &["%"])
            && let Some((unit, 1)) = self.unit_at(rate_end - 1, false).filter(|(u, _)| u.dim() == Dim::TIME)
        {
            period = unit;
            rate_end -= 2;
        }
        if !self.matches_at(rate_end.wrapping_sub(1), &["%"]) {
            return Ok(None);
        }
        let compounds = match compounding.map(|c| self.lower(c + 1).unwrap_or_default()).as_deref() {
            None => None,
            Some("yearly" | "annually" | "annual") => Some(1),
            Some("semiannually" | "biannually") => Some(2),
            Some("quarterly") => Some(4),
            Some("monthly") => Some(12),
            Some("weekly") => Some(52),
            Some("daily") => Some(365),
            Some(other) => crate::error::bail!("unknown compounding period \"{other}\""),
        };
        let time = self.part(time.0, time.1)?;
        let growth = Expr::Growth {
            principal: self.part(start, principal_end)?.boxed(),
            time: time.clone().boxed(),
            rate: self.part(at + 1, rate_end)?.boxed(),
            period,
            compounds,
            result,
        };
        // "monthly repayment": the whole loan spread over its time.
        Ok(Some(match every {
            Some((n, unit)) => {
                let every = Expr::WithUnit(Expr::Number(Number::from_i64(n)).boxed(), registry().get(unit));
                Expr::binary(Op::Mul, Expr::binary(Op::Div, growth, time), every)
            }
            None => growth,
        }))
    }

    /// "% change from 10 to 20", "percent increase from 10 to 20", "what % change is 10 to 20".
    fn percent_change(&self) -> Result<Option<Expr>> {
        let start = usize::from(self.matches_at(0, &["what"]));
        let kind = self.lower(start + 1);
        if !self.matches_at(start, &["%"]) || !matches!(kind.as_deref(), Some("change" | "increase" | "decrease")) {
            return Ok(None);
        }
        let from =
            start + 2 + usize::from(self.matches_at(start + 2, &["from"]) || self.matches_at(start + 2, &["is"]));
        let Some(to) = self.find(from, &["to"]) else { return Ok(None) };
        let (a, b) = (self.part(from, to)?, self.part(to + 1, self.toks.len())?);
        Ok(Some(Self::formatted(change(a, b), Format::Percent)))
    }

    /// "time in Tokyo when it is 9am in London".
    fn when_it_is(&self) -> Result<Option<Expr>> {
        let (at, len) = match (self.find(1, &["when", "it", "is"]), self.find(1, &["when", "it's"])) {
            (Some(at), _) => (at, 3),
            (None, Some(at)) => (at, 2),
            _ => return Ok(None),
        };
        let moment = match self.part(at + len, self.toks.len())? {
            // "9am in London" places the clock in London.
            Expr::Convert(inner, Target::Zone(zone)) => Expr::InZone(inner, zone),
            other => other,
        };
        match self.part(0, at)? {
            Expr::Convert(inner, target) if matches!(*inner, Expr::Time(TimeExpr::Now)) => {
                Ok(Some(Expr::Convert(moment.boxed(), target)))
            }
            _ => Ok(None),
        }
    }

    fn find_sym(&self, from: usize, sym: &str) -> Option<usize> {
        (from..self.toks.len()).find(|&i| self.toks[i].is_sym(sym))
    }

    /// Parses tokens `from..to` as one expression.
    pub(super) fn part(&self, from: usize, to: usize) -> Result<Expr> {
        if from >= to {
            crate::error::bail!("missing a value");
        }
        self.sub(from, to).expr_to_end()
    }

    /// Token `i` starts this word sequence; "%" also matches "percent".
    fn matches_at(&self, i: usize, seq: &[&str]) -> bool {
        seq.iter().enumerate().all(|(k, s)| {
            let Some(t) = self.toks.get(i + k) else { return false };
            match *s {
                "%" => t.is_sym("%") || ["percent", "percentage", "pct"].iter().any(|w| t.is_word(w)),
                w => t.is_word(w),
            }
        })
    }

    fn ends_with(&self, seq: &[&str]) -> bool {
        self.toks.len() >= seq.len() && self.matches_at(self.toks.len() - seq.len(), seq)
    }

    /// First occurrence of `seq` from `from`, outside parentheses.
    pub(super) fn find(&self, from: usize, seq: &[&str]) -> Option<usize> {
        let mut depth = 0;
        for i in from..self.toks.len() {
            match self.toks[i].tok {
                Tok::Sym("(") => depth += 1,
                Tok::Sym(")") => depth -= 1,
                _ => {}
            }
            if depth == 0 && self.matches_at(i, seq) {
                return Some(i);
            }
        }
        None
    }
}

/// Relative change from `a` to `b`: (b - a) / a.
fn change(a: Expr, b: Expr) -> Expr {
    Expr::binary(Op::Div, Expr::binary(Op::Sub, b, a.clone()), a)
}

/// Phrases before the principal and what they ask for; loans need "total" or a period first.
const GROWTH_PHRASES: &[(&[&str], GrowthResult)] = &[
    (&["interest", "repayment", "on"], GrowthResult::LoanInterest),
    (&["interest", "on"], GrowthResult::Interest),
    (&["present", "value", "of"], GrowthResult::Present),
    (&["future", "value", "of"], GrowthResult::Future),
    (&["repayment", "on"], GrowthResult::Repayment),
    (&["repayments", "on"], GrowthResult::Repayment),
    (&["payment", "on"], GrowthResult::Repayment),
    (&[], GrowthResult::Future),
];

/// How long "monthly" is in "monthly repayment": a count of a unit.
fn payment_period(word: &str) -> Option<(i64, &'static str)> {
    Some(match word {
        "daily" => (1, "d"),
        "weekly" => (1, "wk"),
        "monthly" => (1, "mo"),
        "quarterly" => (3, "mo"),
        "yearly" | "annual" | "annually" => (1, "yr"),
        _ => return None,
    })
}
