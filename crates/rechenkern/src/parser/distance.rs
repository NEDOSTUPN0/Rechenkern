//! Distances between places: "distance from Tokyo to London".

use super::Parser;
use crate::ast::Expr;
use crate::error::Result;
use crate::lexer::Tok;
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
            // "distance to London from Tokyo"
            if self.words_at(from, &["to"])
                && let Some((_, n)) = self.place_at(from + 1)
                && self.words_at(from + 1 + n, &["from"])
                && let Some((_, m)) = self.place_at(from + 2 + n)
            {
                return Some(([(from + 2 + n, from + 2 + n + m), (from + 1, from + 1 + n)], 3 + n + m));
            }
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

/// What a coordinates phrase asks for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Geo {
    Coordinates,
    Latitude,
    Longitude,
}

const GEO_WORDS: &[(&str, Geo)] = &[
    ("coordinates", Geo::Coordinates),
    ("coordinate", Geo::Coordinates),
    ("location", Geo::Coordinates),
    ("gps", Geo::Coordinates),
    ("latitude", Geo::Latitude),
    ("longitude", Geo::Longitude),
];

impl Parser<'_> {
    /// "coordinates of Tokyo", "latitude of Paris", "Tokyo coordinates" at token `i`: what it
    /// asks, where the place's name is, and its length in tokens.
    pub(super) fn coordinates_at(&self, i: usize) -> Option<(Geo, (usize, usize), usize)> {
        let geo = |k: usize| {
            let w = self.lower(k)?;
            GEO_WORDS.iter().find(|(word, _)| *word == w).map(|&(_, g)| g)
        };
        if let Some(what) = geo(i) {
            // "gps coordinates of", "gps coordinate of"
            let gps = self.words_at(i + 1, &["coordinates"]) || self.words_at(i + 1, &["coordinate"]);
            let mut k = i + 1 + usize::from(what == Geo::Coordinates && gps);
            if !(self.words_at(k, &["of"]) || self.words_at(k, &["for"])) {
                return None;
            }
            k += 1;
            let (_, n) = self.place_at(k)?;
            return Some((what, (k, k + n), k + n - i));
        }
        // Looking up places is slow: first see if a coordinates word follows at all.
        if !(i + 1..=i + zones::MAX_WORDS).any(|k| geo(k).is_some()) {
            return None;
        }
        let (_, n) = self.place_at(i)?;
        let what = geo(i + n)?;
        Some((what, (i, i + n), n + 1))
    }

    /// "latitude of 35.6762° N": a coordinates word before written coordinates at token `i`.
    pub(super) fn geo_of_number_at(&self, i: usize) -> Option<Geo> {
        let w = self.lower(i)?;
        let &(_, what) = GEO_WORDS.iter().find(|(g, _)| *g == w)?;
        let number = matches!(self.toks.get(i + 2).map(|t| &t.tok), Some(Tok::Num(_)));
        (self.words_at(i + 1, &["of"]) && number).then_some(what)
    }

    /// "35.6762° N, 139.6503° E" after the latitude's angle: coordinates written out.
    pub(super) fn coordinates_literal(&mut self, latitude: &Expr) -> Option<Expr> {
        let hemisphere = |p: &Self, signs: [&str; 2]| {
            let w = p.cur()?.word()?;
            [(signs[0], 1.0), (signs[1], -1.0)].into_iter().find(|(s, _)| w.eq_ignore_ascii_case(s)).map(|(_, v)| v)
        };
        let latitude = angle_degrees(latitude)? * hemisphere(self, ["N", "S"])?;
        let mut p = self.clone();
        p.pos += 1;
        p.eat_sym(",");
        let Some(Tok::Num(n)) = p.cur().map(|t| &t.tok) else { return None };
        p.pos += 1;
        let longitude = p.quantity(Expr::Number(*n)).ok()?;
        let longitude = angle_degrees(&longitude)? * hemisphere(&p, ["E", "W"])?;
        if latitude.abs() > 90.0 || longitude.abs() > 180.0 {
            return None;
        }
        *self = p;
        self.pos += 1;
        Some(Expr::Coordinates(latitude, longitude))
    }

    /// The coordinates, latitude or longitude of a place at the cursor.
    pub(super) fn coordinates(&mut self) -> Result<Option<Expr>> {
        // "latitude of 35.6762° N, 139.6503° E"
        if let Some(what) = self.geo_of_number_at(self.pos)
            && let Some(Tok::Num(n)) = self.toks.get(self.pos + 2).map(|t| &t.tok)
        {
            let mut p = self.clone();
            p.pos += 3;
            if let Ok(Expr::Coordinates(latitude, longitude)) = p.quantity(Expr::Number(*n)) {
                *self = p;
                let degrees = |d: f64| Expr::WithUnit(Expr::Number(Number::from_f64(d)).boxed(), registry().get("°"));
                return Ok(Some(match what {
                    Geo::Coordinates => Expr::Coordinates(latitude, longitude),
                    Geo::Latitude => degrees(latitude),
                    Geo::Longitude => degrees(longitude),
                }));
            }
        }
        let Some((what, (from, to), n)) = self.coordinates_at(self.pos) else { return Ok(None) };
        let (location, _) = self.location_at(from).expect("coordinates_at found a place");
        let place = self.located(location, from, to)?;
        self.pos += n;
        let degrees = |d: f64| Expr::WithUnit(Expr::Number(Number::from_f64(d)).boxed(), registry().get("°"));
        Ok(Some(match what {
            Geo::Coordinates => Expr::Coordinates(place.latitude, place.longitude),
            Geo::Latitude => degrees(place.latitude),
            Geo::Longitude => degrees(place.longitude),
        }))
    }
}

/// An angle written as an amount, in degrees: "35.6762°", "35° 40′ 34″".
fn angle_degrees(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::WithUnit(n, unit) if unit.dim() == crate::units::Dim::ANGLE => {
            let Expr::Number(n) = **n else { return None };
            let def = unit.single()?;
            Some(n.to_f64() * def.scale.to_f64() / registry().get("°").single()?.scale.to_f64())
        }
        Expr::Composite(parts) => parts.iter().map(angle_degrees).sum(),
        _ => None,
    }
}
