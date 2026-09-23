//! Whole-line phrases: percentage questions and proportions.

use super::Parser;
use crate::ast::{Expr, Format, Op};
use crate::error::Result;
use crate::lexer::Tok;
use crate::number::Number;

impl Parser<'_> {
    /// Tries phrases such as "20 is what % of 200" on the whole line.
    pub(super) fn phrase(&mut self) -> Result<Option<Expr>> {
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

        // "20 is what % of 200", "180 is what % off 200", "180 is what % on 150"
        for (seq, skip) in [(&["is", "what", "%"][..], 3), (&["as", "a", "%"][..], 3), (&["as", "%"][..], 2)] {
            let Some(at) = self.find(0, seq) else { continue };
            let Some(kind) = self.lower(at + skip).filter(|w| matches!(w.as_str(), "of" | "on" | "off")) else {
                continue;
            };
            let a = self.part(0, at)?;
            let b = self.part(at + skip + 1, end)?;
            let ratio = match kind.as_str() {
                "of" => Expr::binary(Op::Div, a, b),
                "on" => Expr::binary(Op::Div, Expr::binary(Op::Sub, a, b.clone()), b),
                _ => Expr::binary(Op::Div, Expr::binary(Op::Sub, b.clone(), a), b),
            };
            return Ok(Some(pct(ratio)));
        }

        // "20 is 10% of what", "220 is 10% on what", "180 is 10% off what"
        for kind in ["of", "on", "off"] {
            if !self.ends_with(&[kind, "what"]) {
                continue;
            }
            // "10% of what is 20" is handled below.
            let Some(is) = self.find(0, &["is"]).filter(|&i| i < end - 2) else { continue };
            let a = self.part(0, is)?;
            let p = self.part(is + 1, end - 2)?;
            let divisor = match kind {
                "of" => p,
                "on" => Expr::binary(Op::Add, one(), p),
                _ => Expr::binary(Op::Sub, one(), p),
            };
            return Ok(Some(Expr::binary(Op::Div, a, divisor)));
        }

        // "10% of what is 20"
        if let Some(at) = self.find(0, &["of", "what", "is"]) {
            let p = self.part(0, at)?;
            let a = self.part(at + 3, end)?;
            return Ok(Some(Expr::binary(Op::Div, a, p)));
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
                Format::Percent => Expr::binary(Op::Div, Expr::binary(Op::Sub, b, a.clone()), a),
                _ => Expr::binary(Op::Div, b, a),
            };
            return Ok(Some(Self::formatted(change, format)));
        }

        // "3/20 is what %"
        if self.ends_with(&["is", "what", "%"]) {
            let a = self.part(0, end - 3)?;
            return Ok(Some(pct(a)));
        }
        Ok(None)
    }

    /// Parses tokens `from..to` as one expression.
    fn part(&self, from: usize, to: usize) -> Result<Expr> {
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
                "%" => {
                    t.is_sym("%")
                        || t.word()
                            .is_some_and(|w| matches!(w.to_lowercase().as_str(), "percent" | "percentage" | "pct"))
                }
                w => t.is_word(w),
            }
        })
    }

    fn ends_with(&self, seq: &[&str]) -> bool {
        self.toks.len() >= seq.len() && self.matches_at(self.toks.len() - seq.len(), seq)
    }

    /// First occurrence of `seq` from `from`, outside parentheses.
    fn find(&self, from: usize, seq: &[&str]) -> Option<usize> {
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
