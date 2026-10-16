//! Weather phrases: "weather in Tokyo", "Tokyo temperature", "humidity in London tomorrow".

use super::{Parser, words};
use crate::ast::Expr;
use crate::error::Result;
use crate::weather::WeatherQuery;

impl Parser<'_> {
    /// A weather phrase at token `i`: what it asks and the length of its words. Only the
    /// weather itself goes without a place; "high" and "low" need one: "low in Paris".
    pub(super) fn weather_at(&self, i: usize) -> Option<(WeatherQuery, usize)> {
        if let Some((what, n)) = self.phrase_at(i, &words::WEATHER) {
            let place_follows = ["in", "at", "for"].iter().any(|w| self.words_at(i + n, &[w]));
            return (what == WeatherQuery::Summary || place_follows).then_some((what, n));
        }
        // "Tokyo temperature": first see if a weather word follows at all, as places are slow.
        if !(i + 1..=i + crate::zones::MAX_WORDS).any(|k| self.phrase_at(k, &words::WEATHER).is_some()) {
            return None;
        }
        let (_, n) = self.place_at(i)?;
        let (what, m) = self.phrase_at(i + n, &words::WEATHER)?;
        Some((what, n + m))
    }

    /// The weather at the cursor, with a place and a day in either order.
    pub(super) fn weather(&mut self) -> Result<Option<Expr>> {
        let start = self.pos;
        let Some((what, n)) = self.weather_at(start) else { return Ok(None) };
        let mut place = None;
        if self.phrase_at(start, &words::WEATHER).is_none() {
            let (location, len) = self.location_at(start).expect("weather_at found a place");
            let name = self.src[self.toks[start].start..self.toks[start + len - 1].end].to_string();
            place = Some((name, self.located(location, start, start + len)?));
        }
        self.pos += n;
        let mut on = None;
        loop {
            if place.is_none() && ["in", "at", "for"].iter().any(|w| self.words_at(self.pos, &[w])) {
                let at = self.pos + 1;
                if let Some((location, len)) = self.location_at(at) {
                    let name = self.src[self.toks[at].start..self.toks[at + len - 1].end].to_string();
                    place = Some((name, self.located(location, at, at + len)?));
                    self.pos = at + len;
                    continue;
                }
                if let Some((found, location, len)) = self.found_place(at) {
                    place = Some((found.name, self.located(location, at, at + len)?));
                    self.pos = at + len;
                    continue;
                }
                let mut p = self.clone();
                p.pos = at;
                p.unknown_place()?;
            }
            if on.is_none()
                && self.date_follows()
                && let Some(date) = self.date_word()?
            {
                on = Some(self.date_tail(date)?.boxed());
                continue;
            }
            break;
        }
        Ok(Some(Expr::Weather { what, place, on }))
    }
}
