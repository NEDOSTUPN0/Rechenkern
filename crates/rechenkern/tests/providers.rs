//! Data a host provides: exchange rates of other days, inflation, weather, places.

use jiff::civil::Date;
use jiff::tz::TimeZone;
use rechenkern::{Calculator, Config, Features, Number, RateProvider, RateTable};

/// Wed 23 Sep 2026 14:30 in New York.
fn config() -> Config {
    Config {
        now: Some("2026-09-23T14:30:00-04:00[America/New_York]".parse().unwrap()),
        time_zone: Some(TimeZone::get("America/New_York").unwrap()),
        ..Config::default()
    }
}

fn answers(calc: &mut Calculator, lines: &[(&str, &str)]) {
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

/// Euros at 0.9 today and at 0.8 on any other day; nothing else.
struct Euros;

impl RateProvider for Euros {
    fn rates(&self) -> Result<RateTable, String> {
        let mut table = RateTable::new();
        table.insert("EUR", Number::parse("0.9").unwrap());
        Ok(table)
    }

    fn rates_on(&self, date: Date) -> Result<RateTable, String> {
        if date.year() < 2000 {
            return Err(format!("no rates for {date}"));
        }
        let mut table = RateTable::new();
        table.insert("EUR", Number::parse("0.8").unwrap());
        Ok(table)
    }
}

#[test]
fn rates_of_other_days() {
    let mut calc = Calculator::with_config(config());
    calc.set_rate_provider(Euros);
    answers(
        &mut calc,
        &[
            ("$100 in EUR", "€90.00"),
            ("$100 in EUR on Jan 1, 2020", "€80.00"),
            ("$100 in EUR 2020-01-01", "€80.00"),
            ("$100 to EUR on March 5", "€80.00"),
            ("$100 in EUR on Jan 1, 1990", "<error: no rates for 1990-01-01>"),
            ("$100 in GBP on Jan 1, 2020", "<error: no exchange rate for GBP on 2020-01-01>"),
        ],
    );
    let answer = calc.calculate("$100 in EUR on Jan 1, 2020").unwrap().unwrap();
    assert_eq!(answer.source().unwrap().text, "$100.00");
}

#[test]
fn bundled_rates_fill_in() {
    let mut calc = Calculator::with_config(config());
    calc.set_rate_provider(Euros);
    let answer = calc.calculate("$100 in GBP").unwrap().unwrap();
    assert!(answer.text().starts_with('£'), "{answer}");
    let features = Features { use_default_rates_for_unhandled_currencies: false, ..Features::default() };
    let mut strict = Calculator::with_config(Config { features, ..config() });
    strict.set_rate_provider(Euros);
    answers(&mut strict, &[("$100 in GBP", "<error: no exchange rate for GBP>"), ("$100 in EUR", "€90.00")]);
    // Without any provider the bundled rates are all there is.
    let answer = Calculator::with_config(config()).calculate("€100 in USD").unwrap().unwrap();
    assert!(answer.text().starts_with('$'), "{answer}");
}

#[test]
fn rates_of_other_days_can_be_turned_off() {
    let features = Features { historical_currency_queries: false, ..Features::default() };
    let mut calc = Calculator::with_config(Config { features, ..config() });
    calc.set_rate_provider(Euros);
    answers(&mut calc, &[("$100 in EUR on Jan 1, 2020", "<error: exchange rates of other days are turned off>")]);
}

/// Prices that double every ten years from 2000, in euros only.
struct Doubling;

impl rechenkern::InflationProvider for Doubling {
    fn cpi(&self, currency: &str, year: i16, _month: Option<i8>) -> Option<Number> {
        (currency == "EUR").then(|| Number::from_f64(2f64.powf((year - 2000) as f64 / 10.0)))
    }
}

#[test]
fn inflation() {
    let mut calc = Calculator::with_config(config());
    answers(
        &mut calc,
        &[
            ("$100 from 1990 in 2000", "$131.75"),
            ("$100 from 2000 in 1990", "$75.90"),
            ("what will $100 be worth in 2035 with 3% inflation", "$76.64"),
            ("value of $100 in 2035 with 3% inflation", "$76.64"),
            ("$100 from 1800 in 2000", "<error: no inflation data for 1800>"),
            ("€100 in 1990 euros", "<error: no inflation data for EUR>"),
            ("Born in 1990 $5", "$5.00"),
        ],
    );
    // Today's prices are the latest month known.
    let today = calc.calculate("$100 from 1990 is worth what today").unwrap().unwrap();
    assert!(today.text() > "$250", "{today}");
    calc.set_inflation_provider(Doubling);
    answers(
        &mut calc,
        &[
            ("€100 from 2000 in 2020", "€400.00"),
            ("€1,000 in 2016 euros", "€2,000.00"),
            ("value of €100 in 2016", "€50.00"),
            ("$100 from 1990 in 2000", "$131.75"),
        ],
    );
}

/// Knows Smallville, which keeps Tokyo's time, and counts how often it is asked.
struct Atlas(std::sync::atomic::AtomicUsize);

impl rechenkern::PlaceProvider for Atlas {
    fn place(&self, name: &str) -> Option<rechenkern::CustomPlace> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut place = rechenkern::CustomPlace::new("Smallville", TimeZone::get("Asia/Tokyo").unwrap());
        place.coordinates = Some((35.68, 139.77));
        name.eq_ignore_ascii_case("smallville").then_some(place)
    }
}

#[test]
fn places_from_a_provider() {
    let mut calc = Calculator::with_config(config());
    answers(&mut calc, &[("time in Smallville", "<error: unknown place \"Smallville\", did you mean \"Millville\"?>")]);
    calc.set_place_provider(Atlas(Default::default()));
    answers(
        &mut calc,
        &[
            ("time in Smallville", "Thu, 24 Sep 2026 03:30 JST"),
            ("3pm in smallville", "Thu, 24 Sep 2026 04:00 JST"),
            ("sunrise in Smallville", "Thu, 24 Sep 2026 05:30 JST"),
            ("time in Gotham", "<error: unknown place \"Gotham\", did you mean \"Gotha\"?>"),
            ("Smallville $5", "$5.00"),
        ],
    );
    let answer = calc.calculate("time in smallville").unwrap().unwrap();
    assert_eq!(answer.target().unwrap().label.as_deref(), Some("Smallville"));
    let features = Features { dynamic_place_queries: false, ..Features::default() };
    let mut calc = Calculator::with_config(Config { features, ..config() });
    calc.set_place_provider(Atlas(Default::default()));
    answers(&mut calc, &[("time in Smallville", "<error: unknown place \"Smallville\", did you mean \"Millville\"?>")]);
}

/// Mild weather everywhere, colder on other days; remembers the last request.
struct Mild(std::sync::Mutex<Option<rechenkern::WeatherRequest>>);

impl rechenkern::WeatherProvider for Mild {
    fn weather(&self, request: &rechenkern::WeatherRequest) -> Result<rechenkern::Weather, String> {
        *self.0.lock().unwrap() = Some(request.clone());
        let other_day = request.date.is_some();
        Ok(rechenkern::Weather {
            temperature: Some(if other_day { 10.0 } else { 18.5 }),
            high: Some(21.0),
            low: Some(14.0),
            humidity: Some(60.0),
            wind_speed: Some(12.0),
            wind_direction: Some(290.0),
            uv_index: Some(3.0),
            chance_of_rain: None,
            condition: Some("partly cloudy".into()),
        })
    }
}

#[test]
fn weather() {
    let mut calc = Calculator::with_config(config());
    answers(&mut calc, &[("weather in Tokyo", "<error: the weather needs a weather provider>")]);
    let provider = std::sync::Arc::new(Mild(Default::default()));
    struct Shared(std::sync::Arc<Mild>);
    impl rechenkern::WeatherProvider for Shared {
        fn weather(&self, request: &rechenkern::WeatherRequest) -> Result<rechenkern::Weather, String> {
            self.0.weather(request)
        }
    }
    calc.set_weather_provider(Shared(provider.clone()));
    answers(
        &mut calc,
        &[
            ("weather in Tokyo", "18.5 °C (partly cloudy)"),
            ("Tokyo temperature", "18.5 °C"),
            ("temp in London in °F", "65.3 °F"),
            ("low in Paris", "14 °C"),
            ("Paris high", "21 °C"),
            ("high temp in Paris", "21 °C"),
            ("low temp in Paris", "14 °C"),
            ("Tokyo wind direction", "290° (W)"),
            ("humidity in Tokyo", "60%"),
            ("wind speed in London", "12 km/h"),
            ("uv index in Sydney", "3"),
            ("chance of rain in London", "<error: the weather provider doesn't know that>"),
            ("weather in Tokyo tomorrow", "10 °C (partly cloudy)"),
            ("weather", "18.5 °C (partly cloudy)"),
            ("high five", "5"),
            ("weather in Gotham", "<error: unknown place \"Gotham\", did you mean \"Gotha\"?>"),
        ],
    );
    calc.calculate("weather in Tokyo tomorrow").unwrap();
    let request = provider.0.lock().unwrap().clone().unwrap();
    assert_eq!((request.place.as_str(), request.date.map(|d| d.to_string())), ("Tokyo", Some("2026-09-25".into())));
    calc.calculate("weather").unwrap();
    assert_eq!(provider.0.lock().unwrap().as_ref().unwrap().place, "New York");
    let answer = calc.calculate("weather in Tokyo").unwrap().unwrap();
    assert_eq!(answer.copy_text(), "18.5 °C");
}
