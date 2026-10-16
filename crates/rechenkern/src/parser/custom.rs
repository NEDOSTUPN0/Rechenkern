//! The host's phrases: "tip on $50", "$90 split 3 ways".

use super::Parser;
use crate::ast::{CustomArg, Expr};
use crate::custom::{self, Slot, SlotKind};
use crate::error::Result;
use crate::lexer::Tok;

impl Parser<'_> {
    /// The whole line as one of the host's phrases, or `None`.
    pub(super) fn custom_function(&mut self) -> Result<Option<Expr>> {
        let config = self.config();
        for (function, custom) in config.custom_functions.iter().enumerate() {
            for prototype in std::iter::once(&custom.prototype).chain(&custom.aliases) {
                let Some(slots) = custom::slots(prototype, &config.custom_types) else { continue };
                if let Some(args) = self.match_slots(&slots, 0) {
                    return Ok(Some(Expr::Custom { function, args }));
                }
            }
        }
        Ok(None)
    }

    /// The values of the placeholders if the tokens from `at` to the end match `slots`.
    fn match_slots(&self, slots: &[Slot], at: usize) -> Option<Vec<CustomArg>> {
        let Some((slot, rest)) = slots.split_first() else { return (at == self.toks.len()).then(Vec::new) };
        let with = |arg: CustomArg, next: usize| {
            let mut args = self.match_slots(rest, next)?;
            args.insert(0, arg);
            Some(args)
        };
        match slot {
            Slot::Literal(tok) => self.literal_at(at, tok).then(|| self.match_slots(rest, at + 1))?,
            Slot::Placeholder(SlotKind::Unit) => {
                let (unit, n) = self.unit_at(at, true)?;
                with(CustomArg::Unit(unit), at + n)
            }
            Slot::Placeholder(SlotKind::Place) => {
                let (place, n) = self.named_place_at(at)?;
                with(CustomArg::Text(place.name), at + n)
            }
            Slot::Placeholder(SlotKind::Type(t)) => {
                let words = &self.config().custom_types[*t].words;
                let (word, n) = words.iter().find_map(|w| Some((w.clone(), self.custom_words_at(at, w)?)))?;
                with(CustomArg::Text(word), at + n)
            }
            Slot::Placeholder(kind) => {
                // The value runs to where the rest can match: the next literal, or the end.
                (at + 1..=self.toks.len()).find_map(|end| {
                    let next_fits = match rest.first() {
                        Some(Slot::Literal(tok)) => self.literal_at(end, tok),
                        Some(_) => true,
                        None => end == self.toks.len(),
                    };
                    if !next_fits {
                        return None;
                    }
                    let expr = self.part(at, end).ok()?;
                    with(CustomArg::Expr(expr, *kind), end)
                })
            }
        }
    }

    /// Token `at` is the literal of a prototype: the same word in any case, number or symbol.
    fn literal_at(&self, at: usize, literal: &Tok) -> bool {
        let Some(tok) = self.toks.get(at) else { return false };
        match (&tok.tok, literal) {
            (Tok::Word(a), Tok::Word(b)) => a.eq_ignore_ascii_case(b),
            (a, b) => a == b,
        }
    }

    /// The words of `phrase` at token `at`: its length in tokens.
    fn custom_words_at(&self, at: usize, phrase: &str) -> Option<usize> {
        let words: Vec<&str> = phrase.split_whitespace().collect();
        words
            .iter()
            .enumerate()
            .all(|(k, w)| self.toks.get(at + k).is_some_and(|t| t.is_word(w)))
            .then_some(words.len())
    }
}
