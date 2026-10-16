//! What a host adds or changes: its own units and places, unit systems, holidays.

use jiff::civil::date;
use jiff::tz::TimeZone;
use rechenkern::{Calculator, Config, CustomPlace, CustomUnit, Number, RateTable, UnitSystem};

/// Wed 23 Sep 2026 14:30 in New York, with fixed rates.
fn config() -> Config {
    Config {
        now: Some("2026-09-23T14:30:00-04:00[America/New_York]".parse().unwrap()),
        time_zone: Some(TimeZone::get("America/New_York").unwrap()),
        ..Config::default()
    }
}

fn answers(config: Config, lines: &[(&str, &str)]) {
    let mut calc = Calculator::with_config(config);
    let mut rates = RateTable::new();
    rates.insert("EUR", Number::parse("0.9").unwrap());
    calc.set_rates(rates);
    let failures: Vec<String> = lines
        .iter()
        .filter_map(|(line, expected)| {
            let got = match calc.calculate(line) {
                Ok(Some(answer)) => answer.to_string(),
                Ok(None) => "<none>".into(),
                Err(e) => format!("<error: {e}>"),
            };
            (got != *expected).then(|| format!("{line:30} expected {expected:25} got {got}"))
        })
        .collect();
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

#[test]
fn custom_units() {
    let smoot =
        CustomUnit { symbol: Some("smt".into()), ..CustomUnit::new("smoot", Number::parse("1.7018").unwrap(), "m") };
    let coffee = CustomUnit::new("coffee", Number::from_i64(4), "USD");
    let pace = CustomUnit::new("furlong per fortnight", Number::parse("0.0001663").unwrap(), "m/s");
    let config = Config { custom_units: vec![smoot, coffee, pace], ..config() };
    answers(
        config,
        &[
            ("10 smoots in m", "17.018 m"),
            ("100 m in smoots", "58.76131155 smt"),
            ("2 smt + 1 m", "2.587613116 smt"),
            ("3 coffees in EUR", "€10.80"),
            ("3 coffees", "$12.00"),
            ("1 furlong per fortnight in mm/s", "0.1663 mm/s"),
        ],
    );
}

#[test]
fn custom_units_must_be_known() {
    let config = Config { custom_units: vec![CustomUnit::new("blip", Number::ONE, "zorks")], ..config() };
    answers(config, &[("1 + 1", "<error: custom unit blip: unknown unit zorks>")]);
}

#[test]
fn custom_places() {
    let mut hq = CustomPlace::new("HQ", TimeZone::get("Asia/Tokyo").unwrap());
    hq.coordinates = Some((35.68, 139.77));
    let config = Config { custom_places: vec![hq], ..config() };
    answers(
        config.clone(),
        &[("time in HQ", "Thu, 24 Sep 2026 03:30 JST"), ("sunrise in hq", "Thu, 24 Sep 2026 05:30 JST")],
    );
    let mut calc = Calculator::with_config(config);
    let answer = calc.calculate("time in hq").unwrap().unwrap();
    assert_eq!(answer.target().unwrap().label.as_deref(), Some("HQ"));
}

#[test]
fn unit_systems() {
    answers(
        config(),
        &[("1 gallon in L", "3.785411784 L"), ("1 cup in mL", "236.5882365 mL"), ("1 ton in kg", "907.18474 kg")],
    );
    answers(
        Config { unit_system: UnitSystem::Imperial, ..config() },
        &[
            ("1 gallon in L", "4.54609 L"),
            ("1 us gallon in L", "3.785411784 L"),
            ("2 pints in mL", "1,136.5225 mL"),
            ("1 tbsp in mL", "15 mL"),
            ("1 ton in kg", "1,016.046909 kg"),
        ],
    );
    answers(
        Config { unit_system: UnitSystem::Metric, ..config() },
        &[
            ("1 cup in mL", "250 mL"),
            ("2 tsp in mL", "10 mL"),
            ("1 ton in kg", "1,000 kg"),
            ("1 gallon in L", "3.785411784 L"),
        ],
    );
}

#[test]
fn font_size() {
    answers(config(), &[("2 em in px", "32 px")]);
    answers(
        Config { base_font_size: Number::from_i64(10), ..config() },
        &[("2 em in px", "20 px"), ("30 px in rem", "3 rem")],
    );
}

#[test]
fn holidays() {
    // Friday 25 September is a day off.
    let config = Config { holidays: vec![date(2026, 9, 25), date(2026, 9, 26)], ..config() };
    answers(
        config,
        &[
            ("today + 3 workdays", "Tue, 29 Sep 2026"),
            ("today + 1 workday", "Thu, 24 Sep 2026"),
            ("Sep 30 - 3 workdays", "Thu, 24 Sep 2026"),
            ("workdays from Sep 21 to Oct 3", "9 workdays"),
        ],
    );
}

#[test]
fn weeks_and_locales() {
    // Wednesday 23 September 2026.
    answers(config(), &[("start of week", "Mon, 21 Sep 2026"), ("this sunday", "Sun, 27 Sep 2026")]);
    let sunday = Config { week_start: jiff::civil::Weekday::Sunday, ..config() };
    answers(sunday, &[("start of week", "Sun, 20 Sep 2026"), ("this sunday", "Sun, 20 Sep 2026")]);

    let german = Config::for_locale("de_DE.UTF-8");
    assert!(german.decimal_comma && german.clock_24h);
    assert_eq!((german.unit_system, german.week_start), (UnitSystem::Metric, jiff::civil::Weekday::Monday));
    answers(
        Config { now: config().now, time_zone: config().time_zone, ..german },
        &[("€5 + €2.5", "7,50 €"), ("1 cup in mL", "250 mL")],
    );
    let american = Config::for_locale("en_US");
    assert!(!american.decimal_comma && !american.clock_24h);
    assert_eq!((american.unit_system, american.week_start), (UnitSystem::CustomaryUs, jiff::civil::Weekday::Sunday));
    let canadian = Config::for_locale("en-CA");
    assert_eq!(canadian.dollar, "CAD");
    assert_eq!(Config::for_locale("en_GB").unit_system, UnitSystem::Imperial);
    assert!(!Config::for_locale("de_CH").decimal_comma);
}
