//! Inflation phrases: "$1,000 in 1990 dollars", "value of $100 in 1990",
//! "what will $100 be worth in 2035 with 3% inflation".

use super::Parser;
use crate::ast::Expr;
use crate::error::Result;
use crate::lexer::Tok;
use crate::parser::words;

impl Parser<'_> {
    /// A whole-line inflation phrase, or `None`.
    pub(super) fn inflation(&mut self) -> Result<Option<Expr>> {
        // Every phrase names a year after "in" or "from".
        let Some(at) = (1..self.toks.len()).find(|&i| self.year_at(i).is_some() && self.words_before_year(i)) else {
            return Ok(None);
        };
        let end = self.toks.len();
        let (year, len) = self.year_at(at).map_or((None, 1), |(y, n)| (Some(y), n));
        let after = at + len;
        let inflation = |amount, from, to, rate| Ok(Some(Expr::Inflation { amount, from, to, rate }));

        // "value of $100 in 1990", "what was $100 worth in 1990", "... in 2035 with 3% inflation"
        let lead = [
            &["value", "of"][..],
            &["purchasing", "power", "of"],
            &["what", "was"],
            &["what", "is"],
            &["what", "will"],
        ]
        .into_iter()
        .find(|lead| self.matches_at(0, lead))
        .map_or(0, <[&str]>::len);
        if lead > 0 && self.matches_at(at - 1, &["in"]) {
            let mut amount_end = at - 1;
            for tail in [&["be", "worth"][..], &["worth"]] {
                if amount_end >= tail.len() && self.matches_at(amount_end - tail.len(), tail) {
                    amount_end -= tail.len();
                    break;
                }
            }
            let amount = self.part(lead, amount_end)?.boxed();
            let rate = self.inflation_rate(after)?;
            if rate.is_none() && after != end {
                return Ok(None);
            }
            return inflation(amount, None, year, rate);
        }
        // "value of $500 from 2020": worth today.
        if lead > 0 && self.matches_at(at - 1, &["from"]) && after == end {
            return inflation(self.part(lead, at - 1)?.boxed(), year, None, None);
        }
        // "500 from today in 2030 at 5% inflation"
        if at >= 3 && self.matches_at(at - 3, &["from", "today", "in"]) {
            let rate = self.inflation_rate(after)?;
            if rate.is_some() || after == end {
                return inflation(self.part(lead, at - 3)?.boxed(), None, year, rate);
            }
        }
        // "$100 today in 1990"
        if at >= 2 && self.matches_at(at - 2, &["today", "in"]) && after == end {
            return inflation(self.part(0, at - 2)?.boxed(), None, year, None);
        }
        let from = at - 1;
        // "$1,000 in 1990 dollars", "€500 in 2001 euros"
        if self.matches_at(from, &["in"])
            && self.unit_at(after, false).is_some_and(|(u, n)| u.is_money() && after + n == end)
        {
            return inflation(self.part(0, from)?.boxed(), year, None, None);
        }
        // "$100 from 1990 is worth what today", "$100 in 1990 is worth what today"
        if self.matches_at(after, &["is", "worth", "what", "today"]) && after + 4 == end {
            return inflation(self.part(0, from)?.boxed(), year, None, None);
        }
        // "$100 from 1990 in 2000", "$100 from 1990 to today", "$100 from 2000 worth what in
        // 2010", "what is $100 from 2000 worth in 2010"
        if self.matches_at(from, &["from"]) {
            let mut to = after;
            while ["is", "was", "worth", "what"].iter().any(|w| self.matches_at(to, &[w])) {
                to += 1;
            }
            if self.matches_at(to, &["in"]) || self.matches_at(to, &["to"]) {
                to += 1;
            }
            let amount = || self.part(lead, from).map(Box::new);
            // "what was $100 from 2000 worth": worth today.
            if (self.matches_at(to, &["today"]) && to + 1 == end) || (to == end && to > after) {
                return inflation(amount()?, year, None, None);
            }
            if let Some((later, _)) = self.year_at(to).filter(|&(_, n)| to + n == end && to > after) {
                return inflation(amount()?, year, Some(later), None);
            }
        }
        Ok(None)
    }

    /// A year at token `i` and its length in tokens: a whole number from 1000 to 2999, or a
    /// date that ends with one, whose year counts, as in SoulverCore ("March 5, 2025", "2025-03-05").
    fn year_at(&self, i: usize) -> Option<(i16, usize)> {
        let year = |i: usize| match &self.toks.get(i)?.tok {
            Tok::Num(n) if n.is_integer() => n.to_i64().filter(|y| (1000..3000).contains(y)).map(|y| y as i16),
            _ => None,
        };
        let day = |i: usize| match &self.toks.get(i).map(|t| &t.tok) {
            Some(Tok::Num(n)) if n.is_integer() && (1..=31).contains(&n.to_i64().unwrap_or(0)) => {
                let glued = self
                    .toks
                    .get(i + 1)
                    .is_some_and(|t| !t.space_before && ["st", "nd", "rd", "th"].iter().any(|s| t.is_word(s)));
                Some(1 + usize::from(glued))
            }
            _ => None,
        };
        let month = |i: usize| self.lower(i).is_some_and(|w| words::month(&w).is_some());
        let comma = |i: usize| usize::from(self.toks.get(i).is_some_and(|t| t.is_sym(",")));
        if let Some(y) = year(i) {
            return Some((y, 1));
        }
        if let Some(Tok::Date { parts, .. }) = self.toks.get(i).map(|t| &t.tok) {
            let y = if parts[0] >= 1000 { parts[0] } else { parts[2] };
            return (1000..3000).contains(&y).then_some((y as i16, 1));
        }
        // "March 5, 2025", "March 2025", "5 March 2025"
        let mut j = i;
        if month(j) {
            j += 1;
            j += day(j).unwrap_or(0);
        } else {
            j += day(j)?;
            if !month(j) {
                return None;
            }
            j += 1;
        }
        j += comma(j);
        year(j).map(|y| (y, j + 1 - i))
    }

    /// "in" or "from" comes right before the year at `i`.
    fn words_before_year(&self, i: usize) -> bool {
        self.matches_at(i - 1, &["in"]) || self.matches_at(i - 1, &["from"])
    }

    /// "with 3% inflation" (or "given", "assuming", "at") from token `i` to the end.
    fn inflation_rate(&self, i: usize) -> Result<Option<Box<Expr>>> {
        let end = self.toks.len();
        let with = ["with", "given", "assuming", "at"].iter().any(|w| self.matches_at(i, &[w]));
        if !with || !self.ends_with(&["inflation"]) || i + 2 >= end {
            return Ok(None);
        }
        Ok(Some(self.part(i + 1, end - 1)?.boxed()))
    }
}
