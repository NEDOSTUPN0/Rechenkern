//! Inflation phrases: "$1,000 in 1990 dollars", "value of $100 in 1990",
//! "what will $100 be worth in 2035 with 3% inflation".

use super::Parser;
use crate::ast::Expr;
use crate::error::Result;
use crate::lexer::Tok;

impl Parser<'_> {
    /// A whole-line inflation phrase, or `None`.
    pub(super) fn inflation(&mut self) -> Result<Option<Expr>> {
        // Every phrase names a year after "in" or "from".
        let Some(at) = (1..self.toks.len()).find(|&i| self.year_at(i).is_some() && self.words_before_year(i)) else {
            return Ok(None);
        };
        let end = self.toks.len();
        let year = self.year_at(at);
        let inflation = |amount, from, to, rate| Ok(Some(Expr::Inflation { amount, from, to, rate }));

        // "value of $100 in 1990", "what was $100 worth in 1990", "... in 2035 with 3% inflation"
        let lead = [&["value", "of"][..], &["what", "was"], &["what", "will"]]
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
            let rate = self.inflation_rate(at + 1)?;
            if rate.is_none() && at + 1 != end {
                return Ok(None);
            }
            return inflation(amount, None, year, rate);
        }
        // "$100 today in 1990"
        if at >= 2 && self.matches_at(at - 2, &["today", "in"]) && at + 1 == end {
            return inflation(self.part(0, at - 2)?.boxed(), None, year, None);
        }
        let from = at - 1;
        // "$1,000 in 1990 dollars", "€500 in 2001 euros"
        if self.matches_at(from, &["in"])
            && self.unit_at(at + 1, false).is_some_and(|(u, n)| u.is_money() && at + 1 + n == end)
        {
            return inflation(self.part(0, from)?.boxed(), year, None, None);
        }
        // "$100 from 1990 is worth what today", "$100 in 1990 is worth what today"
        if self.matches_at(at + 1, &["is", "worth", "what", "today"]) && at + 5 == end {
            return inflation(self.part(0, from)?.boxed(), year, None, None);
        }
        // "$100 from 1990 in 2000", "$100 from 1990 to today"
        if self.matches_at(from, &["from"]) {
            let to = match () {
                _ if self.matches_at(at + 1, &["in"]) || self.matches_at(at + 1, &["to"]) => at + 2,
                _ => return Ok(None),
            };
            if self.matches_at(to, &["today"]) && to + 1 == end {
                return inflation(self.part(0, from)?.boxed(), year, None, None);
            }
            if let Some(later) = self.year_at(to).filter(|_| to + 1 == end) {
                return inflation(self.part(0, from)?.boxed(), year, Some(later), None);
            }
        }
        Ok(None)
    }

    /// A year at token `i`: a whole number from 1000 to 2999.
    fn year_at(&self, i: usize) -> Option<i16> {
        match &self.toks.get(i)?.tok {
            Tok::Num(n) if n.is_integer() => n.to_i64().filter(|y| (1000..3000).contains(y)).map(|y| y as i16),
            _ => None,
        }
    }

    /// "in" or "from" comes right before the year at `i`.
    fn words_before_year(&self, i: usize) -> bool {
        self.matches_at(i - 1, &["in"]) || self.matches_at(i - 1, &["from"])
    }

    /// "with 3% inflation" from token `i` to the end.
    fn inflation_rate(&self, i: usize) -> Result<Option<Box<Expr>>> {
        let end = self.toks.len();
        if !self.matches_at(i, &["with"]) || !self.ends_with(&["inflation"]) || i + 2 >= end {
            return Ok(None);
        }
        Ok(Some(self.part(i + 1, end - 1)?.boxed()))
    }
}
