//! How answers are written, as SoulverCore's `FormattingPreferences`.

use jiff::tz::TimeZone;
use rechenkern::{
    Calculator, Config, ConversionRule, CurrencySymbol, Formatting, Notation, Number, RateTable, ResultConversions,
    ScalarSystem, Scale, SymbolPosition, TimespanStyle,
};

/// Wed 23 Sep 2026 14:30 in New York, with fixed rates.
fn config(formatting: Formatting) -> Config {
    Config {
        now: Some("2026-09-23T14:30:00-04:00[America/New_York]".parse().unwrap()),
        time_zone: Some(TimeZone::get("America/New_York").unwrap()),
        formatting,
        ..Config::default()
    }
}

fn answers_with(config: Config, lines: &[(&str, &str)]) {
    let mut calc = Calculator::with_config(config);
    let mut rates = RateTable::new();
    rates.insert("EUR", Number::parse("0.9").unwrap());
    rates.insert("SEK", Number::parse("10").unwrap());
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

fn answers(formatting: Formatting, lines: &[(&str, &str)]) {
    answers_with(config(formatting), lines);
}

fn standard() -> Formatting {
    Formatting::default()
}

#[test]
fn scales() {
    answers(
        standard(),
        &[
            ("$5M", "$5M"),
            ("$5M / 4", "$1.25M"),
            ("$5M / 3", "$1,666,666.67"),
            ("2 billion", "2G"),
            ("$2bn + $500M", "$2.5B"),
            ("2,000,000", "2,000,000"),
            ("5к", "5,000"),
        ],
    );
    answers(
        Formatting { notation: Notation::Off, ..standard() },
        &[("$5M", "$5,000,000.00"), ("2 billion", "2,000,000,000")],
    );
    answers(
        Formatting { notation: Notation::On, ..standard() },
        &[("2,500,000", "2.5M"), ("1234567", "1.234567M"), ("$1,500", "$1.5K"), ("999", "999")],
    );
    answers(
        Formatting { notation_threshold: Some(Scale::Trillion), ..standard() },
        &[("2^50", "1.125899907P"), ("2^30", "1,073,741,824")],
    );
    answers(Formatting { scalars: ScalarSystem::British, ..standard() }, &[("$2bn", "$2bn"), ("$5M", "$5m")]);
    answers(Formatting { scalars: ScalarSystem::Russian, ..standard() }, &[("5 млн RUB", "₽5 млн")]);
    answers(standard(), &[("5 млн RUB", "₽5M")]);
}

#[test]
fn decimal_places() {
    answers(
        Formatting { decimal_places: Some(2), ..standard() },
        &[("1/3", "0.33"), ("2/3 km", "0.67 km"), ("10/4", "2.5")],
    );
}

#[test]
fn scientific_notation() {
    answers(standard(), &[("10^21", "1e21"), ("10^20", "100,000,000,000,000,000,000"), ("10^-10", "1e-10")]);
    answers(
        Formatting { scientific_above: 6, scientific_below: 3, ..standard() },
        &[("1234567", "1.234567e6"), ("123456", "123,456"), ("0.0001", "1e-4"), ("0.001", "0.001")],
    );
}

#[test]
fn money() {
    answers(Formatting { currency_symbols: false, ..standard() }, &[("$5", "5.00 USD"), ("€3.5", "3.50 EUR")]);
    answers(Formatting { round_currency: false, ..standard() }, &[("$10 / 3", "$3.333333333"), ("$1.5", "$1.5")]);
    answers(Formatting { currency_trailing_zeros: false, ..standard() }, &[("$5", "$5"), ("$5.5", "$5.50")]);
    let symbols = vec![
        CurrencySymbol { code: "SEK".into(), symbol: "kr".into(), position: SymbolPosition::AfterWithSpace },
        CurrencySymbol { code: "EUR".into(), symbol: "€".into(), position: SymbolPosition::AfterWithSpace },
    ];
    answers(
        Formatting { symbols, ..standard() },
        &[("$10 in SEK", "100.00 kr"), ("50 kr in USD", "$5.00"), ("€5", "5.00 €"), ("€5M", "5M €")],
    );
}

#[test]
fn references() {
    let rounded = Config { precision: 3, ..config(Formatting { references_take_formatted_value: true, ..standard() }) };
    answers_with(rounded, &[("x = 1/3", "0.333"), ("x * 3", "0.999"), ("$10 / 3", "$3.33"), ("prev * 3", "$9.99")]);
    let exact = Config { precision: 3, ..config(standard()) };
    answers_with(exact, &[("x = 1/3", "0.333"), ("x * 3", "1")]);
}

#[test]
fn timespans() {
    let lines = |days: &'static str| [("Jan 1 to Mar 15", days)];
    answers(standard(), &lines("2 months 2 weeks"));
    answers(Formatting { timespans: TimespanStyle::MonthsAndDays, ..standard() }, &lines("2 months 14 days"));
    answers(Formatting { timespans: TimespanStyle::WeeksAndDays, ..standard() }, &lines("10 weeks 3 days"));
    answers(Formatting { timespans: TimespanStyle::Days, ..standard() }, &lines("73 days"));
}

#[test]
fn compound_imperial_units() {
    answers(
        Formatting { compound_imperial_units: true, ..standard() },
        &[("5.5 ft", "5 ft 6 in"), ("1.25 lb", "1 lb 4 oz"), ("6 ft", "6 ft"), ("2 m in ft", "6 ft 6.74 in")],
    );
}

#[test]
fn conversions() {
    answers(
        Formatting { conversions: ResultConversions::Automatic, ..standard() },
        &[
            ("1500 m", "1.5 km"),
            ("0.25 kg", "250 g"),
            ("2048 MB", "2.048 GB"),
            ("1500 m in m", "1,500 m"),
            ("5 ft", "5 ft"),
        ],
    );
    let rules = vec![
        ConversionRule::Unit { from: "mi".into(), to: "km".into() },
        ConversionRule::Kind { like: "kg".into(), to: "lb".into() },
        ConversionRule::TimeZone(TimeZone::get("Asia/Tokyo").unwrap()),
    ];
    answers(
        Formatting { conversions: ResultConversions::Rules(rules), ..standard() },
        &[
            ("10 mi", "16.09344 km"),
            ("10 ft", "10 ft"),
            ("2 kg + 500 g", "5.511556555 lb"),
            ("3pm", "Thu, 24 Sep 2026 04:00 JST"),
        ],
    );
}

#[test]
fn unformatted() {
    answers_with(
        config(standard()).unformatted(),
        &[("1/3", "0.3333333333333333333333333333"), ("$1234.5", "1234.5 USD"), ("$5M", "5000000 USD")],
    );
}
