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
| `answer.rs` | `Answer`: the value, its text, the line's `Form` and the two sides of a conversion (built lazily) |
| `custom.rs` | What hosts add: `CustomUnit` (registered after the built-in units), `CustomPlace`, `UnitSystem` |
| `alternatives.rs` | `Calculator::alternatives`: the answer in hex, as a fraction, in other units, a date as ISO 8601 |
| `sheet/` | `Sheet`: lines that answer together, recalculated where an edit reaches; subtotals, statistics, undo, exports and archives |
| `json.rs` | Just enough JSON for sheet exports and archives |
| `find.rs` | `Calculator::find`, `date_for`, `date_interval_for`, `Scanner`: data in text (`parser::values_in`) |
| `number.rs` | `Number`: exact `rust_decimal` with `f64` fallback for huge/tiny values |
| `lexer.rs` | Raw tokens; words stay raw, the parser decides their meaning |
| `parser/mod.rs` | Statements and precedence climbing |
| `parser/tokens.rs` | Token helpers: noise skipping, lookahead, unit and name lookup |
| `parser/primary.rs` | Numbers with units, words, functions, variables |
| `parser/time.rs` | Dates, clock times, zones, "days until ..." phrases |
| `parser/target.rs` | `in`/`to`/`as` targets and rounding |
| `parser/phrases.rs` | Whole-line phrases: "20 is what % of 200", rule of three |
| `parser/sky.rs` | Sun and moon phrases: "sunrise in Tokyo", "moon phase on May 1" |
| `parser/distance.rs` | Distances between places: "distance from Tokyo to London" |
| `parser/highlight.rs` | `Calculator::highlight`: what each part of a line is, for editors (a pass of its own, off the hot path) |
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
  everywhere else; a variable of the same name wins. A food may come before its amount
  (`flour 1 cup in grams`) or be the label (`flour: 200 g in cups`), and a variable set
  to an amount of food keeps it (`parser::Kept`: `flour = 2 cups of flour`, then
  `flour in grams`).
- A number before a counted food is `Expr::Counted`: a piece word makes it a weight
  (`1 stick of butter` = 113 g, `3 cloves of garlic`); a food counted by itself stays
  a count unless converted (`2 eggs` = 2, `2 large eggs in grams` = 100 g). The pieces
  are in `units/foods.txt` after the cup, from USDA household measures.

## Answers

- `Answer::form()` says what the top-level expression asks, as SoulverCore's
  `TokenListForm` (`conversion`, `time_at_place`, `date_shift`...). A conversion
  inside math (`5 km in mi + 1`) is a plain calculation.
- `Env::answer_line` keeps the values the sides are made of while the line is
  calculated (the source of a conversion, the ends of a range); `source()`,
  `target()`, `note()` and `copy_text()` write them on first use, so lines that
  nobody asks about cost nothing extra.
- Captions: units by their plural name in title case ("Miles per Hour"),
  currencies by `Currency::plural` ("US Dollars"), places as written
  (`zones::Place`: "Paris", "PST", "JFK") and the home zone by its city.

## Customization

- `Config` holds what SoulverCore's `EngineCustomization` does: custom units and
  places, holidays (days off for workdays), the unit system, the font size of
  `em`, hours per workday.
- Custom units get ids after the built-in ones (`units::define`, never freed);
  only the calculator whose config lists them can spell them
  (`Scope::units`). `Parser::lookup_unit` is the one way the parser reads unit
  names: custom units first, then built-in ones as the unit system reads them
  (`UnitSystem::adapt`: an imperial "gallon", a metric "cup"; "us gallon" stays).

- Host phrases (`Config::custom_functions`): a line that is the whole prototype
  (`tip on __money`) calls the handler; placeholders are parsed as values between
  the literal words (`parser/custom.rs`), custom types match listed words.
- Functions in lines: `area(w, h) = w * h` defines one (`Stmt::Function`), kept with
  the variables in `calculator::Names`; calls are `Expr::Apply`. Names of built-in
  functions, keywords and units can't be taken. A definition line keeps its
  parentheses (they aren't comment groups).
- `Config::for_locale("de_DE")` sets what a locale expects: decimal comma, clock,
  date order, unit system, week start, the local dollar and where the currency
  symbol goes. The CLI starts from it (`LANG`, `LC_NUMERIC`, `LC_TIME`).
- `Config::features` turns parts of the language off or reads them another way, as
  SoulverCore's `EngineFeatureFlags` (units, dates, phrases, conversions,
  comments, labels, list markers, zone codes, date guesses...). The defaults keep
  Rechenkern's own reading; check a flag where the parser decides, not after.

- `Config::formatting` writes answers as SoulverCore's `FormattingPreferences`:
  decimal places, scales (`Notation`), money options and the host's currency
  symbols, timespan styles, compound imperial units, conversions of answers
  (`ResultConversions`, applied in `Env::convert_result` unless the line converts
  itself), rounded references. `ResultConversions::Home` is Rechenkern's own: an
  amount in another system goes to `unit_system` (`18 lb` = 8.16 kg where it's
  metric) and money to `Config::currency`, at rates at hand (never downloaded), and
  the answer is a conversion with the amount as its source (CLI: `--convert home`,
  `--currency`). A line that writes a scale (`$5M`, `2 billion`)
  sets `Scope::scaled` and its answer keeps one while it stays exact
  (`Display::Scaled`), as SoulverCore does; glued Cyrillic and Chinese letters
  don't count.
- `Calculator::alternatives` gives the answer in other formats and units.

## Sheets

- `Calculator::run` works out a line after given lines and variables without changing
  the calculator, and says what the line read (`Reads`: variables, the names the
  parser asked about, earlier lines, the clock) and left (`Effects`).
- `Sheet` keeps each line's effects. An edit works out the edited line, then every
  line below that read a line or a variable whose value changed; a line that changes
  kind (blank, heading, divider, total), or comes or goes, means every line below.
  `properties::sheets_recalculate_as_from_scratch` checks this against a fresh sheet.
- Subtotals, grand totals and running totals or budgets are marks the host sets
  (`Behaviour`); the text of a subtotal line is only its caption.

## Typed results

- Values beyond numbers and dates have types of their own, as SoulverCore's
  `EvaluationResult`: `Value::Range` (`5-10 kg`), `List` (`$10 in EUR, JPY`; later
  lines take the first), `Pitch`, `Coordinates`, `Resolution` (`1920x1080`, both
  sides 100 or more), `Timecode` (`100 frames at 24 fps`), `Cadence` (`every 2 weeks
  until Dec 31`, a whole line). A span between dates is a `Duration` with
  `start()`/`end()`.
- Minutes per a length are a pace, written `m:ss min/km`, which reads back.
- Income tax (`income_tax.rs`, `data/income_tax.txt`): `income tax on $100,000 in US`,
  `€80,000 after tax in Germany`, `take home pay on £50,000 in UK`, `effective tax rate`,
  `marginal tax rate`, for SoulverCore's 25 countries. A single resident employee in 2025:
  the national schedule with the allowance or credit everyone gets (Germany by its
  formula), Nordic municipal tax at the average rate, no social contributions. Income in
  another currency converts first. Update the file each year.
- `compound interest on`, `simple interest on` (never compounded), `investment required
  for $1,000/month at 5%` (what pays that a year at the rate) are growth phrases too.
- Cents, satoshis, gwei and wei show as themselves when converted to (`$10 in cents` =
  1,000 cents, `Display::Minor`). A currency glued before its amount reads as one
  (`zł10`, `CHF10`, `Kč250`); `kr` alone is the Danish krone, as in SoulverCore.
- Sales tax: `8% vat in $108` is the tax in a price, `$108 before 8% tax` (or
  `- 8% vat`) the price without it; a tax word after the percentage makes it so.

## Semantics worth knowing

- Addition of different units: the larger unit wins; for money and rates the
  right-hand unit wins (`$200 + €200` is in euros). Plain numbers take the
  other side's unit (`$20 + 30` = `$50`).
- A plain number over an amount after `/` is a part of it, as in SoulverCore:
  `3/4 cup` = 0.75 cup, `100 / 4 kg` = 25 kg; `per` keeps a rate (`100 per 4 kg`
  = 25/kg).
- Money times a non-money unit stays money (`$30 × 4 days` = `$120`), and a
  money rate divided by a time spreads it (`$500/month / 30 days` = `$16.67/day`).
- Units that multiply into a quantity of their own take its unit
  (`Registry::named_product`): a unit of the same size if there is one (`V·A` = W,
  `lb·ft/s²` = pdl, `lbf/in²` = psi), the SI unit otherwise (`mAh·V` in J,
  `km/h/s` in m/s²). Named compounds (`kWh`, `Ah`), torques (`N·m`) and a length per
  a time squared (`km/h²`) stay as written. SoulverCore gets electric ones wrong
  (`12 V * 2 A` = 24 V).
- `C`, `F`, `H`, `S`, `T` alone are °C, °F and so on, as in SoulverCore; coulombs,
  farads, henries, siemens and teslas are written out and read with a prefix (`µF`).
  `rpm`/`rps` are turns per time, and a turn converts to a cycle (`60 rpm in Hz`).
- `′` and `″` are minutes and seconds of arc, as in SoulverCore (`30′ in degrees` =
  0.5°); `'` and `"` stay feet and inches. `kt` is knots (kilotonnes are written out),
  `2 M` molar (glued `5M` is a million), `5 um`/`10 us` micro when nothing but a
  conversion follows. `5 km cast to miles` keeps the number (`Target::Cast`).
  `speed of 26 inch tire at 200 rpm` rolls the circumference each turn.
- `in per week`, `as per day` and `is what per week` give a rate a new period
  (`Target::Per`).
- Converting to the inverse kind takes the reciprocal (`2 seconds in Hz`,
  `30 mpg in L/100km`); a count per a unit other than time converts that unit
  instead (`3 per kg in lb` = 1.36/lb), as in SoulverCore.
- Time quantities in different units add up to a `Duration` (`1 day - 2 hours`
  = 22 hours); months and longer meet in the smaller unit (`1 year - 2 months`
  = 10 months). Seconds that come out of unit algebra (`3 GB / 10 MB/s`)
  become a duration too.
- A smaller number before an amount, with nothing after but a conversion, is a
  range (`5-10 kg`, `5 to 10 kg`, `between 5 and 10 kg` = "5 – 10 kg"); a
  conversion converts both ends. In a calculation it subtracts (`5-10 kg * 2`
  = -15 kg), and so does money (`$10-20`), as in SoulverCore. The answer is text.
- `if … then … else …`, `x if cond`, `x unless cond`: a false condition with
  no `else` gives no answer (and assigns nothing).
- Compound growth (`$1,000 after 3 years at 7% compounding monthly`) is a
  whole-line phrase in `parser/phrases.rs`; the rate is yearly unless written
  per period (`10% per month`). Loans (`total repayment on`, `total interest on`,
  `monthly repayment on`) use the same phrase and are paid back monthly;
  `monthly`/`weekly` spread the total over the loan's time. A rate may say its period
  and direction (`10% decay every 2 years`). `time from 20 to 30 at 10% every year`
  takes the logarithm, `at 2/day` divides (a duration in that period and shorter
  units); `growth every year from 100 to 200 over 5 years` is the rate back, and
  `$500 invested $1,000 returned` the gain as a multiple.
- Workdays (Mon–Fri, no public holidays yet) have their own dimension, so they
  never mix with days; `in workdays` counts them over a range or a span of days.
  Shorter time is hours of work (`Config::hours_per_workday`, 8): `40 hours in
  workdays` = 5, `$100/hour * 1 workday` = $800. Dates move by whole workdays,
  skipping weekends (`today + 5 workdays`).
- Quarters (`Q3 2026`, `next quarter`) are periods (`Expr::Period`): alone they show
  their first and last day, in a calculation they are their length (`Q3 in days` =
  92 days), and a range starts at their first day (`days until Q4`) or runs through
  them (`Q1 through Q2`). A quarter right before a value or a comment is a label
  (`Q2 sales 600`, `Q3 $10k`). Seasons work the same, as meteorologists in the
  northern hemisphere count them (`summer 2020` is June to August, `winter 2026`
  starts in December).
- `start of`/`end of` a period is its first or last day (`end of Q3`, `start of
  next month`). Weeks start on Monday: `friday in 3 weeks` is the Friday of the
  week three weeks from now.
- `sum`, `total` and `average` take the lines above, up to a blank line, a
  heading or an earlier total; totals right after a total take the same lines
  (`sum`, then `sum in EUR` or `average`). A total with a hashtag takes only the
  lines with that tag (`sum #food`); `eval::Line::tags` keeps them. `median`, `max`,
  `min`, `count` and `std dev` as the whole line (`LineRef::Stat`) take the same lines;
  SoulverCore leaves them to its UI. `previous line` is `prev`.
- `widgets = new unit` declares a unit of its own kind for the lines below
  (`Stmt::Unit`, `Names::units`): `$2/widget * 10 widgets` = $20. Each name takes one of
  `dim::DECLARED` free base quantities, for the whole program.
- `Features::labels_as_variables` (off, as in SoulverCore) makes `Rent: $1,500` set `rent`.
- Nights are a unit of their own, a day each: `nights in March 12 to March 25` = 13 nights.
- Numbers in text: a clock time and a day with words between are that day at that
  time (`3pm cats June 5`), as in SoulverCore.
- A plain whole answer is in the base of the last number written in one (`0xff + 0b11`
  = `0b100000010`, `0xFF & 0x0F` = `0xF`), and so are variables set in one, as in
  SoulverCore (`Scope::radix`, `Names::radixes`). Negative or fractional answers, units
  and conversions (`in decimal`) are decimal.
- `mod` and `%` take the sign of the divisor, as in SoulverCore: `-10 mod 3` = 2.
- Fractions answer as SoulverCore writes them: `0.375 as fraction` = ⅜, `1.25 as frac` =
  1 ¼, `22/7` = 3 1/7; `1 ¼` and `1¼` read back. Inverse trigonometry gives an angle
  (`asin(0.5)` = 0.5235987756 rad). `~` is a bitwise not only before a number in a base
  (`~0xFF` = -256); `~5` is "about 5", as in SoulverCore.
- `K` glued to a number is a thousand, as in SoulverCore (`3K` = 3,000); kelvins take a
  space (`3 K`).
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
- Two-digit years count when the date can't be anything else: `21/04/77` (only
  one of the first two numbers can be the month), `10.05.20`; `3/4/26` divides.
  69 to 99 are the 1900s, 00 to 68 the 2000s, as strptime reads them.
- A date without a year is this year unless that is more than 270 days away.
  Holidays without a year are the next occurrence. In a range the other end
  decides, as in SoulverCore: `Dec 1 to Jan 31` ends next year, `days since Dec 25`
  starts last year, `Jan 1 - Mar 1` is a range too. A period after `until`/`since`
  counts from its first day (`days until March`, `days until next month` = to the 1st,
  as in SoulverCore; but `days since last month` is a month ago). Holidays and quarters too
  (`days from Christmas`, `days since Q1`), and `halfway between`.
- Events with a fixed date (`GTA 6 release date`) live in `words::EVENTS`;
  their note (`(probably)`) is display only (`Expr::Noted`), never part of a
  calculation. Update or drop entries when the date changes or passes.
- Daylight abbreviations (`EDT`, `AEDT`, `BST`) are fixed offsets; standard
  ones (`EST`, `AEST`) follow the region all year, as people use them. Those that
  are words (`CAT`, `EAT`, `WET`) are written in capitals in `places.txt` and count
  only in capitals.
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
- A currency per another is an exchange rate (`exchange rate from 100 USD to 150 AUD`
  = A$1.50/$): it cancels its currency against an amount (`100 USD * 1.5 AUD/USD` =
  A$150) and turns round in a conversion (`in USD/AUD` = $0.67/A$), never at market rates.
- Cents, pence and satoshis (`currency::MINOR_UNITS`) are money units worth a
  fraction of their currency; answers show them in the whole currency.
- The sun needs a city or an airport (`zones::locate`); regions and zone names
  ("Texas", "PST") are an error, not a guess. Without a place it is the local
  zone's principal place. Sunrise and sunset are for today at the place (or
  the date asked for; "tomorrow" counts from the place's today), rounded to
  the minute, and daylight is their difference. They agree with PyEphem within
  seconds; SoulverCore's are often a minute off.
- Distances between places are great-circle distances from their coordinates,
  rounded to the kilometer (`distance from Tokyo to London` = 9,558 km); like the
  sun, they need a city or an airport.
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
  mirror as fallback. Coins it lacks (TON) come from CoinGecko's free price API
  (`online::SUPPLEMENT`), merged into the cached file. Rates are units per USD. Cache: `~/.cache/rechenkern/rates.json`,
  refreshed after 6 hours; a stale cache is used when offline.
- Rates of past days (`$100 in EUR on Jan 1, 2020`, `RateProvider::rates_on`):
  the same API keeps dated snapshots from March 2024; earlier days come from the
  Frankfurter API (ECB reference rates, about 30 currencies since 1999). Each day
  is cached for good as `rates-YYYY-MM-DD.json` next to `rates.json`.
- Bundled rates: `src/data/rates.txt`, a snapshot of the same API made by
  `scripts/rates.py`. They fill in currencies a provider lacks and stand in when
  there are no rates at all (`Features::use_default_rates_for_unhandled_currencies`).
  Refresh them now and then.
- Inflation: `src/data/cpi.txt`, the US CPI-U (BLS, public domain) made by
  `scripts/cpi.py`: annual averages, and the months of the year without one. Now is
  the latest month; a host's `InflationProvider` gives other currencies.
- Places the engine doesn't know: a host's `PlaceProvider`, asked only where a line
  expects a place (`time in X`, `3pm in X`, `sunrise in X`), once per name per
  calculator. The CLI's is `online::OnlinePlaces`: the Open-Meteo geocoding API
  (GeoNames data, CC BY 4.0; free for non-commercial use), cached in
  `~/.cache/rechenkern/places.json`, misses too.
- Weather: a host's `WeatherProvider` (`weather in Tokyo`, `humidity in London
  tomorrow`); without one it is an error. The CLI's is `online::OnlineWeather`:
  Open-Meteo forecasts (16 days ahead, 92 back) and its archive for older days.
  Temperatures are °C (convert with `in °F`); the sky is a note on `weather`.
- Time zones: jiff with the system tzdb (bundled copy as fallback). Countries
  map to their capital's zone.
- Places, in lookup order: `zones/places.txt` (hand-picked: countries, states,
  abbreviations, airports, big cities), IANA zone city names, then
  `zones/cities.txt` — ~57k names from GeoNames `cities15000` (CC BY 4.0).
  `cities.txt` is generated by `scripts/cities.py` (see its docstring) and is
  searched in place with a binary search, so it costs nothing at startup.
  English-word names of small towns are dropped there so they don't clash
  with normal text; add important ones to `places.txt` by hand. A shared name
  goes to the most populous city, but an alternate name counts for less than a
  city's own (`Parma` is Italy's, not an old name of Perm); `Nara-shi` is also
  `Nara`, `Donostia / San Sebastián` both. `places.txt` settles the rest as
  SoulverCore does: Washington is the city, Pasadena California's. Names also
  come as the parser reads them, without dots and loose apostrophes
  (`fort st john` for "Fort St. John", `afak` for "'Afak").
- IANA city names come from jiff's bundled zone list (the same names as the
  system tzdb, without opening every zone file); the zones themselves still
  load from the system tzdb.
- Airports: `zones/airports.txt`, generated by `scripts/airports.py` from OurAirports
  (public domain): large airports and a few medium ones, in the zone of the nearest
  city of `cities.txt`. They come after every other place name; codes that are words
  (`MAN`, `JAN`, `NUM`) count only in capitals.
- Coordinates, for the sun: `cities.txt` has them for every city;
  `places.txt` gives them for listed cities and airports that need them
  (airports use the city they serve); `zones/zone_coordinates.txt` has each
  zone's principal place, generated by `scripts/zone_coordinates.py` from
  tzdb's `zone.tab` (public domain). A city's coordinates count only if its
  zone keeps the same time as the place's: the state of Washington is not the city.
- `in`/`to` + unknown words after a date or time is an error ("unknown place"),
  with a "did you mean" hint by edit distance. After `to` only a clock time
  counts, so "time to go" is not a place.

- Ramadan: `src/data/ramadan.txt`, the Umm al-Qura calendar (1926 to 2077) made by
  `scripts/ramadan.py` with hijridate (MIT). Hanukkah follows the Hebrew calendar,
  worked out (`kislev_25`). Both start the evening before, as in SoulverCore.
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
