//! Features a host turns off or reads another way, as SoulverCore's `EngineFeatureFlags`.

use jiff::tz::TimeZone;
use rechenkern::{Calculator, Config, Converter, Features, MisplacedSeparator, ZoneCodeCorrection};

/// Wed 23 Sep 2026 14:30 in New York.
fn config(features: Features) -> Config {
    Config {
        now: Some("2026-09-23T14:30:00-04:00[America/New_York]".parse().unwrap()),
        time_zone: Some(TimeZone::get("America/New_York").unwrap()),
        features,
        ..Config::default()
    }
}

fn answers(features: Features, lines: &[(&str, &str)]) {
    let mut calc = Calculator::with_config(config(features));
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

fn off() -> Features {
    Features::default()
}

#[test]
fn defaults() {
    answers(
        off(),
        &[
            ("5 km in m", "5,000 m"),
            ("sqrt 16", "4"),
            ("20 is what % of 200", "10%"),
            ("iPhone 16 Pro $999", "$999.00"),
            ("see https://example.com/a?b=5 for $5", "$5.00"),
            ("write to me@example.com $20", "$20.00"),
            ("coffee #food $5", "$5.00"),
            ("1,0000", "1"),
            ("Coffee - $12.50", "-$12.50"),
            ("- Zenvoice: $25", "$25.00"),
            ("* pears $4", "$4.00"),
            ("4 out of 10", "40%"),
            ("1:30", "01:30"),
        ],
    );
}

#[test]
fn turned_off() {
    answers(Features { units: false, ..off() }, &[("5 km + 2", "7"), ("$5 + $2", "$7.00")]);
    answers(Features { calendar_calculations: false, ..off() }, &[("today + 3 days", "<error: dates are turned off>")]);
    answers(Features { word_functions: false, ..off() }, &[("20 is what % of 200", "200")]);
    answers(Features { converters: false, ..off() }, &[("5 km in m", "5 km")]);
    answers(Features { variable_declarations: false, ..off() }, &[("rent = 5", "5"), ("rent", "<none>")]);
    answers(Features { functions_require_parentheses: true, ..off() }, &[("sqrt 16", "16"), ("sqrt(16)", "4")]);
    answers(
        Features { disabled_functions: vec!["average".into()], ..off() },
        &[("avg(2, 4)", "<error: unknown function \"avg\">"), ("max(2, 4)", "4")],
    );
    answers(
        Features { disabled_converters: vec![Converter::Hexadecimal, Converter::Rounding], ..off() },
        &[("255 in hex", "255"), ("255 in binary", "0b11111111"), ("1/3 to 2 dp", "0.3333333333 – 2")],
    );
    answers(
        Features { in_ambiguity_prefer_something_to_nothing: false, ..off() },
        &[("iPhone 16 Pro $999", "<error: didn't understand \"$999\">")],
    );
    answers(Features { url_detection: false, ..off() }, &[("see https://example.com for $5", "<none>")]);
}

#[test]
fn variables_after_dividers() {
    answers(Features::default(), &[("rent = 5", "5"), ("---", "<none>"), ("rent", "<none>")]);
    answers(
        Features { dividers_reset_variable_scope: false, ..off() },
        &[("rent = 5", "5"), ("---", "<none>"), ("rent", "5")],
    );
}

#[test]
fn labels_and_markers() {
    answers(
        Features { dash_separated_labels: true, ..off() },
        &[("Coffee - $12.50", "$12.50"), ("100 - 12.50", "87.5"), ("Coffee -12.50", "-12.5")],
    );
    answers(
        Features { ordinal_label_parsing: true, ..off() },
        &[("Day 8 $50", "$50.00"), ("Week 4 30 km", "30 km"), ("day 8", "8")],
    );
    answers(Features { list_marker_parsing: false, ..off() }, &[("1. 5 + 5", "10")]);
    answers(
        Features { labels_as_variables: true, ..off() },
        &[("Rent: $1,500", "$1,500.00"), ("Phone bill: $40", "$40.00"), ("rent + phone bill", "$1,540.00")],
    );
    answers(off(), &[("Rent: $1,500", "$1,500.00"), ("rent * 2", "2")]);
    let mut comments = off();
    comments.comments.labels = false;
    comments.comments.brackets = false;
    answers(comments, &[("Rent: $1,500", "$1,500.00"), ("$999 (for iPhone 16)", "$15,984.00")]);
}

#[test]
fn numbers() {
    answers(
        Features { misplaced_thousands_separator: MisplacedSeparator::Allow, ..off() },
        &[("1,0000", "10,000"), ("1,50", "1.5")],
    );
    answers(Features { proper_fraction_parsing: true, ..off() }, &[("4 out of 10", "⅖"), ("10 of 20", "50%")]);
}

#[test]
fn dates_and_zones() {
    let mut f = off();
    f.dates.prefer_daytime_for_ambiguous_12_hour_times = true;
    f.dates.prefer_future_for_ambiguous_dates = true;
    f.dates.leading_year_parsing = true;
    f.dates.noon_for_single_dates = true;
    answers(
        f,
        &[
            ("1:30", "13:30"),
            ("9:30", "09:30"),
            ("March 5", "Fri, 5 Mar 2027"),
            ("2025 March 5", "Wed, 5 Mar 2025"),
            ("March 5 2027 + 12 hours", "Sat, 6 Mar 2027 00:00"),
        ],
    );
    let mut f = off();
    f.time_zones.require_uppercase_for_ambiguous_time_zones = true;
    answers(f, &[("time in PST", "11:30 PDT"), ("time in pst", "<error: unknown place \"pst\">")]);
    let mut f = off();
    f.time_zones.code_correction = ZoneCodeCorrection::Never;
    answers(f, &[("time in EST", "13:30 UTC-5")]);
}
