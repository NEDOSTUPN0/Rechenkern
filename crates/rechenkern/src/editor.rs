//! Help for editors: a line's tokens as a tree, its parts, tidy input and what a word is.

use std::ops::Range;

use crate::calculator::Calculator;
use crate::parser::highlight::{Highlight, TokenKind};
use crate::parser::words;
use crate::units::registry;

/// A part of a line, as SoulverCore's `Token`: parentheses hold the tokens inside them, and a
/// function called with parentheses holds its group. Ranges are in bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub range: Range<usize>,
    pub text: String,
    pub children: Vec<Token>,
}

impl Token {
    /// The innermost token at a byte offset.
    pub fn at(&self, offset: usize) -> Option<&Token> {
        if !self.range.contains(&offset) {
            return None;
        }
        Some(self.children.iter().find_map(|c| c.at(offset)).unwrap_or(self))
    }
}

/// The parts of a line, as SoulverCore's `ExpressionComponents`. Ranges are in bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Components {
    /// `1.` or `a)` starting the line.
    pub list_marker: Option<Range<usize>>,
    /// `Rent:` before the expression.
    pub label: Option<Range<usize>>,
    /// The number of `#` of a heading, and its text.
    pub heading: Option<(usize, Range<usize>)>,
    /// The name a line assigns: "monthly rent" in `monthly rent = $1,500`.
    pub variable: Option<String>,
    /// Comments, quoted text and links.
    pub comments: Vec<Range<usize>>,
    /// Hashtags without their `#`, lowercase.
    pub tags: Vec<String>,
    /// Before the first part that counts: the marker, the label, leading comments.
    pub head: Range<usize>,
    /// From the first part that counts to the last.
    pub body: Range<usize>,
    /// After the last part that counts: trailing comments and tags.
    pub tail: Range<usize>,
}

/// How [`Calculator::beautify`] tidies a line, as SoulverCore's `ExpressionBeautificationOptions`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Beautify {
    /// `*` becomes `×`.
    pub multiplication_sign: bool,
    /// `/` between values becomes `÷`.
    pub division_sign: bool,
    /// A minus between values becomes `−`.
    pub minus_sign: bool,
    /// One space on each side of `+`, `-`, `*`, `/`, `=`.
    pub spaced_operators: bool,
    /// Numbers of five digits or more get thousands separators.
    pub thousands_separators: bool,
}

impl Default for Beautify {
    fn default() -> Beautify {
        Beautify {
            multiplication_sign: true,
            division_sign: false,
            minus_sign: true,
            spaced_operators: true,
            thousands_separators: true,
        }
    }
}

/// Symbols the parser reads as operators.
const OPERATORS: &[&str] = &[
    "+", "-", "−", "*", "×", "·", "/", "÷", "^", "%", "=", "+=", "-=", "*=", "/=", "<", ">", "<=", ">=", "==", "!=",
    "≠", "≤", "≥", "&", "|", "<<", ">>", "!", "~",
];

/// Words the parser reads as operators.
const OPERATOR_WORDS: &[&str] = &[
    "plus",
    "minus",
    "times",
    "multiplied by",
    "divided by",
    "mod",
    "modulo",
    "xor",
    "and",
    "or",
    "not",
    "squared",
    "cubed",
    "remainder",
    "twice",
    "double",
    "triple",
    "quadruple",
    "over",
    "percent",
];

impl Calculator {
    /// A line's tokens as a tree: what [`Calculator::highlight`] finds, grouped by parentheses.
    pub fn tokens(&self, line: &str) -> Vec<Token> {
        let flat = self.highlight(line);
        let mut stack: Vec<Vec<Token>> = vec![Vec::new()];
        for h in flat {
            let token =
                Token { kind: h.kind, range: h.range.clone(), text: line[h.range].to_string(), children: vec![] };
            match (token.kind, token.text.as_str()) {
                (TokenKind::Parenthesis, "(") => {
                    stack.push(vec![token]);
                }
                (TokenKind::Parenthesis, ")") if stack.len() > 1 => {
                    let mut inner = stack.pop().unwrap_or_default();
                    let open = inner.remove(0);
                    let group = Token {
                        kind: TokenKind::Parenthesis,
                        range: open.range.start..token.range.end,
                        text: line[open.range.start..token.range.end].to_string(),
                        children: inner,
                    };
                    let level = stack.last_mut().expect("the outer level stays");
                    // "sqrt(16)": the call holds its arguments.
                    match level.last_mut() {
                        Some(f) if f.kind == TokenKind::Function && f.range.end == group.range.start => {
                            f.range.end = group.range.end;
                            f.text = line[f.range.clone()].to_string();
                            f.children.push(group);
                        }
                        _ => level.push(group),
                    }
                }
                _ => stack.last_mut().expect("a level is open").push(token),
            }
        }
        // Parentheses left open keep their tokens at the level around them.
        while stack.len() > 1 {
            let inner = stack.pop().unwrap_or_default();
            stack.last_mut().expect("the outer level stays").extend(inner);
        }
        stack.pop().unwrap_or_default()
    }

    /// The innermost token at a character of `line` (counted in chars, as editors count).
    pub fn token_at(&self, line: &str, char_index: usize) -> Option<Token> {
        let offset = line.char_indices().nth(char_index).map(|(i, _)| i)?;
        self.tokens(line).iter().find_map(|t| t.at(offset)).cloned()
    }

    /// A line's parts: its list marker, label, heading, comments, tags, and head, body and tail.
    pub fn components(&self, line: &str) -> Components {
        let highlights = self.highlight(line);
        let mut parts = Components::default();
        let ranges = |kind| highlights.iter().filter(move |h| h.kind == kind).map(|h| h.range.clone());
        parts.list_marker = ranges(TokenKind::ListMarker).next();
        parts.label = ranges(TokenKind::Label).next();
        if let Some(text) = ranges(TokenKind::Heading).next() {
            let marks = ranges(TokenKind::HeadingIndicator).next().map_or(0, |r| r.len());
            parts.heading = Some((marks, text));
        }
        parts.variable = highlights
            .iter()
            .take_while(|h| h.kind != TokenKind::Operator)
            .find(|h| h.kind == TokenKind::Variable)
            .filter(|_| {
                highlights.iter().any(|h| h.kind == TokenKind::Operator && line[h.range.clone()].ends_with('='))
            })
            .map(|h| line[h.range.clone()].to_string());
        parts.comments = highlights
            .iter()
            .filter(|h| matches!(h.kind, TokenKind::Comment | TokenKind::Url))
            .map(|h| h.range.clone())
            .collect();
        parts.tags = ranges(TokenKind::Tag).map(|r| line[r].to_lowercase()).collect();
        let counts = |h: &&Highlight| {
            !matches!(
                h.kind,
                TokenKind::Comment
                    | TokenKind::Url
                    | TokenKind::Label
                    | TokenKind::ListMarker
                    | TokenKind::Tag
                    | TokenKind::TagIndicator
                    | TokenKind::HeadingIndicator
            )
        };
        let start = highlights.iter().find(counts).map_or(line.len(), |h| h.range.start);
        let end = highlights.iter().rev().find(counts).map_or(start, |h| h.range.end);
        (parts.head, parts.body, parts.tail) = (0..start, start..end, end..line.len());
        parts
    }

    /// The line tidied while typing: `5*3-2` becomes `5 × 3 − 2`, `12000` becomes `12,000`.
    /// Comments and everything but operators and numbers stay as written.
    pub fn beautify(&self, line: &str, options: &Beautify) -> String {
        let highlights = self.highlight(line);
        let mut out = String::with_capacity(line.len() + 8);
        let mut at = 0;
        for (i, h) in highlights.iter().enumerate() {
            let text = &line[h.range.clone()];
            let replaced = match h.kind {
                TokenKind::Operator => {
                    let sign = match text {
                        "*" if options.multiplication_sign => "×",
                        "/" if options.division_sign => "÷",
                        "-" if options.minus_sign => "−",
                        _ => text,
                    };
                    let between = i > 0 && i + 1 < highlights.len();
                    let spaced = options.spaced_operators
                        && between
                        && matches!(text, "+" | "-" | "−" | "*" | "×" | "/" | "÷" | "=" | "+=" | "-=" | "*=" | "/=");
                    if spaced {
                        out.push_str(line[at..h.range.start].trim_end());
                        out.push(' ');
                        out.push_str(sign);
                        out.push(' ');
                        at = h.range.end + line[h.range.end..].len() - line[h.range.end..].trim_start().len();
                        continue;
                    }
                    sign.to_string()
                }
                TokenKind::Number | TokenKind::Negative if options.thousands_separators => self.grouped(text),
                _ => text.to_string(),
            };
            out.push_str(&line[at..h.range.start]);
            out.push_str(&replaced);
            at = h.range.end;
        }
        out.push_str(&line[at..]);
        out
    }

    /// Digits of a plain whole number grouped in thousands: "12000" -> "12,000".
    fn grouped(&self, text: &str) -> String {
        let (sign, digits) = text.strip_prefix('-').map_or(("", text), |d| ("-", d));
        if digits.len() < 5 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return text.to_string();
        }
        let separator = if self.config().decimal_comma { '.' } else { ',' };
        let mut out = String::from(sign);
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                out.push(separator);
            }
            out.push(c);
        }
        out
    }

    /// The text is an operator: `+`, `×`, `times`, `divided by`.
    pub fn is_operator(&self, text: &str) -> bool {
        let text = text.trim();
        OPERATORS.contains(&text) || OPERATOR_WORDS.contains(&text.to_lowercase().as_str())
    }

    /// The text names a function: a built-in one (`sqrt`, `round`) or one the lines defined.
    pub fn is_function(&self, text: &str) -> bool {
        let name = text.trim().to_lowercase();
        words::FUNCTIONS.phrases().any(|p| p == name) || self.function_defined(&name)
    }

    /// The text names a place the calculator knows: `Tokyo`, `JFK`, `PST`, a custom place.
    pub fn is_place(&self, text: &str) -> bool {
        self.place(text).is_some()
    }

    /// The text names a unit: `km`, `miles per hour`, a custom unit.
    pub fn is_unit(&self, text: &str) -> bool {
        self.unit(text.trim()).is_some() || registry().lookup(&text.trim().to_lowercase()).is_some()
    }
}
