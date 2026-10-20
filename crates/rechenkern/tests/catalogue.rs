//! What hosts can look up and list: places, countries, airports and units.

use rechenkern::units::{Dim, registry};
use rechenkern::{Calculator, CustomPlace, Number, PlaceKind, Quantity};

#[test]
fn places_and_countries() {
    let calc = Calculator::new();
    let tokyo = calc.place("tokyo").unwrap();
    assert_eq!((tokyo.name.as_str(), tokyo.kind), ("Tokyo", PlaceKind::City));
    assert_eq!(tokyo.country.unwrap().code, "JP");
    assert_eq!(tokyo.country.unwrap().currency, Some("JPY"));
    assert!((tokyo.location.unwrap().latitude - 35.7).abs() < 0.1);

    let texas = calc.place("Texas").unwrap();
    assert_eq!(texas.kind, PlaceKind::Region);
    assert!(texas.location.is_none());
    assert_eq!(texas.country.unwrap().code, "US");

    let pst = calc.place("pst").unwrap();
    assert_eq!((pst.name.as_str(), pst.kind), ("PST", PlaceKind::TimeZone));
    assert!(pst.country.is_none());

    assert_eq!(calc.place("Germany").unwrap().country.unwrap().currency, Some("EUR"));
    assert_eq!(calc.place("JFK").unwrap().kind, PlaceKind::Airport);
    assert!(calc.place("Smallville").is_none());

    let mut calc = Calculator::new();
    let zone = jiff::tz::TimeZone::get("America/Chicago").unwrap();
    calc.config_mut()
        .custom_places
        .push(CustomPlace { coordinates: Some((39.0, -98.0)), ..CustomPlace::new("Smallville", zone) });
    let smallville = calc.place("smallville").unwrap();
    assert_eq!(smallville.country.unwrap().code, "US");
    assert_eq!(smallville.location.unwrap().longitude, -98.0);

    assert!(rechenkern::countries().len() > 240);
    assert_eq!(rechenkern::country("usa").unwrap().code, "US");
    assert_eq!(rechenkern::country("fr").unwrap().name, "France");
    assert!(rechenkern::cities().any(|c| c.name == "kyoto"));
}

#[test]
fn airports_near_a_city() {
    let calc = Calculator::new();
    let london = calc.place("London").unwrap().location.unwrap();
    let near: Vec<_> = rechenkern::airports_near(&london, 50.0).into_iter().map(|(a, _)| a.code).collect();
    assert_eq!(near.first().map(String::as_str), Some("LHR"));
    assert!(near.iter().any(|c| c == "LGW"));
}

#[test]
fn units_and_conversions() {
    let calc = Calculator::new();
    let km = calc.unit("kilometres").unwrap();
    let miles = calc.unit("mi").unwrap();
    let five = Quantity::new(Number::from_i64(5), km);
    let converted = calc.convert(&five, &miles).unwrap();
    assert_eq!(converted.number.round_dp(4).to_string(), "3.1069");
    assert!(calc.convert(&five, &calc.unit("kg").unwrap()).is_err());

    let lengths: Vec<_> = registry().units().filter(|(_, d)| d.dim == Dim::LENGTH).map(|(_, d)| &d.symbol).collect();
    assert!(lengths.iter().any(|s| *s == "km") && lengths.iter().any(|s| *s == "ly"));
    assert_eq!(Dim::SPEED.name(), "speed");
}
