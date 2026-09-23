//! Dates, clock times, time zones and phrases like "days until christmas".

use jiff::tz::TimeZone;

use super::{Parser, words};
use crate::ast::{Expr, Format, Target, TimeExpr, Which};
use crate::config::DateOrder;
use crate::error::{Result, bail};
use crate::lexer::Tok;
use crate::number::Number;
use crate::units::{Dim, Unit, registry};
use crate::zones;

/// "now" for sub-day amounts, "today" for days and longer: `3 days ago` is a date.
pub(super) fn now_for(expr: &Expr) -> Expr {
    let unit = match expr {
        Expr::WithUnit(_, u) | Expr::BareUnit(u) => Some(u),
        Expr::Composite(parts) => match parts.last() {
            Some(Expr::WithUnit(_, u)) => Some(u),
            _ => None,
        },
        _ => None,
    };
    Expr::Time(if unit.is_some_and(is_date_unit) { TimeExpr::Today(0) } else { TimeExpr::Now })
}

/// Days, weeks, months and years count whole dates.
fn is_date_unit(unit: &Unit) -> bool {
    unit.single().and_then(|d| d.calendar).is_some_and(|(cal, _)| cal >= jiff::Unit::Day)
}

impl Parser<'_> {
    /// `hh:mm[:ss]` with optional am/pm; with seconds and nothing else it is a lap time.
    pub(super) fn clock_literal(&mut self, h: u32, m: u32, s: Option<Number>) -> Result<Expr> {
        let ampm = self.ampm();
        if let (Some(s), None) = (s, ampm)
            && self.zone_at(self.pos).is_none()
            && !self.date_follows()
        {
            let part = |n: Number, unit: &str| Expr::WithUnit(Expr::Number(n).boxed(), registry().get(unit));
            let parts =
                vec![part(Number::from_i64(h as i64), "h"), part(Number::from_i64(m as i64), "min"), part(s, "s")];
            return Ok(Self::formatted(Expr::Composite(parts), Format::Laptime));
        }
        let hour = to_24h(h as i64, ampm)?;
        if m >= 60 || s.is_some_and(|s| s >= Number::from_i64(60)) {
            bail!("invalid time {h}:{m:02}");
        }
        let clock = TimeExpr::Clock { hour, minute: m as i8, second: s.unwrap_or(Number::ZERO), on: None };
        self.clock_tail(clock)
    }

    /// `3pm`, `11 am`.
    pub(super) fn clock_from_hour(&mut self, n: Number) -> Result<Option<Expr>> {
        let Some(h) = n.to_i64() else { return Ok(None) };
        let Some(ampm) = self.ampm() else { return Ok(None) };
        let hour = to_24h(h, Some(ampm))?;
        let clock = TimeExpr::Clock { hour, minute: 0, second: Number::ZERO, on: None };
        self.clock_tail(clock).map(Some)
    }

    /// Eats "am" or "pm"; `true` means pm.
    fn ampm(&mut self) -> Option<bool> {
        let pm = match self.lower(self.pos)?.as_str() {
            "am" => false,
            "pm" => true,
            _ => return None,
        };
        self.pos += 1;
        Some(pm)
    }

    /// Date and zone after a clock time: `3pm tomorrow`, `9am Tokyo`.
    fn clock_tail(&mut self, mut clock: TimeExpr) -> Result<Expr> {
        if self.date_follows() {
            let on = self.date_word()?;
            if let (TimeExpr::Clock { on: slot, .. }, Some(on)) = (&mut clock, on) {
                *slot = Some(on.boxed());
            }
        }
        let expr = Expr::Time(clock);
        Ok(match self.zone_at(self.pos) {
            Some((zone, n)) => {
                self.pos += n;
                Expr::InZone(expr.boxed(), zone)
            }
            None => expr,
        })
    }

    /// A clock time after a date: `tomorrow at 3pm`, `March 12 9:30`.
    pub(super) fn date_tail(&mut self, date: Expr) -> Result<Expr> {
        let mut p = self.clone();
        p.eat_word("at");
        let Some(tok) = p.cur() else { return Ok(date) };
        let clock = match &tok.tok {
            Tok::Clock { h, m, s } => {
                p.pos += 1;
                let hour = to_24h(*h as i64, p.ampm())?;
                Some(TimeExpr::Clock { hour, minute: *m as i8, second: s.unwrap_or(Number::ZERO), on: None })
            }
            Tok::Num(n)
                if p.toks
                    .get(p.pos + 1)
                    .and_then(|t| t.word())
                    .is_some_and(|w| matches!(w.to_lowercase().as_str(), "am" | "pm")) =>
            {
                p.pos += 1;
                let hour = to_24h(n.to_i64().unwrap_or(-1), p.ampm())?;
                Some(TimeExpr::Clock { hour, minute: 0, second: Number::ZERO, on: None })
            }
            Tok::Word(w) if matches!(w.to_lowercase().as_str(), "noon" | "midnight") => {
                p.pos += 1;
                let hour = if w.eq_ignore_ascii_case("noon") { 12 } else { 0 };
                Some(TimeExpr::Clock { hour, minute: 0, second: Number::ZERO, on: None })
            }
            _ => None,
        };
        let Some(TimeExpr::Clock { hour, minute, second, .. }) = clock else { return Ok(date) };
        *self = p;
        self.clock_tail(TimeExpr::Clock { hour, minute, second, on: Some(date.boxed()) })
    }

    /// A date word follows: tomorrow, a weekday, "on March 3"...
    fn date_follows(&self) -> bool {
        let Some(w) = self.lower(self.pos) else { return false };
        matches!(w.as_str(), "today" | "tomorrow" | "yesterday" | "on" | "next" | "last" | "this")
            || words::weekday(&w).is_some()
            || words::month(&w).is_some()
            || matches!(self.toks.get(self.pos).map(|t| &t.tok), Some(Tok::Date { .. }))
    }

    /// Parses the date after a clock time.
    fn date_word(&mut self) -> Result<Option<Expr>> {
        self.eat_word("on");
        if let Some(Tok::Date { parts, sep }) = self.cur().map(|t| &t.tok) {
            self.pos += 1;
            return self.numeric_date(*parts, *sep).map(Some);
        }
        if let Some(Tok::Num(n)) = self.cur().map(|t| &t.tok) {
            self.pos += 1;
            return self.day_first_date(*n);
        }
        self.time_word_date()
    }

    /// `2026-09-23`, `23.09.2026`, `9/23/2026`.
    pub(super) fn numeric_date(&self, parts: [u32; 3], sep: char) -> Result<Expr> {
        let [a, b, c] = parts;
        let (year, month, day) = match sep {
            '-' => (a, b, c),
            '.' => (c, b, a),
            _ => match self.config().date_order {
                // Swap when only the other order is valid: 12/25/2026.
                DateOrder::DayFirst if b <= 12 || a > 12 => (c, b, a),
                DateOrder::MonthFirst if a > 12 && b <= 12 => (c, b, a),
                DateOrder::DayFirst => (c, a, b),
                DateOrder::MonthFirst => (c, a, b),
            },
        };
        date(year as i64, month as i64, day as i64)
    }

    /// `12 March`, `3rd of June 2026` after the number was read.
    pub(super) fn day_first_date(&mut self, n: Number) -> Result<Option<Expr>> {
        let Some(day) = n.to_i64().filter(|d| (1..=31).contains(d)) else { return Ok(None) };
        let mut p = self.clone();
        if p.cur().and_then(|t| t.word()).is_some_and(|w| matches!(w, "st" | "nd" | "rd" | "th")) {
            p.pos += 1;
        }
        p.eat_word("of");
        let Some((month, _)) = p.lower(p.pos).and_then(|w| words::month(&w)) else { return Ok(None) };
        p.pos += 1;
        let year = p.year();
        *self = p;
        match year {
            Some(y) => date(y, month as i64, day).map(Some),
            None => Ok(Some(Expr::Time(TimeExpr::Date { year: None, month, day: day as i8 }))),
        }
    }

    /// `March 12`, `Mar 12th, 2026`, `March 2026`.
    fn month_first_date(&mut self, month: i8) -> Result<Option<Expr>> {
        let mut p = self.clone();
        p.pos += 1;
        p.eat_sym(",");
        let Some(Tok::Num(n)) = p.cur().map(|t| &t.tok) else { return Ok(None) };
        let Some(n) = n.to_i64() else { return Ok(None) };
        p.pos += 1;
        if (1000..=9999).contains(&n) {
            *self = p;
            return date(n, month as i64, 1).map(Some);
        }
        if !(1..=31).contains(&n) {
            return Ok(None);
        }
        // "March 3pm" is not a date.
        if p.lower(p.pos).is_some_and(|w| w == "am" || w == "pm") {
            return Ok(None);
        }
        if p.cur().and_then(|t| t.word()).is_some_and(|w| matches!(w, "st" | "nd" | "rd" | "th")) {
            p.pos += 1;
        }
        let year = p.year();
        *self = p;
        match year {
            Some(y) => date(y, month as i64, n).map(Some),
            None => Ok(Some(Expr::Time(TimeExpr::Date { year: None, month, day: n as i8 }))),
        }
    }

    /// An optional four digit year: `, 2026` or `2026`.
    fn year(&mut self) -> Option<i64> {
        let mut p = self.clone();
        p.eat_sym(",");
        let Some(Tok::Num(n)) = p.cur().map(|t| &t.tok) else { return None };
        let y = n.to_i64().filter(|y| (1000..=9999).contains(y))?;
        // "June 5 2020 + 3" keeps the year, "June 5 1500 m" does not.
        p.pos += 1;
        if p.unit_at(p.pos, true).is_some() {
            return None;
        }
        *self = p;
        Some(y)
    }

    /// Words about time at the cursor: now, today, next friday, christmas...
    pub(super) fn time_word(&mut self) -> Result<Option<Expr>> {
        let w = self.lower(self.pos).unwrap_or_default();
        let next = self.lower(self.pos + 1).unwrap_or_default();
        match w.as_str() {
            "now" => {
                self.pos += 1;
                return Ok(Some(Expr::Time(TimeExpr::Now)));
            }
            "time" | "date" | "difference" => return self.time_phrase(&w),
            "noon" | "midnight" => {
                self.pos += 1;
                let hour = if w == "noon" { 12 } else { 0 };
                return self.clock_tail(TimeExpr::Clock { hour, minute: 0, second: Number::ZERO, on: None }).map(Some);
            }
            "in" if self.operand_follows(1) => {
                // "in 3 days"
                self.pos += 1;
                let amount = self.multiplicative()?;
                return Ok(Some(Expr::binary(crate::ast::Op::Add, now_for(&amount), amount)));
            }
            "from" if self.operand_follows(1) => {
                self.pos += 1;
                let start = self.additive()?;
                if !self.eat_word("to") && !self.eat_word("until") && !self.eat_word("till") {
                    return Ok(Some(start));
                }
                let end = self.additive()?;
                return Ok(Some(Expr::Range(start.boxed(), end.boxed())));
            }
            "current" if next == "timestamp" => {
                self.pos += 2;
                return Ok(Some(Self::formatted(Expr::Time(TimeExpr::Now), Format::Timestamp)));
            }
            "current" if matches!(next.as_str(), "time" | "date") => {
                self.pos += 1;
                return self.time_word();
            }
            "unix" | "epoch" | "timestamp" => {
                self.pos += 1;
                if matches!(next.as_str(), "time" | "timestamp") {
                    self.pos += 1;
                }
                return Ok(Some(Self::formatted(Expr::Time(TimeExpr::Now), Format::Timestamp)));
            }
            _ => {}
        }
        if let Some(expr) = self.time_word_date()? {
            return self.date_tail(expr).map(Some);
        }
        Ok(None)
    }

    /// Words that name a date: today, next friday, christmas 2027, March 12.
    fn time_word_date(&mut self) -> Result<Option<Expr>> {
        let w = self.lower(self.pos).unwrap_or_default();
        let next = self.lower(self.pos + 1).unwrap_or_default();
        let day = |p: &mut Self, n: usize, offset: i64| {
            p.pos += n;
            Ok(Some(Expr::Time(TimeExpr::Today(offset))))
        };
        if self.words_at(self.pos, &["day", "after", "tomorrow"]) {
            return day(self, 3, 2);
        }
        if self.words_at(self.pos, &["day", "before", "yesterday"]) {
            return day(self, 3, -2);
        }
        match w.as_str() {
            "today" => return day(self, 1, 0),
            "tomorrow" => return day(self, 1, 1),
            "yesterday" => return day(self, 1, -1),
            "next" | "last" | "this" => {
                let which = match w.as_str() {
                    "next" => Which::Next,
                    "last" => Which::Last,
                    _ => Which::This,
                };
                if let Some((weekday, _)) = words::weekday(&next) {
                    self.pos += 2;
                    return Ok(Some(Expr::Time(TimeExpr::Weekday(weekday, Some(which)))));
                }
                let period = match next.as_str() {
                    "week" => jiff::Unit::Week,
                    "month" => jiff::Unit::Month,
                    "year" => jiff::Unit::Year,
                    _ => return Ok(None),
                };
                self.pos += 2;
                return Ok(Some(Expr::Time(TimeExpr::Period(period, which))));
            }
            _ => {}
        }
        if let Some((weekday, short)) = words::weekday(&w) {
            let clock_follows =
                matches!(self.toks.get(self.pos + 1).map(|t| &t.tok), Some(Tok::Num(_) | Tok::Clock { .. }));
            if !short || clock_follows {
                self.pos += 1;
                return Ok(Some(Expr::Time(TimeExpr::Weekday(weekday, None))));
            }
        }
        if let Some((month, _)) = words::month(&w)
            && let Some(expr) = self.month_first_date(month)?
        {
            return Ok(Some(expr));
        }
        if let Some((holiday, n)) = self.phrase_at(self.pos, words::HOLIDAYS) {
            self.pos += n;
            let year = self.year().map(|y| y as i16);
            return Ok(Some(Expr::Time(TimeExpr::Holiday { holiday, year })));
        }
        Ok(None)
    }

    /// Phrases starting with "time", "date" or "difference".
    fn time_phrase(&mut self, w: &str) -> Result<Option<Expr>> {
        let now = || Expr::Time(TimeExpr::Now);
        let timespan = |e: Expr| Self::formatted(e, Format::Timespan);
        self.pos += 1;
        if w == "time" && self.at_word("difference") {
            self.pos += 1;
        }
        // "time to upload 3GB at 10 MB/s" is just the division.
        let verbs = ["upload", "download", "transfer", "copy", "send"];
        if w == "time"
            && self.toks.get(self.pos).is_some_and(|t| t.is_word("to"))
            && verbs.iter().any(|v| self.words_at(self.pos + 1, &[v]))
        {
            self.pos += 2;
            return self.additive().map(Some);
        }
        if w == "difference" || self.toks[self.pos - 1].is_word("difference") {
            self.eat_word("between");
            return self.difference().map(Some);
        }
        if w == "time" {
            if self.eat_word("until") || self.eat_word("till") {
                let end = self.additive()?;
                return Ok(Some(timespan(Expr::Range(now().boxed(), end.boxed()))));
            }
            if self.eat_word("since") {
                let start = self.additive()?;
                return Ok(Some(timespan(Expr::Range(start.boxed(), now().boxed()))));
            }
            if self.eat_word("between") {
                let [a, b] = self.pair()?;
                return Ok(Some(timespan(Expr::Range(a.boxed(), b.boxed()))));
            }
            return Ok(Some(now()));
        }
        Ok(Some(Expr::Time(TimeExpr::Today(0))))
    }

    /// `A and B` inside `between`.
    fn pair(&mut self) -> Result<[Expr; 2]> {
        let saved = std::mem::replace(&mut self.in_list, true);
        let a = self.additive()?;
        self.eat_word("and");
        let b = self.additive()?;
        self.in_list = saved;
        Ok([a, b])
    }

    /// "difference between Seattle and Moscow" or between two dates.
    fn difference(&mut self) -> Result<Expr> {
        if let Some((a, n)) = self.zone_at(self.pos)
            && self.toks.get(self.pos + n).is_some_and(|t| t.is_word("and") || t.is_sym("&"))
            && let Some((b, m)) = self.zone_at(self.pos + n + 1)
        {
            self.pos += n + 1 + m;
            return Ok(Expr::ZoneDiff(a, b));
        }
        let [a, b] = self.pair()?;
        Ok(Expr::Range(a.boxed(), b.boxed()))
    }

    /// After a time unit: "days until christmas", "weeks since March 1".
    pub(super) fn time_unit_phrase(&mut self, unit: &Unit) -> Result<Option<Expr>> {
        let start = || now_for(&Expr::BareUnit(unit.clone()));
        let w = self.lower(self.pos).unwrap_or_default();
        let range = match w.as_str() {
            "until" | "till" | "to" if w != "to" || self.date_starts(self.pos + 1) => {
                self.pos += 1;
                Expr::Range(start().boxed(), self.additive()?.boxed())
            }
            "since" => {
                self.pos += 1;
                Expr::Range(self.additive()?.boxed(), start().boxed())
            }
            "between" => {
                self.pos += 1;
                let [a, b] = self.pair()?;
                Expr::Range(a.boxed(), b.boxed())
            }
            "from" if self.date_starts(self.pos + 1) => {
                self.pos += 1;
                let a = self.additive()?;
                self.eat_word("to");
                Expr::Range(a.boxed(), self.additive()?.boxed())
            }
            // "days in February 2020", "days in Q3"
            "in" => match self.period_at(self.pos + 1) {
                Some((start, span, n)) => {
                    self.pos += 1 + n;
                    let end = Expr::binary(crate::ast::Op::Add, start.clone(), span);
                    Expr::Range(start.boxed(), end.boxed())
                }
                None => return Ok(None),
            },
            _ => return Ok(None),
        };
        Ok(Some(Expr::Convert(range.boxed(), Target::Unit(unit.clone()))))
    }

    /// A calendar period at token `i`: its first day, length and size in tokens.
    fn period_at(&self, i: usize) -> Option<(Expr, Expr, usize)> {
        let w = self.lower(i)?;
        let quarter = w.strip_prefix('q').and_then(|q| q.parse::<i8>().ok()).filter(|q| (1..=4).contains(q));
        let (month, months) = match (words::month(&w), quarter) {
            (Some((month, _)), _) => (month, 1),
            (None, Some(q)) => ((q - 1) * 3 + 1, 3),
            _ => return None,
        };
        let mut p = self.clone();
        p.pos = i + 1;
        let year = p.year().map(|y| y as i16);
        let start = Expr::Time(TimeExpr::Date { year, month, day: 1 });
        let span = Expr::WithUnit(Expr::Number(Number::from_i64(months)).boxed(), registry().get("mo"));
        Some((start, span, p.pos - i))
    }

    /// A date or time starts at token `i`.
    fn date_starts(&self, i: usize) -> bool {
        let mut p = self.clone();
        p.pos = i;
        p.skip_noise();
        match p.cur().map(|t| &t.tok) {
            Some(Tok::Date { .. } | Tok::Clock { .. } | Tok::IsoDateTime(_)) => true,
            Some(Tok::Num(_)) => {
                p.pos += 1;
                p.lower(p.pos).is_some_and(|w| w == "am" || w == "pm")
                    || p.day_first_date(Number::ONE).ok().flatten().is_some()
            }
            Some(Tok::Word(_)) => {
                let w = p.lower(p.pos).unwrap_or_default();
                matches!(
                    w.as_str(),
                    "now" | "today" | "tomorrow" | "yesterday" | "next" | "last" | "this" | "noon" | "midnight"
                ) || words::month(&w).is_some()
                    || words::weekday(&w).is_some()
                    || p.phrase_at(p.pos, words::HOLIDAYS).is_some()
                    || p.var_at(p.pos).is_some()
            }
            _ => false,
        }
    }

    /// A time zone at token `i`: place, abbreviation or `UTC+5`.
    pub(super) fn zone_at(&self, i: usize) -> Option<(TimeZone, usize)> {
        let (zone, mut n) = self.offset_zone_at(i).or_else(|| self.place_at(i))?;
        if self.toks.get(i + n).is_some_and(|t| t.is_word("time")) {
            n += 1;
        }
        Some((zone, n))
    }

    /// `UTC+5`, `GMT-3:30`.
    fn offset_zone_at(&self, i: usize) -> Option<(TimeZone, usize)> {
        let w = self.lower(i)?;
        if !matches!(w.as_str(), "utc" | "gmt") {
            return None;
        }
        let sign = match self.toks.get(i + 1)?.tok {
            Tok::Sym("+") => 1,
            Tok::Sym("-") => -1,
            _ => return None,
        };
        let seconds = match &self.toks.get(i + 2)?.tok {
            Tok::Num(n) => n.to_i64().filter(|h| *h <= 14)? * 3600,
            Tok::Clock { h, m, s: None } => (*h as i64) * 3600 + (*m as i64) * 60,
            _ => return None,
        };
        // "now in utc + 5 hours" adds hours instead.
        if self.unit_at(i + 3, true).is_some_and(|(u, _)| u.dim() == Dim::TIME) {
            return None;
        }
        Some((zones::fixed((sign * seconds) as i32)?, 3))
    }
}

fn to_24h(h: i64, pm: Option<bool>) -> Result<i8> {
    let hour = match pm {
        None if (0..=24).contains(&h) => h % 24,
        Some(pm) if (1..=12).contains(&h) => h % 12 + if pm { 12 } else { 0 },
        _ => bail!("invalid hour {h}"),
    };
    Ok(hour as i8)
}

fn date(year: i64, month: i64, day: i64) -> Result<Expr> {
    let (Ok(y), Ok(m), Ok(d)) = (i16::try_from(year), i8::try_from(month), i8::try_from(day)) else {
        bail!("invalid date");
    };
    if jiff::civil::Date::new(y, m, d).is_err() {
        bail!("invalid date {year}-{month:02}-{day:02}");
    }
    Ok(Expr::Time(TimeExpr::Date { year: Some(y), month: m, day: d }))
}
