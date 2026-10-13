//! Distances between places: "distance from Tokyo to London".

use super::Parser;
use crate::ast::Expr;
use crate::error::Result;
use crate::number::Number;
use crate::units::registry;
use crate::zones;

/// Where the two place names of a distance phrase start and end, in tokens.
type Ends = [(usize, usize); 2];

impl Parser<'_> {
    /// "distance from Tokyo to London", "distance between Paris and Berlin", "distance Tokyo
    /// to London" or "Tokyo to London distance" at token `i`: the place names and its length.
    pub(super) fn distance_at(&self, i: usize) -> Option<(Ends, usize)> {
        if self.words_at(i, &["distance"]) {
            let from = i + 1;
            let first = from + usize::from(self.words_at(from, &["from"]) || self.words_at(from, &["between"]));
            let (ends, end) = self.place_pair_at(first)?;
            return Some((ends, end - i));
        }
        // Looking up places is slow: first see if "distance" follows at all.
        if !self.toks[i..].iter().any(|t| t.is_word("distance")) {
            return None;
        }
        let (ends, end) = self.place_pair_at(i)?;
        self.words_at(end, &["distance"]).then_some((ends, end + 1 - i))
    }

    /// "Tokyo to London", "Paris and Berlin": where the names are and the token after them.
    fn place_pair_at(&self, i: usize) -> Option<(Ends, usize)> {
        let (_, n) = self.place_at(i)?;
        let joint = i + n;
        if !["to", "and"].iter().any(|w| self.words_at(joint, &[w])) {
            return None;
        }
        let (_, m) = self.place_at(joint + 1)?;
        Some(([(i, i + n), (joint + 1, joint + 1 + m)], joint + 1 + m))
    }

    /// The distance at the cursor in kilometers, along the surface of the Earth.
    pub(super) fn distance(&mut self) -> Result<Option<Expr>> {
        let Some((ends, n)) = self.distance_at(self.pos) else { return Ok(None) };
        let [a, b] = ends.map(|(from, to)| {
            let (location, _) = self.location_at(from).expect("distance_at found a place");
            self.located(location, from, to)
        });
        let km = zones::distance_km(&a?, &b?).round();
        self.pos += n;
        Ok(Some(Expr::WithUnit(Expr::Number(Number::from_f64(km)).boxed(), registry().get("km"))))
    }
}
