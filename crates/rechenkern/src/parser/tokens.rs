//! Token helpers: looking ahead, skipping comment words, spotting units and names.

use jiff::tz::TimeZone;

use super::{Parser, words};
use crate::ast::Func;
use crate::lexer::{Tok, Token};
use crate::units::{Unit, registry};
use crate::zones;

impl<'a> Parser<'a> {
    pub(super) fn cur(&self) -> Option<&'a Token> {
        self.toks.get(self.pos)
    }

    /// Skips words that mean nothing here, like "for lunch".
    pub(super) fn skip_noise(&mut self) {
        while self.cur().is_some_and(|t| t.word().is_some()) && !self.significant(self.pos) {
            self.pos += 1;
        }
    }

    /// The next meaningful token.
    pub(super) fn peek(&mut self) -> Option<&'a Token> {
        self.skip_noise();
        self.cur()
    }

    pub(super) fn lower(&self, i: usize) -> Option<String> {
        self.toks.get(i)?.word().map(str::to_lowercase)
    }

    /// Moves to the word `w` if only comments come before it.
    pub(super) fn at_word(&mut self, w: &str) -> bool {
        let mut i = self.pos;
        while let Some(t) = self.toks.get(i) {
            if t.is_word(w) {
                self.pos = i;
                return true;
            }
            if t.word().is_none() || self.significant(i) {
                return false;
            }
            i += 1;
        }
        false
    }

    pub(super) fn eat_word(&mut self, w: &str) -> bool {
        let found = self.at_word(w);
        if found {
            self.pos += 1;
        }
        found
    }

    pub(super) fn at_sym(&mut self, s: &str) -> bool {
        self.peek().is_some_and(|t| t.is_sym(s))
    }

    pub(super) fn eat_sym(&mut self, s: &str) -> bool {
        let found = self.at_sym(s);
        if found {
            self.pos += 1;
        }
        found
    }

    /// The raw tokens at `i` are exactly these words.
    pub(super) fn words_at(&self, i: usize, words: &[&str]) -> bool {
        words.iter().enumerate().all(|(k, w)| self.toks.get(i + k).is_some_and(|t| t.is_word(w)))
    }

    /// `n` consecutive words from `i`, joined by spaces.
    pub(super) fn join_words(&self, i: usize, n: usize) -> Option<String> {
        let words: Option<Vec<&str>> = (i..i + n).map(|k| self.toks.get(k)?.word()).collect();
        Some(words?.join(" "))
    }

    /// Longest phrase from `table` starting at token `i`.
    pub(super) fn phrase_at<T: Copy>(&self, i: usize, table: &[(&str, T)]) -> Option<(T, usize)> {
        let mut best: Option<(T, usize)> = None;
        for &(phrase, value) in table {
            let n = phrase.split(' ').count();
            if best.is_some_and(|(_, m)| m >= n) {
                continue;
            }
            if self.join_words(i, n).is_some_and(|w| w.eq_ignore_ascii_case(phrase)) {
                best = Some((value, n));
            }
        }
        best
    }

    /// Whether the token at `i` is a word worth parsing.
    pub(super) fn significant(&self, i: usize) -> bool {
        let Some(tok) = self.toks.get(i) else { return false };
        let Some(word) = tok.word() else { return true };
        let w = word.to_lowercase();
        let w = w.as_str();
        words::KEYWORDS.contains(&w)
            || w == "x"
            || self.article_unit_at(i)
            // "for 2 hours" and "split 4 ways" only matter before an amount.
            || (w == "for" && self.duration_at(i + 1))
            || (w == "split" && matches!(self.toks.get(i + 1).map(|t| &t.tok), Some(Tok::Num(_))))
            || words::number_word(w).is_some()
            || words::scale_word(w).is_some()
            || words::fraction_word(w).is_some()
            || words::constant(w).is_some()
            || words::month(w).is_some()
            || words::weekday(w).is_some()
            || words::is_line_word(w)
            || self.phrase_at(i, words::FUNCTIONS).is_some()
            || self.phrase_at(i, words::HOLIDAYS).is_some()
            || self.phrase_at(i, words::DATE_PARTS).is_some()
            || self.phrase_at(i, words::PHYSICAL_CONSTANTS).is_some()
            || self.unit_at(i, false).is_some()
            || self.var_at(i).is_some()
            || self.place_time_at(i).is_some()
    }

    /// A unit spelled at token `i`: its value and length in tokens.
    pub(super) fn unit_at(&self, i: usize, after_number: bool) -> Option<(Unit, usize)> {
        let tok = self.toks.get(i)?;
        if tok.is_sym("°") {
            // "°C" or a lone degree sign.
            if let Some(next) = self.toks.get(i + 1).filter(|t| !t.space_before).and_then(Token::word)
                && let Some(u) = registry().lookup(&format!("°{next}"))
            {
                return Some((u, 2));
            }
            return Some((registry().get("°"), 1));
        }
        let word = tok.word()?;
        if word == "$" {
            return Some((self.dollar(), 1));
        }
        if word.eq_ignore_ascii_case("in") || (!after_number && matches!(word, "a" | "A" | "an")) {
            return None;
        }
        // "min(" is a function.
        if self.toks.get(i + 1).is_some_and(|t| t.is_sym("(") && !t.space_before)
            && self.phrase_at(i, words::FUNCTIONS).is_some()
        {
            return None;
        }
        let max = registry().max_words();
        (1..=max).rev().find_map(|n| Some((registry().lookup(&self.join_words(i, n)?)?, n)))
    }

    pub(super) fn dollar(&self) -> Unit {
        registry().currency(&self.config().dollar).unwrap_or_else(|| registry().get("USD"))
    }

    /// A variable name made of the words at `i`.
    pub(super) fn var_at(&self, i: usize) -> Option<(String, usize)> {
        (1..=6).rev().find_map(|n| {
            let name = self.join_words(i, n)?.to_lowercase();
            (self.scope.is_var)(&name).then_some((name, n))
        })
    }

    /// A place name at token `i`; words may be joined by hyphens: "Aix-en-Provence".
    pub(super) fn place_at(&self, i: usize) -> Option<(TimeZone, usize)> {
        let (words, ends) = self.name_words(i);
        (1..=words.len()).rev().find_map(|n| Some((zones::find(&words[..n].join(" "))?, ends[n - 1] - i)))
    }

    /// Up to `MAX_WORDS` lowercase words from token `i`, and the token index after each.
    pub(super) fn name_words(&self, i: usize) -> (Vec<String>, Vec<usize>) {
        let (mut words, mut ends) = (Vec::new(), Vec::new());
        let mut k = i;
        while words.len() < zones::MAX_WORDS {
            let Some(word) = self.toks.get(k).and_then(Token::word) else { break };
            words.push(word.to_lowercase());
            k += 1;
            ends.push(k);
            let hyphen = self.toks.get(k).is_some_and(|t| t.is_sym("-") && !t.space_before);
            if hyphen && self.toks.get(k + 1).is_some_and(|t| t.word().is_some() && !t.space_before) {
                k += 1;
            }
        }
        (words, ends)
    }

    /// "Tokyo time" or "Paris date".
    pub(super) fn place_time_at(&self, i: usize) -> Option<(TimeZone, usize, bool)> {
        let (tz, n) = self.place_at(i)?;
        let next = self.lower(i + n)?;
        matches!(next.as_str(), "time" | "date").then_some((tz, n + 1, next == "date"))
    }

    /// An operand starts after skipping `skip` tokens.
    pub(super) fn operand_follows(&self, skip: usize) -> bool {
        let mut p = self.clone();
        p.pos += skip;
        let Some(tok) = p.peek() else { return false };
        match &tok.tok {
            Tok::Num(_) | Tok::Clock { .. } | Tok::Date { .. } | Tok::IsoDateTime(_) => true,
            Tok::Sym(s) => matches!(*s, "(" | "-" | "+" | "√" | "∛"),
            Tok::Word(w) => {
                let w = w.to_lowercase();
                !matches!(
                    w.as_str(),
                    "in" | "to"
                        | "as"
                        | "into"
                        | "of"
                        | "on"
                        | "off"
                        | "at"
                        | "per"
                        | "and"
                        | "or"
                        | "xor"
                        | "mod"
                        | "ago"
                        | "times"
                        | "plus"
                )
            }
        }
    }

    /// A number or an amount of money starts `skip` tokens ahead.
    pub(super) fn amount_follows(&self, skip: usize) -> bool {
        let mut p = self.clone();
        p.pos += skip;
        match p.peek().map(|t| &t.tok) {
            Some(Tok::Num(_)) => true,
            Some(Tok::Word(_)) => p.unit_at(p.pos, false).is_some_and(|(u, n)| {
                u.is_money() && matches!(p.toks.get(p.pos + n).map(|t| &t.tok), Some(Tok::Num(_)))
            }),
            _ => false,
        }
    }

    /// "a day", "each month": an article and a unit at token `i`.
    pub(super) fn article_unit_at(&self, i: usize) -> bool {
        self.toks.get(i).is_some_and(|t| ["a", "an", "each", "every"].iter().any(|w| t.is_word(w)))
            && self.unit_at(i + 1, false).is_some()
    }

    /// A time amount starts at token `i`: "2 hours", "a year".
    pub(super) fn duration_at(&self, i: usize) -> bool {
        let amount = self
            .toks
            .get(i)
            .is_some_and(|t| matches!(t.tok, Tok::Num(_)) || ["a", "an", "one"].iter().any(|w| t.is_word(w)));
        amount && self.unit_at(i + 1, true).is_some_and(|(u, _)| u.dim() == crate::units::Dim::TIME)
    }

    /// The `%` at the cursor is modulo: `10 % 3`, but not `10% + 5` or `10%3`.
    pub(super) fn modulo_follows(&self) -> bool {
        let spaced = self.toks.get(self.pos + 1).is_some_and(|t| t.space_before);
        let sign = self.toks.get(self.pos + 1).is_some_and(|t| t.is_sym("+") || t.is_sym("-"));
        spaced && !sign && self.operand_follows(1)
    }

    /// Something that multiplies without an operator follows: "(", a function,
    /// a constant or a variable.
    pub(super) fn implicit_mul_follows(&mut self) -> bool {
        let Some(tok) = self.peek() else { return false };
        if tok.is_sym("(") {
            return true;
        }
        let Some(w) = self.lower(self.pos) else { return false };
        words::constant(&w).is_some()
            || self.var_at(self.pos).is_some()
            || (self
                .phrase_at(self.pos, words::FUNCTIONS)
                .is_some_and(|(f, _)| !matches!(f, Func::Sum | Func::Average | Func::Count))
                && self.toks.get(self.pos + 1).is_some_and(|t| t.is_sym("(")))
    }

    /// Removes parentheses that hold comments: "$999 (for iPhone 16)".
    pub(super) fn without_comment_groups(&self) -> Vec<Token> {
        let mut keep = vec![true; self.toks.len()];
        let mut i = 0;
        while i < self.toks.len() {
            if self.toks[i].is_sym("(") {
                let mut depth = 0;
                let close = (i..self.toks.len()).find(|&k| {
                    match self.toks[k].tok {
                        Tok::Sym("(") => depth += 1,
                        Tok::Sym(")") => depth -= 1,
                        _ => {}
                    }
                    depth == 0
                });
                if let Some(close) = close {
                    let comment = (i + 1..close)
                        .any(|k| self.toks[k].word().is_some() && !self.significant(k) && self.place_at(k).is_none());
                    if comment {
                        keep[i..=close].iter_mut().for_each(|k| *k = false);
                        i = close;
                    }
                }
            }
            i += 1;
        }
        self.toks.iter().zip(keep).filter(|(_, k)| *k).map(|(t, _)| t.clone()).collect()
    }
}
