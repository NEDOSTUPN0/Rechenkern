//! Token helpers: looking ahead, skipping comment words, spotting units and names.

use jiff::tz::TimeZone;

use super::{Parser, words};
use crate::ast::Func;
use crate::custom::CustomPlace;
use crate::lexer::{Tok, Token};
use crate::number::Number;
use crate::units::foods::Food;
use crate::units::{Dim, Unit, registry};
use crate::zones::{self, Location, Place};

/// Words glued by a hyphen are somewhere in the tokens: "x-ray".
pub(super) fn has_hyphenated_words(toks: &[Token]) -> bool {
    toks.windows(3).any(|w| {
        w[0].word().is_some() && w[1].is_sym("-") && !w[1].space_before && w[2].word().is_some() && !w[2].space_before
    })
}

impl<'a> Parser<'a> {
    pub(super) fn cur(&self) -> Option<&'a Token> {
        self.toks.get(self.pos)
    }

    /// Skips words that mean nothing here, like "for lunch".
    pub(super) fn skip_noise(&mut self) {
        while self.cur().is_some_and(|t| t.word().is_some()) && !self.significant(self.pos) {
            self.pos += 1;
        }
    }

    /// The next meaningful token.
    pub(super) fn peek(&mut self) -> Option<&'a Token> {
        self.skip_noise();
        self.cur()
    }

    pub(super) fn lower(&self, i: usize) -> Option<String> {
        self.toks.get(i)?.word().map(str::to_lowercase)
    }

    /// Moves to the word `w` if only comments come before it.
    pub(super) fn at_word(&mut self, w: &str) -> bool {
        let mut i = self.pos;
        while let Some(t) = self.toks.get(i) {
            if t.is_word(w) {
                self.pos = i;
                return true;
            }
            if t.word().is_none() || self.significant(i) {
                return false;
            }
            i += 1;
        }
        false
    }

    pub(super) fn eat_word(&mut self, w: &str) -> bool {
        let found = self.at_word(w);
        if found {
            self.pos += 1;
        }
        found
    }

    pub(super) fn at_sym(&mut self, s: &str) -> bool {
        self.peek().is_some_and(|t| t.is_sym(s))
    }

    pub(super) fn eat_sym(&mut self, s: &str) -> bool {
        let found = self.at_sym(s);
        if found {
            self.pos += 1;
        }
        found
    }

    /// The raw tokens at `i` are exactly these words.
    pub(super) fn words_at(&self, i: usize, words: &[&str]) -> bool {
        words.iter().enumerate().all(|(k, w)| self.toks.get(i + k).is_some_and(|t| t.is_word(w)))
    }

    /// Skips the ending of an ordinal number: the "th" of "9th".
    pub(super) fn eat_ordinal(&mut self) -> bool {
        let ordinal = self.cur().and_then(Token::word).is_some_and(|w| matches!(w, "st" | "nd" | "rd" | "th"));
        if ordinal {
            self.pos += 1;
        }
        ordinal
    }

    /// Up to `max` consecutive words from `i`, joined by spaces (lowercase if
    /// asked); a hyphen between words stays, as in "light-year". Also where
    /// each word ends in the text, and the tokens it takes up to there.
    pub(super) fn joined_words(&self, i: usize, max: usize, lowercase: bool) -> (String, Vec<(usize, usize)>) {
        let (mut text, mut ends) = (String::new(), Vec::new());
        let mut k = i;
        while ends.len() < max {
            let Some(word) = self.toks.get(k).and_then(Token::word) else { break };
            if lowercase {
                text.push_str(&word.to_lowercase());
            } else {
                text.push_str(word);
            }
            k += 1;
            ends.push((text.len(), k - i));
            let glued = |t: &Token| !t.space_before;
            let hyphen = self.toks.get(k).is_some_and(|t| t.is_sym("-") && glued(t))
                && self.toks.get(k + 1).is_some_and(|t| t.word().is_some() && glued(t));
            if hyphen {
                text.push('-');
                k += 1;
            } else {
                text.push(' ');
            }
        }
        (text, ends)
    }

    /// Length in tokens of `phrase` if the tokens from `i` spell it; numbers
    /// count as written: "base 16", "iso 8601".
    pub(super) fn phrase_len_at(&self, i: usize, phrase: &str) -> Option<usize> {
        let (mut rest, mut n) = (phrase, 0);
        loop {
            let tok = self.toks.get(i + n)?;
            let word = match &tok.tok {
                Tok::Num(_) => &self.src[tok.start..tok.end],
                _ => tok.word()?,
            };
            if !rest.get(..word.len())?.eq_ignore_ascii_case(word) {
                return None;
            }
            n += 1;
            rest = &rest[word.len()..];
            if rest.is_empty() {
                return Some(n);
            }
            // "mid-point": a hyphen joins the words.
            if let Some(after) = rest.strip_prefix('-')
                && self.toks.get(i + n).is_some_and(|t| t.is_sym("-") && !t.space_before)
            {
                n += 1;
                rest = after;
                continue;
            }
            rest = rest.strip_prefix(' ')?;
        }
    }

    /// Longest phrase from `table` starting at token `i`.
    pub(super) fn phrase_at<T: Copy>(&self, i: usize, table: &words::Phrases<T>) -> Option<(T, usize)> {
        let word = self.toks.get(i)?.word()?;
        // "all-purpose flour": a word joined by hyphens is indexed by its first part.
        let first = word.split('-').next().unwrap_or(word);
        let mut best: Option<(T, usize)> = None;
        for &(phrase, value) in table.starting_with(first) {
            if let Some(n) = self.phrase_len_at(i, phrase)
                && best.is_none_or(|(_, m)| n > m)
            {
                best = Some((value, n));
            }
        }
        best
    }

    /// Whether the token at `i` is a word worth parsing.
    pub(super) fn significant(&self, i: usize) -> bool {
        let Some(memo) = self.memo.get(i) else { return false };
        memo.significant.get().unwrap_or_else(|| {
            let significant = self.find_significance(i);
            memo.significant.set(Some(significant));
            significant
        })
    }

    fn find_significance(&self, i: usize) -> bool {
        let Some(tok) = self.toks.get(i) else { return false };
        let Some(word) = tok.word() else { return true };
        let w = word.to_lowercase();
        let w = w.as_str();
        words::KEYWORDS.contains(&w)
            // "3 x^2" is no product: the x is a comment there.
            || (w == "x" && !self.toks.get(i + 1).is_some_and(|t| t.is_sym("^")))
            || self.article_unit_at(i)
            // "for 2 hours" and "split 4 ways" only matter before an amount.
            || (w == "for" && (self.duration_at(i + 1) || self.pace_distance_at(i)))
            // "once a week", "thrice per day"
            || (matches!(w, "once" | "thrice") && (self.article_unit_at(i + 1) || self.per_unit_at(i + 1)))
            || (matches!(w, "every" | "each") && self.counted_unit_at(i + 1))
            // "daily", "monthly until June": a cadence, as a whole line.
            || (i == 0 && words::is_cadence_word(w) && self.config().features.cadence_parsing && self.var_at(0).is_none())
            // "not true", "not (5 > 3)", "not 5 > 3": logic, not a comment.
            || (w == "not" && self.not_operand_at(i + 1))
            || (w == "split" && matches!(self.toks.get(i + 1).map(|t| &t.tok), Some(Tok::Num(_))))
            || (w == "out" && self.words_at(i + 1, &["of"]))
            || (w == "cast" && ["to", "as", "into", "in"].iter().any(|t| self.words_at(i + 1, &[t])))
            || (w == "diff" && (self.words_at(i + 1, &["between"]) || self.zone_at(i + 1).is_some()))
            // "zł10", "CHF10": a currency glued before its amount.
            // "zł10", "apr3": letters with digits glued after them.
            || (word.ends_with(|c: char| c.is_ascii_digit()) && (self.glued_money(i).is_some() || self.glued_date(i).is_some()))
            // "first monday of the month", "first day in March"
            || (self.nth_at(i).is_some_and(|(_, n)| n == 1)
                && (self.day_in_month_at(i + 1).is_some() || (w == "first" && self.weekday_ends_at(i + 1))))
            // "flour 1 cup": a food before its amount.
            || (matches!(self.toks.get(i + 1).map(|t| &t.tok), Some(Tok::Num(_)))
                && self.unit_at(i + 2, false).is_some_and(|(u, _)| matches!(u.dim(), Dim::MASS | Dim::VOLUME))
                && self.food_at(i).is_some_and(|(_, n)| n == 1))
            // "sq ft", "cu m"
            || (matches!(w, "sq" | "cu") && self.unit_at(i + 1, false).is_some())
            || (matches!(w, "through" | "thru") && self.date_starts(i + 1))
            || (matches!(w, "start" | "beginning" | "end") && self.period_bound_at(i).is_some())
            // "round 56 down": a direction only after "round".
            || (matches!(w, "up" | "down") && self.toks[..i].iter().any(|t| t.is_word("round")))
            // "cot(1)" is a call even if the function is unknown.
            || self.toks.get(i + 1).is_some_and(|t| t.is_sym("(") && !t.space_before)
            || words::number_word(w).is_some()
            || words::scale_word(word).is_some()
            || words::fraction_word(w).is_some()
            || (words::ordinal_word(w).is_some() && self.words_at(i + 1, &["root"]))
            || words::constant(w).is_some()
            || words::month(w).is_some()
            || (words::three_months(w).is_some() && self.quarter_starts(i))
            || words::weekday(w).is_some()
            || words::is_line_word(w)
            || self.function_at(i).is_some()
            || (self.scope.is_function)(w)
            || self.phrase_at(i, &words::HOLIDAYS).is_some()
            || self.event_at(i).is_some()
            || self.named_day_at(i).is_some()
            || self.phrase_at(i, &words::DATE_PARTS).is_some()
            || self.phrase_at(i, &words::PHYSICAL_CONSTANTS).is_some()
            || self.unit_at(i, false).is_some()
            || self.var_at(i).is_some()
            || self.place_time_at(i).is_some()
            || self.place_converted_at(i).is_some()
            || self.sky_at(i).is_some()
            || self.weather_at(i).is_some()
            || self.distance_at(i).is_some()
            || self.coordinates_at(i).is_some()
            || self.geo_of_number_at(i).is_some()
            || self.metal_price_at(i).is_some()
            || self.note_at(i).is_some()
            || self.place_sky_at(i).is_some()
            || self.density_at(i).is_some()
    }

    /// A month and a day written as one word: "apr3", "dec25".
    pub(super) fn glued_date(&self, i: usize) -> Option<(i8, i8)> {
        let word = self.toks.get(i)?.word()?;
        let split = word.find(|c: char| c.is_ascii_digit()).filter(|&s| s >= 3)?;
        let (name, digits) = word.split_at(split);
        let (month, _) = words::month(&name.to_lowercase())?;
        let day = digits.parse::<i8>().ok().filter(|d| (1..=31).contains(d) && digits.len() <= 2)?;
        Some((month, day))
    }

    /// A currency and an amount written as one word: "zł10", "CHF10", "Kč250".
    pub(super) fn glued_money(&self, i: usize) -> Option<(Unit, Number)> {
        let word = self.toks.get(i)?.word()?;
        let split = word.find(|c: char| c.is_ascii_digit()).filter(|&s| s > 0)?;
        let (symbol, digits) = word.split_at(split);
        let amount = Number::parse(digits)?;
        let unit = self.lookup_unit(symbol).filter(Unit::is_money)?;
        Some((unit, amount))
    }

    /// A food named at token `i` and its length in tokens: "olive oil".
    pub(super) fn food_at(&self, i: usize) -> Option<(Food, usize)> {
        // "half & half" is the cream, not a bitwise and.
        if self.words_at(i, &["half"])
            && self.toks.get(i + 1).is_some_and(|t| t.is_sym("&"))
            && self.words_at(i + 2, &["half"])
        {
            return words::FOODS.starting_with("half").find(|(name, _)| *name == "half and half").map(|&(_, f)| (f, 3));
        }
        self.phrase_at(i, &words::FOODS)
    }

    /// "density of butter" or "butter density" at token `i`: the food and the phrase's length.
    pub(super) fn density_at(&self, i: usize) -> Option<(Food, usize)> {
        if self.words_at(i, &["density"]) {
            let of = usize::from(self.words_at(i + 1, &["of"]));
            let (food, n) = self.food_at(i + 1 + of)?;
            return Some((food, 1 + of + n));
        }
        // Most lines don't say "density": skip looking up every word as a food.
        if !self.toks[i..].iter().any(|t| t.is_word("density")) {
            return None;
        }
        let (food, n) = self.food_at(i)?;
        self.words_at(i + n, &["density"]).then_some((food, n + 1))
    }

    /// The token at `i` is a word that means nothing here.
    pub(super) fn comment_at(&self, i: usize) -> bool {
        self.toks.get(i).is_some_and(|t| t.word().is_some()) && !self.significant(i)
    }

    /// A unit spelled at token `i`: its value and length in tokens.
    pub(super) fn unit_at(&self, i: usize, after_number: bool) -> Option<(Unit, usize)> {
        let tok = self.toks.get(i)?;
        if tok.is_sym("°") {
            // "°C" or a lone degree sign.
            if let Some(next) = self.toks.get(i + 1).filter(|t| !t.space_before).and_then(Token::word)
                && let Some(u) = self.lookup_unit(&format!("°{next}"))
            {
                return Some((u, 2));
            }
            // "° celsius"
            if let Some(next) = self.toks.get(i + 1).and_then(Token::word).filter(|w| w.len() > 1)
                && let Some(u) = self.lookup_unit(next).filter(|u| u.dim() == Dim::TEMPERATURE)
            {
                return Some((u, 2));
            }
            return Some((registry().get("°"), 1));
        }
        let word = tok.word()?;
        if word == "$" {
            // "$AUD": the code says which dollar.
            if let Some(next) = self.toks.get(i + 1).filter(|t| !t.space_before).and_then(Token::word)
                && let Some(u) = self.lookup_unit(next).filter(Unit::is_money)
            {
                return Some((u, 2));
            }
            return Some((self.dollar(), 1));
        }
        // "A" is an article before a word ("A coffee"), amperes otherwise ("5 V/A").
        let article = matches!(word, "a" | "an")
            || (word == "A" && self.toks.get(i + 1).is_some_and(|t| t.word().is_some() && t.space_before));
        // Units that are also words count only after a number, a slash, a product dot or
        // in a target: "there are 5", "PS: 5", but "$5/are", "kcal·PS".
        let word_unit = word.eq_ignore_ascii_case("are") || word.eq_ignore_ascii_case("ares") || word == "PS";
        let after_slash = i > 0 && (self.toks[i - 1].is_sym("/") || self.toks[i - 1].is_sym("*"));
        // "ares in km²": a unit alone starting a conversion.
        let converted = i == 0 && ["in", "to"].iter().any(|w| self.words_at(1, &[w]));
        if word.eq_ignore_ascii_case("in")
            || (!after_number && (article || (word_unit && !after_slash && !(converted && word != "PS"))))
        {
            return None;
        }
        // "3 weekdays" are workdays, but "today as weekday" is the day of the week.
        if after_number && (word.eq_ignore_ascii_case("weekday") || word.eq_ignore_ascii_case("weekdays")) {
            return Some((registry().get("workday"), 1));
        }
        // "5 H", "5 S", "5 T": hours, seconds and tonnes after a number, as in SoulverCore.
        if after_number && let Some(name) = [("H", "h"), ("S", "s"), ("T", "t")].iter().find(|(w, _)| *w == word) {
            return Some((registry().get(name.1), 1));
        }
        // "min(" is a function.
        if self.toks.get(i + 1).is_some_and(|t| t.is_sym("(") && !t.space_before)
            && self.phrase_at(i, &words::FUNCTIONS).is_some()
        {
            return None;
        }
        let (text, ends) = self.joined_words(i, registry().max_words().max(self.scope.units.max_words), false);
        let found = ends
            .iter()
            .rev()
            .find_map(|&(end, n)| Some((self.lookup_unit(&text[..end])?, n)))
            .or_else(|| self.time_power_at(i));
        // "100 euros in pounds", "10 pounds + $5": pounds converted or added with money are sterling.
        match found {
            Some((unit, 1)) if is_pound_word(word) && self.pounds_are_money(i) => {
                Some((registry().currency("GBP").unwrap_or(unit), 1))
            }
            found => found,
        }
    }

    /// The pounds at token `i` are converted to or from money, or added to it.
    fn pounds_are_money(&self, i: usize) -> bool {
        let conversion = |t: Option<&Token>| t.is_some_and(|t| ["in", "to", "as", "into"].iter().any(|w| t.is_word(w)));
        let sum = |t: Option<&Token>| {
            t.is_some_and(|t| t.is_sym("+") || t.is_sym("-") || t.is_word("plus") || t.is_word("minus"))
        };
        let money_at = |k: usize| {
            self.toks[k].word().is_none_or(|w| !is_pound_word(w))
                && (self.unit_at(k, true).is_some_and(|(u, _)| u.is_money()) || self.glued_money(k).is_some())
        };
        let before = i.checked_sub(1).and_then(|k| self.toks.get(k));
        let (next, after) = (self.toks.get(i + 1), self.toks.get(i + 2));
        // "€100 in pounds"
        (conversion(before) && (0..i).any(money_at))
            // "10 pounds in euros"
            || (conversion(next) && after.is_some() && money_at(i + 2))
            // "10 pounds + $5", "$5 + 10 pounds"
            || ((sum(next) || sum(i.checked_sub(2).and_then(|k| self.toks.get(k))))
                && (0..self.toks.len()).any(|k| k != i && money_at(k)))
    }

    /// A time with a power after a slash: the "s2" of "m/s2".
    fn time_power_at(&self, i: usize) -> Option<(Unit, usize)> {
        if !self.toks.get(i.checked_sub(1)?).is_some_and(|t| t.is_sym("/")) {
            return None;
        }
        let word = self.toks.get(i)?.word()?;
        let exp = match word.chars().last()? {
            '2' => 2,
            '3' => 3,
            _ => return None,
        };
        let unit = self.lookup_unit(&word[..word.len() - 1]).filter(|u| u.dim() == Dim::TIME)?;
        Some((unit.pow(exp), 1))
    }

    /// A unit spelled `text`: the host's own units first, then built-in ones as
    /// the unit system reads them.
    pub(super) fn lookup_unit(&self, text: &str) -> Option<Unit> {
        let declared = || match self.scope.declared.is_empty() {
            true => None,
            false => self.scope.declared.get(&text.to_lowercase()).cloned(),
        };
        let unit = match self.scope.units.lookup(text).or_else(declared) {
            Some(unit) => unit,
            None => self.config().unit_system.adapt(text, registry().lookup(text)?),
        };
        // Without units only money is left.
        (self.config().features.units || unit.is_money()).then_some(unit)
    }

    /// A function named at token `i`, unless the config leaves it out or wants parentheses.
    pub(super) fn function_at(&self, i: usize) -> Option<(Func, usize)> {
        let (func, n) = self.phrase_at(i, &words::FUNCTIONS)?;
        let features = &self.config().features;
        if features.functions_require_parentheses && !self.toks.get(i + n).is_some_and(|t| t.is_sym("(")) {
            return None;
        }
        let disabled = features.disabled_functions.iter().any(|name| words::function_named(name) == Some(func));
        (!disabled).then_some((func, n))
    }

    pub(super) fn dollar(&self) -> Unit {
        registry().currency(&self.config().dollar).unwrap_or_else(|| registry().get("USD"))
    }

    /// A variable name made of the words at `i`.
    pub(super) fn var_at(&self, i: usize) -> Option<(String, usize)> {
        let (text, ends) = self.joined_words(i, 6, true);
        ends.iter().rev().find_map(|&(end, n)| {
            let name = &text[..end];
            (self.scope.is_var)(name).then(|| (name.to_string(), n))
        })
    }

    /// A place name at token `i`; words may be joined by hyphens: "Aix-en-Provence".
    pub(super) fn place_at(&self, i: usize) -> Option<(TimeZone, usize)> {
        match self.memo.get(i) {
            Some(memo) => memo.place.get_or_init(|| self.find_place(i)).clone(),
            None => self.find_place(i),
        }
    }

    fn find_place(&self, i: usize) -> Option<(TimeZone, usize)> {
        self.place_name_at(i).map(|(_, zone, n)| (zone, n))
    }

    /// The longest place name at token `i` (lowercase), its zone and its length in tokens.
    fn place_name_at(&self, i: usize) -> Option<(String, TimeZone, usize)> {
        let (words, ends) = self.name_words(i);
        let mut text = words.join(" ");
        let options = &self.config().features.time_zones;
        for n in (1..=words.len()).rev() {
            let custom = || self.custom_place(&text).map(|p| p.zone.clone());
            // "pst" is a zone only in capitals, if the config asks for them; "man" is never an airport.
            let lowercase = || self.toks[i].word().is_some_and(|w| w.chars().any(char::is_lowercase));
            let lowercase_code = lowercase()
                && ((options.require_uppercase_for_ambiguous_time_zones && zones::is_code(&text))
                    || zones::capitals_only(&text));
            if !lowercase_code && let Some(zone) = custom().or_else(|| zones::find(&text)) {
                let zone = zones::corrected(&text, zone, options.code_correction);
                return Some((text, zone, ends[n - 1] - i));
            }
            // "Curaçao" as "Curacao", when only the plain spelling is listed.
            if let Some(plain) = zones::without_diacritics(&text)
                && let Some(zone) = zones::find(&plain)
            {
                return Some((plain, zone, ends[n - 1] - i));
            }
            // "WAW" is the airport, "Waw" the town.
            if lowercase_code
                && zones::capitals_only(&text)
                && let Some(zone) = zones::find_city(&text)
            {
                return Some((text, zone, ends[n - 1] - i));
            }
            text.truncate(text.len().saturating_sub(words[n - 1].len() + 1));
        }
        // "japanese time", "time in kazakh": a country by its adjective, only with "time".
        let with_time = self.words_at(i + 1, &["time"]) || (i >= 2 && self.words_at(i - 2, &["time", "in"]));
        let country = with_time.then(|| self.lower(i).and_then(|w| words::demonym(&w))).flatten()?;
        Some((country.to_string(), zones::find(country)?, 1))
    }

    /// Where the place at token `i` is, for the sun, and its length in tokens.
    /// A region has a zone but no single place: "Texas" gives `(None, 1)`.
    pub(super) fn location_at(&self, i: usize) -> Option<(Option<Location>, usize)> {
        let (name, _, n) = self.place_name_at(i)?;
        let location = match self.custom_place(&name) {
            Some(place) => place.coordinates.map(|(latitude, longitude)| Location {
                zone: place.zone.clone(),
                latitude,
                longitude,
            }),
            // "waw" is the town, "WAW" the airport.
            None if self.toks[i].word().is_some_and(|w| w.chars().any(char::is_lowercase))
                && zones::capitals_only(&name) =>
            {
                zones::locate_city(&name)
            }
            None => zones::locate(&name),
        };
        Some((location, n))
    }

    /// The host's place of this name (lowercase).
    fn custom_place(&self, name: &str) -> Option<&CustomPlace> {
        self.config().custom_places.iter().find(|p| p.is_named(name))
    }

    /// Up to `MAX_WORDS` lowercase words from token `i`, and the token index after each.
    pub(super) fn name_words(&self, i: usize) -> (Vec<String>, Vec<usize>) {
        let (mut words, mut ends) = (Vec::new(), Vec::new());
        let mut k = i;
        while words.len() < zones::MAX_WORDS {
            let Some(word) = self.toks.get(k).and_then(Token::word) else { break };
            words.push(word.to_lowercase());
            k += 1;
            ends.push(k);
            let hyphen = self.toks.get(k).is_some_and(|t| t.is_sym("-") && !t.space_before);
            if hyphen && self.toks.get(k + 1).is_some_and(|t| t.word().is_some() && !t.space_before) {
                k += 1;
            }
        }
        (words, ends)
    }

    /// The place at token `i` with its name as written.
    pub(super) fn named_place_at(&self, i: usize) -> Option<(Place, usize)> {
        let (zone, n) = self.place_at(i)?;
        let written = &self.src[self.toks[i].start..self.toks[i + n - 1].end];
        if let Some(custom) = self.custom_place(&written.replace('-', " ").to_lowercase()) {
            return Some((Place { name: custom.name.clone(), zone, kind: custom.kind }, n));
        }
        Some((Place::written(written, zone), n))
    }

    /// "Tokyo time", "Tokyo local time" or "Paris date".
    pub(super) fn place_time_at(&self, i: usize) -> Option<(Place, usize, bool)> {
        // "GMT+3 time", "Hawaii time" (a place of its own name).
        let (tz, mut n) = self.zone_at(i)?;
        if self.toks[i + n - 1].is_word("time") {
            return Some((tz, n, false));
        }
        if self.words_at(i + n, &["local"]) {
            n += 1;
        }
        let next = self.lower(i + n)?;
        matches!(next.as_str(), "time" | "date").then_some((tz, n + 1, next == "date"))
    }

    /// A place followed by a zone conversion: "PST to EST", "Tokyo in London".
    pub(super) fn place_converted_at(&self, i: usize) -> Option<(Place, usize)> {
        let (tz, n) = self.named_place_at(i)?;
        let next = self.toks.get(i + n)?;
        let is_keyword = next.is_sym("->") || ["in", "to", "into"].iter().any(|w| next.is_word(w));
        (is_keyword && self.zone_at(i + n + 1).is_some()).then_some((tz, n))
    }

    /// An operand starts after skipping `skip` tokens.
    pub(super) fn operand_follows(&self, skip: usize) -> bool {
        let mut p = self.clone();
        p.pos += skip;
        let Some(tok) = p.peek() else { return false };
        match &tok.tok {
            Tok::Num(_) | Tok::Clock { .. } | Tok::Date { .. } | Tok::IsoDateTime(_) | Tok::Timecode(_) => true,
            // "°" is a unit: "fl oz·°F", "ct·°".
            Tok::Sym(s) => matches!(*s, "(" | "-" | "+" | "√" | "∛" | "°"),
            Tok::Word(w) => {
                let w = w.to_lowercase();
                !matches!(
                    w.as_str(),
                    "in" | "to"
                        | "as"
                        | "into"
                        | "of"
                        | "on"
                        | "off"
                        | "at"
                        | "per"
                        | "and"
                        | "or"
                        | "xor"
                        | "mod"
                        | "ago"
                        | "times"
                        | "plus"
                )
            }
        }
    }

    /// A number or an amount of money starts `skip` tokens ahead.
    pub(super) fn amount_follows(&self, skip: usize) -> bool {
        let mut p = self.clone();
        p.pos += skip;
        match p.peek().map(|t| &t.tok) {
            Some(Tok::Num(_)) => true,
            Some(Tok::Word(_)) => p.unit_at(p.pos, false).is_some_and(|(u, n)| {
                u.is_money() && matches!(p.toks.get(p.pos + n).map(|t| &t.tok), Some(Tok::Num(_)))
            }),
            _ => false,
        }
    }

    /// "a day", "each month": an article and a unit at token `i`, but not an amount
    /// of money: "each $50".
    pub(super) fn article_unit_at(&self, i: usize) -> bool {
        self.toks.get(i).is_some_and(|t| ["a", "an", "each", "every"].iter().any(|w| t.is_word(w)))
            && self.unit_at(i + 1, false).is_some_and(|(u, n)| {
                !(u.is_money() && self.toks.get(i + 1 + n).is_some_and(|t| matches!(t.tok, Tok::Num(_))))
            })
    }

    /// "per" and a unit at token `i`: "per week".
    pub(super) fn per_unit_at(&self, i: usize) -> bool {
        self.words_at(i, &["per"]) && self.unit_at(i + 1, false).is_some()
    }

    /// A number and a unit at token `i`: "2 weeks", "two weeks", "100 km".
    pub(super) fn counted_unit_at(&self, i: usize) -> bool {
        let number = self.toks.get(i).is_some_and(|t| matches!(t.tok, Tok::Num(_)))
            || self.lower(i).is_some_and(|w| words::number_word(&w).is_some());
        number && self.unit_at(i + 1, true).is_some()
    }

    /// A time amount starts at token `i`: "2 hours", "a year".
    /// A boolean follows "not" at `i`: `true`, `false`, parentheses or a comparison.
    pub(super) fn not_operand_at(&self, i: usize) -> bool {
        let Some(t) = self.toks.get(i) else { return false };
        if t.is_word("true") || t.is_word("false") || t.is_sym("(") {
            return true;
        }
        let mut p = self.clone();
        p.pos = i;
        p.additive().is_ok()
            && p.cur().is_some_and(|t| matches!(t.tok, Tok::Sym("==" | "!=" | "<" | ">" | "<=" | ">=")))
    }

    /// "5:30 min/km for 10 km": a distance after a time per a length, at the "for" at `i`.
    pub(super) fn pace_distance_at(&self, i: usize) -> bool {
        let length = |k: usize| self.unit_at(k, true).is_some_and(|(u, _)| u.dim() == crate::units::Dim::LENGTH);
        let per = i >= 2 && (self.toks[i - 2].is_sym("/") || self.toks[i - 2].is_word("per"));
        per && length(i - 1) && self.toks.get(i + 1).is_some_and(|t| matches!(t.tok, Tok::Num(_))) && length(i + 2)
    }

    pub(super) fn duration_at(&self, i: usize) -> bool {
        let amount = self
            .toks
            .get(i)
            .is_some_and(|t| matches!(t.tok, Tok::Num(_)) || ["a", "an", "one"].iter().any(|w| t.is_word(w)));
        amount && self.unit_at(i + 1, true).is_some_and(|(u, _)| u.dim() == crate::units::Dim::TIME)
    }

    /// The `%` at the cursor is modulo: `10 % 3`, but not `10% + 5`, `10%3` or `10% coffee 5`.
    pub(super) fn modulo_follows(&self) -> bool {
        let Some(next) = self.toks.get(self.pos + 1) else { return false };
        let sign = next.is_sym("+") || next.is_sym("-");
        let comment = next.word().is_some() && !self.significant(self.pos + 1);
        // "10% from 200", "5% of 20": a percentage before its operator.
        let operator = ["from", "of", "on", "off", "after", "before"].iter().any(|w| next.is_word(w));
        next.space_before && !sign && !comment && !operator && self.operand_follows(1)
    }

    /// Something that multiplies without an operator follows: "(", a function,
    /// a constant or a variable. Not after a comment: "5 cats (3)" is two values.
    pub(super) fn implicit_mul_follows(&mut self) -> bool {
        let Some(tok) = self.peek() else { return false };
        let comment_before =
            self.pos > 0 && self.toks[self.pos - 1].word().is_some() && !self.significant(self.pos - 1);
        if comment_before {
            return false;
        }
        if tok.is_sym("(") {
            return true;
        }
        let Some(w) = self.lower(self.pos) else { return false };
        words::constant(&w).is_some()
            || self.var_at(self.pos).is_some()
            // "3 gold price": three ounces.
            || self.metal_price_at(self.pos).is_some()
            || (self.function_at(self.pos).is_some_and(|(f, _)| !matches!(f, Func::Sum | Func::Average | Func::Count))
                && self.toks.get(self.pos + 1).is_some_and(|t| t.is_sym("(")))
    }

    /// Tokens that start the line without being part of it: a list marker and a
    /// label such as "Day 8", as the config asks.
    pub(super) fn leading_labels(&self) -> usize {
        let features = &self.config().features;
        let marker = if features.list_marker_parsing { self.list_marker() } else { 0 };
        marker + if features.ordinal_label_parsing { self.ordinal_label(marker) } else { 0 }
    }

    /// "Day 8", "Week 4", "mo 28" at token `i` before more of the line: a unit and a whole number.
    fn ordinal_label(&self, i: usize) -> usize {
        let unit = self.unit_at(i, false).filter(|&(ref u, n)| n == 1 && u.dim() == Dim::TIME);
        let number = self.toks.get(i + 1).is_some_and(|t| matches!(&t.tok, Tok::Num(n) if n.is_integer()));
        let more = self.toks.len() > i + 2;
        if unit.is_some() && number && more { 2 } else { 0 }
    }

    /// Tokens of a list marker that starts the line: "1. ", "2) ", "a) ", "- ", "* ".
    /// "1. May" stays a date, "- 5" a negative number.
    pub(super) fn list_marker(&self) -> usize {
        let Some(first) = self.toks.first() else { return 0 };
        // "- Zenvoice: $25", "* pears $4": a bullet before words, but "- $10" and "- pi" are negative.
        let bullet = first.is_sym("-") || first.is_sym("*");
        if bullet && self.toks.get(1).is_some_and(|t| t.space_before && t.word().is_some()) && !self.significant(1) {
            return 1;
        }
        let text = &self.src[first.start..first.end];
        let is_label = match first.tok {
            Tok::Num(_) => text.len() <= 3 && text.bytes().all(|b| b.is_ascii_digit()),
            Tok::Word(_) => text.chars().count() == 1 && text.chars().all(char::is_alphabetic),
            _ => false,
        };
        let rest = &self.src[first.end..];
        let Some(after) = [".)", ".", ")"].iter().find_map(|m| rest.strip_prefix(m)) else { return 0 };
        if !is_label || !after.starts_with(char::is_whitespace) || self.date_starts(0) {
            return 0;
        }
        if self.toks.get(1).is_some_and(|t| t.is_sym(")")) { 2 } else { 1 }
    }

    /// Joins words glued by hyphens into one comment word: "half-life", "x-ray",
    /// "T-shirt". Units, places and functions stay ("light-year", "Aix-en-Provence",
    /// "mid-point"), and so does a minus between values ("pi-e", "twenty-five").
    pub(super) fn with_hyphenated_words(&self) -> Vec<Token> {
        let mut out = Vec::with_capacity(self.toks.len());
        let mut i = 0;
        while i < self.toks.len() {
            let end = self.hyphen_chain_end(i);
            if end > i + 1 && self.hyphenated_comment(i, end) {
                let (first, last) = (&self.toks[i], &self.toks[end - 1]);
                let word = self.src[first.start..last.end].to_string();
                out.push(Token { tok: Tok::Word(word), start: first.start, end: last.end, ..first.clone() });
            } else {
                out.extend_from_slice(&self.toks[i..end]);
            }
            i = end;
        }
        out
    }

    /// Where words glued by hyphens from token `i` end: "x-ray" takes three tokens.
    fn hyphen_chain_end(&self, i: usize) -> usize {
        let glued = |k: usize, sym: bool| {
            self.toks.get(k).is_some_and(|t| !t.space_before && if sym { t.is_sym("-") } else { t.word().is_some() })
        };
        if self.toks.get(i).and_then(Token::word).is_none() {
            return i + 1;
        }
        let mut k = i;
        while glued(k + 1, true) && glued(k + 2, false) {
            k += 2;
        }
        k + 1
    }

    /// The hyphenated words in tokens `i..end` mean nothing together.
    fn hyphenated_comment(&self, i: usize, end: usize) -> bool {
        let n = end - i;
        let known = self.unit_at(i, false).is_some_and(|(_, m)| m >= n)
            || self.place_at(i).is_some_and(|(_, m)| m >= n)
            || self.phrase_at(i, &words::FUNCTIONS).is_some_and(|(_, m)| m >= n);
        let value = |k: usize| {
            let w = self.lower(k).unwrap_or_default();
            words::constant(&w).is_some() || words::number_word(&w).is_some() || self.var_at(k).is_some()
        };
        !known && !(i..end).step_by(2).all(value)
    }

    /// Removes parentheses that hold comments: "$999 (for iPhone 16)".
    pub(super) fn without_comment_groups(&self) -> Vec<Token> {
        // "area(w, h) = w * h" names parameters, and its body may use them in calls.
        let definition = self.toks.first().is_some_and(|t| t.word().is_some())
            && self.toks.get(1).is_some_and(|t| t.is_sym("(") && !t.space_before)
            && self
                .toks
                .iter()
                .position(|t| t.is_sym(")"))
                .is_some_and(|c| self.toks.get(c + 1).is_some_and(|t| t.is_sym("=")));
        if !self.config().features.comments.brackets || definition {
            return self.toks.to_vec();
        }
        let mut keep = vec![true; self.toks.len()];
        let mut i = 0;
        while i < self.toks.len() {
            if self.toks[i].is_sym("(") {
                let mut depth = 0;
                let close = (i..self.toks.len()).find(|&k| {
                    match self.toks[k].tok {
                        Tok::Sym("(") => depth += 1,
                        Tok::Sym(")") => depth -= 1,
                        _ => {}
                    }
                    depth == 0
                });
                if let Some(close) = close {
                    let comment = (i + 1..close)
                        .any(|k| {
                            self.toks[k].word().is_some()
                                && !self.significant(k)
                                && self.place_at(k).is_none()
                                // "(1 cup of flour) in grams"
                                && !(self.food_at(k).is_some() && self.toks[i + 1..k].iter().any(|t| matches!(t.tok, Tok::Num(_))))
                        });
                    if comment {
                        keep[i..=close].iter_mut().for_each(|k| *k = false);
                        i = close;
                    }
                }
            }
            i += 1;
        }
        self.toks.iter().zip(keep).filter(|(_, k)| *k).map(|(t, _)| t.clone()).collect()
    }
}

/// "pound" or "pounds", which are sterling next to money.
fn is_pound_word(word: &str) -> bool {
    word.eq_ignore_ascii_case("pound") || word.eq_ignore_ascii_case("pounds")
}
