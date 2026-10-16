//! Whole-line phrases: percentage questions and proportions.

use super::Parser;
use crate::ast::{Direction, Expr, Format, Func, GrowthResult, Op, Target, TaxPart, TimeExpr};
use crate::error::Result;
use crate::lexer::Tok;
use crate::number::Number;
use crate::units::{Dim, registry};

impl Parser<'_> {
    /// Tries phrases such as "20 is what % of 200" on the whole line.
    pub(super) fn phrase(&mut self) -> Result<Option<Expr>> {
        if let Some(expr) = self.cadence()? {
            return Ok(Some(expr));
        }
        if let Some(expr) = self.sales_tax()? {
            return Ok(Some(expr));
        }
        if let Some(expr) = self.inflation()? {
            return Ok(Some(expr));
        }
        if let Some(expr) = self.growth()? {
            return Ok(Some(expr));
        }
        if let Some(expr) = self.screen_density()? {
            return Ok(Some(expr));
        }
        if let Some(expr) = self.annual_return()? {
            return Ok(Some(expr));
        }
        if let Some(expr) = self.when_it_is()? {
            return Ok(Some(expr));
        }
        if let Some(expr) = self.percent_change()? {
            return Ok(Some(expr));
        }
        if let Some(expr) = self.prime_check() {
            return Ok(Some(expr));
        }
        if let Some(expr) = self.digits_of_pi() {
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
                return Ok(Some(proportion(c, a, b)));
            }
            if self.matches_at(as_ + 1, &["what", "is", "to"]) {
                let d = self.part(as_ + 4, end)?;
                return Ok(Some(proportion(d, b, a)));
            }
        }

        // "6 to 600 as 8 to what", "6 to 600 is what to 8"
        if let Some(to) = self.find(1, &["to"])
            && let Some(mid) = ["as", "is"].iter().filter_map(|w| self.find(to + 2, &[w])).min()
        {
            if self.ends_with(&["to", "what"]) && mid + 1 < end - 2 {
                let (a, b, c) = (self.part(0, to)?, self.part(to + 1, mid)?, self.part(mid + 1, end - 2)?);
                return Ok(Some(proportion(c, a, b)));
            }
            if self.matches_at(mid + 1, &["what", "to"]) {
                let (a, b, d) = (self.part(0, to)?, self.part(to + 1, mid)?, self.part(mid + 3, end)?);
                return Ok(Some(proportion(d, b, a)));
            }
        }

        if let Some(expr) = self.percent_proportion()? {
            return Ok(Some(expr));
        }

        // "what % of 200 is 20", "what % is 20 of 200"
        if self.matches_at(0, &["what", "%", "of"])
            && let Some(is) = self.find(3, &["is"])
        {
            let whole = self.part(3, is)?;
            let part = self.part(is + 1, end)?;
            return Ok(Some(pct(Expr::binary(Op::Div, part, whole))));
        }
        // "what % is 25 with 100"
        if self.matches_at(0, &["what", "%", "is"])
            && let Some((at, kind)) = SHARES.iter().find_map(|&kind| Some((self.find(3, &[kind])?, kind)))
        {
            let part = self.part(3, at)?;
            let whole = self.share_part(kind, at + 1, end)?;
            return Ok(Some(pct(share(kind, part, whole))));
        }

        // "20 is what % of 200", "20 is % of 200", "180 is what discount off 200", "5 is what multiplier on 1"
        for seq in [&["is", "what"][..], &["is"][..], &["as", "a"][..], &["as"][..]] {
            let Some(at) = self.find(0, seq) else { continue };
            let k = at + seq.len();
            let format = if self.matches_at(k, &["%"]) || self.matches_at(k, &["discount"]) {
                Format::Percent
            // "5 is x on 1" could be a variable x, so a multiplier needs "what" or "as".
            } else if (self.matches_at(k, &["multiplier"]) || self.matches_at(k, &["x"])) && seq != ["is"] {
                Format::Multiplier
            } else {
                continue;
            };
            // "10 is what % 20" means "of"; "15 is what % after adding 3".
            let kind = SHARES.iter().copied().find(|kind| self.matches_at(k + 1, &[kind]));
            let from = match kind {
                Some("after") => k + 2 + usize::from(self.matches_at(k + 2, &["adding"])),
                Some(_) => k + 2,
                None => k + 1,
            };
            let comment = self.toks.get(from).is_some_and(|t| t.word().is_some() && !self.significant(from));
            if from >= end || comment {
                continue;
            }
            let kind = kind.unwrap_or("of");
            let a = self.part(0, at)?;
            let b = self.share_part(kind, from, end)?;
            return Ok(Some(Self::formatted(share(kind, a, b), format)));
        }

        // "81 is 9 to what power", "20 is 4 to the power of what"
        for tail in [
            &["to", "what", "power"][..],
            &["to", "the", "what", "power"][..],
            &["to", "the", "power", "of", "what"][..],
            &["to", "the", "what"][..],
        ] {
            if self.ends_with(tail)
                && let Some(is) = self.find(0, &["is"])
            {
                let (a, b) = (self.part(0, is)?, self.part(is + 1, end - tail.len())?);
                return Ok(Some(Expr::Call(crate::ast::Func::Log, vec![a, b])));
            }
        }
        // "4 to what power is 20", "4 with what exponent is 20"
        for seq in [
            &["to", "what", "power", "is"][..],
            &["to", "what", "exponent", "is"],
            &["with", "what", "exponent", "is"],
            &["with", "what", "power", "is"],
        ] {
            if let Some(at) = self.find(0, seq) {
                let (a, b) = (self.part(at + seq.len(), end)?, self.part(0, at)?);
                return Ok(Some(Expr::Call(crate::ast::Func::Log, vec![a, b])));
            }
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
        // "for 3 years at 7%", "at 7% for 3 years", "at 5% compounded daily for 2 years":
        // each part runs to the next one.
        let stops = [Some(during), Some(at), compounding, Some(self.toks.len())];
        let end_after = |from: usize| stops.iter().flatten().copied().filter(|&s| s > from).min().unwrap_or(from);
        let (principal_end, time, mut rate_end) = (during.min(at), (during + 1, end_after(during)), end_after(at));
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

    /// "ppi of 5.5 inch at 1920x1080": the pixels per inch of a screen.
    fn screen_density(&self) -> Result<Option<Expr>> {
        if !(self.matches_at(0, &["ppi"]) || self.matches_at(0, &["dpi"])) {
            return Ok(None);
        }
        let start = 1 + usize::from(self.matches_at(1, &["of"]));
        let resolution = (start + 1..self.toks.len()).find_map(|i| Some((i, self.resolution_at(i)?)));
        let Some((at, (width, height, n))) = resolution else {
            return Ok(None);
        };
        if at + n != self.toks.len() {
            return Ok(None);
        }
        let end = at - usize::from(self.matches_at(at - 1, &["at"]));
        let reg = registry();
        let inches = Expr::Convert(self.part(start, end)?.boxed(), Target::Unit(reg.get("in")));
        let pixels = Expr::Call(Func::Sqrt, vec![Expr::Number(width * width + height * height)]);
        let density = Expr::binary(Op::Div, pixels, Self::formatted(inches, Format::Number));
        Ok(Some(Expr::WithUnit(density.boxed(), reg.get("ppi"))))
    }

    /// A screen resolution at token `i`: "1920x1080", "1920 x 1080"; its width, height and length.
    fn resolution_at(&self, i: usize) -> Option<(Number, Number, usize)> {
        let Tok::Num(width) = self.toks.get(i)?.tok else { return None };
        let glued = self.toks.get(i + 1).filter(|t| !t.space_before).and_then(|t| t.word());
        if let Some(height) = glued.and_then(|w| w.strip_prefix(['x', 'X'])).and_then(Number::parse) {
            return Some((width, height, 2));
        }
        let times = self.toks.get(i + 1).is_some_and(|t| t.is_word("x") || t.is_sym("*"));
        match self.toks.get(i + 2)?.tok {
            Tok::Num(height) if times => Some((width, height, 3)),
            _ => None,
        }
    }

    /// "annual return on $500 that grew to $1,000 over 5 years": the yearly growth rate.
    fn annual_return(&self) -> Result<Option<Expr>> {
        if !self.matches_at(0, &["annual", "return", "on"]) {
            return Ok(None);
        }
        let Some(during) = ["over", "after", "in", "for"].iter().filter_map(|w| self.find(4, &[w])).min() else {
            return Ok(None);
        };
        // The amount at the start, then at the end, with words around them.
        let mut amounts = self.sub(3, during);
        let start = amounts.additive()?;
        amounts.eat_word("to");
        let end = amounts.additive()?;
        amounts.finish()?;
        let reg = registry();
        let years = Expr::Convert(self.part(during + 1, self.toks.len())?.boxed(), Target::Unit(reg.get("yr")));
        let years = Self::formatted(years, Format::Number);
        let one = || Expr::Number(Number::ONE);
        let growth = Expr::binary(Op::Pow, Expr::binary(Op::Div, end, start), Expr::binary(Op::Div, one(), years));
        Ok(Some(Self::formatted(Expr::binary(Op::Sub, growth, one()), Format::Percent)))
    }

    /// "if 30 is 20%, what is 50%" = 75, "20% is 30, what is 60" = 40%: a value known
    /// as a percentage of a whole gives other parts of it.
    fn percent_proportion(&self) -> Result<Option<Expr>> {
        let first = usize::from(self.matches_at(0, &["if"]));
        let Some(is) = self.find(first, &["is"]) else { return Ok(None) };
        let Some(what) = self.find(is + 1, &["what", "is"]) else { return Ok(None) };
        let known_end = what - usize::from(self.toks[what - 1].is_sym(","));
        let (value, percent) = if self.matches_at(known_end.wrapping_sub(1), &["%"]) {
            (self.part(first, is)?, self.part(is + 1, known_end)?)
        } else if self.matches_at(is.wrapping_sub(1), &["%"]) {
            (self.part(is + 1, known_end)?, self.part(first, is)?)
        } else {
            return Ok(None);
        };
        let whole = Expr::binary(Op::Div, value, percent);
        let asked = self.part(what + 2, self.toks.len())?;
        Ok(Some(match self.ends_with(&["%"]) {
            true => Expr::binary(Op::Of, asked, whole),
            false => Self::formatted(Expr::binary(Op::Div, asked, whole), Format::Percent),
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

    /// "time in Tokyo when it is 9am in London", "time in Tokyo when 9am in London".
    fn when_it_is(&self) -> Result<Option<Expr>> {
        let Some(at) = self.find(1, &["when"]) else { return Ok(None) };
        let len = if self.matches_at(at, &["when", "it", "is"]) {
            3
        } else if self.matches_at(at, &["when", "it's"]) {
            2
        } else {
            1
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

    /// A whole line that repeats: "every 2 weeks", "every other day", "daily", with
    /// "from" a day and "until" another in either order.
    fn cadence(&self) -> Result<Option<Expr>> {
        const EVERY: &[(&str, &str, i64)] = &[
            ("hourly", "hour", 1),
            ("daily", "day", 1),
            ("weekly", "week", 1),
            ("biweekly", "week", 2),
            ("fortnightly", "week", 2),
            ("monthly", "month", 1),
            ("quarterly", "month", 3),
            ("yearly", "year", 1),
            ("annually", "year", 1),
        ];
        if !self.config().features.cadence_parsing || self.toks.is_empty() {
            return Ok(None);
        }
        let amount = |n: i64, unit: crate::units::Unit| Expr::WithUnit(Expr::Number(Number::from_i64(n)).boxed(), unit);
        let first = self.lower(0).unwrap_or_default();
        let (every, mut at) = if let Some(&(_, unit, n)) = EVERY.iter().find(|(w, _, _)| *w == first) {
            (amount(n, registry().lookup(unit).expect("a unit of time")), 1)
        } else if first == "every" {
            // "every other day", "every 2 weeks", "every week"
            let (n, at) = match &self.toks.get(1).map(|t| &t.tok) {
                Some(Tok::Num(n)) if n.is_integer() => (n.to_i64().unwrap_or(1), 2),
                _ if self.words_at(1, &["other"]) => (2, 2),
                _ => (1, 1),
            };
            let Some((unit, len)) = self.unit_at(at, true).filter(|(u, _)| u.dim() == Dim::TIME) else {
                return Ok(None);
            };
            (amount(n, unit), at + len)
        } else {
            return Ok(None);
        };
        let end = self.toks.len();
        let (mut start, mut until) = (None, None);
        while at < end {
            let any = |words: &[&str]| words.iter().any(|w| self.words_at(at, &[w]));
            let slot =
                match (any(&["from", "starting", "beginning"]), any(&["until", "till", "through", "to", "ending"])) {
                    (true, _) if start.is_none() => &mut start,
                    (_, true) if until.is_none() => &mut until,
                    _ => return Ok(None),
                };
            let stop = ["from", "starting", "beginning", "until", "till", "through", "ending"]
                .iter()
                .filter_map(|w| self.find(at + 1, &[w]))
                .min()
                .unwrap_or(end);
            *slot = Some(self.part(at + 1, stop)?.boxed());
            at = stop;
        }
        Ok(Some(Expr::Cadence { every: every.boxed(), start, until }))
    }

    /// "8% vat in $108" (the tax in a price), "$108 before 8% tax" (the price without it),
    /// "$100 with 8% tax" (the price with it).
    fn sales_tax(&self) -> Result<Option<Expr>> {
        const TAXES: &[&[&str]] = &[&["sales", "tax"], &["tax"], &["vat"], &["gst"], &["hst"], &["mwst"], &["iva"]];
        let end = self.toks.len();
        let tax_at = |k: usize| TAXES.iter().find(|t| self.words_at(k, t)).map(|t| t.len());
        let Some((t, n)) = (2..end).find_map(|k| Some((k, tax_at(k).filter(|_| self.toks[k - 1].is_sym("%"))?))) else {
            return Ok(None);
        };
        let pct = t - 2;
        if !matches!(self.toks[pct].tok, Tok::Num(_)) {
            return Ok(None);
        }
        let rate = self.part(pct, t)?.boxed();
        let after = t + n;
        // "8% vat in $108", "10% sales tax off $110", "8% tax on $100"
        let lead = [("in", TaxPart::Tax), ("off", TaxPart::Net), ("on", TaxPart::Gross)];
        if pct == 0
            && let Some(&(_, part)) = lead.iter().find(|(w, _)| self.words_at(after, &[w]))
        {
            let amount = self.part(after + 1, end)?.boxed();
            return Ok(Some(Expr::SalesTax { amount, rate, part }));
        }
        if after != end || pct < 2 {
            return Ok(None);
        }
        let joint = &self.toks[pct - 1];
        let word = joint.word().map(str::to_lowercase).unwrap_or_default();
        let part = match word.as_str() {
            "before" | "without" | "ex" | "excluding" | "excl" | "less" | "minus" => TaxPart::Net,
            _ if joint.is_sym("-") => TaxPart::Net,
            "with" | "including" | "incl" | "plus" | "after" => TaxPart::Gross,
            _ if joint.is_sym("+") => TaxPart::Gross,
            _ => return Ok(None),
        };
        Ok(Some(Expr::SalesTax { amount: self.part(0, pct - 1)?.boxed(), rate, part }))
    }

    fn find_sym(&self, from: usize, sym: &str) -> Option<usize> {
        (from..self.toks.len()).find(|&i| self.toks[i].is_sym(sym))
    }

    /// "is 17 prime", "is 17 a prime number", "is prime 17".
    fn prime_check(&self) -> Option<Expr> {
        if !self.matches_at(0, &["is"]) {
            return None;
        }
        let end = self.toks.len();
        let n = if self.matches_at(1, &["prime"]) {
            self.part(2, end)
        } else {
            let tails = [&["a", "prime", "number"][..], &["prime", "number"], &["prime"]];
            let tail = tails.into_iter().find(|t| self.ends_with(t))?;
            self.part(1, end - tail.len())
        };
        // Anything but a number there is ordinary text.
        n.ok().map(|n| Expr::Call(Func::IsPrime, vec![n]))
    }

    /// "100 digits of pi", "the first 1 million decimals of π": pi cut off after them.
    fn digits_of_pi(&self) -> Option<Expr> {
        let end = self.toks.len();
        if !self.ends_with(&["of", "pi"]) && !self.ends_with(&["of", "π"]) {
            return None;
        }
        let tails = [&["digits"][..], &["decimals"], &["decimal", "places"]];
        let to = tails.into_iter().find_map(|t| end.checked_sub(2 + t.len()).filter(|&i| self.matches_at(i, t)))?;
        let mut from = 0;
        for w in ["the", "first"] {
            if self.matches_at(from, &[w]) {
                from += 1;
            }
        }
        let Ok(Expr::Number(n)) = self.part(from, to) else { return None };
        if !n.is_integer() || n.is_negative() {
            return None;
        }
        let places = n.to_i64().and_then(|n| u32::try_from(n).ok()).unwrap_or(u32::MAX);
        Some(Expr::Pi(places, Direction::Down))
    }

    /// What a share is taken of: the sum of a list after "with" ("10 is what % with 20, 30").
    fn share_part(&self, kind: &str, from: usize, to: usize) -> Result<Expr> {
        if kind != "with" {
            return self.part(from, to);
        }
        let mut p = self.sub(from, to);
        let mut items = p.list()?;
        p.finish()?;
        Ok(if items.len() == 1 { items.remove(0) } else { Expr::Call(Func::Sum, items) })
    }

    /// Parses tokens `from..to` as one expression.
    pub(super) fn part(&self, from: usize, to: usize) -> Result<Expr> {
        if from >= to {
            crate::error::bail!("missing a value");
        }
        self.sub(from, to).expr_to_end()
    }

    /// Token `i` starts this word sequence; "%" also matches "percent".
    pub(super) fn matches_at(&self, i: usize, seq: &[&str]) -> bool {
        seq.iter().enumerate().all(|(k, s)| {
            let Some(t) = self.toks.get(i + k) else { return false };
            match *s {
                "%" => t.is_sym("%") || ["percent", "percentage", "pct"].iter().any(|w| t.is_word(w)),
                w => t.is_word(w),
            }
        })
    }

    pub(super) fn ends_with(&self, seq: &[&str]) -> bool {
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
/// Words saying what a share is taken of: "20 is what % of 200".
const SHARES: [&str; 5] = ["of", "on", "off", "with", "after"];

/// The share `a` is of `b`: "on" and "off" compare the difference with `b`,
/// "with" takes the total of both, and "after" adding `b` compares it with what `a` was before.
fn share(kind: &str, a: Expr, b: Expr) -> Expr {
    let div = |x, y| Expr::binary(Op::Div, x, y);
    let sub = |x, y| Expr::binary(Op::Sub, x, y);
    match kind {
        "on" => div(sub(a, b.clone()), b),
        "off" => div(sub(b.clone(), a), b),
        "with" => div(a.clone(), Expr::binary(Op::Add, a, b)),
        "after" => div(b.clone(), sub(a, b)),
        _ => div(a, b),
    }
}

/// The missing term of a proportion: `x / of × by`. Dividing first keeps the
/// units apart: "$6 to 2 kg as $9 to what" is 3 kg.
fn proportion(x: Expr, of: Expr, by: Expr) -> Expr {
    Expr::binary(Op::Mul, Expr::binary(Op::Div, x, of), by)
}

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
