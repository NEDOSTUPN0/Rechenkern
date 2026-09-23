//! Splits a line into tokens. Words stay raw; the parser decides what they mean.

use crate::number::Number;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// Number literal: decimal, hex, binary or octal.
    Num(Number),
    /// Letters, digits and underscores ("km", "line1", "US$"), or a currency sign.
    Word(String),
    /// Operator or punctuation, normalized ("×" becomes "*").
    Sym(&'static str),
    /// Clock time `h:mm[:ss[.fff]]`.
    Clock { h: u32, m: u32, s: Option<Number> },
    /// Date made of numbers; `sep` tells the order: '-' is year-month-day.
    Date { parts: [u32; 3], sep: char },
    /// Full ISO 8601 timestamp such as `2019-04-01T15:30:00Z`.
    IsoDateTime(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    /// Byte range in the source line.
    pub start: usize,
    pub end: usize,
    /// Whitespace precedes the token.
    pub space_before: bool,
}

impl Token {
    pub fn word(&self) -> Option<&str> {
        match &self.tok {
            Tok::Word(w) => Some(w),
            _ => None,
        }
    }

    pub fn is_word(&self, w: &str) -> bool {
        self.word().is_some_and(|x| x.eq_ignore_ascii_case(w))
    }

    pub fn is_sym(&self, s: &str) -> bool {
        matches!(self.tok, Tok::Sym(x) if x == s)
    }
}

const CURRENCY_SIGNS: &str = "$€£¥₹₽₸₴₩₪₺₫₱₦₿฿¢₡₾₼₮₭₵৳֏元円";

/// Multi-character operators, longest first, with their normalized form.
const MULTI_SYMBOLS: &[(&str, &str)] = &[
    ("**", "^"),
    ("<<", "<<"),
    (">>", ">>"),
    ("->", "->"),
    ("==", "=="),
    ("!=", "!="),
    ("<=", "<="),
    (">=", ">="),
    ("+=", "+="),
    ("-=", "-="),
    ("&&", "&&"),
    ("||", "||"),
];

fn single_symbol(c: char) -> Option<&'static str> {
    Some(match c {
        '+' => "+",
        '-' | '−' | '–' | '—' => "-",
        '*' | '×' | '·' | '⋅' | '∙' => "*",
        '/' | '÷' | '∕' => "/",
        '^' => "^",
        '%' => "%",
        '!' => "!",
        '(' | '[' | '{' => "(",
        ')' | ']' | '}' => ")",
        ',' | ';' => ",",
        '=' => "=",
        '<' => "<",
        '>' => ">",
        '&' => "&",
        '|' => "|",
        '~' => "~",
        '@' => "@",
        '°' | 'º' => "°",
        '√' => "√",
        '∛' => "∛",
        '²' => "²",
        '³' => "³",
        '→' => "->",
        _ => return None,
    })
}

fn vulgar_fraction(c: char) -> Option<(i64, i64)> {
    Some(match c {
        '½' => (1, 2),
        '⅓' => (1, 3),
        '⅔' => (2, 3),
        '¼' => (1, 4),
        '¾' => (3, 4),
        '⅕' => (1, 5),
        '⅛' => (1, 8),
        _ => return None,
    })
}

struct Lexer<'a> {
    src: &'a str,
    chars: Vec<(usize, char)>,
    i: usize,
    tokens: Vec<Token>,
    space: bool,
    /// Decides `1,500` and `1.500`: with a decimal comma they are 1500.
    decimal_comma: bool,
    /// Open parentheses; `true` for a call like `max(`, where commas separate arguments.
    parens: Vec<bool>,
}

pub fn lex(src: &str, decimal_comma: bool) -> Vec<Token> {
    let mut lx = Lexer {
        src,
        chars: src.char_indices().collect(),
        i: 0,
        tokens: Vec::new(),
        space: false,
        decimal_comma,
        parens: Vec::new(),
    };
    lx.run();
    lx.tokens
}

impl Lexer<'_> {
    fn peek(&self, k: usize) -> Option<char> {
        self.chars.get(self.i + k).map(|&(_, c)| c)
    }

    fn offset(&self, i: usize) -> usize {
        self.chars.get(i).map_or(self.src.len(), |&(o, _)| o)
    }

    fn push(&mut self, tok: Tok, start: usize) {
        if tok == Tok::Sym("(") {
            let call = !self.space && matches!(self.tokens.last(), Some(Token { tok: Tok::Word(_), .. }));
            self.parens.push(call);
        } else if tok == Tok::Sym(")") {
            self.parens.pop();
        }
        let token = Token { tok, start: self.offset(start), end: self.offset(self.i), space_before: self.space };
        self.tokens.push(token);
        self.space = false;
    }

    /// The previous token is a number glued to the current position.
    fn after_number(&self) -> bool {
        !self.space && matches!(self.tokens.last(), Some(Token { tok: Tok::Num(_), .. }))
    }

    fn run(&mut self) {
        while let Some(c) = self.peek(0) {
            let start = self.i;
            if c.is_whitespace() {
                self.space = true;
                self.i += 1;
            } else if c == '/' && self.peek(1) == Some('/') {
                break;
            } else if c.is_ascii_digit() || (c == '.' && self.peek(1).is_some_and(|d| d.is_ascii_digit())) {
                let tok = self.number();
                self.push(tok, start);
            } else if let Some((a, b)) = vulgar_fraction(c) {
                self.i += 1;
                let frac = Number::ratio(a, b);
                let end_offset = self.offset(self.i);
                match self.tokens.last_mut() {
                    // "1½" is one and a half.
                    Some(Token { tok: Tok::Num(n), end, .. }) if !self.space => {
                        *n = *n + frac;
                        *end = end_offset;
                    }
                    _ => self.push(Tok::Num(frac), start),
                }
            } else if c.is_alphabetic() || c == '_' {
                let tok = self.word();
                self.push(tok, start);
            } else if CURRENCY_SIGNS.contains(c) {
                self.i += 1;
                self.push(Tok::Word(c.to_string()), start);
            } else if matches!(c, '"' | '″' | '“' | '”') {
                self.i += 1;
                if self.after_number() {
                    self.push(Tok::Sym("\""), start);
                } else {
                    // "Quoted text" is an inline comment.
                    while self.peek(0).is_some_and(|c| !matches!(c, '"' | '“' | '”')) {
                        self.i += 1;
                    }
                    self.i += 1;
                    self.space = true;
                }
            } else if matches!(c, '\'' | '′' | '’') {
                self.i += 1;
                if self.after_number() {
                    self.push(Tok::Sym("'"), start);
                }
            } else if let Some(&(text, sym)) =
                MULTI_SYMBOLS.iter().find(|(t, _)| self.src[self.offset(self.i)..].starts_with(t))
            {
                self.i += text.chars().count();
                self.push(Tok::Sym(sym), start);
            } else if let Some(sym) = single_symbol(c) {
                self.i += 1;
                self.push(Tok::Sym(sym), start);
            } else {
                // Unknown punctuation such as ':' or '?' separates words.
                self.i += 1;
                self.space = true;
            }
        }
    }

    fn word(&mut self) -> Tok {
        let start = self.i;
        while let Some(c) = self.peek(0) {
            let apostrophe = matches!(c, '\'' | '’') && self.peek(1).is_some_and(char::is_alphabetic);
            if c.is_alphanumeric() || c == '_' || apostrophe {
                self.i += 1;
            } else {
                break;
            }
        }
        let mut word: String =
            self.chars[start..self.i].iter().map(|&(_, c)| if c == '’' { '\'' } else { c }).collect();
        // "a.m." and "p.m."
        if matches!(word.as_str(), "a" | "p" | "A" | "P")
            && self.peek(0) == Some('.')
            && matches!(self.peek(1), Some('m' | 'M'))
        {
            self.i += 2;
            if self.peek(0) == Some('.') {
                self.i += 1;
            }
            word = format!("{}m", word.to_lowercase());
        }
        // "US$", "C$", "R$"
        if self.peek(0) == Some('$') && word.chars().all(|c| c.is_ascii_uppercase()) {
            self.i += 1;
            word.push('$');
        }
        Tok::Word(word)
    }

    fn digits(&self, from: usize) -> usize {
        self.chars[from..].iter().take_while(|(_, c)| c.is_ascii_digit()).count()
    }

    fn slice(&self, from: usize, to: usize) -> &str {
        &self.src[self.offset(from)..self.offset(to)]
    }

    fn number(&mut self) -> Tok {
        if let Some(tok) = self.radix_number().or_else(|| self.date()).or_else(|| self.clock()) {
            return tok;
        }
        let at = |lx: &Self, k: usize| lx.chars.get(k).map(|&(_, c)| c);
        let mut text = self.digit_run();
        let mut groups = Vec::new();
        while let Some(sep @ (',' | '.')) = self.peek(0)
            && self.peek(1).is_some_and(|c| c.is_ascii_digit())
        {
            let start = self.i;
            self.i += 1;
            groups.push((sep, self.digit_run(), start));
        }
        let (used, decimal) = self.separators(&text, &groups);
        if let Some(&(_, _, end)) = groups.get(used) {
            self.i = end;
        }
        for (k, (_, digits, _)) in groups[..used].iter().enumerate() {
            if Some(k) == decimal {
                text.push('.');
            }
            text.push_str(digits);
        }
        // Exponent: "1.5e-3".
        if matches!(self.peek(0), Some('e' | 'E')) {
            let sign = matches!(self.peek(1), Some('+' | '-')) as usize;
            if at(self, self.i + 1 + sign).is_some_and(|c| c.is_ascii_digit()) {
                let digits = self.digits(self.i + 1 + sign);
                text.push_str(self.slice(self.i, self.i + 1 + sign + digits));
                self.i += 1 + sign + digits;
            }
        }
        Tok::Num(Number::parse(&text).unwrap_or(Number::ZERO))
    }

    /// Digits, allowing `_` between them: `1_000`.
    fn digit_run(&mut self) -> String {
        let mut digits = String::new();
        loop {
            match self.peek(0) {
                Some(c) if c.is_ascii_digit() => digits.push(c),
                Some('_') if !digits.is_empty() && self.peek(1).is_some_and(|c| c.is_ascii_digit()) => {}
                _ => return digits,
            }
            self.i += 1;
        }
    }

    /// How many separator groups belong to the number, and which one is the decimal point.
    /// Thousands come in groups of three, so only `1,500` or `1.500` needs the setting.
    fn separators(&self, int: &str, groups: &[(char, String, usize)]) -> (usize, Option<usize>) {
        let Some(&(first, _, _)) = groups.first() else { return (0, None) };
        let whole = !int.is_empty() && !int.starts_with('0');
        let run = groups.iter().take_while(|(sep, digits, _)| *sep == first && digits.len() == 3).count();
        let decimal_after = groups.get(run).is_some_and(|(sep, _, _)| *sep != first);
        let grouping = if self.decimal_comma { '.' } else { ',' };
        let (used, decimal) = if whole && int.len() <= 3 && (run >= 2 || (run == 1 && decimal_after)) {
            // "1,234,567", "1.234,56": the other separator marks the decimals.
            if decimal_after { (run + 1, Some(run)) } else { (run, None) }
        } else if run >= 1 && whole && first == grouping {
            (1, None)
        } else if first == ',' && groups.len() > 1 {
            // "1,5,3" is a list.
            (0, None)
        } else {
            (1, Some(0))
        };
        // In `max(1,5)` the comma separates arguments, in `1,2,3` items.
        let in_call = self.parens.last() == Some(&true);
        if (in_call || self.in_glued_list()) && decimal.is_some_and(|k| groups[k].0 == ',') {
            return (0, None);
        }
        (used, decimal)
    }

    /// The number continues a list without spaces, like the `3` in `1,2,3`.
    fn in_glued_list(&self) -> bool {
        match self.tokens.as_slice() {
            [.., Token { tok: Tok::Num(_), .. }, comma] => comma.is_sym(",") && !comma.space_before && !self.space,
            _ => false,
        }
    }

    fn radix_number(&mut self) -> Option<Tok> {
        if self.peek(0) != Some('0') {
            return None;
        }
        let radix = match self.peek(1)? {
            'x' | 'X' => 16,
            'b' | 'B' => 2,
            'o' | 'O' => 8,
            _ => return None,
        };
        let len = self.chars[self.i + 2..].iter().take_while(|(_, c)| c.is_digit(radix) || *c == '_').count();
        if len == 0 {
            return None;
        }
        let digits: String = self.slice(self.i + 2, self.i + 2 + len).chars().filter(|&c| c != '_').collect();
        let value = i128::from_str_radix(&digits, radix).ok()?;
        self.i += 2 + len;
        let n = i64::try_from(value).map(Number::from_i64).unwrap_or_else(|_| Number::from_f64(value as f64));
        Some(Tok::Num(n))
    }

    /// `2026-09-23[T...]`, `23.09.2026` or `9/23/2026`.
    fn date(&mut self) -> Option<Tok> {
        let first = self.digits(self.i);
        let sep = self.peek(first)?;
        let second_at = self.i + first + 1;
        let second = self.digits(second_at);
        if !matches!(second, 1 | 2) || self.chars.get(second_at + second).map(|&(_, c)| c) != Some(sep) {
            return None;
        }
        let third_at = second_at + second + 1;
        let third = self.digits(third_at);
        let valid = match sep {
            '-' => first == 4 && matches!(third, 1 | 2),
            '.' | '/' => matches!(first, 1 | 2) && third == 4,
            _ => false,
        };
        if !valid {
            return None;
        }
        let parts = [
            self.slice(self.i, self.i + first).parse().ok()?,
            self.slice(second_at, second_at + second).parse().ok()?,
            self.slice(third_at, third_at + third).parse().ok()?,
        ];
        let start = self.i;
        self.i = third_at + third;
        if sep == '-' && self.peek(0) == Some('T') && self.peek(1).is_some_and(|c| c.is_ascii_digit()) {
            while self.peek(0).is_some_and(|c| c.is_ascii_alphanumeric() || "+-:.[]/_".contains(c)) {
                self.i += 1;
            }
            return Some(Tok::IsoDateTime(self.slice(start, self.i).to_string()));
        }
        Some(Tok::Date { parts, sep })
    }

    /// `9:30`, `21:05:10`, `00:00:01.5`.
    fn clock(&mut self) -> Option<Tok> {
        let h_len = self.digits(self.i);
        if h_len > 2 || self.peek(h_len) != Some(':') || self.digits(self.i + h_len + 1) != 2 {
            return None;
        }
        let h = self.slice(self.i, self.i + h_len).parse().ok()?;
        let m = self.slice(self.i + h_len + 1, self.i + h_len + 3).parse().ok()?;
        self.i += h_len + 3;
        let mut s = None;
        if self.peek(0) == Some(':') && self.digits(self.i + 1) == 2 {
            let mut end = self.i + 3;
            if self.chars.get(end).map(|&(_, c)| c) == Some('.') && self.digits(end + 1) > 0 {
                end += 1 + self.digits(end + 1);
            }
            s = Number::parse(self.slice(self.i + 1, end));
            self.i = end;
        }
        Some(Tok::Clock { h, m, s })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        lex(s, false).into_iter().map(|t| t.tok).collect()
    }

    fn num(s: &str) -> Tok {
        Tok::Num(Number::parse(s).unwrap())
    }

    #[test]
    fn numbers() {
        assert_eq!(toks("1,234.5"), vec![num("1234.5")]);
        assert_eq!(toks("1_000 0x1F 0b101"), vec![num("1000"), num("31"), num("5")]);
        assert_eq!(toks("1.5e3"), vec![num("1500")]);
        assert_eq!(toks("max(1,2)")[2..5], [num("1"), Tok::Sym(","), num("2")]);
        assert_eq!(toks("1.234,5 10,50 10.50"), vec![num("1234.5"), num("10.5"), num("10.5")]);
        assert_eq!(toks("1,000,000 0.125")[..2], [num("1000000"), num("0.125")]);
        assert_eq!(toks("1,5,3"), vec![num("1"), Tok::Sym(","), num("5"), Tok::Sym(","), num("3")]);
        assert_eq!(toks("1½"), vec![num("1.5")]);
    }

    #[test]
    fn lone_separator_before_three_digits() {
        let toks_with = |s: &str, comma: bool| lex(s, comma).into_iter().map(|t| t.tok).collect::<Vec<_>>();
        assert_eq!(toks_with("1,500 1.500", false), vec![num("1500"), num("1.5")]);
        assert_eq!(toks_with("1,500 1.500", true), vec![num("1.5"), num("1500")]);
    }

    #[test]
    fn words_and_symbols() {
        assert_eq!(toks("5km×2"), vec![num("5"), Tok::Word("km".into()), Tok::Sym("*"), num("2")]);
        assert_eq!(toks("US$5"), vec![Tok::Word("US$".into()), num("5")]);
        assert_eq!(toks("3 p.m."), vec![num("3"), Tok::Word("pm".into())]);
        assert_eq!(toks(r#"5'11""#), vec![num("5"), Tok::Sym("'"), num("11"), Tok::Sym("\"")]);
        assert_eq!(toks(r#"Boeing "747" is 5 // note"#).len(), 3);
    }

    #[test]
    fn dates_and_clocks() {
        assert_eq!(toks("2026-09-23"), vec![Tok::Date { parts: [2026, 9, 23], sep: '-' }]);
        assert_eq!(toks("23.09.2026"), vec![Tok::Date { parts: [23, 9, 2026], sep: '.' }]);
        assert_eq!(toks("15:30"), vec![Tok::Clock { h: 15, m: 30, s: None }]);
        assert!(matches!(toks("2019-04-01T15:30:00Z")[0], Tok::IsoDateTime(_)));
        assert_eq!(toks("10-3"), vec![num("10"), Tok::Sym("-"), num("3")]);
    }
}
