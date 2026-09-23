//! Turns tokens into a syntax tree.
//!
//! Words the parser doesn't know are skipped as comments, so
//! `$20 for lunch + $15 for taxi` reads as `$20 + $15`.

mod phrases;
mod primary;
mod target;
mod time;
mod words;

use jiff::tz::TimeZone;

use crate::ast::{Expr, Func, Op, Stmt, TimeExpr};
use crate::config::Config;
use crate::error::{Result, bail};
use crate::lexer::{self, Tok, Token};
use crate::units::{Unit, registry};
use crate::zones;

/// What the parser needs to know about the calculator.
pub struct Scope<'a> {
    pub config: &'a Config,
    pub is_var: &'a dyn Fn(&str) -> bool,
}

/// Parses one line; `None` means the line has nothing to calculate.
pub fn parse(src: &str, scope: &Scope) -> Result<Option<Stmt>> {
    let tokens = lexer::lex(src);
    let tokens = Parser::new(src, &tokens, scope).without_comment_groups();
    let mut parser = Parser::new(src, &tokens, scope);
    parser.statement()
}

#[derive(Clone)]
pub(crate) struct Parser<'a> {
    src: &'a str,
    toks: &'a [Token],
    pos: usize,
    scope: &'a Scope<'a>,
    /// Inside a list or `between`, "and" separates items instead of adding.
    in_list: bool,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str, toks: &'a [Token], scope: &'a Scope<'a>) -> Parser<'a> {
        Parser { src, toks, pos: 0, scope, in_list: false }
    }

    /// A parser over part of the tokens.
    fn sub(&self, from: usize, to: usize) -> Parser<'a> {
        Parser { toks: &self.toks[from..to], pos: 0, in_list: false, ..self.clone() }
    }

    fn config(&self) -> &Config {
        self.scope.config
    }

    // ---- Statements ----

    fn statement(&mut self) -> Result<Option<Stmt>> {
        if !(0..self.toks.len()).any(|i| self.significant(i)) {
            return Ok(None);
        }
        // "if earnings > $30k then tax = 20% else tax = 5%"
        if self.toks.first().is_some_and(|t| t.is_word("if"))
            && let Some(then) = self.find(1, &["then"])
        {
            let end = self.toks.len();
            let otherwise = self.find(then + 1, &["else"]);
            let cond = self.part(1, then)?;
            let branch = |from: usize, to: usize| -> Result<Box<Stmt>> {
                self.sub(from, to).statement()?.map(Box::new).ok_or_else(|| crate::Error::new("empty branch"))
            };
            let then = branch(then + 1, otherwise.unwrap_or(end))?;
            let otherwise = otherwise.map(|e| branch(e + 1, end)).transpose()?;
            return Ok(Some(Stmt::If { cond, then, otherwise }));
        }
        if let Some(stmt) = self.assignment()? {
            return Ok(Some(stmt));
        }
        Ok(Some(Stmt::Expr(self.value()?)))
    }

    /// A whole-line value, with an optional trailing condition:
    /// "true if income > expenses", "false unless expenses > income".
    fn value(&mut self) -> Result<Expr> {
        let end = self.toks.len();
        for (word, negate) in [("if", false), ("unless", true)] {
            let Some(at) = self.find(1, &[word]) else { continue };
            let otherwise = self.find(at + 1, &["else"]);
            let value = Some(self.part(0, at)?.boxed());
            let cond = self.part(at + 1, otherwise.unwrap_or(end))?.boxed();
            let other = otherwise.map(|e| self.part(e + 1, end)).transpose()?.map(Expr::boxed);
            let (then, otherwise) = if negate { (other, value) } else { (value, other) };
            return Ok(Expr::If { cond, then, otherwise });
        }
        match self.phrase()? {
            Some(expr) => Ok(expr),
            None => self.expr_to_end(),
        }
    }

    /// Parses an expression that must use up the tokens.
    fn expr_to_end(&mut self) -> Result<Expr> {
        let expr = self.expr()?;
        self.finish()?;
        Ok(expr)
    }

    /// `name = expr`, where the name is one or more words.
    fn assignment(&mut self) -> Result<Option<Stmt>> {
        let Some(eq) = self.toks.iter().position(|t| matches!(t.tok, Tok::Sym("=" | "+=" | "-="))) else {
            return Ok(None);
        };
        let name_toks = &self.toks[..eq];
        if name_toks.is_empty() || !name_toks.iter().all(|t| t.word().is_some_and(|w| w != "$")) {
            return Ok(None);
        }
        let name = name_toks.iter().filter_map(Token::word).collect::<Vec<_>>().join(" ").to_lowercase();
        let op = match self.toks[eq].tok {
            Tok::Sym("+=") => Some(Op::Add),
            Tok::Sym("-=") => Some(Op::Sub),
            _ => None,
        };
        let expr = self.sub(eq + 1, self.toks.len()).value()?;
        Ok(Some(Stmt::Assign { name, op, expr }))
    }

    /// Fails if meaningful tokens are left over.
    fn finish(&mut self) -> Result<()> {
        while let Some(tok) = self.peek() {
            let weak = match &tok.tok {
                Tok::Sym(")") => true,
                Tok::Word(w) => words::KEYWORDS.contains(&w.to_lowercase().as_str()),
                _ => false,
            };
            if !weak {
                bail!("didn't understand \"{}\"", self.src[tok.start..].trim());
            }
            self.pos += 1;
        }
        Ok(())
    }

    // ---- Expressions, lowest precedence first ----

    pub(crate) fn expr(&mut self) -> Result<Expr> {
        let mut expr = self.logic()?;
        while let Some(next) = self.conversion(&expr)? {
            expr = next;
        }
        Ok(expr)
    }

    /// `and` / `or` between comparisons.
    fn logic(&mut self) -> Result<Expr> {
        let mut lhs = self.comparison()?;
        loop {
            let is_comparison =
                matches!(lhs, Expr::Binary(Op::Eq | Op::Ne | Op::Lt | Op::Gt | Op::Le | Op::Ge | Op::And | Op::Or, ..));
            let op = if self.eat_sym("&&") || (is_comparison && self.eat_word("and")) {
                Op::And
            } else if self.eat_sym("||") || (is_comparison && self.eat_word("or")) {
                Op::Or
            } else {
                return Ok(lhs);
            };
            lhs = Expr::binary(op, lhs, self.comparison()?);
        }
    }

    fn comparison(&mut self) -> Result<Expr> {
        let lhs = self.bit_or()?;
        let op = match self.peek().map(|t| &t.tok) {
            Some(Tok::Sym("==")) => Op::Eq,
            Some(Tok::Sym("!=")) => Op::Ne,
            Some(Tok::Sym("<")) => Op::Lt,
            Some(Tok::Sym(">")) => Op::Gt,
            Some(Tok::Sym("<=")) => Op::Le,
            Some(Tok::Sym(">=")) => Op::Ge,
            _ => return Ok(lhs),
        };
        self.pos += 1;
        Ok(Expr::binary(op, lhs, self.bit_or()?))
    }

    fn bit_or(&mut self) -> Result<Expr> {
        let mut lhs = self.bit_xor()?;
        while self.at_sym("|") && self.operand_follows(1) {
            self.pos += 1;
            lhs = Expr::binary(Op::BitOr, lhs, self.bit_xor()?);
        }
        Ok(lhs)
    }

    fn bit_xor(&mut self) -> Result<Expr> {
        let mut lhs = self.bit_and()?;
        while self.at_word("xor") && self.operand_follows(1) {
            self.pos += 1;
            lhs = Expr::binary(Op::BitXor, lhs, self.bit_and()?);
        }
        Ok(lhs)
    }

    fn bit_and(&mut self) -> Result<Expr> {
        let mut lhs = self.shift()?;
        while self.at_sym("&") && self.operand_follows(1) {
            self.pos += 1;
            lhs = Expr::binary(Op::BitAnd, lhs, self.shift()?);
        }
        Ok(lhs)
    }

    fn shift(&mut self) -> Result<Expr> {
        let mut lhs = self.additive()?;
        loop {
            let op = if self.at_sym("<<") {
                Op::Shl
            } else if self.at_sym(">>") {
                Op::Shr
            } else {
                return Ok(lhs);
            };
            self.pos += 1;
            lhs = Expr::binary(op, lhs, self.additive()?);
        }
    }

    fn additive(&mut self) -> Result<Expr> {
        let mut lhs = self.multiplicative()?;
        loop {
            let Some(tok) = self.peek() else { return Ok(lhs) };
            let (op, len, swap) = match &tok.tok {
                Tok::Sym("+") => (Op::Add, 1, false),
                Tok::Sym("-") => (Op::Sub, 1, false),
                Tok::Word(w) => match w.to_lowercase().as_str() {
                    "plus" => (Op::Add, 1, false),
                    "minus" => (Op::Sub, 1, false),
                    // "$20 and $15" adds, "x > 1 and x < 5" is logic.
                    "and" if !self.in_list && self.amount_follows(1) => (Op::Add, 1, false),
                    // "3 days after March 1", "2 weeks from now"
                    "after" | "from" => (Op::Add, 1, true),
                    "before" => (Op::Sub, 1, true),
                    _ => return Ok(lhs),
                },
                _ => return Ok(lhs),
            };
            if !self.operand_follows(len) {
                return Ok(lhs);
            }
            self.pos += len;
            let mut rhs = self.multiplicative()?;
            // "4 days from now" is a date, not a clock time.
            if swap && matches!(rhs, Expr::Time(TimeExpr::Now)) {
                rhs = time::now_for(&lhs);
            }
            lhs = if swap { Expr::binary(op, rhs, lhs) } else { Expr::binary(op, lhs, rhs) };
        }
    }

    fn multiplicative(&mut self) -> Result<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let Some((op, len)) = self.mul_operator() else {
                // Implicit multiplication: "2(3 + 4)", "2 pi", "3 sqrt 4".
                if self.implicit_mul_follows() {
                    lhs = Expr::binary(Op::Mul, lhs, self.unary()?);
                    continue;
                }
                return Ok(lhs);
            };
            if !self.operand_follows(len) {
                return Ok(lhs);
            }
            // "3 permutations of 10" means 10 permutation 3.
            let swap = matches!(op, Op::Perm | Op::Comb) && len == 2;
            self.pos += len;
            // "30 hours at $30/hour": the rate on the right is one value.
            let rhs =
                if matches!(op, Op::At | Op::Of | Op::On | Op::Off) { self.multiplicative()? } else { self.unary()? };
            // "per day" and "/ day" make a rate instead of cancelling units.
            let op = if op == Op::Div && matches!(rhs, Expr::BareUnit(_)) { Op::Per } else { op };
            lhs = if swap { Expr::binary(op, rhs, lhs) } else { Expr::binary(op, lhs, rhs) };
        }
    }

    /// The multiplicative operator at the cursor and its length in tokens.
    fn mul_operator(&mut self) -> Option<(Op, usize)> {
        // "$24 a day", "3 times a week": an article before a unit means "per".
        if self.article_unit_at(self.pos) {
            return Some((Op::Per, 1));
        }
        if self.cur().is_some_and(|t| t.is_word("times")) && self.article_unit_at(self.pos + 1) {
            return Some((Op::Per, 2));
        }
        let tok = self.peek()?;
        let next_is = |p: &Self, w: &str| p.toks.get(p.pos + 1).is_some_and(|t| t.is_word(w));
        Some(match &tok.tok {
            Tok::Sym("*") => (Op::Mul, 1),
            Tok::Sym("/") => (Op::Div, 1),
            Tok::Sym("@") => (Op::At, 1),
            // "10 % 3" is modulo when a value follows; otherwise a percentage.
            Tok::Sym("%") if self.modulo_follows() => (Op::Mod, 1),
            Tok::Word(w) => match w.to_lowercase().as_str() {
                "x" if !(self.scope.is_var)("x") || self.operand_follows(1) => (Op::Mul, 1),
                "times" => (Op::Mul, 1),
                "multiplied" if next_is(self, "by") => (Op::Mul, 2),
                "divided" if next_is(self, "by") => (Op::Div, 2),
                "per" => (Op::Div, 1),
                "split" => (Op::Div, 1),
                // "$24 a day for a year": a rate over a time span.
                "for" if self.duration_at(self.pos + 1) => (Op::Mul, 1),
                "mod" | "modulo" => (Op::Mod, 1),
                "of" => (Op::Of, 1),
                "on" => (Op::On, 1),
                "off" => (Op::Off, 1),
                "at" => (Op::At, 1),
                "permutation" | "permutations" if next_is(self, "of") => (Op::Perm, 2),
                "combination" | "combinations" if next_is(self, "of") => (Op::Comb, 2),
                "permutation" | "permutations" => (Op::Perm, 1),
                "combination" | "combinations" | "choose" => (Op::Comb, 1),
                _ => return None,
            },
            _ => return None,
        })
    }

    fn unary(&mut self) -> Result<Expr> {
        let Some(tok) = self.peek() else { return self.primary() };
        match &tok.tok {
            Tok::Sym("-") => {
                self.pos += 1;
                Ok(Expr::Neg(self.unary()?.boxed()))
            }
            Tok::Sym("+") => {
                self.pos += 1;
                self.unary()
            }
            Tok::Sym("√") => {
                self.pos += 1;
                Ok(Expr::Call(Func::Sqrt, vec![self.unary()?]))
            }
            Tok::Sym("∛") => {
                self.pos += 1;
                Ok(Expr::Call(Func::Cbrt, vec![self.unary()?]))
            }
            Tok::Word(w) => {
                let factor = match w.to_lowercase().as_str() {
                    "minus" | "negative" => -1,
                    "twice" | "double" => 2,
                    "triple" => 3,
                    "quadruple" => 4,
                    _ => return self.power(),
                };
                self.pos += 1;
                self.eat_word("of");
                // "twice a day" is a rate.
                if self.article_unit_at(self.pos) {
                    self.pos += 1;
                    let unit = self.unary()?;
                    return Ok(Expr::binary(Op::Per, Expr::Number(factor.into()), unit));
                }
                let operand = self.unary()?;
                Ok(Expr::binary(Op::Mul, Expr::Number(factor.into()), operand))
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<Expr> {
        let base = self.postfix()?;
        if self.at_sym("^") {
            self.pos += 1;
            return Ok(Expr::binary(Op::Pow, base, self.unary()?));
        }
        // "3 to the power of 2"
        if self.at_word("to") && self.words_at(self.pos + 1, &["the", "power", "of"]) {
            self.pos += 4;
            return Ok(Expr::binary(Op::Pow, base, self.unary()?));
        }
        Ok(base)
    }

    fn postfix(&mut self) -> Result<Expr> {
        let mut expr = self.primary()?;
        loop {
            let Some(tok) = self.cur() else { return Ok(expr) };
            // "$120/night" and "5 km per hour" are rates that bind tightly:
            // "4 nights * $120/night" multiplies by the rate.
            let rate = tok.is_sym("/") || tok.is_word("per");
            if rate && self.unit_at(self.pos + 1, false).is_some() {
                self.pos += 1;
                let unit = self.unit_phrase(false).expect("unit checked above");
                expr = Expr::binary(Op::Per, expr, Expr::BareUnit(unit));
                continue;
            }
            expr = match &tok.tok {
                Tok::Sym("%") if !self.modulo_follows() => {
                    self.pos += 1;
                    Expr::Percent(expr.boxed())
                }
                Tok::Sym("!") => {
                    self.pos += 1;
                    Expr::Factorial(expr.boxed())
                }
                Tok::Sym("²") | Tok::Sym("³") => {
                    let exp = if tok.is_sym("²") { 2 } else { 3 };
                    self.pos += 1;
                    Expr::binary(Op::Pow, expr, Expr::Number(exp.into()))
                }
                Tok::Word(w) => match w.to_lowercase().as_str() {
                    "percent" | "pct" => {
                        self.pos += 1;
                        Expr::Percent(expr.boxed())
                    }
                    "per" if self.toks.get(self.pos + 1).is_some_and(|t| t.is_word("cent")) => {
                        self.pos += 2;
                        Expr::Percent(expr.boxed())
                    }
                    "squared" | "cubed" => {
                        let exp = if w.eq_ignore_ascii_case("squared") { 2 } else { 3 };
                        self.pos += 1;
                        Expr::binary(Op::Pow, expr, Expr::Number(exp.into()))
                    }
                    "ago" => {
                        self.pos += 1;
                        Expr::binary(Op::Sub, time::now_for(&expr), expr)
                    }
                    "later" => {
                        self.pos += 1;
                        Expr::binary(Op::Add, time::now_for(&expr), expr)
                    }
                    _ => return Ok(expr),
                },
                _ => return Ok(expr),
            };
        }
    }

    // ---- Token helpers ----

    fn cur(&self) -> Option<&'a Token> {
        self.toks.get(self.pos)
    }

    /// Skips words that mean nothing here, like "for lunch".
    fn skip_noise(&mut self) {
        while self.cur().is_some_and(|t| t.word().is_some()) && !self.significant(self.pos) {
            self.pos += 1;
        }
    }

    /// The next meaningful token.
    fn peek(&mut self) -> Option<&'a Token> {
        self.skip_noise();
        self.cur()
    }

    fn lower(&self, i: usize) -> Option<String> {
        self.toks.get(i)?.word().map(str::to_lowercase)
    }

    /// Moves to the word `w` if only comments come before it.
    fn at_word(&mut self, w: &str) -> bool {
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

    fn eat_word(&mut self, w: &str) -> bool {
        let found = self.at_word(w);
        if found {
            self.pos += 1;
        }
        found
    }

    fn at_sym(&mut self, s: &str) -> bool {
        self.peek().is_some_and(|t| t.is_sym(s))
    }

    fn eat_sym(&mut self, s: &str) -> bool {
        let found = self.at_sym(s);
        if found {
            self.pos += 1;
        }
        found
    }

    /// The raw tokens at `i` are exactly these words.
    fn words_at(&self, i: usize, words: &[&str]) -> bool {
        words.iter().enumerate().all(|(k, w)| self.toks.get(i + k).is_some_and(|t| t.is_word(w)))
    }

    /// `n` consecutive words from `i`, joined by spaces.
    fn join_words(&self, i: usize, n: usize) -> Option<String> {
        let words: Option<Vec<&str>> = (i..i + n).map(|k| self.toks.get(k)?.word()).collect();
        Some(words?.join(" "))
    }

    /// Longest phrase from `table` starting at token `i`.
    fn phrase_at<T: Copy>(&self, i: usize, table: &[(&str, T)]) -> Option<(T, usize)> {
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
    fn significant(&self, i: usize) -> bool {
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
            || self.phrase_at(i, words::PHYSICAL_CONSTANTS).is_some()
            || self.unit_at(i, false).is_some()
            || self.var_at(i).is_some()
            || self.place_time_at(i).is_some()
    }

    /// A unit spelled at token `i`: its value and length in tokens.
    fn unit_at(&self, i: usize, after_number: bool) -> Option<(Unit, usize)> {
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

    fn dollar(&self) -> Unit {
        registry().currency(&self.config().dollar).unwrap_or_else(|| registry().get("USD"))
    }

    /// A variable name made of the words at `i`.
    fn var_at(&self, i: usize) -> Option<(String, usize)> {
        (1..=6).rev().find_map(|n| {
            let name = self.join_words(i, n)?.to_lowercase();
            (self.scope.is_var)(&name).then_some((name, n))
        })
    }

    fn place_at(&self, i: usize) -> Option<(TimeZone, usize)> {
        (1..=zones::MAX_WORDS).rev().find_map(|n| Some((zones::find(&self.join_words(i, n)?.to_lowercase())?, n)))
    }

    /// "Tokyo time" or "Paris date".
    fn place_time_at(&self, i: usize) -> Option<(TimeZone, usize, bool)> {
        let (tz, n) = self.place_at(i)?;
        let next = self.lower(i + n)?;
        matches!(next.as_str(), "time" | "date").then_some((tz, n + 1, next == "date"))
    }

    /// An operand starts after skipping `skip` tokens.
    fn operand_follows(&self, skip: usize) -> bool {
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
    fn amount_follows(&self, skip: usize) -> bool {
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
    fn article_unit_at(&self, i: usize) -> bool {
        self.toks.get(i).is_some_and(|t| ["a", "an", "each", "every"].iter().any(|w| t.is_word(w)))
            && self.unit_at(i + 1, false).is_some()
    }

    /// A time amount starts at token `i`: "2 hours", "a year".
    fn duration_at(&self, i: usize) -> bool {
        let amount = self
            .toks
            .get(i)
            .is_some_and(|t| matches!(t.tok, Tok::Num(_)) || ["a", "an", "one"].iter().any(|w| t.is_word(w)));
        amount && self.unit_at(i + 1, true).is_some_and(|(u, _)| u.dim() == crate::units::Dim::TIME)
    }

    /// The `%` at the cursor is modulo: `10 % 3`, but not `10% + 5` or `10%3`.
    fn modulo_follows(&self) -> bool {
        let spaced = self.toks.get(self.pos + 1).is_some_and(|t| t.space_before);
        let sign = self.toks.get(self.pos + 1).is_some_and(|t| t.is_sym("+") || t.is_sym("-"));
        spaced && !sign && self.operand_follows(1)
    }

    /// Something that multiplies without an operator follows: "(", a function,
    /// a constant or a variable.
    fn implicit_mul_follows(&mut self) -> bool {
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
    fn without_comment_groups(&self) -> Vec<Token> {
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
