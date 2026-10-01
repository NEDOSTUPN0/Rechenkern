//! The sun and the moon: "sunrise in Tokyo", "London daylight hours", "moon phase on May 1".

use super::{Parser, words};
use crate::ast::{Expr, Op, Sky};
use crate::error::{Result, bail};
use crate::zones::{self, Location};

impl Parser<'_> {
    /// A sun or moon phrase at token `i` and its length in tokens.
    pub(super) fn sky_at(&self, i: usize) -> Option<(Sky, usize)> {
        let (what, n) = self.phrase_at(i, &words::SKY)?;
        // "daylight" alone needs a place or a date: "daylight saving time" is not about the sun.
        if n == 1 && what == Sky::Daylight && self.sky_place_at(i + 1).is_none() {
            let mut p = self.clone();
            p.pos = i + 1;
            if !p.date_follows() {
                return None;
            }
        }
        Some((what, n))
    }

    /// A place, then the sun: "Tokyo sunrise", "London daylight hours". Their length in tokens.
    pub(super) fn place_sky_at(&self, i: usize) -> Option<(Sky, usize)> {
        // Looking up places is slow: first see if the sun follows at all.
        if !(i + 1..=i + zones::MAX_WORDS).any(|k| self.phrase_at(k, &words::SKY).is_some()) {
            return None;
        }
        let (_, n) = self.place_at(i)?;
        let (what, m) = self.phrase_at(i + n, &words::SKY)?;
        let bare_daylight = m == 1 && what == Sky::Daylight;
        (what.is_sun() && !bare_daylight).then_some((what, n + m))
    }

    /// A place after the sun: "in Tokyo", "at JFK", "for London" or just "Tokyo".
    /// Where it is (`None` for a region), where its name starts, and the length in tokens.
    fn sky_place_at(&self, i: usize) -> Option<(Option<Location>, usize, usize)> {
        let skip = usize::from(["in", "at", "for"].iter().any(|w| self.words_at(i, &[w])));
        let (location, n) = self.location_at(i + skip)?;
        Some((location, i + skip, skip + n))
    }

    /// The sun or the moon at the cursor, then a place and a date in either order:
    /// "sunrise in Tokyo tomorrow", "lunar day on May 1".
    pub(super) fn sky(&mut self) -> Result<Option<Expr>> {
        let start = self.pos;
        let (what, mut place) = match self.sky_at(start) {
            Some((what, n)) => {
                self.pos += n;
                (what, None)
            }
            None => {
                let Some((what, n)) = self.place_sky_at(start) else { return Ok(None) };
                let (location, name_len) = self.location_at(start).expect("place_sky_at found a place");
                self.pos += n;
                (what, Some(self.located(location, start, start + name_len)?))
            }
        };
        // "sunrise time in Tokyo"
        if matches!(what, Sky::Sunrise | Sky::Sunset) && self.words_at(self.pos, &["time"]) {
            self.pos += 1;
        }
        let mut on = None;
        loop {
            if what.is_sun()
                && place.is_none()
                && let Some((location, name, n)) = self.sky_place_at(self.pos)
            {
                place = Some(self.located(location, name, self.pos + n)?);
                self.pos += n;
                continue;
            }
            if on.is_none()
                && self.date_follows()
                && let Some(date) = self.date_word()?
            {
                on = Some(self.date_tail(date)?.boxed());
                continue;
            }
            // "moon phase in 2 weeks"
            if on.is_none() && self.words_at(self.pos, &["in"]) && self.duration_at(self.pos + 1) {
                self.pos += 1;
                let amount = self.multiplicative()?;
                on = Some(Expr::binary(Op::Add, super::time::now_for(&amount), amount).boxed());
                continue;
            }
            break;
        }
        // "hours of daylight in Narnia" must not quietly mean here.
        if what.is_sun() && place.is_none() && ["in", "at"].iter().any(|w| self.words_at(self.pos, &[w])) {
            let mut p = self.clone();
            p.pos += 1;
            p.unknown_place()?;
        }
        Ok(Some(Expr::Sky { what, place, on }))
    }

    /// A place the sun can be seen from; regions and zone names have no single place.
    fn located(&self, location: Option<Location>, from: usize, to: usize) -> Result<Location> {
        match location {
            Some(location) => Ok(location),
            None => {
                let name = &self.src[self.toks[from].start..self.toks[to - 1].end];
                bail!("don't know where exactly \"{name}\" is: name a city")
            }
        }
    }
}
