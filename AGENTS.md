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
- Try things by hand: `cargo run -q -- "10 usd to eur"`, pipe a file into
  `cargo run -q -- -a` for sheet mode.

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
| `parser/words.rs` | Vocabulary tables (keywords, functions, months, holidays) |
| `eval/` | Arithmetic on values, functions, dates and durations |
| `units/table.rs` | All unit definitions; `currency.rs` all currencies |
| `zones/places.txt` | Place name -> IANA zone; IANA city names are added automatically |
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
- `in` is never looked up as a unit. After a number it is inches only when no
  conversion target follows (`5 in`, `5 in to cm`, but `5 in cm`).
- `/unit` and `per unit` right after a value form a rate at postfix level, so
  `4 nights * $120/night` multiplies by the rate (`Op::Per` keeps units apart,
  plain `Op::Div` merges units of the same kind: `3 hours / 3 days` = 0.04).
- `BareUnit in X` is reversed: `seconds in a day` = 1 day in seconds.
- A clock time followed by a zone is read in that zone (`3pm Tokyo`);
  `3pm in Tokyo` converts local 3pm; `9am in New York to Tokyo` places, then converts.
- A place right before `in`/`to` + another zone means "now there": `PST to EST`.
- `in feet and inches` splits into parts, but a list of currencies converts to
  each one (`$10 in EUR, JPY` = `€9.00, ¥1,500`, `Display::Each`).

## Semantics worth knowing

- Addition of different units: the larger unit wins; for money and rates the
  right-hand unit wins (`$200 + €200` is in euros). Plain numbers take the
  other side's unit (`$20 + 30` = `$50`).
- Money times a non-money unit stays money (`$30 × 4 days` = `$120`).
- Time quantities in different units add up to a `Duration` (`1 day - 2 hours`
  = 22 hours); months and longer meet in the smaller unit (`1 year - 2 months`
  = 10 months). Seconds that come out of unit algebra (`3 GB / 10 MB/s`)
  become a duration too.
- `if … then … else …`, `x if cond`, `x unless cond`: a false condition with
  no `else` gives no answer (and assigns nothing).
- Compound growth (`$1,000 after 3 years at 7% compounding monthly`) is a
  whole-line phrase in `parser/phrases.rs`; the rate is yearly unless written
  per period (`10% per month`).
- Workdays (Mon–Fri, no public holidays yet) have their own dimension, so they
  never mix with time; `in workdays` counts them over a range or duration.
- Percentages are `Value::Percent(p)`: `X + p%`, `p% of X`, `p% on/off X`.
- Month = 30.436875 days, year = 365.2425 days (as in Soulver). Date
  arithmetic uses calendar spans (`Jan 31 + 1 month` = `Feb 29` in 2020).
- A date without a year is this year unless that is more than 270 days away.
  Holidays without a year are the next occurrence.
- Clock-only answers show the date when it isn't today; `Moment::seconds`
  hides seconds of `now`.
- Numbers show `Config::precision` significant digits (default 10) but never
  round the whole part; currencies use their minor units.
- Numbers take both `10.50` and `10,50`: thousands come in groups of three, so
  only a lone separator before three digits (`1,500`, `1.500`) depends on
  `Config::decimal_comma` (CLI: from `LC_NUMERIC`/`LANG`), which also sets the
  output style. A comma is never a decimal in a call (`max(1,5)`) or a glued
  list (`1,2,3`).
- `$` means `Config::dollar` (USD by default). Lowercase currency codes that
  are English words (`all`, `try`, `top`...) only work in uppercase.

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
  with normal text; add important ones to `places.txt` by hand.
- `in`/`to` + unknown words after a date or time is an error ("unknown place"),
  with a "did you mean" hint by edit distance. After `to` only a clock time
  counts, so "time to go" is not a place.

## Conventions

- Code, comments and commit messages in English.
- Comments short and to the point, usually one line.
- Commits: short one-line conventional messages (`feat: ...`, `fix: ...`).
- Readable code over clever code: small functions, names that say what they do.
