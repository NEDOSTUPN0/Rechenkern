//! Turns tokens into a syntax tree.
//!
//! Words the parser doesn't know are skipped as comments, so
//! `$20 for lunch + $15 for taxi` reads as `$20 + $15`.

mod phrases;
mod primary;
mod target;
mod time;
mod tokens;
mod words;

use std::cell::{Cell, OnceCell};
use std::rc::Rc;

use jiff::tz::TimeZone;

use crate::ast::{Expr, Func, Op, Stmt, TimeExpr};
use crate::config::Config;
use crate::error::{Result, bail};
use crate::lexer::{self, Tok, Token};

/// What the parser needs to know about the calculator.
pub struct Scope<'a> {
    pub config: &'a Config,
    pub is_var: &'a dyn Fn(&str) -> bool,
}

/// Parses one line; `None` means the line has nothing to calculate.
pub fn parse(src: &str, scope: &Scope) -> Result<Option<Stmt>> {
    let tokens = lexer::lex(src, scope.config.decimal_comma);
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
    /// Lookups remembered per token, shared by clones over the same tokens.
    memo: Rc<[TokenMemo]>,
}

/// Answers about one token that don't change while parsing a line.
#[derive(Default)]
struct TokenMemo {
    significant: Cell<Option<bool>>,
    place: OnceCell<Option<(TimeZone, usize)>>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str, toks: &'a [Token], scope: &'a Scope<'a>) -> Parser<'a> {
        let memo = toks.iter().map(|_| TokenMemo::default()).collect();
        Parser { src, toks, pos: 0, scope, in_list: false, memo }
    }

    /// A parser over part of the tokens.
    fn sub(&self, from: usize, to: usize) -> Parser<'a> {
        Parser::new(self.src, &self.toks[from..to], self.scope)
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
            // "$100 from 1990" is not date arithmetic.
            if swap && !time::maybe_duration(&lhs) {
                bail!("expected a length of time before \"{}\"", self.src[tok.start..tok.end].trim());
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
                "out" if next_is(self, "of") => (Op::OutOf, 2),
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
        let from_now =
            |expr: Expr, later: bool| Expr::binary(if later { Op::Add } else { Op::Sub }, time::now_for(&expr), expr);
        let mut expr = self.primary()?;
        loop {
            let Some(tok) = self.cur() else { return Ok(expr) };
            // "$120/night" and "5 km per hour" are rates that bind tightly:
            // "4 nights * $120/night" multiplies by the rate. "per cent" is a percentage.
            let rate = tok.is_sym("/") || (tok.is_word("per") && !self.words_at(self.pos + 1, &["cent"]));
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
                    "ago" | "later" => {
                        self.pos += 1;
                        from_now(expr, w.eq_ignore_ascii_case("later"))
                    }
                    // "3 days in the past", "2 weeks in the future"
                    "in" if ["past", "future"].iter().any(|w| self.words_at(self.pos + 1, &["the", w])) => {
                        let later = self.words_at(self.pos + 1, &["the", "future"]);
                        self.pos += 3;
                        from_now(expr, later)
                    }
                    _ => return Ok(expr),
                },
                _ => return Ok(expr),
            };
        }
    }
}
