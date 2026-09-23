//! End-to-end checks: input line -> formatted answer.

use jiff::tz::TimeZone;
use rechenkern::{Calculator, Config, Number, RateTable};

/// A calculator at Wed 23 Sep 2026 14:30 in New York, with fixed rates.
fn calculator() -> Calculator {
    let config = Config {
        now: Some("2026-09-23T14:30:00-04:00[America/New_York]".parse().unwrap()),
        time_zone: Some(TimeZone::get("America/New_York").unwrap()),
        ..Config::default()
    };
    let mut calc = Calculator::with_config(config);
    let mut rates = RateTable::new();
    for (code, rate) in
        [("EUR", "0.9"), ("GBP", "0.8"), ("JPY", "150"), ("KZT", "500"), ("RUB", "90"), ("BTC", "0.00001")]
    {
        rates.insert(code, Number::parse(rate).unwrap());
    }
    calc.set_rates(rates);
    calc
}

/// Checks each line on a fresh calculator and reports all mismatches at once.
fn check(cases: &[(&str, &str)]) {
    let mut failures = Vec::new();
    for (input, expected) in cases {
        let got = match calculator().calculate(input) {
            Ok(Some(answer)) => answer.to_string(),
            Ok(None) => "<none>".into(),
            Err(e) => format!("<error: {e}>"),
        };
        if got != *expected {
            failures.push(format!("{input:45} expected {expected:25} got {got}"));
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

/// Runs a sheet and returns the answer of its last line.
fn sheet(text: &str) -> String {
    let results = calculator().calculate_sheet(text);
    match results.last() {
        Some(Ok(Some(answer))) => answer.to_string(),
        Some(Ok(None)) => "<none>".into(),
        Some(Err(e)) => format!("<error: {e}>"),
        None => "<empty>".into(),
    }
}

#[test]
fn arithmetic() {
    check(&[
        ("2+2", "4"),
        ("0.1 + 0.2", "0.3"),
        ("2 + 3 * 4", "14"),
        ("(2 + 3) * 4", "20"),
        ("2^10", "1,024"),
        ("-2^2", "-4"),
        ("2^3^2", "512"),
        ("10 / 4", "2.5"),
        ("1/3", "0.3333333333"),
        ("7 mod 3", "1"),
        ("10 % 3", "1"),
        ("5!", "120"),
        ("3 x 4", "12"),
        ("30 plus 20", "50"),
        ("3,000 minus 12", "2,988"),
        ("3 multiplied by 4", "12"),
        ("1,000 divided by 200", "5"),
        ("3 to the power of 2", "9"),
        ("remainder of 21 divided by 5", "1"),
        ("2(3+4)", "14"),
        ("2 pi", "6.283185307"),
        ("1.5e3", "1,500"),
        ("2^64", "18,446,744,073,709,551,616"),
        ("2^100", "1.2676506e30"),
        ("5k", "5,000"),
        ("2.5M", "2,500,000"),
        ("3 million", "3,000,000"),
        ("2 dozen", "24"),
        ("half of 175", "87.5"),
        ("twice 21", "42"),
        ("seven times six", "42"),
        ("1/0", "<error: division by zero>"),
        ("2^99999", "<error: result is too large>"),
    ]);
}

#[test]
fn decimal_separators() {
    check(&[
        ("10,50 eur", "€10.50"),
        ("€10,50 + €2,25", "€12.75"),
        ("1,5 + 2,5", "4"),
        ("1.234,56 * 2", "2,469.12"),
        ("1,500 + 1", "1,501"),
        ("1.500 + 1", "2.5"),
        ("max(1,5)", "5"),
        ("sum of 1,2,3", "6"),
    ]);
    let mut calc = calculator();
    calc.config_mut().decimal_comma = true;
    for (input, expected) in [
        ("1,500 + 1", "2,5"),
        ("1.500 + 1", "1.501"),
        ("10.50 eur", "€10,50"),
        ("$1,234,567.8", "$1.234.567,80"),
        ("10 USD in EUR, JPY", "€9,00; ¥1.500"),
    ] {
        assert_eq!(calc.calculate(input).unwrap().unwrap().text(), expected, "{input}");
    }
}

#[test]
fn bases_and_bits() {
    check(&[
        ("0xFF + 1", "256"),
        ("0b101", "5"),
        ("255 in hex", "0xFF"),
        ("99 in binary", "0b1100011"),
        ("0x9F31 to decimal", "40,753"),
        ("0b1000101 to octal", "0o105"),
        ("0xFF & 0x0F", "15"),
        ("1 << 4", "16"),
        ("5 xor 3", "6"),
        ("hex(99)", "0x63"),
    ]);
}

#[test]
fn functions() {
    check(&[
        ("sqrt(16)", "4"),
        ("sqrt 2", "1.414213562"),
        ("√81", "9"),
        ("square root of 81", "9"),
        ("cube root of 27", "3"),
        ("cbrt(343)", "7"),
        ("root 5 of 100", "2.511886432"),
        ("log(1000)", "3"),
        ("ln(e)", "1"),
        ("log 20 base 4", "2.160964047"),
        ("log2(8)", "3"),
        ("sin(pi/2)", "1"),
        ("sin(90 degrees)", "1"),
        ("sind(90)", "1"),
        ("cos(0)", "1"),
        ("sin(pi)", "0"),
        ("asind(0.5)", "30°"),
        ("abs(-5)", "5"),
        ("round(2.5)", "3"),
        ("floor(2.7)", "2"),
        ("ceil(2.1)", "3"),
        ("fact(5)", "120"),
        ("gcd of 20 and 30", "10"),
        ("lcm of 5 and 8", "40"),
        ("total of 3, 4, 7 and 9", "23"),
        ("average of 36, 42, 19 and 81", "44.5"),
        ("median of 10, 20 and 30", "20"),
        ("count of 1, 2, 3, 4, 5", "5"),
        ("standard deviation of 20, 30 and 40", "10"),
        ("larger of 100 and 200", "200"),
        ("smaller of 5 and 10", "5"),
        ("max(3, 7, 2)", "7"),
        ("midpoint between 150 and 300", "225"),
        ("10 permutation 3", "720"),
        ("25 combination 3", "2,300"),
        ("3 permutations of 10", "720"),
        ("clamp 26 between 5 and 25", "25"),
        ("sqrt(16 m^2)", "4 m"),
    ]);
}

#[test]
fn rounding() {
    check(&[
        ("1/3 to 2 dp", "0.33"),
        ("pi to 5 digits", "3.14159"),
        ("5.5 rounded", "6"),
        ("5.5 rounded down", "5"),
        ("5.5 rounded up", "6"),
        ("37 to nearest 10", "40"),
        ("$490 rounded to nearest hundred", "$500.00"),
        ("2,100 to nearest thousand", "2,000"),
        ("21 rounded up to nearest 5", "25"),
        ("17 rounded down to nearest 3", "15"),
        ("0.534 to nearest 16th", "9/16"),
        ("round 1/3 to 2 dp", "0.33"),
        ("123456 to 2 sf", "120,000"),
    ]);
}

#[test]
fn percentages() {
    check(&[
        ("20% of 50", "10"),
        ("25% × 200", "50"),
        ("200 + 10%", "220"),
        ("10% on 200", "220"),
        ("200 - 10%", "180"),
        ("10% off 200", "180"),
        ("20 is 10% of what", "200"),
        ("180 is 10% off what", "200"),
        ("220 is 10% on what", "200"),
        ("$150 is 25% on what", "$120.00"),
        ("50 to 75 is what %", "50%"),
        ("40 to 90 as %", "125%"),
        ("180 is what % off 200", "10%"),
        ("180 is what % on 150", "20%"),
        ("20 is what % of 200", "10%"),
        ("40 is what % of 90", "44.44444444%"),
        ("20 as a % of 200", "10%"),
        ("what % of 200 is 20", "10%"),
        ("10% of what is 20", "200"),
        ("20/200 as %", "10%"),
        ("0.35 as %", "35%"),
        ("2/5 as percent", "40%"),
        ("3/20 is what %", "15%"),
        ("10% + 20%", "30%"),
        ("90% - 40%", "50%"),
        ("30% + 0.4", "70%"),
        ("100% - 1/2", "50%"),
        ("50% × 30", "15"),
        ("30 × 50%", "15"),
        ("2/10 as fraction", "1/5"),
        ("50% as fraction", "1/2"),
        ("0.333333 as fraction", "333333/1000000"),
        ("81 is 9 to what power", "2"),
        ("2/3 of 600", "400"),
        ("50 is 1/5 of what", "250"),
        ("20/5 as multiplier", "4x"),
        ("50 to 75 is what x", "1.5x"),
        ("20% as dec", "0.2"),
        ("$10 for lunch + 15% tip", "$11.50"),
        ("6 is to 60 as 8 is to what", "80"),
        ("5 is to 10 as what is to 80", "40"),
    ]);
}

#[test]
fn units() {
    check(&[
        ("10 km in m", "10,000 m"),
        ("5 hours 30 minutes to seconds", "19,800 seconds"),
        ("100 pounds in kg", "45.359237 kg"),
        ("65 kg in pounds", "143.3004704 lb"),
        ("5 km in miles", "3.106855961 mi"),
        ("meters in 10 km", "10,000 m"),
        ("days in 3 weeks", "21 days"),
        ("seconds in a day", "86,400 seconds"),
        ("300 + 20 km", "320 km"),
        ("1km + 1,000m", "2 km"),
        ("10m × 10m", "100 m²"),
        ("5 m/s in km/h", "18 km/h"),
        ("60 mph in km/h", "96.56064 km/h"),
        ("100 km/h in mph", "62.13711922 mph"),
        ("90 km / 3 day", "30 km/day"),
        ("5 ft 3 in", "5 ft 3 in"),
        ("5'11\" in cm", "180.34 cm"),
        ("12.5 minutes in minutes and seconds", "12 minutes 30 seconds"),
        ("4.5 weeks in days and hours", "31 days 12 hours"),
        ("5.5 minutes as timespan", "5 minutes 30 seconds"),
        ("72 days as timespan", "10 weeks 2 days"),
        ("5.5 minutes as laptime", "00:05:30"),
        ("03:04:05 + 01:02:03", "04:06:08"),
        ("3h 5m 10s in seconds", "11,110 seconds"),
        ("1 GB in MB", "1,000 MB"),
        ("1 GiB in MiB", "1,024 MiB"),
        ("8 bits in bytes", "1 B"),
        ("100 f to c", "37.77777778 °C"),
        ("0 c in f", "32 °F"),
        ("300 K in C", "26.85 °C"),
        ("1 acre in m2", "4,046.856422 m²"),
        ("1 cup in ml", "236.5882365 mL"),
        ("1 kWh in J", "3,600,000 J"),
        ("5 kWh / 2 h", "2.5 kW"),
        ("180 deg in rad", "3.141592654 rad"),
        ("2 sq ft in sq in", "288 in²"),
        ("3 cubic feet in liters", "84.95053978 L"),
        ("20km == 20,000 m", "true"),
        ("5 km in kg", "<error: can't convert km to kg>"),
        ("1 light year in km", "9,460,730,472,581 km"),
        ("1 mb in kb", "1,000 kB"),
        ("5 Mb in kB", "625 kB"),
        ("180 cm in feet and inches", "5 ft 10.87 in"),
        ("72 in in feet and inches", "6 ft"),
        ("1 day - 2 hours", "22 hours"),
        ("1 hour + 30 minutes", "1 hour 30 minutes"),
        ("1 year - 2 months", "10 months"),
        ("100 km / 50 km/h", "2 hours"),
        ("time to upload 3GB at 10 MB/s", "5 minutes"),
        ("1 hour 30 minutes at 1.5x", "1 hour"),
        ("speed of light in km/h", "1,079,252,849 km/h"),
    ]);
}

#[test]
fn rates() {
    check(&[
        ("3 hours / day", "3 h/day"),
        ("$99 per week", "$99.00/week"),
        ("30 bottles / week", "30/week"),
        ("$20/day + $300/week", "$440.00/week"),
        ("€30/day in €/month", "€913.11/month"),
        ("$50/week × 12 weeks", "$600.00"),
        ("$25/hour * 14 hours of work", "$350.00"),
        ("30 hours at $30/hour", "$900.00"),
        ("$500 at $20/hour", "25 hours"),
        ("$30 × 4 days", "$120.00"),
        ("$24 a day for a year", "$8,765.82"),
        ("twice a day", "2/day"),
        ("3 times a week", "3/week"),
        ("5 km a day for 2 weeks", "70 km"),
        ("4 nights * $120/night", "$480.00"),
        ("$100 split 4 ways", "$25.00"),
        ("60 mph for 2.5 hours", "150 mi"),
        ("1 m/s^2 * 2 s", "2 m/s"),
    ]);
}

#[test]
fn currencies() {
    check(&[
        ("10 usd to eur", "€9.00"),
        ("10 USD in EUR", "€9.00"),
        ("$5k in eur", "€4,500.00"),
        ("€9 in dollars", "$10.00"),
        ("£8 in yen", "¥1,500"),
        ("500 tenge in rubles", "₽90.00"),
        ("1 btc in usd", "$100,000.00"),
        ("$100 in btc", "0.001 BTC"),
        ("$200 + €200", "€380.00"),
        ("$20 + 30", "$50.00"),
        ("$5m", "$5,000,000.00"),
        ("$3bn", "$3,000,000,000.00"),
        ("USD 20", "$20.00"),
        ("20 dollars", "$20.00"),
        ("15 bucks", "$15.00"),
        ("$19 for breakfast + $22 for the uber", "$41.00"),
        ("10 USD in EUR, JPY", "€9.00, ¥1,500"),
        ("$10 in eur and gbp", "€9.00, £8.00"),
    ]);
}

#[test]
fn dates() {
    check(&[
        ("today", "Wed, 23 Sep 2026"),
        ("tomorrow", "Thu, 24 Sep 2026"),
        ("today + 3 weeks", "Wed, 14 Oct 2026"),
        ("10 June + 3 weeks", "Wed, 1 Jul 2026"),
        ("April 1, 2019 - 3 months 5 days", "Thu, 27 Dec 2018"),
        ("12/02/1988 + 32 years", "Wed, 12 Feb 2020"),
        ("01.05.2005 + 3 years 2 months 3 weeks", "Tue, 22 Jul 2008"),
        ("3 weeks after March 14, 2019", "Thu, 4 Apr 2019"),
        ("28 days before March 12, 2026", "Thu, 12 Feb 2026"),
        ("January 30 2020 + 3 months 2 weeks 5 days", "Tue, 19 May 2020"),
        ("January 31 2020 + 1 month", "Sat, 29 Feb 2020"),
        ("3 days ago", "Sun, 20 Sep 2026"),
        ("4 days from now", "Sun, 27 Sep 2026"),
        ("in 3 days", "Sat, 26 Sep 2026"),
        ("next friday", "Fri, 25 Sep 2026"),
        ("last monday", "Mon, 21 Sep 2026"),
        ("friday", "Fri, 25 Sep 2026"),
        ("wednesday", "Wed, 23 Sep 2026"),
        ("next week", "Wed, 30 Sep 2026"),
        ("christmas", "Fri, 25 Dec 2026"),
        ("easter 2027", "Sun, 28 Mar 2027"),
        ("days until christmas", "93 days"),
        ("days since July 15", "70 days"),
        ("days between 3 March and 30 May", "88 days"),
        ("3 March to 30 May", "2 months 3 weeks 6 days"),
        ("January 10 - February 5", "3 weeks 5 days"),
        ("weeks until new year", "14.28571429 weeks"),
        ("week of year", "39"),
        ("day of the week on March 9, 2024", "Saturday"),
        ("day number on March 15, 2024", "75"),
        ("1559740303 to date", "Wed, 5 Jun 2019 09:11:43"),
        ("1733823083000 to date", "Tue, 10 Dec 2024 04:31:23"),
        ("April 1, 2019 to timestamp", "1554091200"),
        ("2019-04-01T15:30:00Z to date", "Mon, 1 Apr 2019 11:30"),
        ("2026-09-23 + 1 day", "Thu, 24 Sep 2026"),
        ("now as iso8601", "2026-09-23T14:30:00-04:00"),
        ("days in February 2020", "29 days"),
        ("days in Q3", "92 days"),
        ("hours in a week", "168 hours"),
        ("workdays until christmas", "66 workdays"),
        ("10 March to 17 March in workdays", "5 workdays"),
        ("workdays between April 12 and June 15", "45 workdays"),
        ("workdays in 3 weeks", "15 workdays"),
        ("$500/workday * 20 workdays", "$10,000.00"),
        ("weekday on march 9, 2024", "Saturday"),
    ]);
}

#[test]
fn clock_times() {
    check(&[
        ("3pm", "15:00"),
        ("7:30am to 8:45pm", "13 hours 15 minutes"),
        ("4pm to 3am", "11 hours"),
        ("5pm - 7pm", "2 hours"),
        ("3:45pm + 4 hr 10 min", "19:55"),
        ("now + 3 hours 15 minutes", "17:45"),
        ("9:45 am - 15 hours 10 minutes", "Tue, 22 Sep 2026 18:35"),
        ("16:00 + 3 hours 12 minutes", "19:12"),
        ("tomorrow at 3pm", "Thu, 24 Sep 2026 15:00"),
        ("noon to 5:30pm in minutes", "330 minutes"),
        ("1:30 + 0:45", "02:15"),
        ("now to midnight", "9 hours 30 minutes"),
    ]);
}

#[test]
fn time_zones() {
    check(&[
        ("time in tokyo", "Thu, 24 Sep 2026 03:30 JST"),
        ("time in uzbekistan", "23:30 UTC+5"),
        ("time in london", "19:30 BST"),
        ("tokyo time", "Thu, 24 Sep 2026 03:30 JST"),
        ("now in utc", "18:30 UTC"),
        ("3pm in tokyo", "Thu, 24 Sep 2026 04:00 JST"),
        ("6pm Sydney in Chicago", "03:00 CDT"),
        ("2am PST to GMT", "09:00 UTC"),
        ("3pm GMT+8 to Paris", "09:00 CEST"),
        ("9:35am in New York to Japan", "22:35 JST"),
        ("time difference between Seattle and Tokyo", "16 hours"),
        ("date in vancouver", "Wed, 23 Sep 2026"),
        ("time in auckland", "Thu, 24 Sep 2026 06:30 NZST"),
        ("current time in paris", "20:30 CEST"),
        ("time in Tokyo when it is 9am in London", "17:00 JST"),
        ("time in marseille", "20:30 CEST"),
        ("time in aix-en-provence", "20:30 CEST"),
        ("time in sao jose dos campos", "15:30 UTC-3"),
        ("time in qwertyville", "<error: unknown place \"qwertyville\">"),
        ("time in marseile", "<error: unknown place \"marseile\", did you mean \"Marseille\"?>"),
        ("3pm to gotham city", "<error: unknown place \"gotham city\">"),
        ("time to go", "14:30"),
        ("PST to EST", "14:30 EDT"),
        ("tokyo in london", "19:30 BST"),
    ]);
}

#[test]
fn comments_and_labels() {
    check(&[
        ("5 // comment 7", "5"),
        ("Boeing \"747\" is $386.8M", "$386,800,000.00"),
        ("$999 (for iPhone 16)", "$999.00"),
        ("I spent $128 + $45 on clothes // on 10-02-2019", "$173.00"),
        ("Cost of 128 GB iPhone 16: $999", "$999.00"),
        ("# Heading 5", "<none>"),
        ("just some text", "<none>"),
    ]);
}

#[test]
fn sheets() {
    assert_eq!(sheet("rent = $1500\nrent * 12"), "$18,000.00");
    assert_eq!(sheet("monthly rent = $2,150\nmonthly rent / 4 people"), "$537.50");
    assert_eq!(sheet("discount = 10%\ncost = $550\ncost - discount"), "$495.00");
    assert_eq!(sheet("x = 5\nx += 3\nx * 2"), "16");
    assert_eq!(sheet("10\n20\nsum"), "30");
    assert_eq!(sheet("10\n20\n\n5\n7\nsum"), "12");
    assert_eq!(sheet("# Costs\nLunch: $20\nTaxi: $15\ntotal"), "$35.00");
    assert_eq!(sheet("10\n20\naverage"), "15");
    assert_eq!(sheet("10\nprev * 2"), "20");
    assert_eq!(sheet("10\n20\nline1 + line 2"), "30");
    assert_eq!(sheet("a = 5\n---\na"), "<none>");
}

#[test]
fn compound_growth() {
    check(&[
        ("$1,000 after 3 years at 7%", "$1,225.04"),
        ("$1,000 for 3 years at 7% compounding monthly", "$1,232.93"),
        ("$1,000 for 3 years at 7% compounding quarterly", "$1,231.44"),
        ("interest on $1,000 after 3 years @ 7%", "$225.04"),
        ("present value of $1,000 after 20 years at 10%", "$148.64"),
        ("$25k over 10 years at 7.5%", "$51,525.79"),
        ("20k after 6 months at 10% per month", "35,431.22"),
        ("$100 for 2 years at 1%/month", "$126.97"),
    ]);
}

#[test]
fn conditionals() {
    let tax = "earnings = $45k\nif earnings > $30k then tax = 20% else tax = 5%\nearnings * tax";
    assert_eq!(sheet(tax), "$9,000.00");
    assert_eq!(sheet("income = $35k\nexpenses = $21.5k\nprofitable = true if income > expenses"), "true");
    assert_eq!(sheet("income = $35k\nexpenses = $21.5k\ninsolvent = false unless expenses > income"), "false");
    assert_eq!(sheet("BMI = 24\nhealthy = BMI >= 18.5 and BMI < 25"), "true");
    assert_eq!(sheet("cost = $500\ndiscount = true\nif discount then cost = cost - 10%\ncost"), "$450.00");
    assert_eq!(sheet("x = 1\nif x > 5 then 10"), "<none>");
}

#[test]
fn odd_input_never_panics() {
    let words = [
        "",
        "(",
        ")",
        "$",
        "€",
        "°",
        "in",
        "to",
        "of",
        "at",
        "per",
        "a",
        "the",
        "-",
        "+",
        "*",
        "/",
        "^",
        "%",
        "!",
        "=",
        "5",
        "0",
        "1e400",
        "0x",
        "12:",
        "12:30",
        "2026-13-45",
        "31/02/2026",
        "Feb",
        "30",
        "km",
        "usd",
        "tokyo",
        "now",
        "today",
        "christmas",
        "sqrt",
        "sum",
        "prev",
        "line",
        "is",
        "what",
        "if",
        "then",
        "else",
        "x",
        "and",
        "or",
        "999999999999999999999999999999",
        "½",
        "\"",
        "'",
        "//",
        "#",
        ":",
        "π",
        "tenge",
        "->",
        "@",
        "&",
        "<<",
        "√",
    ];
    // A small deterministic generator keeps the test reproducible.
    let mut seed = 42u64;
    let mut next = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let mut calc = calculator();
    for _ in 0..20_000 {
        let len = 1 + next(6);
        let line: Vec<&str> = (0..len).map(|_| words[next(words.len())]).collect();
        let _ = calc.calculate(&line.join(" "));
    }
    for line in ["170!", "171!", "10!!!", "2^99999", "1e28 * 1e28", "0.1^9999", "-1^0.5", "sqrt(-1)", "ln(0)", "1/0%"] {
        let _ = calc.calculate(line);
    }
}
