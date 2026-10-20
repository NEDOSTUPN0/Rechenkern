//! Dates, clock times, time zones and phrases like "days until christmas".

use jiff::tz::TimeZone;

use super::{Parser, words};
use crate::ast::{Expr, Format, Func, MonthDay, Op, Target, TimeExpr, Which};
use crate::config::DateOrder;
use crate::error::{Result, bail};
use crate::lexer::Tok;
use crate::number::Number;
use crate::units::{Dim, Unit, registry};
use crate::zones::{self, Place};

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

/// Whether `expr` may be a length of time: "$100 from 1990" is not date arithmetic.
pub(super) fn maybe_duration(expr: &Expr) -> bool {
    match expr {
        Expr::Number(_) | Expr::Percent(_) | Expr::Bool(_) => false,
        Expr::WithUnit(_, u) | Expr::BareUnit(u) => u.dim() == Dim::TIME || u.dim() == Dim::WORKDAY,
        _ => true,
    }
}

/// Days, weeks, months, years and workdays count whole dates.
fn is_date_unit(unit: &Unit) -> bool {
    unit.dim() == Dim::WORKDAY || unit.single().and_then(|d| d.calendar).is_some_and(|(cal, _)| cal >= jiff::Unit::Day)
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
        if let (None, None) = (s, ampm)
            && let Some(amount) = self.clock_as_duration(h, m)
        {
            return Ok(amount);
        }
        let mut hour = to_24h(h as i64, ampm)?;
        // "3:30" is in the afternoon when the config prefers the day: 1 to 6 o'clock.
        if ampm.is_none()
            && (1..=6).contains(&hour)
            && self.config().features.dates.prefer_daytime_for_ambiguous_12_hour_times
        {
            hour += 12;
        }
        if m >= 60 || s.is_some_and(|s| s >= Number::from_i64(60)) {
            bail!("invalid time {h}:{m:02}");
        }
        let clock = TimeExpr::Clock { hour, minute: m as i8, second: s.unwrap_or(Number::ZERO), on: None };
        self.clock_tail(clock)
    }

    /// `h:mm` as a length of time: `5:30 min/km` is 5 minutes 30 seconds a kilometer,
    /// `5:30 in minutes` is 5 hours 30 minutes.
    fn clock_as_duration(&mut self, h: u32, m: u32) -> Option<Expr> {
        let reg = registry();
        // "00:45 + 01:30": times added are lengths of time, as in SoulverCore.
        let bare_clock = |i: usize| {
            matches!(self.toks.get(i).map(|t| &t.tok), Some(Tok::Clock { s: None, .. }))
                && !self.lower(i + 1).is_some_and(|w| matches!(w.as_str(), "am" | "pm" | "a.m." | "p.m."))
        };
        let sign = |i: usize| self.toks.get(i).is_some_and(|t| t.is_sym("+") || t.is_sym("-"));
        let summed = (sign(self.pos) && bare_clock(self.pos + 1))
            || (self.pos >= 3 && sign(self.pos - 2) && bare_clock(self.pos - 3));
        if summed && m < 60 {
            let part =
                |n: u32, unit: &str| Expr::WithUnit(Expr::Number(Number::from_i64(n.into())).boxed(), reg.get(unit));
            return Some(match (h, m) {
                (0, m) => part(m, "min"),
                (h, 0) => part(h, "h"),
                (h, m) => Expr::binary(Op::Add, part(h, "h"), part(m, "min")),
            });
        }
        // "5:30/km": minutes and seconds a kilometer, a pace.
        if m < 60
            && self.toks.get(self.pos).is_some_and(|t| t.is_sym("/"))
            && self.unit_at(self.pos + 1, true).is_some_and(|(u, _)| u.dim() == Dim::LENGTH)
        {
            let minutes = Expr::binary(
                Op::Add,
                Expr::Number(Number::from_i64(h.into())),
                Expr::Number(Number::ratio(m.into(), 60)),
            );
            return Some(Expr::WithUnit(minutes.boxed(), reg.get("min")));
        }
        let time_unit = |i: usize| self.unit_at(i, true).filter(|(u, _)| u.dim() == Dim::TIME);
        let converted = ["in", "to", "as", "into"].iter().any(|w| self.words_at(self.pos, &[w]));
        let (unit, len) = match converted {
            // "5:30 in minutes"
            true => time_unit(self.pos + 1).map(|_| (reg.get("h"), 0))?,
            false => time_unit(self.pos).filter(|(u, _)| *u == reg.get("h") || *u == reg.get("min"))?,
        };
        if m >= 60 {
            return None;
        }
        self.pos += len;
        let parts =
            Expr::binary(Op::Add, Expr::Number(Number::from_i64(h.into())), Expr::Number(Number::ratio(m.into(), 60)));
        Some(Expr::WithUnit(parts.boxed(), unit))
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
            Tok::Word(w) if matches!(w.to_lowercase().as_str(), "noon" | "midday" | "midnight") => {
                p.pos += 1;
                let hour = if w.eq_ignore_ascii_case("midnight") { 0 } else { 12 };
                Some(TimeExpr::Clock { hour, minute: 0, second: Number::ZERO, on: None })
            }
            _ => None,
        };
        let Some(TimeExpr::Clock { hour, minute, second, .. }) = clock else { return Ok(date) };
        *self = p;
        self.clock_tail(TimeExpr::Clock { hour, minute, second, on: Some(date.boxed()) })
    }

    /// A weekday or a period at token `i`: "previous monday", "previous month".
    pub(super) fn date_follows_at(&self, i: usize) -> bool {
        self.lower(i).is_some_and(|w| {
            words::weekday(&w).is_some()
                || matches!(
                    w.as_str(),
                    "week" | "month" | "year" | "quarter" | "spring" | "summer" | "autumn" | "fall" | "winter"
                )
        })
    }

    /// A date word follows: tomorrow, a weekday, "on March 3"...
    pub(super) fn date_follows(&self) -> bool {
        let Some(w) = self.lower(self.pos) else { return false };
        matches!(w.as_str(), "today" | "tomorrow" | "yesterday" | "on" | "next" | "last" | "previous" | "this")
            || words::weekday(&w).is_some()
            || words::month(&w).is_some()
            || matches!(self.toks.get(self.pos).map(|t| &t.tok), Some(Tok::Date { .. }))
    }

    /// Parses the date after a clock time.
    pub(super) fn date_word(&mut self) -> Result<Option<Expr>> {
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

    /// "first monday of the month", "last friday of October", "third friday in March",
    /// "last day of February", "first day of next month", "the first of next month".
    pub(super) fn day_in_month(&mut self) -> Result<Option<Expr>> {
        let Some((nth, n)) = self.nth_at(self.pos) else { return Ok(None) };
        let Some(len) = self.day_in_month_at(self.pos + n) else { return self.nth_weekday(nth, n) };
        let pick = match self.lower(self.pos + n).and_then(|w| words::weekday(&w)) {
            Some((weekday, _)) => MonthDay::Nth(nth, weekday),
            None => MonthDay::Day(nth),
        };
        self.pos += n + len;
        let month = self.month_of()?;
        Ok(Some(Expr::Time(TimeExpr::InMonth(pick, month))))
    }

    /// "second tuesday" alone: that one of this month, or of the next once it has passed.
    fn nth_weekday(&mut self, nth: i8, n: usize) -> Result<Option<Expr>> {
        // "last friday" is the one before today.
        if nth < 1 || !self.weekday_ends_at(self.pos + n) {
            return Ok(None);
        }
        let Some((weekday, _)) = self.lower(self.pos + n).and_then(|w| words::weekday(&w)) else { return Ok(None) };
        self.pos += n + 1;
        Ok(Some(Expr::Time(TimeExpr::InMonth(MonthDay::Nth(nth, weekday), None))))
    }

    /// A weekday at token `i` that ends the value.
    pub(super) fn weekday_ends_at(&self, i: usize) -> bool {
        if self.lower(i).and_then(|w| words::weekday(&w)).is_none() {
            return false;
        }
        let mut p = self.clone();
        p.pos = i + 1;
        p.value_ends()
    }

    /// After an ordinal at token `i`: a weekday or "day" (or nothing), then "of" or "in"
    /// a month. The length of the words before the month.
    pub(super) fn day_in_month_at(&self, i: usize) -> Option<usize> {
        let day = self.lower(i).is_some_and(|w| w == "day" || words::weekday(&w).is_some());
        let at = i + usize::from(day);
        let of = self.words_at(at, &["of"]) && (day || self.month_follows_at(at + 1));
        (of || (self.words_at(at, &["in"]) && self.month_follows_at(at + 1))).then_some(at + 1 - i)
    }

    /// A month at token `i`: "March", "the month", "next month", "this month".
    fn month_follows_at(&self, i: usize) -> bool {
        self.lower(i).is_some_and(|w| words::month(&w).is_some())
            || [&["the", "month"][..], &["month"], &["next", "month"], &["this", "month"], &["last", "month"]]
                .iter()
                .any(|ws| self.words_at(i, ws))
    }

    /// "first", "2nd", "last" at token `i`: which one, and its length in tokens.
    pub(super) fn nth_at(&self, i: usize) -> Option<(i8, usize)> {
        if let Some(Tok::Num(n)) = self.toks.get(i).map(|t| &t.tok)
            && self
                .toks
                .get(i + 1)
                .is_some_and(|t| !t.space_before && ["st", "nd", "rd", "th"].iter().any(|s| t.is_word(s)))
        {
            return n.to_i64().filter(|n| (1..=31).contains(n)).map(|n| (n as i8, 2));
        }
        // "tenth friday of May" is no day at all, not a tenth.
        let words = ["first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth"];
        let w = self.lower(i)?;
        let n = match words.iter().position(|o| *o == w) {
            Some(k) => k as i8 + 1,
            None if w == "last" => -1,
            None => return None,
        };
        Some((n, 1))
    }

    /// The month after "of": "the month" (`None`, the next one from today), "next month",
    /// "October", "March 2027".
    pub(super) fn month_of(&mut self) -> Result<Option<Box<Expr>>> {
        for every in [&["the", "month"][..], &["every", "month"], &["each", "month"], &["month"]] {
            if self.words_at(self.pos, every) {
                self.pos += every.len();
                return Ok(None);
            }
        }
        Ok(Some(self.date_or_period(true)?.boxed()))
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
        p.eat_ordinal();
        p.eat_word("of");
        let Some((month, _)) = p.lower(p.pos).and_then(|w| words::month(&w)) else { return Ok(None) };
        p.pos += 1;
        let year = p.year();
        *self = p;
        match year {
            Some(y) => date(y, month as i64, day).map(Some),
            None => Ok(Some(self.in_year(TimeExpr::Date { year: None, month, day: day as i8 }))),
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
        p.eat_ordinal();
        let year = p.year();
        *self = p;
        match year {
            Some(y) => date(y, month as i64, n).map(Some),
            None => Ok(Some(self.in_year(TimeExpr::Date { year: None, month, day: n as i8 }))),
        }
    }

    /// An optional four digit year: `, 2026` or `2026`.
    fn year(&mut self) -> Option<i64> {
        let mut p = self.clone();
        p.eat_sym(",");
        let Some(Tok::Num(n)) = p.cur().map(|t| &t.tok) else { return None };
        let y = n.to_i64().filter(|y| (1000..=9999).contains(y))?;
        // "June 5 2020 + 3" and "March 9, 2024 day of the week" keep the year, "June 5 1500 m" does not.
        p.pos += 1;
        if p.unit_at(p.pos, true).is_some() && p.phrase_at(p.pos, &words::DATE_PARTS).is_none() {
            return None;
        }
        *self = p;
        Some(y)
    }

    /// A date without a year, maybe in a year named after it: "April 21 next year".
    fn in_year(&mut self, date: TimeExpr) -> Expr {
        let yearless = matches!(date, TimeExpr::Date { year: None, .. } | TimeExpr::Holiday { year: None, .. });
        let which = self.lower(self.pos).and_then(|w| words::which(&w));
        match which {
            Some(which) if yearless && self.words_at(self.pos + 1, &["year"]) => {
                self.pos += 2;
                Expr::Time(TimeExpr::InYear(date.into(), which))
            }
            _ => Expr::Time(date),
        }
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
            // "diff between Tokyo and London", "diff Tokyo London"
            "diff" if self.words_at(self.pos + 1, &["between"]) || self.zone_at(self.pos + 1).is_some() => {
                return self.time_phrase("difference");
            }
            "noon" | "midday" | "midnight" => {
                self.pos += 1;
                let hour = if w == "midnight" { 0 } else { 12 };
                return self.clock_tail(TimeExpr::Clock { hour, minute: 0, second: Number::ZERO, on: None }).map(Some);
            }
            "in" if self.operand_follows(1) => {
                // "in 3 days"
                self.pos += 1;
                // "in 3 days at 5pm": a clock time on that day.
                let mut p = self.clone();
                if let Ok(amount) = p.unary()
                    && let today @ Expr::Time(TimeExpr::Today(_)) = now_for(&amount)
                {
                    let at = p.pos;
                    let date = p.date_tail(Expr::binary(Op::Add, today, amount))?;
                    if p.pos > at {
                        *self = p;
                        return Ok(Some(date));
                    }
                }
                let amount = self.multiplicative()?;
                // "in Q3" is the quarter itself.
                if let Expr::Period(..) = amount {
                    return Ok(Some(amount));
                }
                return Ok(Some(Expr::binary(Op::Add, now_for(&amount), amount)));
            }
            // "between 5 and 10 kg"
            "between" if self.operand_follows(1) => {
                self.pos += 1;
                let [a, b] = self.pair()?;
                return Ok(Some(Expr::Range(a.boxed(), b.boxed())));
            }
            "from" if self.operand_follows(1) => {
                self.pos += 1;
                let start = self.additive()?;
                let Some(through) = self.range_word(&["to", "until", "till"]) else { return Ok(Some(start)) };
                let end = self.range_end(through)?;
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
            // "fromunix(1559740303)": the date of a timestamp.
            "fromunix" if self.operand_follows(1) => {
                self.pos += 1;
                let seconds = self.unary()?;
                return Ok(Some(Self::formatted(seconds, Format::Date)));
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
        if let Some((date, n)) = self.period_bound_at(self.pos) {
            self.pos += n;
            return self.date_tail(date).map(Some);
        }
        if let Some((first, length, n)) = self.quarter_at(self.pos, &w) {
            self.pos += n;
            return Ok(Some(Expr::Period(first.boxed(), length.boxed())));
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
        if let Some(expr) = self.day_in_month()? {
            return Ok(Some(expr));
        }
        match w.as_str() {
            "today" => return day(self, 1, 0),
            "tomorrow" => return day(self, 1, 1),
            "yesterday" => return day(self, 1, -1),
            "next" | "last" | "previous" | "this" => {
                let which = words::which(&w).unwrap_or(Which::This);
                if let Some((weekday, _)) = words::weekday(&next) {
                    self.pos += 2;
                    // "this sunday" is the one ahead, as the weekday alone.
                    let which = Some(which).filter(|&w| w != Which::This);
                    return Ok(Some(Expr::Time(TimeExpr::Weekday(weekday, which))));
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
            // "mon" alone is a day too.
            let alone = self.toks.len() == 1;
            if !short || clock_follows || alone {
                self.pos += 1;
                // "friday in 3 weeks": that day of the week 3 weeks from now.
                if let Some(weeks) = self.in_weeks() {
                    let day = Expr::Time(TimeExpr::Weekday(weekday, Some(Which::This)));
                    return Ok(Some(Expr::binary(Op::Add, day, weeks)));
                }
                return Ok(Some(Expr::Time(TimeExpr::Weekday(weekday, None))));
            }
        }
        if let Some((month, _)) = words::month(&w)
            && let Some(expr) = self.month_first_date(month)?
        {
            return Ok(Some(expr));
        }
        if let Some((holiday, n)) = self.phrase_at(self.pos, &words::HOLIDAYS) {
            // "Christmas day of the week": "day" starts the date part.
            let n = match n > 1 && self.words_at(self.pos + n - 1, &["day", "of"]) {
                true => self.phrase_at(self.pos, &words::HOLIDAYS).filter(|&(_, m)| m < n).map_or(n - 1, |(_, m)| m),
                false => n,
            };
            self.pos += n;
            let year = self.year().map(|y| y as i16);
            return Ok(Some(self.in_year(TimeExpr::Holiday { holiday, year })));
        }
        if let Some((event, n)) = self.event_at(self.pos) {
            self.pos += n;
            if !self.eat_word("release") {
                self.eat_word("launch");
            }
            self.eat_word("date");
            let (year, month, day) = event.date;
            let date = Expr::Time(TimeExpr::Date { year: Some(year), month, day });
            return Ok(Some(Expr::Noted(date.boxed(), event.note)));
        }
        Ok(None)
    }

    /// "in 3 weeks", "in a fortnight" at the cursor: the weeks, read past.
    fn in_weeks(&mut self) -> Option<Expr> {
        if !self.words_at(self.pos, &["in"]) {
            return None;
        }
        let mut p = self.clone();
        p.pos += 1;
        let amount = p.multiplicative().ok()?;
        let unit = match &amount {
            Expr::WithUnit(_, u) | Expr::BareUnit(u) => u,
            _ => return None,
        };
        unit.single()?.calendar.filter(|(cal, _)| *cal == jiff::Unit::Week)?;
        *self = p;
        Some(amount)
    }

    /// An event name at token `i`, such as `GTA 6`.
    pub(super) fn event_at(&self, i: usize) -> Option<(&'static words::Event, usize)> {
        // Names are lowercase; numbers in them are whole.
        let spells = |k: usize, part: &str| match self.toks.get(k).map(|t| &t.tok) {
            Some(Tok::Word(w)) => w.chars().flat_map(char::to_lowercase).eq(part.chars()),
            Some(Tok::Num(n)) => n.is_integer() && n.to_string() == part,
            _ => false,
        };
        words::EVENTS.iter().find_map(|event| {
            event.names.iter().find_map(|name| {
                let mut n = 0;
                for part in name.split(' ') {
                    if !spells(i + n, part) {
                        return None;
                    }
                    n += 1;
                }
                Some((event, n))
            })
        })
    }

    /// "time" after an unknown word is not now, as in SoulverCore: "Qwertyville time",
    /// "dinner time". "what time is it" and "the time" still are.
    pub(super) fn time_after_word(&self, i: usize) -> bool {
        self.toks[i].is_word("time")
            && i > 0
            && self.comment_at(i - 1)
            && !self.lower(i - 1).is_some_and(|w| matches!(w.as_str(), "the" | "what" | "what's" | "whats" | "local"))
    }

    /// Phrases starting with "time", "date" or "difference".
    fn time_phrase(&mut self, w: &str) -> Result<Option<Expr>> {
        let now = || Expr::Time(TimeExpr::Now);
        let timespan = |e: Expr| Self::formatted(e, Format::Timespan);
        if self.time_after_word(self.pos) {
            return Ok(None);
        }
        self.pos += 1;
        let difference = w == "difference" || (w == "time" && (self.eat_word("difference") || self.eat_word("diff")));
        // "time to upload 3GB at 10 MB/s" is just the division.
        let verbs = ["upload", "download", "transfer", "copy", "send"];
        if w == "time"
            && self.toks.get(self.pos).is_some_and(|t| t.is_word("to"))
            && verbs.iter().any(|v| self.words_at(self.pos + 1, &[v]))
        {
            self.pos += 2;
            return self.additive().map(Some);
        }
        if difference {
            self.eat_word("between");
            return self.difference().map(Some);
        }
        if w == "time" {
            if self.eat_word("until") || self.eat_word("till") {
                let end = self.additive()?;
                return Ok(Some(timespan(Expr::SignedRange(now().boxed(), end.boxed()))));
            }
            if self.eat_word("since") {
                let start = self.additive()?;
                return Ok(Some(timespan(Expr::SignedRange(start.boxed(), now().boxed()))));
            }
            if self.eat_word("between") {
                let [a, b] = self.pair()?;
                return Ok(Some(timespan(Expr::Range(a.boxed(), b.boxed()))));
            }
            // "time from 9am to 5pm"; "time from 9am" counts to now.
            if self.date_starts(self.pos + 1) && self.eat_word("from") {
                let a = self.additive()?;
                let b = match self.range_word(&["to", "until", "till"]) {
                    Some(through) => self.range_end(through)?,
                    None => now(),
                };
                return Ok(Some(timespan(Expr::Range(a.boxed(), b.boxed()))));
            }
            // "time in qld", "time in tx": a region's code, only here, as in SoulverCore.
            // In capitals an airport code wins: "TAS" is Tashkent, "tas" Tasmania.
            let airport =
                |w: &str| w.chars().all(|c| c.is_ascii_uppercase()) && zones::find(&w.to_lowercase()).is_some();
            if self.words_at(self.pos, &["in"])
                && self.toks.len() == self.pos + 2
                && !self.toks[self.pos + 1].word().is_some_and(airport)
                && let Some(zone) = self.lower(self.pos + 1).and_then(|w| words::region_zone(&w))
                && let Ok(zone) = TimeZone::get(zone)
            {
                let written = self.toks[self.pos + 1].word().unwrap_or_default().to_uppercase();
                self.pos += 2;
                return Ok(Some(Expr::Convert(now().boxed(), Target::Zone(Place::written(&written, zone)))));
            }
            // "time Tokyo", "time at Tokyo now", "time now in Tokyo"
            self.eat_word("now");
            let mut p = self.clone();
            p.eat_word("at");
            if let Some((zone, n)) = p.zone_at(p.pos) {
                *self = p;
                self.pos += n;
                self.eat_word("now");
                return Ok(Some(Expr::Convert(now().boxed(), Target::Zone(zone))));
            }
            return Ok(Some(now()));
        }
        // "date Tokyo", "date at Tokyo": the date there.
        let today = Expr::Time(TimeExpr::Today(0));
        let mut p = self.clone();
        p.eat_word("at");
        if let Some((zone, n)) = p.zone_at(p.pos) {
            *self = p;
            self.pos += n;
            return Ok(Some(Expr::Convert(today.boxed(), Target::Zone(zone))));
        }
        Ok(Some(today))
    }

    /// `A and B` inside `between`; also `A to B` and `A - B`.
    pub(super) fn pair(&mut self) -> Result<[Expr; 2]> {
        let saved = std::mem::replace(&mut self.in_list, true);
        let a = self.additive()?;
        let through = match self.range_word(&["and", "to"]) {
            Some(through) => through,
            // "between March 3 - May 30" reads as a subtraction of dates.
            None => match a {
                Expr::Binary(Op::Sub, a, b) if matches!((&*a, &*b), (Expr::Time(_), Expr::Time(_))) => {
                    self.in_list = saved;
                    return Ok([*a, *b]);
                }
                _ => false,
            },
        };
        let b = self.range_end(through)?;
        self.in_list = saved;
        Ok([a, b])
    }

    /// Eats a word that joins a range; `Some(true)` for "through".
    pub(super) fn range_word(&mut self, words: &[&str]) -> Option<bool> {
        if self.eat_word("through") || self.eat_word("thru") {
            return Some(true);
        }
        words.iter().any(|w| self.eat_word(w)).then_some(false)
    }

    /// The end of a range; "through March 10" counts March 10 too, so it ends a day later,
    /// and "through Q4" ends with the quarter.
    pub(super) fn range_end(&mut self, through: bool) -> Result<Expr> {
        let end = self.additive()?;
        if !through || super::target::is_clock(&end) {
            return Ok(end);
        }
        let (end, length) = match end {
            Expr::Period(first, length) => (*first, *length),
            end => (end, Expr::WithUnit(Expr::Number(Number::ONE).boxed(), registry().get("d"))),
        };
        Ok(Expr::binary(Op::Add, end, length))
    }

    /// "difference between Seattle and Tokyo", "Seattle Tokyo", "Tokyo" (from here)
    /// or between two dates.
    fn difference(&mut self) -> Result<Expr> {
        // "here" is the local zone: "time difference between here and Tokyo".
        let place_at = |p: &Self, i: usize| match p.words_at(i, &["here"]) {
            true => Some((Place::of_zone(&p.config().local_zone()), 1)),
            false => p.zone_at(i),
        };
        if let Some((a, n)) = place_at(self, self.pos) {
            let and = self.toks.get(self.pos + n).is_some_and(|t| t.is_word("and") || t.is_sym("&"));
            let next = self.pos + n + usize::from(and);
            if let Some((b, m)) = place_at(self, next) {
                self.pos = next + m;
                return Ok(Expr::ZoneDiff(a, b));
            }
            if !and {
                self.pos = next;
                return Ok(Expr::ZoneDiff(Place::of_zone(&self.config().local_zone()), a));
            }
        }
        let [a, b] = self.pair()?;
        Ok(Expr::Range(a.boxed(), b.boxed()))
    }

    /// After a time unit: "days until christmas", "weeks since March 1".
    pub(super) fn time_unit_phrase(&mut self, unit: &Unit) -> Result<Option<Expr>> {
        let start = || now_for(&Expr::BareUnit(unit.clone()));
        let w = self.lower(self.pos).unwrap_or_default();
        // "days before March 5" counts the days, as "days until"; "day before" is a date.
        let plural = self.lower(self.pos.wrapping_sub(1)).is_some_and(|w| w.ends_with('s'));
        let range = match w.as_str() {
            "until" | "till" | "to" | "before" if (w != "to" && w != "before") || self.date_starts(self.pos + 1) => {
                if w == "before" && !plural {
                    return Ok(None);
                }
                self.pos += 1;
                Expr::SignedRange(start().boxed(), self.date_or_period(true)?.boxed())
            }
            "since" => {
                self.pos += 1;
                Expr::SignedRange(self.date_or_period(false)?.boxed(), start().boxed())
            }
            // "days in March 3 to May 30" is the days between them.
            "in" if self.range_follows(self.pos + 1) => {
                self.pos += 1;
                let [a, b] = self.pair()?;
                Expr::Range(a.boxed(), b.boxed())
            }
            "between" => {
                self.pos += 1;
                let [a, b] = self.pair()?;
                Expr::SignedRange(a.boxed(), b.boxed())
            }
            // "days from 2020 to 2025": years count from their first day.
            "from" if self.date_starts(self.pos + 1) || self.year_range_at(self.pos + 1) => {
                self.pos += 1;
                if self.year_range_at(self.pos) {
                    let a = self.date_or_period(false)?;
                    self.range_word(&["to", "until", "till"]);
                    return Ok(Some(Expr::Convert(
                        Expr::SignedRange(a.boxed(), self.date_or_period(true)?.boxed()).boxed(),
                        Target::Unit(unit.clone()),
                    )));
                }
                let a = self.additive()?;
                match self.range_word(&["to", "until", "till"]) {
                    Some(through) => Expr::SignedRange(a.boxed(), self.range_end(through)?.boxed()),
                    // "days from Jan 1" counts to now, as "since" does.
                    None => Expr::Range(a.boxed(), start().boxed()),
                }
            }
            // "days in February 2020", "days in Q3", "days left in 2026", "days left in the year"
            "in" | "left" | "remaining" => {
                let left = w != "in";
                let at = self.pos + if left { 2 } else { 1 };
                // "the month" and "next month" name a period, but "days in month" is the unit's;
                // what is left is always of this one: "days left in month".
                let the = usize::from(self.words_at(at, &["the"]));
                let named = the == 1 || left || self.lower(at).is_some_and(|w| words::which(&w).is_some());
                let mut period = if named { self.named_period_at(at + the) } else { self.period_at(at) };
                // "days in March 5, 2025": a day, not its month.
                if let Some((_, _, n)) = &period
                    && self.date_starts(at + the)
                {
                    let mut p = self.clone();
                    p.pos = at + the + n;
                    if !p.value_ends() {
                        p.pos = at + the;
                        let day = Expr::WithUnit(Expr::Number(Number::ONE).boxed(), registry().get("d"));
                        period = p.additive().ok().map(|date| (date, day, p.pos - at - the));
                    }
                }
                let Some((mut first, span, n)) = period.filter(|_| !left || self.words_at(at - 1, &["in"])) else {
                    return Ok(None);
                };
                let n = n + the;
                self.pos = at + n;
                let mut end = Expr::binary(Op::Add, first.clone(), span.clone());
                // "days left in Q3" after Q3: what is left of the next one, as "days until Q3" counts to it.
                if left && matches!(first, Expr::Time(TimeExpr::Date { year: None, .. })) {
                    let year = Expr::WithUnit(Expr::Number(Number::ONE).boxed(), registry().get("yr"));
                    let over = Expr::binary(Op::Le, end.clone(), start()).boxed();
                    let next = Expr::binary(Op::Add, first.clone(), year);
                    first = Expr::If { cond: over, then: Some(next.boxed()), otherwise: Some(first.boxed()) };
                    end = Expr::binary(Op::Add, first.clone(), span);
                }
                if left {
                    // What is left starts now, or when the period does; a past period has none.
                    let from = Expr::Call(Func::Max, vec![start(), first]);
                    let end = Expr::Call(Func::Max, vec![end, from.clone()]);
                    Expr::Range(from.boxed(), end.boxed())
                } else {
                    Expr::Range(first.boxed(), end.boxed())
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(Expr::Convert(range.boxed(), Target::Unit(unit.clone()))))
    }

    /// A date at the cursor, or the first day of a period: "March" in "days until March",
    /// "Q4", "2027"; ahead, "next month" is the 1st, as in SoulverCore ("since last month"
    /// is a month ago).
    fn date_or_period(&mut self, ahead: bool) -> Result<Expr> {
        let i = self.pos;
        let named = ahead
            && self.lower(i).is_some_and(|w| words::which(&w).is_some())
            && self.lower(i + 1).is_some_and(|w| matches!(w.as_str(), "week" | "month" | "quarter" | "year"));
        let period = if named { self.named_period_at(i) } else { self.period_at(i) };
        if let Some((first, _, n)) = period {
            let mut p = self.clone();
            p.pos = i + n;
            // Only the whole period: "March 3" is a date.
            if named || p.value_ends() {
                self.pos = i + n;
                return Ok(first);
            }
        }
        self.additive()
    }

    /// Two years at token `i`: "2020 to 2025".
    fn year_range_at(&self, i: usize) -> bool {
        let year = |k: usize| matches!(self.toks.get(k).map(|t| &t.tok), Some(Tok::Num(n)) if n.is_integer() && (1000..3000).contains(&n.to_i64().unwrap_or(0)));
        let to = ["to", "until", "till"].iter().any(|w| self.words_at(i + 1, &[w]));
        year(i) && to && year(i + 2) && i + 3 == self.toks.len()
    }

    /// A date, then a word that ends a range, from token `i`: "March 3 to May 30".
    fn range_follows(&self, i: usize) -> bool {
        if !self.date_starts(i) {
            return false;
        }
        let mut p = self.clone();
        p.pos = i;
        let saved = std::mem::replace(&mut p.in_list, true);
        let Ok(a) = p.additive() else { return false };
        p.in_list = saved;
        let dates = matches!(&a, Expr::Binary(Op::Sub, x, y) if matches!((&**x, &**y), (Expr::Time(_), Expr::Time(_))));
        dates || p.range_word(&["to", "until", "till"]).is_some()
    }

    /// A calendar period at token `i`: its first day, length and size in tokens.
    fn period_at(&self, i: usize) -> Option<(Expr, Expr, usize)> {
        let months = |n: i64| Expr::WithUnit(Expr::Number(Number::from_i64(n)).boxed(), registry().get("mo"));
        let mut p = self.clone();
        p.pos = i;
        // "2024" is the whole year.
        if let Some(year) = p.year() {
            let start = Expr::Time(TimeExpr::Date { year: Some(year as i16), month: 1, day: 1 });
            return Some((start, months(12), p.pos - i));
        }
        let w = self.lower(i)?;
        if let Some(quarter) = self.quarter_at(i, &w) {
            return Some(quarter);
        }
        let (month, _) = words::month(&w)?;
        p.pos = i + 1;
        let year = p.year().map(|y| y as i16);
        let start = Expr::Time(TimeExpr::Date { year, month, day: 1 });
        Some((start, months(1), p.pos - i))
    }

    /// "start of next month", "end of Q3", "beginning of the year": the first or last
    /// day of a period, and its length in tokens.
    pub(super) fn period_bound_at(&self, i: usize) -> Option<(Expr, usize)> {
        let last = match self.lower(i)?.as_str() {
            "start" | "beginning" => false,
            "end" => true,
            _ => return None,
        };
        if !self.words_at(i + 1, &["of"]) {
            return None;
        }
        let at = i + 2 + usize::from(self.words_at(i + 2, &["the"]));
        let (first, length, n) = self.named_period_at(at)?;
        let date = match last {
            true => {
                let day = Expr::WithUnit(Expr::Number(Number::ONE).boxed(), registry().get("d"));
                Expr::binary(Op::Sub, Expr::binary(Op::Add, first, length), day)
            }
            false => first,
        };
        Some((date, at + n - i))
    }

    /// A period at token `i`: "next month", "week" (as in "the week"), "the day", "this quarter", "Q3", "March", "2027".
    fn named_period_at(&self, i: usize) -> Option<(Expr, Expr, usize)> {
        let which = self.lower(i).and_then(|w| words::which(&w));
        let at = i + usize::from(which.is_some());
        let which = which.unwrap_or(Which::This);
        let (start, length) = match self.lower(at).as_deref() {
            Some("day") => (TimeExpr::Today(which.step()), (1, "d")),
            Some("week") => (TimeExpr::PeriodStart(jiff::Unit::Week, which), (1, "wk")),
            Some("month") => (TimeExpr::PeriodStart(jiff::Unit::Month, which), (1, "mo")),
            Some("year") => (TimeExpr::PeriodStart(jiff::Unit::Year, which), (1, "yr")),
            Some("quarter") => (TimeExpr::Quarter(which), (3, "mo")),
            _ => return self.period_at(i),
        };
        let length = Expr::WithUnit(Expr::Number(Number::from_i64(length.0)).boxed(), registry().get(length.1));
        Some((Expr::Time(start), length, at + 1 - i))
    }

    /// The quarter at token `i` is a value, not a label as in "Q2 sales 600" or "Q3 $10k".
    pub(super) fn quarter_starts(&self, i: usize) -> bool {
        let mut p = self.clone();
        p.pos = i + 1;
        match p.cur().map(|t| &t.tok) {
            Some(Tok::Num(_)) => p.year().is_some(),
            Some(Tok::Word(_)) => self.significant(i + 1) && self.unit_at(i + 1, false).is_none(),
            _ => true,
        }
    }

    /// A quarter or a season at token `i`, whose word is `w`: `Q3`, `Q3 2026`, `Q3 of 2026`,
    /// `next quarter`, `summer 2020`.
    fn quarter_at(&self, i: usize, w: &str) -> Option<(Expr, Expr, usize)> {
        let (first, n) = match (words::which(w), words::three_months(w)) {
            // "this quarters" too, as SoulverCore reads it.
            (Some(which), _) if self.words_at(i + 1, &["quarter"]) || self.words_at(i + 1, &["quarters"]) => {
                (TimeExpr::Quarter(which), 2)
            }
            // "spring festival" is a holiday.
            (_, Some(month)) if self.phrase_at(i, &words::HOLIDAYS).is_none() => {
                let mut p = self.clone();
                p.pos = i + 1 + usize::from(self.words_at(i + 1, &["of"]));
                let year = p.year().map(|y| y as i16);
                let n = if year.is_some() { p.pos - i } else { 1 };
                (TimeExpr::Date { year, month, day: 1 }, n)
            }
            _ => return None,
        };
        let length = Expr::WithUnit(Expr::Number(Number::from_i64(3)).boxed(), registry().get("mo"));
        Some((Expr::Time(first), length, n))
    }

    /// A date or time starts at token `i`.
    pub(super) fn date_starts(&self, i: usize) -> bool {
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
                    "now"
                        | "today"
                        | "tomorrow"
                        | "yesterday"
                        | "next"
                        | "last"
                        | "previous"
                        | "this"
                        | "noon"
                        | "midday"
                        | "midnight"
                ) || words::month(&w).is_some()
                    || words::three_months(&w).is_some()
                    || words::weekday(&w).is_some()
                    || p.phrase_at(p.pos, &words::HOLIDAYS).is_some()
                    || p.event_at(p.pos).is_some()
                    || p.var_at(p.pos).is_some()
            }
            _ => false,
        }
    }

    /// A time zone at token `i`: place, abbreviation or `UTC+5`.
    pub(super) fn zone_at(&self, i: usize) -> Option<(Place, usize)> {
        let (place, mut n) = match self.offset_zone_at(i) {
            Some((zone, n)) => (Place::offset(zone)?, n),
            None => self.named_place_at(i)?,
        };
        if self.toks.get(i + n).is_some_and(|t| t.is_word("time")) {
            n += 1;
        }
        Some((place, n))
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
