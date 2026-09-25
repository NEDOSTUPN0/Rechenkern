# Rechenkern

An open, blazingly fast alternative to [SoulverCore](https://github.com/soulverteam/SoulverCore).

Rechenkern is a natural language calculator engine written in Rust. You give it a
line of text like `$20 for lunch + 15% tip` or `3pm in Tokyo`, and it gives you the
answer. It comes as a library and as a small command line tool.

- **Fast.** Up to 500 times faster than SoulverCore on the same machine (about 2 µs
  per line). See [Performance](#performance).
- **Open.** MIT licensed. SoulverCore is a closed binary that needs a license for
  public or commercial projects.
- **Small.** One Rust binary with no runtime to ship. Works offline; exchange rates are
  cached.
- **Understands a lot.** Units, currencies, percentages, dates, time zones, rates,
  compound interest, and notes with running totals and variables.

## Examples

```
$20 for lunch + $15 for taxi + 15% tip          $40.25
5 km in miles                                   3.106855961 mi
180 cm in feet and inches                       5 ft 10.87 in
$50 in EUR, GBP                                 €43.00, £37.00
20 is what % of 200                             10%
$1,000 after 3 years at 7% compounding monthly  $1,232.93
$24 a day for a year                            $8,765.82
time to upload 3GB at 10 MB/s                   5 minutes
6 is to 60 as 8 is to what                      80
days until christmas                            93 days
3 weeks after March 14, 2019                    Thu, 4 Apr 2019
workdays until christmas                        66 workdays
time in Tokyo when it is 9am in London          17:00 JST
6pm Sydney in Chicago                           03:00 CDT
255 in binary                                   0b11111111
time in pariss                                  unknown place "pariss", did you mean "Paris"?
```

Lines are remembered, so a text works like a small spreadsheet:

```
flights = $1,240                                $1,240.00
hotel = 6 nights * $145/night                   $870.00
food = $45 a day for a week                     $315.00
total = flights + hotel + food                  $2,425.00
total / 3                                       $808.33
```

Words that mean nothing to the calculator are skipped, so you can write notes around
the numbers. Earlier answers are available as `prev`, `sum`, `total`, `average` and
`line 2`.

## Install

With Nix:

```sh
nix run github:NEDOSTUPN0/Rechenkern -- "5 km in miles"
```

With Cargo:

```sh
cargo install --git https://github.com/NEDOSTUPN0/Rechenkern rechenkern-cli
```

## Command line

```sh
rechenkern 10 usd to eur                # one expression
rechenkern -a -f budget.txt             # every line of a file, next to its answer
cat budget.txt | rechenkern --json      # one JSON object per line
rechenkern                              # interactive prompt
```

Useful options: `--tz America/New_York`, `--12h`, `--date-order mdy`, `--dollar CAD`,
`--degrees`, `--offline`. Run `rechenkern --help` for the full list.

Numbers can be written as `1,234.5` or `1.234,5`; the output style follows your locale.

## Library

```toml
[dependencies]
rechenkern = { git = "https://github.com/NEDOSTUPN0/Rechenkern" }
```

```rust
use rechenkern::Calculator;

let mut calc = Calculator::new();
let answer = calc.calculate("3 hours 30 minutes in minutes").unwrap().unwrap();
assert_eq!(answer.to_string(), "210 minutes");
```

Exchange rates come from a `RateProvider` you set on the calculator. The `online`
feature adds one that downloads rates and caches them on disk.

## Rechenkern and SoulverCore

Checked by running the same lines through both engines (SoulverCore 3.5.1, Linux
build). The Soulver app may understand more than the engine alone.

|                                                  | Rechenkern |   SoulverCore    |
| ------------------------------------------------ | :--------: | :--------------: |
| Arithmetic, functions, number bases, rounding    |     ✓      |        ✓         |
| Units, currencies, percentages, rates            |     ✓      |        ✓         |
| Dates, clock times, time zones, workdays         |     ✓      |        ✓         |
| Compound interest, rule of three                 |     ✓      |        ✓         |
| Sheets with variables                            |     ✓      |        ✓         |
| Cities known for time zones                      |  ~57,000   |       ~650       |
| "Did you mean" for misspelled places             |     ✓      |        ✗         |
| Several currencies at once: `$50 in EUR, GBP`    |     ✓      |        ✗         |
| Both decimal styles: `10,50` and `10.50`         |     ✓      |        ✗         |
| `cast`, angles in DMS and hours                  |     ✗      |        ✓         |
| Rate divided by quantity: `$500/month / 30 days` |     ✗      |        ✓         |
| Inflation, cooking by density, music, timecode   |     ✗      |        ✓         |
| Languages                                        |  English   |   English + 8    |
| API for custom units and functions               |     ✗      |        ✓         |
| Command line tool                                |     ✓      |        ✗         |
| Runtime to ship                                  |    none    | Swift (on Linux) |
| Source                                           | open, MIT  |      closed      |

SoulverCore is free for personal use and needs a license for public or commercial
projects.

## Performance

Measured on one machine (AMD Ryzen 9 9900X, NixOS) against SoulverCore 3.5.1 for
Linux x86_64 with Swift 6.3.3. Both engines got the same 62 lines, chosen so that both
answer them the same way, and the same 31 line sheet.

| Scenario                                 | Rechenkern | SoulverCore | Faster |
| ---------------------------------------- | ---------: | ----------: | -----: |
| One line, median                         |     1.7 µs |      127 µs |   ~75× |
| One line, mean                           |     1.9 µs |      122 µs |   ~64× |
| Lines per second                         |    526,000 |       8,200 |        |
| Sheet of 31 lines with variables         |      48 µs |       24 ms |  ~500× |
| Start, answer one line, exit             |     0.6 ms |       63 ms |  ~100× |

Notes:

- Each line is timed many times on a warm calculator. The sheet is recalculated from
  scratch; for SoulverCore that is `LineCollection.evaluateAll()`.
- SoulverCore's own README says "7k+ calculations/second on Apple silicon", which
  matches the 8,200 measured here, so the Linux build doesn't look slow.
- SoulverCore does more than Rechenkern (see above), so this compares the cases both
  handle.

To reproduce: `cargo bench -p rechenkern` runs
[`crates/rechenkern/benches`](crates/rechenkern/benches), and
[`soulver.swift`](crates/rechenkern/benches/soulver.swift) runs the same lines through
SoulverCore.

## Data

- Exchange rates: [fawazahmed0/exchange-api](https://github.com/fawazahmed0/exchange-api) (CC0)
- Time zones: the IANA time zone database, through [jiff](https://github.com/BurntSushi/jiff)
- Cities: [GeoNames](https://www.geonames.org/), [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/), filtered by `scripts/cities.py`

## License

MIT

---

Rechenkern was written entirely with Claude Code
So far it seems to work just fine.
