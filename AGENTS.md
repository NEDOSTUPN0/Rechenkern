# AGENTS.md

Notes for anyone (human or agent) working on Rechenkern, an open natural
language calculator engine in the spirit of SoulverCore.

## Build and test

- NixOS: the toolchain comes from the flake dev shell. Run everything through
  it: `nix develop -c cargo test`, `nix develop -c cargo clippy --all-targets`,
  `nix develop -c cargo fmt`.
- `nix build` / `nix run . -- "5 km in miles"` build the CLI package (runs the
  tests too, offline, with jiff's bundled tzdb).
- Keep `cargo clippy --all-targets` warning free and run `cargo fmt`
  (`rustfmt.toml`: width 120). Data tables may use `#[rustfmt::skip]`.
- `crates/rechenkern/tests/calculations.rs` is the main test suite: input line
  -> expected answer text, with a fixed "now" (Wed 23 Sep 2026 14:30 New York)
  and fixed exchange rates. Every new phrase or fix gets a case there.
- `src/tests/` sweeps the whole engine, at the same "now" and rates (every
  other currency is 2 to the dollar):
  - `snapshots.rs`: thousands of lines and their answers in `src/tests/snapshots/`,
    from every unit, function, format, holiday and place, plus hand-written
    lines in `src/tests/corpus/*.txt` (and sheets in `corpus/sheets/`). Any
    changed answer fails the test: review the diff with `cargo insta review`
    (or `INSTA_UPDATE=always cargo test` to accept all) and read it — an
    unexpected change is a bug. Lines worth keeping an eye on go in the corpus.
  - `properties.rs`: proptest checks random lines: no panics, answers read back
    as the same value, answers that never read the clock don't change with it,
    both decimal styles agree, a line answers the same after other lines unless
    it reads them, unit conversions round-trip. The seed is fixed; look further
    with `PROPTEST_RNG_SEED=$RANDOM PROPTEST_CASES=100000 cargo test --release --lib properties`.
    A failure is shrunk to the shortest line: fix it, or exclude the case in the
    property with a comment saying why.
  - Test-only hooks: `tests::CLOCK_READS`/`SHEET_READS` count reads of the clock
    and of earlier lines; `random()` is always 0.5 in these tests.
- Try things by hand: `cargo run -q -- "10 usd to eur"`, pipe a file into
  `cargo run -q -- -a` for sheet mode.
- `nix develop -c cargo bench` times every line of `benches/lines.txt` and the
  sheet `benches/sheet.txt`; `benches/soulver.swift` times SoulverCore on the
  same input. Check it after touching the parser.

## Layout

- `crates/rechenkern` — the library. `Calculator` is the entry point.
- `crates/rechenkern-cli` — the `rechenkern` binary: one expression from
  args, a sheet from stdin/`--file`, or a prompt (rustyline).

Pipeline for one line (`calculator.rs`):
label strip -> `lexer` -> `parser` -> `ast::Stmt` -> `eval::Env` -> `format`.

| Module | Role |
| --- | --- |
| `number.rs` | `Number`: exact `rust_decimal` with `f64` fallback for huge/tiny values |
| `lexer.rs` | Raw tokens; words stay raw, the parser decides their meaning |
| `parser/mod.rs` | Statements and precedence climbing |
| `parser/tokens.rs` | Token helpers: noise skipping, lookahead, unit and name lookup |
| `parser/primary.rs` | Numbers with units, words, functions, variables |
| `parser/time.rs` | Dates, clock times, zones, "days until ..." phrases |
| `parser/target.rs` | `in`/`to`/`as` targets and rounding |
| `parser/phrases.rs` | Whole-line phrases: "20 is what % of 200", rule of three |
| `parser/sky.rs` | Sun and moon phrases: "sunrise in Tokyo", "moon phase on May 1" |
| `parser/words.rs` | Vocabulary tables (keywords, functions, months, holidays) |
| `eval/` | Arithmetic on values, functions, dates and durations, the sun and moon |
| `astro.rs` | Sunrise and sunset (NOAA), the moon's phases (Meeus, ch. 49) |
| `pi.rs` | Pi to any number of places (Chudnovsky, binary splitting on `dashu-int`) |
| `units/table.rs` | All unit definitions; `currency.rs` all currencies |
| `units/foods.txt` | Food name -> grams in a cup (generated) |
| `zones/places.txt` | Place name -> IANA zone; IANA city names are added automatically |
| `zones/cities.txt` | GeoNames cities: name, zone, latitude, longitude (generated) |
| `online.rs` | Feature `online`: rates download + disk cache |

## How parsing works (read before touching the parser)

- Words that are not `significant()` are comments and get skipped by `peek()`
  (`$20 for lunch + $15 for taxi` -> `$20 + $15`). Making a word significant
  changes this: a significant word that nothing consumes ends the expression.
  That's why `for`, `split`, `a`/`an` are significant only in context
  (`for 2 hours`, `split 4 ways`, `a day`).
- `peek()`/`at_sym()` skip noise permanently. To look for a specific word use
  `at_word()`/`eat_word()`, which scan past noise to that word.
- Leftover tokens: keywords are ignored, anything else is an error.
- Parentheses that contain unknown words are comments: `$999 (for iPhone 16)`.
- Values of the same kind side by side add up, also across comments and commas
  (`$20 for lunch, $15 for taxi`, `2 x $3 coffee $5` = $11); a rate never does,
  and in a list a comma separates items (`max($10, $20)`).
  Implicit multiplication (`2 pi`, `2(3 + 4)`) never crosses a comment word.
- Numbers in text: when the words make no calculation (`Error::is_unclear`),
  the answer is the last value of the strongest kind, as in SoulverCore: a
  calculation, then money, then other units, then plain numbers, then dates
  (`iPhone 16 Pro $999`, `2 x 4 lumber $5` = 8, `2 cats 3 dogs` = 3). Other
  errors (unknown place, unknown function) stay errors.
- A list marker starting the line (`1. `, `2) `, `a) `) is skipped, unless a date
  starts there (`1. May 2024`).
- `in` is never looked up as a unit. After a number it is inches only when no
  conversion target follows (`5 in`, `5 in to cm`, but `5 in cm`); answers write
  inches next to other units as `inch` (`N·inch³`, `$5/inch`).
- `A` is an article before a word (`A coffee $5`), amperes otherwise (`5 V/A`);
  `are` and `PS` are units only after a number or in a target (`there are 5`).
- A dot glued to `in` is the inch: `lbf·in`.
- Words joined by a hyphen are read together for units, places and functions
  (`light-year`, `pound-force`, `Aix-en-Provence`, `mid-point`); otherwise they
  are one comment word (`T-shirt $20`, `half-life`), unless every part is a
  value (`pi-e`, `twenty-five`).
- `of` between an amount and a comment word only describes the amount:
  `2 hours of work at $50/hour` = $100, `1 cup of flour in grams` is an error.
- `and` adds amounts, but after a comparison or a boolean it is logic
  (`5 > 3 and 2 > 1`, `true and false`).
- `/unit` and `per unit` right after a value form a rate at postfix level, so
  `4 nights * $120/night` multiplies by the rate (`Op::Per` keeps units apart,
  plain `Op::Div` merges units of the same kind: `3 hours / 3 days` = 0.04).
  Units multiplied in parentheses are a rate's units too: `W/(m·K)`, `$/(kW·h)`.
- A unit per an amount (`L/100km`) is a unit of its own, found by its symbol:
  `liters per 100 km` builds "L/100km". Against another unit it works as the
  plain rate (`8 L/100km * 500 km` = 40 L).
- `BareUnit in X` is reversed: `seconds in a day` = 1 day in seconds.
- A clock time followed by a zone is read in that zone (`3pm Tokyo`);
  `3pm in Tokyo` converts local 3pm; `9am in New York to Tokyo` places, then converts.
- A place right before `in`/`to` + another zone means "now there": `PST to EST`.
- `in feet and inches` splits into parts, but a list of currencies converts to
  each one (`$10 in EUR, JPY` = `€9.00, ¥1,500`, `Display::Each`).
- A food right after a weight or a volume, with or without `of`, is read with it
  (`Expr::Food`): `1 cup of flour`, `200 g sugar`, `1 cup melted butter`
  (`words::FOOD_STATES` don't change the weight). Food words stay comments
  everywhere else; a variable of the same name wins.

## Semantics worth knowing

- Addition of different units: the larger unit wins; for money and rates the
  right-hand unit wins (`$200 + €200` is in euros). Plain numbers take the
  other side's unit (`$20 + 30` = `$50`).
- A plain number over an amount after `/` is a part of it, as in SoulverCore:
  `3/4 cup` = 0.75 cup, `100 / 4 kg` = 25 kg; `per` keeps a rate (`100 per 4 kg`
  = 25/kg).
- Money times a non-money unit stays money (`$30 × 4 days` = `$120`), and a
  money rate divided by a time spreads it (`$500/month / 30 days` = `$16.67/day`).
- `in per week`, `as per day` and `is what per week` give a rate a new period
  (`Target::Per`).
- Converting to the inverse kind takes the reciprocal (`2 seconds in Hz`,
  `30 mpg in L/100km`); a count per a unit other than time converts that unit
  instead (`3 per kg in lb` = 1.36/lb), as in SoulverCore.
- Time quantities in different units add up to a `Duration` (`1 day - 2 hours`
  = 22 hours); months and longer meet in the smaller unit (`1 year - 2 months`
  = 10 months). Seconds that come out of unit algebra (`3 GB / 10 MB/s`)
  become a duration too.
- `if … then … else …`, `x if cond`, `x unless cond`: a false condition with
  no `else` gives no answer (and assigns nothing).
- Compound growth (`$1,000 after 3 years at 7% compounding monthly`) is a
  whole-line phrase in `parser/phrases.rs`; the rate is yearly unless written
  per period (`10% per month`). Loans (`total repayment on`, `total interest on`,
  `monthly repayment on`) use the same phrase and are paid back monthly;
  `monthly`/`weekly` spread the total over the loan's time.
- Workdays (Mon–Fri, no public holidays yet) have their own dimension, so they
  never mix with days; `in workdays` counts them over a range or a span of days.
  Shorter time is hours of work (`Config::hours_per_workday`, 8): `40 hours in
  workdays` = 5, `$100/hour * 1 workday` = $800. Dates move by whole workdays,
  skipping weekends (`today + 5 workdays`).
- Quarters (`Q3 2026`, `next quarter`) are periods (`Expr::Period`): alone they show
  their first and last day, in a calculation they are their length (`Q3 in days` =
  92 days), and a range starts at their first day (`days until Q4`) or runs through
  them (`Q1 through Q2`). A quarter right before a value or a comment is a label
  (`Q2 sales 600`, `Q3 $10k`).
- `start of`/`end of` a period is its first or last day (`end of Q3`, `start of
  next month`). Weeks start on Monday: `friday in 3 weeks` is the Friday of the
  week three weeks from now.
- `sum`, `total` and `average` take the lines above, up to a blank line, a
  heading or an earlier total; totals right after a total take the same lines
  (`sum`, then `sum in EUR` or `average`).
- Percentages are `Value::Percent(p)`: `X + p%`, `p% of X`, `p% on/off X`.
  A written amount of/on/off another is its share, as in SoulverCore (`10 of 20`
  = 50%, `$5 off $20` = 25%; `half of 10` = 5); words around the operator or a
  date after it make it text (`$500 for team of 5` = $500).
- `pt` is the typographic point, as in SoulverCore: 96 px or 72 pt to the inch
  and 16 px to the em, as in CSS. Pints are written out (`2.11 pints`).
- A quarter is three months (`1 quarter in months`), a fraction only before
  `of` (`3 quarters of an hour`).
- Month = 30.436875 days, year = 365.2425 days (as in Soulver). Date
  arithmetic uses calendar spans (`Jan 31 + 1 month` = `Feb 29` in 2020).
- A date without a year is this year unless that is more than 270 days away.
  Holidays without a year are the next occurrence. In a range the other end
  decides, as in SoulverCore: `Dec 1 to Jan 31` ends next year, `days since Dec 25`
  starts last year, `Jan 1 - Mar 1` is a range too.
- Events with a fixed date (`GTA 6 release date`) live in `words::EVENTS`;
  their note (`(probably)`) is display only (`Expr::Noted`), never part of a
  calculation. Update or drop entries when the date changes or passes.
- Daylight abbreviations (`EDT`, `AEDT`, `BST`) are fixed offsets; standard
  ones (`EST`, `AEST`) follow the region all year, as people use them.
  Answers show an abbreviation only if it reads back as the same time ("JST",
  "BST"), the offset otherwise: Belize's "CST" would be Chicago's (`UTC-6`).
- Answers read back as the same value; that is what `answers_read_back` checks.
  So picometers are written out (`1 pm` is a clock time) and an infinite result
  is an error ("result is too large"), not `∞`.
- Clock-only answers show the date when it isn't today; `Moment::seconds`
  hides seconds of `now`.
- Numbers show `Config::precision` significant digits (default 10) but never
  round the whole part; currencies use their minor units. A rounding asks for
  its own digits: `1/3 to 15 dp` shows all 15 (up to the 28 a number holds).
- Pi goes further: `pi to 1000 dp`, `pi to 50 sf` round it, `100 digits of pi`
  cuts it off (`Expr::Pi`), up to `pi::MAX_PLACES` (a million, about half a
  second, worked out on up to 8 threads). The answer shows every digit
  (`Display::Digits`); as a value it is a number like any other. Digits found
  once are kept for the next line.
- Numbers take both `10.50` and `10,50`: thousands come in groups of three, so
  only a lone separator before three digits (`1,500`, `1.500`) depends on
  `Config::decimal_comma` (CLI: from `LC_NUMERIC`/`LANG`), which also sets the
  output style. A comma is never a decimal in a call (`max(1,5)`) or a glued
  list (`1,2,3`).
- `$` means `Config::dollar` (USD by default). Lowercase currency codes that
  are English words (`all`, `try`, `top`...) only work in uppercase.
- Cents, pence and satoshis (`currency::MINOR_UNITS`) are money units worth a
  fraction of their currency; answers show them in the whole currency.
- The sun needs a city or an airport (`zones::locate`); regions and zone names
  ("Texas", "PST") are an error, not a guess. Without a place it is the local
  zone's principal place. Sunrise and sunset are for today at the place (or
  the date asked for; "tomorrow" counts from the place's today), rounded to
  the minute, and daylight is their difference. They agree with PyEphem within
  seconds; SoulverCore's are often a minute off.
- Amounts of food convert between weight and volume only inside a conversion
  (`1 cup of flour in grams` = 125 g); without one they stay as written
  (`1 cup of flour` = 1 cup). Every food in the conversion takes the target's
  kind, so different foods add up (`1 cup of flour + 1 cup of sugar in grams` =
  325 g). `density of butter` is in g/cm³.
- Moon phases come from the exact times of the main phases; each main phase
  lasts an eighth of the month around its time. SoulverCore uses a mean month
  and can be half a day off. `lunar day` is 1 on the day of the new moon.

## Data sources

- Exchange rates: fawazahmed0 currency API (CC0), jsDelivr with a Cloudflare
  mirror as fallback. Rates are units per USD. Cache: `~/.cache/rechenkern/rates.json`,
  refreshed after 6 hours; a stale cache is used when offline.
- Time zones: jiff with the system tzdb (bundled copy as fallback). Countries
  map to their capital's zone.
- Places, in lookup order: `zones/places.txt` (hand-picked: countries, states,
  abbreviations, airports, big cities), IANA zone city names, then
  `zones/cities.txt` — ~57k names from GeoNames `cities15000` (CC BY 4.0).
  `cities.txt` is generated by `scripts/cities.py` (see its docstring) and is
  searched in place with a binary search, so it costs nothing at startup.
  English-word names of small towns are dropped there so they don't clash
  with normal text; add important ones to `places.txt` by hand. Names also
  come as the parser reads them, without dots and loose apostrophes
  (`fort st john` for "Fort St. John", `afak` for "'Afak").
- IANA city names come from jiff's bundled zone list (the same names as the
  system tzdb, without opening every zone file); the zones themselves still
  load from the system tzdb.
- Coordinates, for the sun: `cities.txt` has them for every city;
  `places.txt` gives them for listed cities and airports that need them
  (airports use the city they serve); `zones/zone_coordinates.txt` has each
  zone's principal place, generated by `scripts/zone_coordinates.py` from
  tzdb's `zone.tab` (public domain). A city's coordinates count only if its
  zone keeps the same time as the place's: the state of Washington is not the city.
- `in`/`to` + unknown words after a date or time is an error ("unknown place"),
  with a "did you mean" hint by edit distance. After `to` only a clock time
  counts, so "time to go" is not a place.

- Foods: USDA FoodData Central SR Legacy (public domain). `units/foods.txt` is
  generated by `scripts/foods.py` (see its docstring), which names the USDA food
  and household measure ("cup, packed", "tbsp") for each recipe name, so a cup
  of sugar is exactly 200 g. Water is 1 g/mL.

## Performance

A line takes about 2 µs, so keep new work off the hot path:

- `significant()` and `place_at()` are remembered per token (`TokenMemo`).
  Clones share the memo; `sub()` parsers get a new one, since look-ahead
  depends on where the slice ends.
- Phrase tables are `words::Phrases`, looked up by their first word.
- The binary search over `cities.txt` reads only names; longer lines cost
  nothing until a city is found.
- Built-in lookup tables use `hash::TableMap` (Fx hash), not the default
  hasher. User data (variables, rates) keeps the default.
- The current time is read lazily per line (`config::Now`), so lines without
  dates never touch the time zone database.

## Conventions

- Code, comments and commit messages in English.
- Comments short and to the point, usually one line.
- Commits: short one-line conventional messages (`feat: ...`, `fix: ...`).
- Readable code over clever code: small functions, names that say what they do.
