//! What each stretch of a line is, for syntax highlighting: numbers, units,
//! functions, comments... Read with the parser's own lookups, after parsing
//! decided nothing; lines that are only calculated never come here.

use std::ops::Range;

use super::{Parser, Scope, tokens, words};
use crate::ast::{Direction, Expr, Target};
use crate::lexer::{self, Tok};
use crate::zones::PlaceKind;

/// What a stretch of a line is, as SoulverCore's `SemanticToken.PartOfExpression`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Number,
    /// `+`, `times`, `mod`, `and` between conditions.
    Operator,
    Unit,
    Boolean,
    Function,
    Parenthesis,
    /// The word before a conversion target: `in`, `to`, `as`, `->`.
    Converter,
    /// A format a value is shown in: `hex`, `scientific`.
    Format,
    /// A word that joins values: `of`, `at`, `per`, `from`.
    Connector,
    Variable,
    /// An earlier answer: `prev`, `sum`, `line 2`.
    Reference,
    /// `if`, `then`, `else` and other words the line acts on.
    Keyword,
    Date,
    Place(PlaceKind),
    Food,
    /// `pi`, `speed of light`.
    Constant,
    Comment,
    /// `Rent:` before a line.
    Label,
    Heading,
    /// `#food`.
    Tag,
    /// `1.` or `a)` starting a line.
    ListMarker,
}

impl TokenKind {
    /// The name in snake case: "converter", "city".
    pub fn name(self) -> &'static str {
        match self {
            TokenKind::Number => "number",
            TokenKind::Operator => "operator",
            TokenKind::Unit => "unit",
            TokenKind::Boolean => "boolean",
            TokenKind::Function => "function",
            TokenKind::Parenthesis => "parenthesis",
            TokenKind::Converter => "converter",
            TokenKind::Format => "format",
            TokenKind::Connector => "connector",
            TokenKind::Variable => "variable",
            TokenKind::Reference => "reference",
            TokenKind::Keyword => "keyword",
            TokenKind::Date => "date",
            TokenKind::Place(PlaceKind::City) => "city",
            TokenKind::Place(PlaceKind::Country) => "country",
            TokenKind::Place(PlaceKind::Region) => "region",
            TokenKind::Place(PlaceKind::Airport) => "airport",
            TokenKind::Place(PlaceKind::TimeZone) => "time_zone",
            TokenKind::Food => "food",
            TokenKind::Constant => "constant",
            TokenKind::Comment => "comment",
            TokenKind::Label => "label",
            TokenKind::Heading => "heading",
            TokenKind::Tag => "tag",
            TokenKind::ListMarker => "list_marker",
        }
    }
}

/// A stretch of a line and what it is; the range is in bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

/// The parts of an expression `src`, with ranges moved by `offset` (the label before it).
pub fn highlight(src: &str, offset: usize, scope: &Scope) -> Vec<Highlight> {
    let mut toks = lexer::lex(src, scope.config);
    if tokens::has_hyphenated_words(&toks) {
        toks = Parser::new(src, &toks, scope).with_hyphenated_words();
    }
    let mut out = Vec::new();
    let marker = Parser::new(src, &toks, scope).leading_labels();
    if marker > 0 {
        // "1." and "2)": the dot or parenthesis after the marker belongs to it.
        let end = toks[marker - 1].end;
        let end = end + src[end..].len() - src[end..].trim_start_matches(['.', ')']).len();
        out.push(Highlight { range: toks[0].start..end, kind: TokenKind::ListMarker });
    }
    let kept = Parser::new(src, &toks[marker..], scope).without_comment_groups();
    // Parentheses that hold comments: "(for iPhone 16)".
    for t in &toks[marker..] {
        if !kept.iter().any(|k| k.start == t.start) {
            out.push(Highlight { range: t.start..t.end, kind: TokenKind::Comment });
        }
    }
    let p = Parser::new(src, &kept, scope);
    let mut i = 0;
    // "monthly rent = $1,500": the words before "=" name a variable.
    let eq = kept.iter().position(|t| matches!(t.tok, Tok::Sym("=" | "+=" | "-=" | "*=" | "/=")));
    if let Some(eq) = eq.filter(|&eq| eq > 0 && kept[..eq].iter().all(|t| t.word().is_some_and(|w| w != "$"))) {
        out.push(Highlight { range: kept[0].start..kept[eq - 1].end, kind: TokenKind::Variable });
        i = eq;
    }
    while i < kept.len() {
        for (kind, n) in p.kinds_at(i) {
            out.push(Highlight { range: kept[i].start..kept[i + n - 1].end, kind });
            i += n;
        }
    }
    out.sort_by_key(|h| h.range.start);
    gaps(src, &mut out);
    for h in &mut out {
        h.range = h.range.start + offset..h.range.end + offset;
    }
    out
}

/// Text between tokens: quoted text, links and `// notes` are comments, `#food` is a tag.
fn gaps(src: &str, out: &mut Vec<Highlight>) {
    let mut found = Vec::new();
    let mut at = 0;
    for h in out.iter().map(|h| h.range.clone()).chain(std::iter::once(src.len()..src.len())) {
        if h.start > at {
            let text = &src[at..h.start];
            let start = at + (text.len() - text.trim_start().len());
            let stop = at + text.trim_end().len();
            // A gap may hold several parts: "#food "quoted"".
            let mut k = start;
            while k < stop {
                let rest = &src[k..stop];
                let len = rest.find(char::is_whitespace).unwrap_or(rest.len());
                let tag = rest.starts_with('#') && rest[1..].starts_with(char::is_alphabetic);
                let kind = if tag { TokenKind::Tag } else { TokenKind::Comment };
                let len = if tag { len } else { rest.len() };
                found.push(Highlight { range: k..k + len, kind });
                k += len;
                k += src[k..stop].len() - src[k..stop].trim_start().len();
            }
        }
        at = at.max(h.end);
    }
    out.extend(found);
    out.sort_by_key(|h| h.range.start);
}

impl Parser<'_> {
    /// The kinds of the tokens from `i`: one kind and its length in tokens, or a
    /// converter word with its target.
    fn kinds_at(&self, i: usize) -> Vec<(TokenKind, usize)> {
        let tok = &self.toks[i];
        if let Some(kinds) = self.date_at(i) {
            return kinds;
        }
        let kind = match &tok.tok {
            Tok::Num(_) => TokenKind::Number,
            Tok::Clock { .. } | Tok::Date { .. } | Tok::IsoDateTime(_) => TokenKind::Date,
            Tok::Sym("(" | ")") => TokenKind::Parenthesis,
            Tok::Sym("->") => return self.conversion_at(i).unwrap_or(vec![(TokenKind::Converter, 1)]),
            Tok::Sym("°") => return vec![(TokenKind::Unit, self.unit_at(i, true).map_or(1, |(_, n)| n))],
            Tok::Sym(_) => TokenKind::Operator,
            Tok::Word(_) => return self.word_kinds_at(i),
        };
        vec![(kind, 1)]
    }

    fn word_kinds_at(&self, i: usize) -> Vec<(TokenKind, usize)> {
        let w = self.lower(i).unwrap_or_default();
        let after_number = i > 0 && matches!(self.toks[i - 1].tok, Tok::Num(_));
        let one = |kind| vec![(kind, 1)];
        if let Some((_, n)) = self.var_at(i) {
            return vec![(TokenKind::Variable, n)];
        }
        if matches!(w.as_str(), "in" | "to" | "as" | "into") {
            if let Some(kinds) = self.conversion_at(i) {
                return kinds;
            }
            if w == "in" && after_number {
                return one(TokenKind::Unit);
            }
            return one(TokenKind::Connector);
        }
        if words::is_line_word(&w) {
            let number = w == "line" && matches!(self.toks.get(i + 1).map(|t| &t.tok), Some(Tok::Num(_)));
            return vec![(TokenKind::Reference, 1 + usize::from(number))];
        }
        let kind = match w.as_str() {
            "if" | "then" | "else" | "unless" | "when" => Some(TokenKind::Keyword),
            "true" | "false" => Some(TokenKind::Boolean),
            "plus" | "minus" | "times" | "multiplied" | "divided" | "mod" | "modulo" | "xor" | "and" | "or"
            | "squared" | "cubed" | "remainder" | "twice" | "double" | "triple" | "quadruple" => {
                Some(TokenKind::Operator)
            }
            "x" if self.significant(i) => Some(TokenKind::Operator),
            "of" | "on" | "off" | "at" | "per" | "from" | "after" | "before" | "until" | "till" | "since"
            | "between" | "through" | "thru" | "ago" | "later" | "by" | "every" | "each" | "split" | "out" => {
                Some(TokenKind::Connector)
            }
            "for" if self.significant(i) => Some(TokenKind::Connector),
            "now" | "today" | "tomorrow" | "yesterday" | "noon" | "midday" | "midnight" | "am" | "pm" | "next"
            | "last" | "this" | "date" | "time" | "current" | "epoch" => Some(TokenKind::Date),
            _ => None,
        };
        if let Some(kind) = kind {
            return one(kind);
        }
        if let Some((_, n)) = self.phrase_at(i, &words::FUNCTIONS) {
            return vec![(TokenKind::Function, n)];
        }
        let phrase = [
            self.phrase_at(i, &words::HOLIDAYS).map(|(_, n)| (TokenKind::Date, n)),
            self.event_at(i).map(|(_, n)| (TokenKind::Date, n)),
            self.period_bound_at(i).map(|(_, n)| (TokenKind::Date, n)),
            self.phrase_at(i, &words::DATE_PARTS).map(|(_, n)| (TokenKind::Function, n)),
            self.phrase_at(i, &words::PHYSICAL_CONSTANTS).map(|(_, n)| (TokenKind::Constant, n)),
            self.sky_at(i).map(|(_, n)| (TokenKind::Function, n)),
            self.phrase_at(i, &words::WEATHER)
                .filter(|_| self.weather_at(i).is_some())
                .map(|(_, n)| (TokenKind::Function, n)),
        ];
        if let Some(found) = phrase.into_iter().flatten().max_by_key(|&(_, n)| n) {
            return vec![found];
        }
        if words::month(&w).is_some() || words::weekday(&w).is_some() {
            return one(TokenKind::Date);
        }
        if words::constant(&w).is_some() {
            return one(TokenKind::Constant);
        }
        if words::number_word(&w).is_some() || words::scale_word(&w).is_some() || words::fraction_word(&w).is_some() {
            return one(TokenKind::Number);
        }
        let unit = self.unit_at(i, after_number);
        let place = self.named_place_at(i);
        match (unit, place) {
            (Some((_, n)), Some((place, m))) if m > n || !self.significant(i) => {
                return vec![(TokenKind::Place(place.kind), m)];
            }
            (Some((_, n)), _) => return vec![(TokenKind::Unit, n)],
            (None, Some((place, m))) if self.place_counts(i) => return vec![(TokenKind::Place(place.kind), m)],
            _ => {}
        }
        if i > 0
            && let Some((_, n)) = self.food_at(i)
            && self.unit_before(i)
        {
            return vec![(TokenKind::Food, n)];
        }
        if self.significant(i) { one(TokenKind::Keyword) } else { one(TokenKind::Comment) }
    }

    /// A place counts where the parser reads one: "Tokyo time", "PST to EST", "sunrise in Tokyo".
    fn place_counts(&self, i: usize) -> bool {
        self.significant(i) || (i > 0 && self.toks[..i].iter().any(|t| t.is_word("in") || t.is_word("to")))
    }

    /// A weight or volume ends right before token `i` ("of" may come between): "1 cup of flour".
    fn unit_before(&self, i: usize) -> bool {
        let end = if self.toks[i - 1].is_word("of") { i - 1 } else { i };
        (end.saturating_sub(3)..end).any(|k| self.unit_at(k, true).is_some_and(|(_, n)| k + n == end))
    }

    /// A date or time written from token `i`, and the place it is in: "3pm", "5 May 2027", "9am Tokyo".
    fn date_at(&self, i: usize) -> Option<Vec<(TokenKind, usize)>> {
        let starts = match &self.toks[i].tok {
            Tok::Num(_) | Tok::Clock { .. } | Tok::Date { .. } => true,
            Tok::Word(w) => {
                let w = w.to_lowercase();
                words::month(&w).is_some() || words::weekday(&w).is_some()
            }
            _ => false,
        };
        if !starts {
            return None;
        }
        let mut p = self.clone();
        p.pos = i;
        let end = match p.primary().ok()? {
            Expr::Time(_) => p.pos,
            Expr::InZone(time, _) if matches!(*time, Expr::Time(_)) => {
                let zone = (i + 1..p.pos).find(|&k| self.zone_at(k).is_some_and(|(_, n)| k + n == p.pos))?;
                let (place, n) = self.zone_at(zone)?;
                return Some(vec![(TokenKind::Date, zone - i), (TokenKind::Place(place.kind), n)]);
            }
            _ => return None,
        };
        Some(vec![(TokenKind::Date, end - i)])
    }

    /// A conversion word at `i` with what follows it: `in` + `km/h`, `to` + `Tokyo`, `as` + `hex`.
    fn conversion_at(&self, i: usize) -> Option<Vec<(TokenKind, usize)>> {
        let mut p = self.clone();
        p.pos = i + 1;
        let target = match p.zone_at(p.pos) {
            Some((place, n)) => {
                p.pos += n;
                Target::Zone(place)
            }
            None => match p.target() {
                Some(target) => target,
                // "to 2 dp", "to nearest 10"
                None => {
                    p.pos = i + 1;
                    p.rounding(Direction::Nearest).ok()??;
                    return Some(vec![(TokenKind::Converter, 1), (TokenKind::Format, p.pos - (i + 1))]);
                }
            },
        };
        let length = p.pos - (i + 1);
        let kind = match target {
            Target::Unit(_) | Target::Units(_) | Target::Per(_) | Target::Cast(_) => TokenKind::Unit,
            Target::Zone(place) => TokenKind::Place(place.kind),
            Target::Format(_) => TokenKind::Format,
        };
        Some(vec![(TokenKind::Converter, 1), (kind, length)].into_iter().filter(|&(_, n)| n > 0).collect())
    }
}
