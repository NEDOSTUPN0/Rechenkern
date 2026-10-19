//! The shape of answers: form, the two sides of a line, notes and text to copy.

use jiff::tz::TimeZone;
use rechenkern::{Calculator, Config, Number, RateTable, Side};

/// A calculator at Wed 23 Sep 2026 14:30 in New York, with fixed rates.
fn calculator() -> Calculator {
    let config = Config {
        now: Some("2026-09-23T14:30:00-04:00[America/New_York]".parse().unwrap()),
        time_zone: Some(TimeZone::get("America/New_York").unwrap()),
        ..Config::default()
    };
    let mut calc = Calculator::with_config(config);
    let mut rates = RateTable::new();
    rates.insert("EUR", Number::parse("0.9").unwrap());
    calc.set_rates(rates);
    calc
}

/// "form | source text (label) | target text (label) | note | copy", leaving out empty parts.
fn shape(line: &str) -> String {
    let answer = match calculator().calculate(line) {
        Ok(Some(answer)) => answer,
        Ok(None) => return "<none>".into(),
        Err(e) => return format!("<error: {e}>"),
    };
    let side = |side: &Side| match &side.label {
        Some(label) => format!("{} ({label})", side.text),
        None => side.text.clone(),
    };
    let mut parts = vec![answer.form().name().to_string()];
    if let (Some(source), Some(target)) = (answer.source(), answer.target()) {
        parts.push(side(source));
        parts.push(side(target));
    }
    if let Some(note) = answer.note() {
        parts.push(note.to_string());
    }
    if answer.copy_text() != answer.text() {
        parts.push(format!("copy {}", answer.copy_text()));
    }
    parts.join(" | ")
}

fn check(cases: &[(&str, &str)]) {
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(input, expected)| {
            let got = shape(input);
            (got != *expected).then(|| format!("{input:30} expected {expected}\n{:30}      got {got}", ""))
        })
        .collect();
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

#[test]
fn conversions() {
    check(&[
        ("500 km to mi", "conversion | 500 km (Kilometers) | 310.6855961 mi (Miles)"),
        ("10 usd in eur", "conversion | $10.00 (US Dollars) | €9.00 (Euros)"),
        ("100 f in c", "conversion | 100 °F (Fahrenheit) | 37.77777778 °C (Celsius)"),
        ("60 mph in m/s", "conversion | 60 mph (Miles per Hour) | 26.8224 m/s (Meters per Second)"),
        ("$30/hour in per day", "conversion | $30.00/hour (US Dollars per Hour) | $720.00/day (US Dollars per Day)"),
        ("20 m^2 in sq ft", "conversion | 20 m² (Square Meters) | 215.2782083 ft² (Square Feet)"),
        ("5.5 ft in feet and inches", "conversion | 5.5 ft (Feet) | 5 ft 6 in (Feet and Inches)"),
        ("1 kWh in J", "conversion | 1 kWh (Kilowatt Hours) | 3,600,000 J (Joules)"),
        ("255 in hex", "conversion | 255 | 0xFF (Hexadecimal)"),
        ("3 hours 30 minutes in minutes", "conversion | 3 hours 30 minutes | 210 minutes (Minutes)"),
    ]);
}

#[test]
fn times_and_places() {
    check(&[
        ("time in paris", "time_at_place | 14:30 (New York) | 20:30 (Paris) | copy 20:30"),
        ("Tokyo time", "time_at_place | 14:30 (New York) | 03:30 (Tokyo) | +1 day | copy 03:30"),
        ("date in Tokyo", "time_at_place | Wed, 23 Sep 2026 (New York) | Thu, 24 Sep 2026 (Tokyo) | +1 day"),
        ("15:00 berlin in tokyo", "time_conversion | 15:00 (Berlin) | 22:00 (Tokyo) | copy 22:00"),
        ("11pm berlin in tokyo", "time_conversion | 23:00 (Berlin) | 06:00 (Tokyo) | +1 day | copy 06:00"),
        ("3pm in Tokyo", "time_conversion | 15:00 (New York) | 04:00 (Tokyo) | +1 day | copy 04:00"),
        ("PST to EST", "time_conversion | 11:30 (PST) | 14:30 (EST) | copy 14:30"),
        ("time in utc+5:30", "time_at_place | 14:30 (New York) | 00:00 (UTC+5:30) | +1 day | copy 00:00"),
        ("lax time", "time_at_place | 14:30 (New York) | 11:30 (LAX) | copy 11:30"),
        ("time in Tokyo when 9am in London", "time_conversion | 09:00 (London) | 17:00 (Tokyo) | copy 17:00"),
        ("time diff Tokyo London", "time_difference | 03:30 (Tokyo) | 19:30 (London) | -1 day"),
        ("3pm Tokyo", "value | copy 15:00"),
    ]);
}

#[test]
fn dates() {
    check(&[
        ("today + 30 days", "date_shift | Wed, 23 Sep 2026 | Fri, 23 Oct 2026"),
        ("3 days ago", "date_shift | Wed, 23 Sep 2026 | Sun, 20 Sep 2026"),
        ("days until christmas", "date_interval | Wed, 23 Sep 2026 | Fri, 25 Dec 2026"),
        ("Jan 1 to Mar 1", "date_interval | Thu, 1 Jan 2026 | Sun, 1 Mar 2026"),
        ("today", "value"),
        ("GTA 6 release date", "value | copy Thu, 19 Nov 2026"),
    ]);
}

#[test]
fn calculations_and_values() {
    check(&[
        ("2+2*3", "calculation"),
        ("20% of 150", "calculation"),
        ("5 km", "value"),
        ("5 ft 3 in", "value"),
        ("-5", "value"),
        ("x = 5 km in mi", "conversion | 5 km (Kilometers) | 3.106855961 mi (Miles)"),
    ]);
}

/// "kind text" of each alternative.
fn alternatives(line: &str) -> Vec<String> {
    let mut calc = calculator();
    let answer = calc.calculate(line).unwrap().unwrap();
    calc.alternatives(&answer).iter().map(|a| format!("{:?} {}", a.kind, a.text)).collect()
}

#[test]
fn alternative_results() {
    assert_eq!(alternatives("255"), ["Hexadecimal 0xFF", "Binary 0b11111111", "Octal 0o377"]);
    assert_eq!(alternatives("0.375"), ["Fraction ⅜"]);
    assert_eq!(alternatives("20%"), ["Decimal 0.2", "Fraction ⅕"]);
    assert_eq!(alternatives("1 / 7 * 10^9")[..1], ["Scientific 1.428571429e8"]);
    let km = alternatives("5 km");
    assert!(km.contains(&"Unit 3.106855961 mi".to_string()), "{km:?}");
    assert!(!km.iter().any(|a| a.ends_with(" km")), "{km:?}");
    assert_eq!(alternatives("100 °C"), ["Unit 212 °F", "Unit 373.15 K"]);
    let hours = alternatives("3.5 hours");
    assert!(hours.contains(&"Laptime 03:30:00".to_string()), "{hours:?}");
    let today = alternatives("3pm");
    assert_eq!(
        today,
        ["Date Wednesday, 23 September 2026 15:00", "Iso8601 2026-09-23T15:00:00-04:00", "Timestamp 1790190000"]
    );
}

#[test]
fn typed_values() {
    use rechenkern::Value;
    let value = |line: &str| calculator().calculate(line).unwrap().unwrap().value().clone();
    assert!(matches!(value("5-10 kg"), Value::Range(..)));
    assert!(matches!(value("10 usd in eur, gbp"), Value::List(v) if v.len() == 2));
    assert!(matches!(value("440 Hz as pitch"), Value::Pitch(p) if p.midi == 69));
    assert!(matches!(value("coordinates of Tokyo"), Value::Coordinates(c) if c.latitude > 35.0));
    assert!(matches!(value("1920x1080"), Value::Resolution(r) if r.width == 1920 && r.height == 1080));
    assert!(matches!(value("100 frames at 24 fps"), Value::Timecode(t) if t.frames == 100));
    let Value::Cadence(c) = value("every 2 weeks until Oct 31") else { panic!("not a cadence") };
    assert_eq!(c.dates().len(), 3);
    let Value::Duration(span) = value("Jan 1 to Mar 1") else { panic!("not a span") };
    assert_eq!(span.start().unwrap().date().to_string(), "2026-01-01");
    assert_eq!(span.end().unwrap().date().to_string(), "2026-03-01");
}
