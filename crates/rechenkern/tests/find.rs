//! Data in text: numbers, dates, amounts, places, links.

use jiff::tz::TimeZone;
use rechenkern::{Calculator, Config, DataKind};

/// Wed 23 Sep 2026 14:30 in New York.
fn calc() -> Calculator {
    Calculator::with_config(Config {
        now: Some("2026-09-23T14:30:00-04:00[America/New_York]".parse().unwrap()),
        time_zone: Some(TimeZone::get("America/New_York").unwrap()),
        ..Config::default()
    })
}

/// "kind text = value" for each piece found.
fn found(text: &str, kinds: &[DataKind]) -> Vec<String> {
    calc()
        .find(text, kinds)
        .iter()
        .map(|f| {
            let value = match (&f.value, &f.place) {
                (Some(value), _) => format!(" = {}", describe(value)),
                (None, Some(place)) => format!(" = {}", place.name),
                _ => String::new(),
            };
            format!("{:?} {}{value}", f.kind, f.text)
        })
        .collect()
}

fn describe(value: &rechenkern::Value) -> String {
    match value {
        rechenkern::Value::Moment(m) => m.time.strftime("%Y-%m-%d %H:%M").to_string(),
        rechenkern::Value::Quantity(q) => format!("{} {}", q.number, q.unit),
        rechenkern::Value::Percent(p) => format!("{p}%"),
        other => format!("{other:?}"),
    }
}

#[test]
fn numbers_and_dates() {
    assert_eq!(
        found("We sold 130 copies yesterday", &[DataKind::Number, DataKind::Date]),
        ["Number 130 = 130 ", "Date yesterday = 2026-09-22 00:00",]
    );
    assert_eq!(
        found("QF 123 tomorrow at 3pm", &[DataKind::Number, DataKind::Date]),
        ["Number 123 = 123 ", "Date tomorrow at 3pm = 2026-09-24 15:00",]
    );
    let weather = "On August 23, 2022 the temperature in Chicago was 68.3 ºF (with a humidity of 74%)";
    assert_eq!(
        found(weather, &[DataKind::Date, DataKind::Temperature, DataKind::Percentage, DataKind::City]),
        [
            "Date August 23, 2022 = 2022-08-23 00:00",
            "City Chicago = Chicago",
            "Temperature 68.3 ºF = 68.3 °F",
            "Percentage 74% = 74%",
        ]
    );
    assert_eq!(
        found("30k people, 50_000 visits and .5 of them", &[DataKind::Number]),
        ["Number 30k = 30000 ", "Number 50_000 = 50000 ", "Number .5 = 0.5 ",]
    );
}

#[test]
fn amounts_and_kinds() {
    let text = "Paid $999 for 3 GB of storage, 2 hours, 5 km run, at 01:30:45";
    assert_eq!(
        found(text, &[DataKind::Money, DataKind::DataSize, DataKind::Duration, DataKind::Length, DataKind::Laptime]),
        [
            "Money $999 = 999 USD",
            "DataSize 3 GB = 3 GB",
            "Duration 2 hours = 2 hour",
            "Length 5 km = 5 km",
            "Laptime 01:30:45 = Duration(Duration { span: 1h 30m 45s, anchor: None, laptime: true })"
        ]
    );
    assert_eq!(found("5 km", &[DataKind::Quantity]), ["Quantity 5 km = 5 km"]);
    assert_eq!(found("1559740303 and 42", &[DataKind::Timestamp]), ["Timestamp 1559740303 = 2019-06-05 09:11"]);
}

#[test]
fn places_links_and_more() {
    let text = "Flight from JFK to Tokyo, see https://example.com or mail me@example.com #travel, paid in EUR, true";
    assert_eq!(
        found(
            text,
            &[
                DataKind::Airport,
                DataKind::City,
                DataKind::Url,
                DataKind::Email,
                DataKind::Hashtag,
                DataKind::Currency,
                DataKind::Boolean
            ]
        ),
        [
            "Airport JFK = JFK",
            "City Tokyo = Tokyo",
            "Url https://example.com",
            "Email me@example.com",
            "Hashtag #travel",
            "Currency EUR",
            "Boolean true = Bool(true)",
        ]
    );
}

#[test]
fn dates_in_sentences() {
    let calc = calc();
    let day = |text: &str| calc.date_for(text).map(|m| m.time.date().to_string());
    assert_eq!(day("Tom's birthday party this sunday").as_deref(), Some("2026-09-27"));
    assert_eq!(day("Schedule the meeting for April 21st next year").as_deref(), Some("2027-04-21"));
    assert_eq!(day("June 5 the kids get off school").as_deref(), Some("2026-06-05"));
    assert_eq!(day("nothing here"), None);
    let (start, end) = calc.date_interval_for("The conference runs from March 3 to March 10").unwrap();
    assert_eq!(
        (start.time.date().to_string(), end.time.date().to_string()),
        ("2026-03-03".into(), "2026-03-10".into())
    );
    let (start, end) = calc.date_interval_for("Sales in Q3 2026 were up").unwrap();
    assert_eq!(
        (start.time.date().to_string(), end.time.date().to_string()),
        ("2026-07-01".into(), "2026-09-30".into())
    );
}

#[test]
fn scanning_in_order() {
    let calc = calc();
    let mut scanner = calc.scanner("QF 123 tomorrow, 2 seats for $450");
    assert_eq!(scanner.number().unwrap().to_string(), "123");
    assert_eq!(scanner.date().unwrap().time.date().to_string(), "2026-09-24");
    assert_eq!(scanner.number().unwrap().to_string(), "2");
    assert_eq!(scanner.scan(DataKind::Money).unwrap().text, "$450");
    assert!(scanner.number().is_err());
    assert_eq!(scanner.find(DataKind::Date).unwrap().text, "tomorrow");
}

#[test]
fn more_kinds() {
    use DataKind::*;
    let kinds = [
        HexNumber,
        BinaryNumber,
        Digits,
        Timecode,
        Resolution,
        FuelEconomy,
        TransferSpeed,
        Frequency,
        Power,
        Pressure,
        Angle,
        Number,
    ];
    let text = "0xFF and 0b101, code 007, at 01:00:10:12 on 1920x1080, 30 mpg, 100 Mbps, 440 Hz, 5 kW, 2 bar, 90°, 3.5";
    let kinds_found: Vec<String> =
        calc().find(text, &kinds).iter().map(|f| format!("{:?} {}", f.kind, f.text)).collect();
    assert_eq!(
        kinds_found,
        [
            "HexNumber 0xFF",
            "BinaryNumber 0b101",
            "Digits 007",
            "Timecode 01:00:10:12",
            "Resolution 1920x1080",
            "FuelEconomy 30 mpg",
            "TransferSpeed 100 Mbps",
            "Frequency 440 Hz",
            "Power 5 kW",
            "Pressure 2 bar",
            "Angle 90°",
            "Number 3.5",
        ]
    );
    let words = calc().find("5 km in miles", &[Whitespace, Keyword]);
    let words: Vec<String> = words.iter().map(|f| format!("{:?} {:?}", f.kind, f.text)).collect();
    assert_eq!(words, ["Whitespace \" \"", "Whitespace \" \"", "Keyword \"in\"", "Whitespace \" \""]);
}

#[test]
fn scanning_in_any_order() {
    let calc = calc();
    let mut scanner = calc.scanner("tomorrow QF 123");
    assert_eq!(scanner.take(DataKind::Number).unwrap().text, "123");
    assert_eq!(scanner.take(DataKind::Date).unwrap().text, "tomorrow");
    assert!(scanner.take(DataKind::Date).is_err());
}
