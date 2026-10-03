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
        ("（2+3）*4", "20"),
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
        ("3 raised to 9", "19,683"),
        ("3 raised to the 9th power", "19,683"),
        ("3 to the 9", "19,683"),
        ("3 exponent 2 + 1", "10"),
        ("5 km raised to 2", "25 km²"),
        ("remainder of 21 divided by 5", "1"),
        ("2(3+4)", "14"),
        ("2 pi", "6.283185307"),
        ("5 cats (3)", "3"),
        ("10% coffee 5", "5"),
        ("1.5e3", "1,500"),
        ("2^64", "18,446,744,073,709,551,616"),
        ("2^100", "1.2676506e30"),
        ("5k", "5,000"),
        ("2.5M", "2,500,000"),
        ("3 million", "3,000,000"),
        ("2 dozen", "24"),
        ("half of 175", "87.5"),
        ("two thirds of 90", "60"),
        ("1 1/2 + 1", "2.5"),
        ("2 3/4 cups in ml", "650.6176504 mL"),
        ("3 quarters of an hour in minutes", "45 minutes"),
        ("a quarter of 100", "25"),
        ("1 quarter in months", "3 months"),
        ("2 quarters in days", "182.62125 days"),
        ("$100 a quarter", "$100.00/quarter"),
        ("$100 per quarter in per year", "$400.00/year"),
        ("twice 21", "42"),
        ("seven times six", "42"),
        ("1/0", "<error: division by zero>"),
        ("2^99999", "<error: result is too large>"),
        ("169! * 4211", "<error: result is too large>"),
        // Tiny products keep their digits.
        ("1.234567e-25 * 1.1", "1.3580237e-25"),
        ("-1 hour", "-1 hour"),
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
        ("1,00,000", "100,000"),
        ("₹12,34,567.89", "₹1,234,567.89"),
        ("max(1,00,000)", "1"),
    ]);
    let mut calc = calculator();
    calc.config_mut().decimal_comma = true;
    for (input, expected) in [
        ("1,500 + 1", "2,5"),
        ("1.500 + 1", "1.501"),
        ("10.50 eur", "€10,50"),
        ("$1,234,567.8", "$1.234.567,80"),
        ("10 USD in EUR, JPY", "€9,00; ¥1.500"),
        ("3^45", "2,954312707e21"),
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
        ("255 in base 16", "0xFF"),
        ("255 as base 2", "0b11111111"),
        ("255 in %", "25,500%"),
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
        ("is 17 prime?", "true"),
        ("is 91 a prime number", "false"),
        ("is prime 2", "true"),
        ("is 9223372036854775783 prime", "true"),
        ("is 17.5 prime", "false"),
        ("logarithm of 64 to base 4", "3"),
        ("natural logarithm of 1", "0"),
        ("binary log of 8", "3"),
        ("squared root of 100", "10"),
        ("4th root of 16", "2"),
        ("fourth root of 16 m^4", "2 m"),
        ("5 root of 100", "2.511886432"),
        ("root of 16", "4"),
        ("cbrt(343)", "7"),
        ("root 5 of 100", "2.511886432"),
        ("log(1000)", "3"),
        ("ln(e)", "1"),
        ("log 20 base 4", "2.160964047"),
        ("log 2 of 8", "3"),
        ("logarithm base 2 of 8", "3"),
        ("logarithm of 100", "2"),
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
        ("greatest common divisor of 20 and 30", "10"),
        ("least common multiple of 4 and 6", "12"),
        ("total of 3, 4, 7 and 9", "23"),
        ("average of 36, 42, 19 and 81", "44.5"),
        ("median of 10, 20 and 30", "20"),
        ("count of 1, 2, 3, 4, 5", "5"),
        ("standard deviation of 20, 30 and 40", "10"),
        ("std dev of 20, 30 and 40", "10"),
        ("average 36, 42, 19, 81", "44.5"),
        ("sum 1, 2, 3", "6"),
        ("max 5, 3 and 9", "9"),
        ("average 36 + 42", "78"),
        ("mid point of 3 and 7", "5"),
        ("mid-point of 3 and 7", "5"),
        ("larger of 100 and 200", "200"),
        ("smaller of 5 and 10", "5"),
        ("max(3, 7, 2)", "7"),
        ("midpoint between 150 and 300", "225"),
        ("10 permutation 3", "720"),
        ("25 combination 3", "2,300"),
        ("3 permutations of 10", "720"),
        ("clamp 26 between 5 and 25", "25"),
        ("sqrt(16 m^2)", "4 m"),
        ("log()", "<error: missing a value>"),
        ("cot(1)", "<error: unknown function \"cot\">"),
        ("root(", "<error: missing a value>"),
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
        ("round 56 down to nearest 10", "50"),
        ("round up 56.2", "57"),
        ("round 5.99 down", "5"),
        ("π rounded 3 dp", "3.142"),
        ("37 round up to nearest 10", "40"),
        ("37 rounded down to last 10", "30"),
        ("37 rounded down to 10", "30"),
        ("round 56 to next 10", "60"),
        ("round down 56 to 10", "50"),
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
        ("20% off of 50", "40"),
        ("20 is 10% of what", "200"),
        ("180 is 10% off what", "200"),
        ("220 is 10% on what", "200"),
        ("$150 is 25% on what", "$120.00"),
        ("50 to 75 is what %", "50%"),
        ("% change from 10 to 20", "100%"),
        ("percent decrease from 20 to 10", "-50%"),
        ("what % change is $10 to $15", "50%"),
        ("40 to 90 as %", "125%"),
        ("180 is what % off 200", "10%"),
        ("180 is what % on 150", "20%"),
        ("20 is what % of 200", "10%"),
        ("40 is what % of 90", "44.44444444%"),
        ("20 as a % of 200", "10%"),
        ("10 is what % 20", "50%"),
        ("4 out of 10", "40%"),
        ("$30 out of $120", "25%"),
        ("5 is what multiplier on 1", "4x"),
        ("5 as a multiplier of 2", "2.5x"),
        ("what % of 200 is 20", "10%"),
        ("10% of what is 20", "200"),
        ("20% off what is 30", "37.5"),
        ("20% on what is 30", "25"),
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
        ("81 is 9 to the power of what", "2"),
        ("9 to what power is 81", "2"),
        ("9 with what exponent is 81", "2"),
        ("2/3 of 600", "400"),
        ("50 is 1/5 of what", "250"),
        ("20/5 as multiplier", "4x"),
        ("50 to 75 is what x", "1.5x"),
        ("20% as dec", "0.2"),
        ("$10 for lunch + 15% tip", "$11.50"),
        ("6 is to 60 as 8 is to what", "80"),
        ("6 to 600 as 8 to what", "800"),
        ("6 to 600 is what to 8", "0.08"),
        ("$6 to 2 kg as $9 to what", "3 kg"),
        ("5 is to 10 as what is to 80", "40"),
        ("10 is % of 20", "50%"),
        ("what percent is 25 of 100", "25%"),
        ("20 is % on 10", "100%"),
        ("10 is what discount off 20", "50%"),
        ("20 percentage of 50", "10"),
        ("10 is what % with 20, 30", "16.66666667%"),
        ("what % is 25 with 100", "20%"),
        ("15 as % after adding 3", "25%"),
        ("if $30 is 20%, what is 50%", "$75.00"),
        ("30 is 20% what is 60", "40%"),
        ("if 20% is 30, what is 50%", "75"),
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
        ("12pt in px", "16 px"),
        ("100 hp at 3000 rpm", "237.3636413 N·m"),
        ("300 Nm at 3000 rpm", "94,247.77961 W"),
        ("100 kW at 3000 rpm", "318.3098862 N·m"),
        ("1 N·m in J", "1 J"),
        ("1 cup of flour in grams", "<error: can't convert cup to g>"),
        ("2 cups of flour in ml", "473.176473 mL"),
        ("10 kg of apples at $2/kg", "$20.00"),
        ("12 pt in mm", "4.233333333 mm"),
        ("2 rem in px", "32 px"),
        ("1.5 em in pt", "18 pt"),
        ("1 L in pints", "2.113376419 pints"),
        ("1 pint in ml", "473.176473 mL"),
        ("1 Kib in bits", "1,024 bit"),
        ("1 Gib in Mib", "1,024 Mibit"),
        ("1 Mib in bytes", "131,072 B"),
        ("1 gib in mb", "1,073.741824 MB"),
        ("1 Eb in Pb", "1,000 Pbit"),
        ("8 bits in bytes", "1 B"),
        ("100 f to c", "37.77777778 °C"),
        ("0 c in f", "32 °F"),
        ("300 K in C", "26.85 °C"),
        ("30 ℃ in ℉", "86 °F"),
        ("30℃ in ℉", "86 °F"),
        ("100 farenheit in celcius", "37.77777778 °C"),
        ("20 degrees celcius in f", "68 °F"),
        ("20 °c in °f", "68 °F"),
        ("20 ºC in F", "68 °F"),
        ("1 acre in m2", "4,046.856422 m²"),
        ("10 m² in ft²", "107.6391042 ft²"),
        ("1 m³ in L", "1,000 L"),
        ("1 ft/s² in m/s^2", "0.3048 m/s²"),
        ("5 mN in N", "0.005 N"),
        ("5 MN in kN", "5,000 kN"),
        ("2 BN", "2,000,000,000"),
        ("1 cup in ml", "236.5882365 mL"),
        ("1 kWh in J", "3,600,000 J"),
        ("5 kWh / 2 h", "2.5 kW"),
        ("180 deg in rad", "3.141592654 rad"),
        ("2 sq ft in sq in", "288 in²"),
        ("3 cubic feet in liters", "84.95053978 L"),
        ("20km == 20,000 m", "true"),
        ("5 => 3", "true"),
        ("5 =< 3", "false"),
        ("5 =! 3", "true"),
        ("5 > 3 and 2 > 1", "true"),
        ("5 > 3 and 2 > 5", "false"),
        ("5 > 3 or 2 > 5", "true"),
        ("true and false", "false"),
        ("true or false", "true"),
        ("is 17 prime and is 9 prime", "false"),
        ("$20 and $15 > $30", "true"),
        ("5 km in kg", "<error: can't convert km to kg>"),
        ("1 light year in km", "9,460,730,472,581 km"),
        ("1 mb in kb", "1,000 kB"),
        ("5 Mb in kB", "625 kB"),
        ("10 MBps in Mbps", "80 Mbps"),
        ("1 GB / 100 MBps", "10 seconds"),
        ("180 cm in feet and inches", "5 ft 10.87 in"),
        ("72 in in feet and inches", "6 ft"),
        ("180 cm in inches and feet", "5 ft 10.87 in"),
        ("1 day - 2 hours", "22 hours"),
        ("1 hour + 30 minutes", "1 hour 30 minutes"),
        ("1 year - 2 months", "10 months"),
        ("100 km / 50 km/h", "2 hours"),
        ("time to upload 3GB at 10 MB/s", "5 minutes"),
        ("1 hour 30 minutes at 1.5x", "1 hour"),
        ("speed of light in km/h", "1,079,252,849 km/h"),
        ("3/4 cup in ml", "177.4411774 mL"),
        ("100 / 4 kg", "25 kg"),
        ("1/2 km + 3 km", "3.5 km"),
        ("10 / 2 km/h", "5 km/h"),
        ("100 per 4 kg", "25/kg"),
        ("2 seconds in Hz", "0.5 Hz"),
        ("2 per hour in seconds", "1,800 seconds"),
        ("0.08 L/km in mpg", "29.40182292 mpg"),
        ("3 per kg in lb", "1.36077711/lb"),
        ("0 Hz in seconds", "<error: division by zero>"),
        ("30 mpg in L/100km", "7.840486111 L/100km"),
        ("8 liters per 100 km in mpg", "29.40182292 mpg"),
        ("5 l/100 km in km/L", "20 km/L"),
        ("40 L / 500 km in L/100km", "8 L/100km"),
        ("8 L/100km * 500 km", "40 L"),
        ("8 L/100km * 2", "16 L/100km"),
        ("cost of 500 km at 8 L/100km at $1.5/L", "$60.00"),
        ("2 in²", "2 in²"),
        ("2 in^2 in cm²", "12.9032 cm²"),
        ("1 N * 1 in", "1 N·inch"),
        ("1 picometer", "1 picometer"),
        ("3 picometers in m", "3e-12 m"),
        ("2.5 pound-force in N", "11.12055404 N"),
        ("1 light-year in km", "9,460,730,472,581 km"),
        ("3 kilowatt-hours in J", "10,800,000 J"),
        ("2.5 kA in A", "2,500 A"),
        ("5 V/A", "5 V/A"),
        ("1 W/(m·K)", "1 W/(m·K)"),
        ("1 Mbit/(s·week)", "1 Mbit/(s·week)"),
        ("1 BTU·min", "1 BTU·min"),
        ("1 fl oz·°F", "1 fl oz·°F"),
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
        ("$50/hour * 3 workdays", "$1,200.00"),
        ("$100/hour in per workday", "$800.00/workday"),
        ("$800/workday in per hour", "$100.00/hour"),
        ("$800/workday + $50/hour", "$150.00/hour"),
        ("1 workday / 1 hour", "8"),
        ("40 hours / 1 workday", "5"),
        ("$1,000/month is what per week", "$229.98/week"),
        ("$1,000/month as per day", "$32.85/day"),
        ("60 mph is how much per minute", "1 mi/min"),
        ("5 km is what in miles", "3.106855961 mi"),
        ("$500/month / 30 days", "$16.67/day"),
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
        ("CN¥5 + CN¥1.50", "CN¥6.50"),
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
        ("500 kazakhstani tenges in rubles", "₽90.00"),
        ("1000 hungarian forints", "1,000.00 HUF"),
        ("2 aussie dollars", "A$2.00"),
        ("1000 bosnia and herzegovina convertible marks", "1,000.00 BAM"),
        ("10 TL", "₺10.00"),
        ("$CAD 20", "C$20.00"),
        ("HKD$ 10", "HK$10.00"),
        ("3 DASH", "3 DASH"),
        ("$19 for breakfast + $22 for the uber", "$41.00"),
        ("$20 for lunch $15 for taxi", "$35.00"),
        ("2 x $3 coffee $5", "$11.00"),
        ("3 km walk 2 km run", "5 km"),
        ("10 USD in EUR, JPY", "€9.00, ¥1,500"),
        ("$10 in eur and gbp", "€9.00, £8.00"),
        ("50 cents in dollars", "$0.50"),
        ("$4.99 + 50¢", "$5.49"),
        ("80 pence in USD", "$1.00"),
        ("1000 sats in USD", "$1.00"),
        ("20 per cent of 50", "10"),
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
        ("3 days in the past", "Sun, 20 Sep 2026"),
        ("2 weeks in the future", "Wed, 7 Oct 2026"),
        ("4 days from now", "Sun, 27 Sep 2026"),
        ("$100 from 1990 is worth what today", "<error: expected a length of time before \"from\">"),
        ("in 3 days", "Sat, 26 Sep 2026"),
        ("next friday", "Fri, 25 Sep 2026"),
        ("last monday", "Mon, 21 Sep 2026"),
        ("friday", "Fri, 25 Sep 2026"),
        ("wednesday", "Wed, 23 Sep 2026"),
        ("next week", "Wed, 30 Sep 2026"),
        ("christmas", "Fri, 25 Dec 2026"),
        ("easter 2027", "Sun, 28 Mar 2027"),
        ("easter eve 2027", "Sat, 27 Mar 2027"),
        ("greek easter 2027", "Sun, 2 May 2027"),
        ("orthodox good friday 2027", "Fri, 30 Apr 2027"),
        ("serbian christmas 2027", "Thu, 7 Jan 2027"),
        ("chinese new year 2027", "Sat, 6 Feb 2027"),
        ("days until lunar new year", "136 days"),
        ("days until christmas", "93 days"),
        ("GTA 6 release date", "Thu, 19 Nov 2026 (probably)"),
        ("days until GTA VI", "57 days"),
        ("gta 6 release date + 1 week", "Thu, 26 Nov 2026"),
        ("days since July 15", "70 days"),
        ("days between 3 March and 30 May", "88 days"),
        ("days between 3 March and 3 March", "0 days"),
        ("whole weeks between 3 March and 30 May", "12 weeks"),
        ("days between March 3 through March 10", "8 days"),
        ("from March 3 through March 10", "1 week 1 day"),
        ("March 3 through March 10", "1 week 1 day"),
        ("3 March to 30 May", "2 months 3 weeks 6 days"),
        ("January 10 - February 5", "3 weeks 5 days"),
        ("weeks until new year", "14.28571429 weeks"),
        ("week of year", "39"),
        ("day of the week on March 9, 2024", "Saturday"),
        ("day number on March 15, 2024", "75"),
        ("1559740303 to date", "Wed, 5 Jun 2019 09:11:43"),
        ("1733823083000 to date", "Tue, 10 Dec 2024 04:31:23"),
        ("April 1, 2019 to timestamp", "1554091200"),
        ("April 1, 2019 to time stamp", "1554091200"),
        ("2019-04-01T15:30:00Z to date", "Mon, 1 Apr 2019 11:30"),
        ("2026-09-23 + 1 day", "Thu, 24 Sep 2026"),
        ("2026/09/23 + 1 day", "Thu, 24 Sep 2026"),
        ("now as iso8601", "2026-09-23T14:30:00-04:00"),
        ("days in February 2020", "29 days"),
        ("days in Q3", "92 days"),
        ("days in Q1 2027", "90 days"),
        ("Q3 2026", "1 Jul – 30 Sep 2026"),
        ("Q4 of 2026", "1 Oct – 31 Dec 2026"),
        ("next quarter", "1 Oct – 31 Dec 2026"),
        ("start of next month", "Thu, 1 Oct 2026"),
        ("friday in 3 weeks", "Fri, 16 Oct 2026"),
        ("monday in 2 weeks", "Mon, 5 Oct 2026"),
        ("friday in a fortnight", "Fri, 9 Oct 2026"),
        ("week number March 5, 2026", "10"),
        ("day of week March 9, 2024", "Saturday"),
        ("days between March 3 to May 30", "88 days"),
        ("days between March 3 - May 30", "88 days"),
        ("beginning of next week", "Mon, 28 Sep 2026"),
        ("end of the month", "Wed, 30 Sep 2026"),
        ("end of this week", "Sun, 27 Sep 2026"),
        ("start of the year", "Thu, 1 Jan 2026"),
        ("end of year", "Thu, 31 Dec 2026"),
        ("start of Q4", "Thu, 1 Oct 2026"),
        ("end of Q3", "Wed, 30 Sep 2026"),
        ("end of next quarter", "Thu, 31 Dec 2026"),
        ("end of February 2028", "Tue, 29 Feb 2028"),
        ("start of 2027", "Fri, 1 Jan 2027"),
        ("days until end of the year", "99 days"),
        ("end of next month at 5pm", "Sat, 31 Oct 2026 17:00"),
        ("last quarter", "1 Apr – 30 Jun 2026"),
        ("in Q2", "1 Apr – 30 Jun 2026"),
        ("Q3 in days", "92 days"),
        ("this quarter in workdays", "66 workdays"),
        ("days until Q4", "8 days"),
        ("days since Q2", "175 days"),
        ("Q1 through Q2 in days", "181 days"),
        ("Q1 sales 500 + Q2 sales 600", "1,100"),
        ("Q3 $10k + Q4 $12k", "$22,000.00"),
        ("days in 2024", "366 days"),
        ("days left in 2026", "100 days"),
        ("days left in March", "0 days"),
        ("weeks remaining in 2027", "52.14285714 weeks"),
        ("hours in a week", "168 hours"),
        ("workdays until christmas", "67 workdays"),
        ("workdays until Oct 15", "16 workdays"),
        ("time until Oct 15 in workdays", "16 workdays"),
        ("10 days in workdays", "8 workdays"),
        ("10 March to 17 March in workdays", "5 workdays"),
        ("workdays between April 12 and June 15", "45 workdays"),
        ("workdays in 3 weeks", "15 workdays"),
        ("$500/workday * 20 workdays", "$10,000.00"),
        // Hours are hours of work, 8 to a workday.
        ("40 hours in workdays", "5 workdays"),
        ("4 hours in workdays", "0.5 workdays"),
        ("9am to 5pm in workdays", "1 workday"),
        ("3 workdays in hours", "24 hours"),
        ("1 business day in seconds", "28,800 seconds"),
        ("1 workday + 4 hours", "1.5 workdays"),
        ("2 workdays - 4 hours", "1.5 workdays"),
        ("1 workday > 4 hours", "true"),
        ("2 workdays in days", "<error: can't convert workdays to days>"),
        ("5 workdays + 1 day", "<error: can't combine workdays and days>"),
        // Dates move by workdays, skipping weekends.
        ("today + 5 workdays", "Wed, 30 Sep 2026"),
        ("3 business days from today", "Mon, 28 Sep 2026"),
        ("5 workdays ago", "Wed, 16 Sep 2026"),
        ("Saturday + 1 workday", "Mon, 28 Sep 2026"),
        ("Saturday - 1 workday", "Fri, 25 Sep 2026"),
        ("Sunday + 5 workdays", "Fri, 2 Oct 2026"),
        ("today + 2.5 workdays", "<error: only whole workdays can be added to a date>"),
        ("weekday on march 9, 2024", "Saturday"),
    ]);
}

#[test]
fn sun_and_moon() {
    check(&[
        // Today at the place, to the minute; PyEphem agrees within seconds.
        ("sunrise in Tokyo", "Thu, 24 Sep 2026 05:30 JST"),
        ("Tokyo sunset", "Thu, 24 Sep 2026 17:36 JST"),
        ("sunrise time in Tokyo", "Thu, 24 Sep 2026 05:30 JST"),
        ("sunrise in London", "06:48 BST"),
        ("sunset in London", "18:57 BST"),
        ("hours of daylight in London", "12 hours 9 minutes"),
        ("London daylight hours", "12 hours 9 minutes"),
        ("sunset in London - sunrise in London", "12 hours 9 minutes"),
        ("daylight hours in Sydney in hours", "12.16666667 hours"),
        ("sunrise at JFK", "06:45 EDT"),
        ("sunrise in San Fernando del Valle de Catamarca", "07:11 UTC-3"),
        // Without a place, here; "tomorrow" is tomorrow at the place.
        ("sunrise", "06:45"),
        ("time until sunset", "4 hours 21 minutes"),
        ("sunrise in Paris tomorrow", "Thu, 24 Sep 2026 07:40 CEST"),
        ("sunrise in Tokyo tomorrow", "Fri, 25 Sep 2026 05:31 JST"),
        ("sunrise in Tokyo in 2 days", "Sat, 26 Sep 2026 05:32 JST"),
        ("sunrise in Tokyo on Oct 5", "Mon, 5 Oct 2026 05:39 JST"),
        ("sunrise in Tokyo in New York", "16:30 EDT"),
        ("daylight on March 21, 2027", "12 hours 11 minutes"),
        ("hours of daylight in Tromso on June 21", "24 hours"),
        ("hours of daylight in Tromso on Dec 21", "0 hours"),
        ("sunrise in Tromso on June 21", "<error: polar day: the sun doesn't set there on Sun, 21 Jun 2026>"),
        ("sunrise in Texas", "<error: don't know where exactly \"Texas\" is: name a city>"),
        ("hours of sunlight in Qwertyville", "<error: unknown place \"Qwertyville\">"),
        // The moon's phases from their exact times; a main phase lasts an eighth of the month.
        ("moon phase", "Waxing Gibbous"),
        ("phase of the moon on Sep 26", "Full Moon"),
        ("moon phase on Oct 5", "Waning Crescent"),
        ("lunar day", "13"),
        ("lunar day on Oct 5", "25"),
        ("moon day on Jan 1, 2027", "24"),
        // Words of the sun in text.
        ("Daylight saving starts Mar 8", "Sun, 8 Mar 2026"),
        ("Sunset Boulevard 123", "123"),
        ("$20 for sunset cruise", "$20.00"),
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
        ("time at tokyo", "Thu, 24 Sep 2026 03:30 JST"),
        ("time now in tokyo", "Thu, 24 Sep 2026 03:30 JST"),
        ("local time tokyo", "Thu, 24 Sep 2026 03:30 JST"),
        ("tokyo local time", "Thu, 24 Sep 2026 03:30 JST"),
        ("now in utc", "18:30 UTC"),
        ("3pm in tokyo", "Thu, 24 Sep 2026 04:00 JST"),
        ("6pm Sydney in Chicago", "03:00 CDT"),
        ("2am PST to GMT", "09:00 GMT"),
        ("3pm GMT+8 to Paris", "09:00 CEST"),
        ("9:35am in New York to Japan", "22:35 JST"),
        ("time difference between Seattle and Tokyo", "16 hours"),
        ("time diff Seattle Tokyo", "16 hours"),
        ("time difference Tokyo", "13 hours"),
        ("date in vancouver", "Wed, 23 Sep 2026"),
        ("time in auckland", "Thu, 24 Sep 2026 06:30 NZST"),
        ("current time in paris", "20:30 CEST"),
        ("time in Tokyo when it is 9am in London", "17:00 JST"),
        ("time in Tokyo when 9am in London", "17:00 JST"),
        ("time in Tokyo in 3 hours", "Thu, 24 Sep 2026 06:30 JST"),
        ("time in London in 2 hours", "21:30 BST"),
        ("time in 3 hours", "17:30"),
        ("time in 3 hours in Tokyo", "Thu, 24 Sep 2026 06:30 JST"),
        ("now in an hour", "15:30"),
        ("time in marseille", "20:30 CEST"),
        ("time in cordoba", "15:30 UTC-3"),
        ("time in the hague", "20:30 CEST"),
        ("time in port-au-prince", "14:30 EDT"),
        // Abbreviations only where they read back: Belize's "CST" would be Chicago's.
        ("3pm Belize", "15:00 UTC-6"),
        ("3pm Kampala", "15:00 UTC+3"),
        ("today as iso 8601", "2026-09-23T00:00:00-04:00"),
        ("3pm WEST in UTC", "14:00 UTC"),
        ("Jan 10 3pm EDT in UTC", "Sat, 10 Jan 2026 19:00 UTC"),
        ("Jan 10 3pm EST in UTC", "Sat, 10 Jan 2026 20:00 UTC"),
        ("time in aix-en-provence", "20:30 CEST"),
        ("time in sao jose dos campos", "15:30 UTC-3"),
        ("time in San Fernando del Valle de Catamarca", "15:30 UTC-3"),
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
        ("Groceries — $200", "$200.00"),
        ("half-life 5", "5"),
        ("x-ray 5", "5"),
        ("T-shirt $20", "$20.00"),
        ("Coca-Cola $2 + Pepsi $3", "$5.00"),
        ("follow-up in 3 days", "Sat, 26 Sep 2026"),
        ("one-time fee $50", "$50.00"),
        ("twenty-five + 1", "26"),
        ("pi-e", "0.4233108251"),
        ("iPhone 16 Pro $999", "$999.00"),
        ("$999 for iPhone 16", "$999.00"),
        ("Room 101 costs $10", "$10.00"),
        ("Model 3 $40k", "$40,000.00"),
        ("2 cats 3 dogs", "3"),
        ("2 x 4 lumber $5", "8"),
        ("5 + 5 then 3", "10"),
        ("June 5 cats 3", "3"),
        ("1. $10", "$10.00"),
        ("2) Rent $900", "$900.00"),
        ("a) 3 + 4", "7"),
        ("1. May 2024", "Wed, 1 May 2024"),
        ("# Heading 5", "<none>"),
        ("just some text", "<none>"),
        ("$20 for lunch, $15 for taxi", "$35.00"),
        ("3 km walk, 5 km run", "8 km"),
        ("max($10, $20)", "$20.00"),
        ("A coffee $5", "$5.00"),
        ("5:30 min/km", "05:30"),
    ]);
}

#[test]
fn sheets() {
    assert_eq!(sheet("rent = $1500\nrent * 12"), "$18,000.00");
    assert_eq!(sheet("monthly rent = $2,150\nmonthly rent / 4 people"), "$537.50");
    assert_eq!(sheet("discount = 10%\ncost = $550\ncost - discount"), "$495.00");
    assert_eq!(sheet("x = 5\nx += 3\nx * 2"), "16");
    assert_eq!(sheet("10\n20\nsum"), "30");
    assert_eq!(sheet("$10\n$20\nsum\nsum in EUR"), "€27.00");
    assert_eq!(sheet("$10\n$20\nsum in EUR\nsum"), "$30.00");
    assert_eq!(sheet("10\n20\nsum\naverage"), "15");
    assert_eq!(sheet("10\n20\naverage\nsum"), "30");
    assert_eq!(sheet("$10\nsum\n$5\nsum"), "$5.00");
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
        ("$100 at 10% for 5 years", "$161.05"),
        ("interest on $100 at 10% for 5 years", "$61.05"),
        ("monthly repayment on $300,000 for 30 years at 6%", "$1,798.65"),
        ("weekly repayment on $10,000 at 5% for 5 years", "$43.40"),
        ("total repayment on $10,000 for 5 years at 5%", "$11,322.74"),
        ("total interest on $10,000 for 5 years at 5%", "$1,322.74"),
        ("monthly interest on $10,000 for 5 years at 5%", "$22.05"),
        ("monthly payment on $12,000 for 1 year at 0%", "$1,000.00"),
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
    for line in [
        "170!",
        "171!",
        "10!!!",
        "2^99999",
        "1e28 * 1e28",
        "0.1^9999",
        "-1^0.5",
        "sqrt(-1)",
        "ln(0)",
        "1/0%",
        "clamp()",
        "npr{}",
        "midpoint(",
    ] {
        let _ = calc.calculate(line);
    }
}
