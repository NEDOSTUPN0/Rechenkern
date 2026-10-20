//! Settings beyond the feature flags: conversions of answers, fractions, scales, symbols,
//! places and days of the host's, locales.

use jiff::civil::date;
use jiff::tz::TimeZone;
use rechenkern::{
    Calculator, Config, ConversionRule, Converter, CustomPlace, DisplayKind, Formatting, NamedDay, Notation, Phrase,
    PlaceKind, ResultConversions, ScalarSystem, Scale, Sheet, UnitSymbol,
};

/// Wed 23 Sep 2026 14:30 in New York.
fn base() -> Config {
    Config {
        now: Some("2026-09-23T14:30:00-04:00[America/New_York]".parse().unwrap()),
        time_zone: Some(TimeZone::get("America/New_York").unwrap()),
        ..Config::default()
    }
}

fn answers(config: Config, lines: &[(&str, &str)]) {
    let mut calc = Calculator::with_config(config);
    let failures: Vec<String> = lines
        .iter()
        .filter_map(|(line, expected)| {
            let got = match calc.calculate(line) {
                Ok(Some(answer)) => answer.to_string(),
                Ok(None) => "<none>".into(),
                Err(e) => format!("<error: {e}>"),
            };
            (got != *expected).then(|| format!("{line:35} expected {expected:25} got {got}"))
        })
        .collect();
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

fn with_rules(rules: Vec<ConversionRule>) -> Config {
    let formatting = Formatting { conversions: ResultConversions::Rules(rules), ..Formatting::default() };
    Config { formatting, ..base() }
}

#[test]
fn conversion_rules() {
    answers(
        with_rules(vec![
            ConversionRule::Display { from: DisplayKind::Base(16), to: Converter::Decimal },
            ConversionRule::Display { from: DisplayKind::Timestamp, to: Converter::Date },
        ]),
        &[("0xff + 1", "256"), ("0xff in hex", "0xFF"), ("0b11 + 1", "0b100")],
    );
    answers(
        with_rules(vec![ConversionRule::Durations { to: "hours".into() }]),
        &[("2 days + 3 hours", "51 hours"), ("90 minutes", "1.5 hours"), ("5 km", "5 km")],
    );
    answers(
        with_rules(vec![ConversionRule::Parts { like: "m".into(), units: vec!["ft".into(), "in".into()] }]),
        &[("2 m", "6 ft 6.74 in"), ("2 kg", "2 kg")],
    );
    answers(
        with_rules(vec![ConversionRule::RelativeDates]),
        &[("today + 3 days", "in 3 days"), ("yesterday", "yesterday"), ("5 + 5", "10")],
    );
}

#[test]
fn fractions_and_scales() {
    let fractions = Config { formatting: Formatting { fractions: true, ..Formatting::default() }, ..base() };
    answers(
        fractions,
        &[("3/8", "⅜"), ("10/4 cups", "2 ½ cup"), ("1/3", "⅓"), ("0.1234567", "0.1234567"), ("$2.50", "$2.50")],
    );
    let indian = Formatting {
        scalars: ScalarSystem::Indian,
        notation_threshold: Some(Scale::HundredThousand),
        ..Formatting::default()
    };
    answers(
        Config { formatting: indian, dollar: "INR".into(), ..base() },
        &[("$250,000", "₹2.5 lakh"), ("$30,000,000", "₹3 crore"), ("5 lakh + 2 crore", "20.5M")],
    );
    let big =
        Formatting { notation: Notation::Auto, notation_threshold: Some(Scale::Quadrillion), ..Formatting::default() };
    answers(Config { formatting: big, ..base() }, &[("$2 * 10^15", "$2P"), ("$5 * 10^12", "$5,000,000,000,000.00")]);
}

#[test]
fn symbols_places_and_days() {
    let symbols = Formatting {
        unit_symbols: vec![UnitSymbol { unit: "kWh".into(), symbol: "kW·h".into() }],
        ..Formatting::default()
    };
    answers(Config { formatting: symbols, ..base() }, &[("3 kWh * 2", "6 kW·h"), ("2 kW·h in J", "7,200,000 J")]);

    let office = CustomPlace {
        aliases: vec!["HQ".into()],
        kind: PlaceKind::Region,
        ..CustomPlace::new("Head Office", TimeZone::get("Asia/Tokyo").unwrap())
    };
    let config = Config {
        custom_places: vec![office],
        named_days: vec![NamedDay { name: "Company Day".into(), date: date(2026, 11, 2) }],
        ..base()
    };
    let calc = Calculator::with_config(config.clone());
    assert_eq!(calc.place("hq").unwrap().kind, PlaceKind::Region);
    answers(
        config,
        &[
            ("3:30 am in hq", "16:30 JST"),
            ("days until company day", "40 days"),
            ("Company Day + 1 day", "Tue, 3 Nov 2026"),
        ],
    );
}

#[test]
fn flags() {
    let mut lenient = Config { decimal_comma: true, ..base() };
    answers(lenient.clone(), &[("1,000 + 1", "2")]);
    lenient.features.lenient_comma_thousands = true;
    answers(lenient, &[("1,000 + 1", "1.001"), ("1,5 + 1", "2,5")]);

    let mut config = base();
    config.features.set_remote_queries(false);
    assert!(!config.features.historical_currency_queries && !config.features.dynamic_place_queries);

    config.features.disabled_phrases = vec![Phrase::Percentages, Phrase::Growth];
    answers(config.clone(), &[("20 is what % of 200", "200"), ("10 + 10", "20")]);
    config.features.disabled_converters = vec![Converter::Hms];
    answers(config, &[("3.5 hours in hms", "3.5 hours"), ("3.5 hours in minutes", "210 minutes")]);
}

#[test]
fn locales() {
    let host = Config { custom_places: vec![CustomPlace::new("Base", TimeZone::UTC)], ..Config::for_locale("en_US") };
    let german = host.with_locale("de_DE");
    assert!(german.decimal_comma && german.clock_24h);
    assert_eq!(german.currency.as_deref(), Some("EUR"));
    assert_eq!(german.locale.as_deref(), Some("de_DE"));
    assert_eq!(german.custom_places.len(), 1);

    let mut sheet = Sheet::with_config(Config { now: base().now, ..Config::for_locale("de_DE") });
    sheet.set_text("1,5 + 1.234,5\nx = 2,25");
    sheet.convert_to_locale("en_US");
    assert_eq!(sheet.text(), "1.5 + 1,234.5\nx = 2.25");
    assert_eq!(sheet.lines()[0].answer().unwrap().unwrap().text(), "1,236");
}
