//! Built-in unit definitions and the registry builder.

use jiff::Unit as Cal;

use super::{Dim, Registry, Unit, UnitDef, UnitId};
use crate::currency::{AMBIGUOUS_CODES, CURRENCIES, MINOR_UNITS, find};
use crate::hash::TableMap;
use crate::number::Number;

struct Prefix {
    symbol: &'static str,
    name: &'static str,
    base: i64,
    exp: i32,
}

const fn si(symbol: &'static str, name: &'static str, exp: i32) -> Prefix {
    Prefix { symbol, name, base: 10, exp }
}

const fn bin(symbol: &'static str, name: &'static str, exp: i32) -> Prefix {
    Prefix { symbol, name, base: 2, exp }
}

const KILO: Prefix = si("k", "kilo", 3);
const MEGA: Prefix = si("M", "mega", 6);
const GIGA: Prefix = si("G", "giga", 9);
const TERA: Prefix = si("T", "tera", 12);
const PETA: Prefix = si("P", "peta", 15);
const EXA: Prefix = si("E", "exa", 18);
const HECTO: Prefix = si("h", "hecto", 2);
const DECI: Prefix = si("d", "deci", -1);
const CENTI: Prefix = si("c", "centi", -2);
const MILLI: Prefix = si("m", "milli", -3);
const MICRO: Prefix = si("µ", "micro", -6);
const MICRO_GREEK: Prefix = si("μ", "micro", -6);
const NANO: Prefix = si("n", "nano", -9);
const PICO: Prefix = si("p", "pico", -12);

const BIG: &[Prefix] = &[KILO, MEGA, GIGA, TERA, PETA, EXA];
const SMALL: &[Prefix] = &[MILLI, MICRO, MICRO_GREEK, NANO, PICO];
const LENGTH: &[Prefix] = &[KILO, HECTO, DECI, CENTI, MILLI, MICRO, MICRO_GREEK, NANO, PICO];
const MASS: &[Prefix] = &[KILO, CENTI, MILLI, MICRO, MICRO_GREEK, NANO];
const VOLUME: &[Prefix] = &[KILO, HECTO, DECI, CENTI, MILLI, MICRO, MICRO_GREEK];
const BYTES: &[Prefix] = &[
    KILO,
    MEGA,
    GIGA,
    TERA,
    PETA,
    EXA,
    bin("Ki", "kibi", 10),
    bin("Mi", "mebi", 20),
    bin("Gi", "gibi", 30),
    bin("Ti", "tebi", 40),
    bin("Pi", "pebi", 50),
    bin("Ei", "exbi", 60),
];

/// A unit definition before it gets an id.
struct Def {
    symbol: &'static str,
    name: &'static str,
    plural: Option<&'static str>,
    /// Extra spellings: lowercase ones match case-insensitively.
    aliases: &'static [&'static str],
    dim: Dim,
    scale: Number,
    offset: Number,
    prefixes: &'static [Prefix],
    spelled: bool,
    tight: bool,
    calendar: Option<(Cal, i64)>,
}

fn unit(symbol: &'static str, name: &'static str, dim: Dim, scale: &str) -> Def {
    Def {
        symbol,
        name,
        plural: None,
        aliases: &[],
        dim,
        scale: Number::parse(scale).expect("valid unit scale"),
        offset: Number::ZERO,
        prefixes: &[],
        spelled: false,
        tight: false,
        calendar: None,
    }
}

impl Def {
    fn plural(mut self, plural: &'static str) -> Def {
        self.plural = Some(plural);
        self
    }
    fn aliases(mut self, aliases: &'static [&'static str]) -> Def {
        self.aliases = aliases;
        self
    }
    fn prefixes(mut self, prefixes: &'static [Prefix]) -> Def {
        self.prefixes = prefixes;
        self
    }
    fn scale(mut self, scale: Number) -> Def {
        self.scale = scale;
        self
    }
    fn offset(mut self, offset: Number) -> Def {
        self.offset = offset;
        self
    }
    fn spelled(mut self) -> Def {
        self.spelled = true;
        self
    }
    fn tight(mut self) -> Def {
        self.tight = true;
        self
    }
    fn calendar(mut self, unit: Cal, multiple: i64) -> Def {
        self.calendar = Some((unit, multiple));
        self
    }
}

const PI: &str = "3.1415926535897932384626433833";
const DAY: i64 = 86_400;

fn definitions() -> Vec<Def> {
    let pi = Number::parse(PI).unwrap();
    let days = |n: &str| Number::parse(n).unwrap() * Number::from_i64(DAY);
    vec![
        // Time
        unit("s", "second", Dim::TIME, "1")
            .aliases(&["sec", "secs"])
            .prefixes(SMALL)
            .spelled()
            .calendar(Cal::Second, 1),
        unit("min", "minute", Dim::TIME, "60").aliases(&["mins"]).spelled().calendar(Cal::Minute, 1),
        unit("h", "hour", Dim::TIME, "3600").aliases(&["hr", "hrs"]).spelled().calendar(Cal::Hour, 1),
        unit("d", "day", Dim::TIME, "86400").aliases(&["night", "nights"]).spelled().calendar(Cal::Day, 1),
        unit("wk", "week", Dim::TIME, "604800").aliases(&["wks"]).spelled().calendar(Cal::Week, 1),
        unit("fortnight", "fortnight", Dim::TIME, "1209600").spelled().calendar(Cal::Week, 2),
        unit("mo", "month", Dim::TIME, "1")
            .scale(days("30.436875"))
            .aliases(&["mos"])
            .spelled()
            .calendar(Cal::Month, 1),
        unit("yr", "year", Dim::TIME, "1")
            .scale(days("365.2425"))
            .aliases(&["yrs", "annum"])
            .spelled()
            .calendar(Cal::Year, 1),
        unit("decade", "decade", Dim::TIME, "1").scale(days("3652.425")).spelled().calendar(Cal::Year, 10),
        unit("century", "century", Dim::TIME, "1")
            .scale(days("36524.25"))
            .plural("centuries")
            .spelled()
            .calendar(Cal::Year, 100),
        unit("millennium", "millennium", Dim::TIME, "1")
            .scale(days("365242.5"))
            .plural("millennia")
            .spelled()
            .calendar(Cal::Year, 1000),
        unit("workday", "workday", Dim::WORKDAY, "1")
            .aliases(&["workdays", "work day", "work days", "business day", "business days", "weekdays"])
            .spelled(),
        // Length
        unit("m", "meter", Dim::LENGTH, "1").aliases(&["metre", "metres"]).prefixes(LENGTH),
        unit("in", "inch", Dim::LENGTH, "0.0254").plural("inches").aliases(&["\""]),
        unit("ft", "foot", Dim::LENGTH, "0.3048").plural("feet").aliases(&["'"]),
        unit("yd", "yard", Dim::LENGTH, "0.9144"),
        unit("mi", "mile", Dim::LENGTH, "1609.344"),
        unit("nmi", "nautical mile", Dim::LENGTH, "1852").aliases(&["NM"]),
        unit("fur", "furlong", Dim::LENGTH, "201.168"),
        unit("ftm", "fathom", Dim::LENGTH, "1.8288"),
        unit("Å", "angstrom", Dim::LENGTH, "1e-10").aliases(&["ångström"]),
        unit("au", "astronomical unit", Dim::LENGTH, "149597870700").aliases(&["AU"]),
        unit("ly", "light year", Dim::LENGTH, "9460730472580800").aliases(&[
            "lightyear",
            "lightyears",
            "light-year",
            "light-years",
        ]),
        unit("pc", "parsec", Dim::LENGTH, "30856775814913673").prefixes(&[KILO, MEGA, GIGA]),
        // Mass
        unit("g", "gram", Dim::MASS, "0.001").aliases(&["gramme", "grammes"]).prefixes(MASS),
        unit("t", "tonne", Dim::MASS, "1000")
            .aliases(&["metric ton", "metric tons", "metric tonne", "metric tonnes"])
            .prefixes(&[KILO, MEGA, GIGA]),
        unit("lb", "pound", Dim::MASS, "0.45359237").aliases(&["lbs"]),
        unit("oz", "ounce", Dim::MASS, "0.028349523125"),
        unit("st", "stone", Dim::MASS, "6.35029318").plural("stone").aliases(&["stones"]),
        unit("ton", "short ton", Dim::MASS, "907.18474").aliases(&["ton", "tons", "us ton", "us tons"]),
        unit("long ton", "long ton", Dim::MASS, "1016.0469088").aliases(&["imperial ton", "imperial tons"]),
        unit("oz t", "troy ounce", Dim::MASS, "0.0311034768").aliases(&["ozt", "troy oz"]),
        unit("ct", "carat", Dim::MASS, "0.0002"),
        unit("grain", "grain", Dim::MASS, "0.00006479891").spelled(),
        // Area
        unit("ha", "hectare", Dim::AREA, "10000"),
        unit("ac", "acre", Dim::AREA, "4046.8564224"),
        // Volume
        unit("L", "liter", Dim::VOLUME, "0.001").aliases(&["l", "ℓ", "litre", "litres"]).prefixes(VOLUME),
        unit("gal", "gallon", Dim::VOLUME, "0.003785411784").aliases(&["us gallon", "us gallons"]),
        unit("imp gal", "imperial gallon", Dim::VOLUME, "0.00454609").aliases(&["uk gallon", "uk gallons"]),
        unit("qt", "quart", Dim::VOLUME, "0.000946352946"),
        unit("pt", "pint", Dim::VOLUME, "0.000473176473").aliases(&["us pint", "us pints"]),
        unit("imp pt", "imperial pint", Dim::VOLUME, "0.00056826125").aliases(&["uk pint", "uk pints"]),
        unit("cup", "cup", Dim::VOLUME, "0.0002365882365").aliases(&["us cup", "us cups"]),
        unit("metric cup", "metric cup", Dim::VOLUME, "0.00025"),
        unit("fl oz", "fluid ounce", Dim::VOLUME, "0.0000295735295625").aliases(&["floz", "fl. oz", "fl oz"]),
        unit("tbsp", "tablespoon", Dim::VOLUME, "0.00001478676478125").aliases(&["Tbsp", "tbs"]),
        unit("tsp", "teaspoon", Dim::VOLUME, "0.00000492892159375"),
        unit("bbl", "barrel", Dim::VOLUME, "0.158987294928"),
        // Angle (base: radian)
        unit("rad", "radian", Dim::ANGLE, "1").prefixes(&[MILLI]),
        unit("°", "degree", Dim::ANGLE, "1").scale(pi / Number::from_i64(180)).aliases(&["deg", "degs"]).tight(),
        unit("grad", "gradian", Dim::ANGLE, "1").scale(pi / Number::from_i64(200)).aliases(&["gon", "gons"]),
        unit("rev", "revolution", Dim::ANGLE, "1").scale(pi * Number::from_i64(2)).aliases(&["turn", "turns", "revs"]),
        unit("arcmin", "arcminute", Dim::ANGLE, "1").scale(pi / Number::from_i64(10_800)).aliases(&["arcmins"]),
        unit("arcsec", "arcsecond", Dim::ANGLE, "1").scale(pi / Number::from_i64(648_000)).aliases(&["arcsecs"]),
        // Temperature (base: kelvin)
        unit("K", "kelvin", Dim::TEMPERATURE, "1").plural("kelvin").aliases(&["°K", "kelvins"]),
        unit("°C", "degree Celsius", Dim::TEMPERATURE, "1")
            .plural("degrees Celsius")
            .offset(Number::parse("273.15").unwrap())
            .aliases(&[
                "C",
                "c",
                "℃",
                "celsius",
                "centigrade",
                "degc",
                "deg c",
                "degree c",
                "degrees c",
                "degree celsius",
                "degrees celsius",
            ]),
        unit("°F", "degree Fahrenheit", Dim::TEMPERATURE, "1")
            .plural("degrees Fahrenheit")
            .scale(Number::ratio(5, 9))
            .offset(Number::ratio(229_835, 900))
            .aliases(&[
                "F",
                "f",
                "℉",
                "fahrenheit",
                "degf",
                "deg f",
                "degree f",
                "degrees f",
                "degree fahrenheit",
                "degrees fahrenheit",
            ]),
        unit("°R", "degree Rankine", Dim::TEMPERATURE, "1")
            .plural("degrees Rankine")
            .scale(Number::ratio(5, 9))
            .aliases(&["rankine"]),
        // Frequency
        unit("Hz", "hertz", Dim::FREQUENCY, "1").plural("hertz").prefixes(&[KILO, MEGA, GIGA, TERA, MILLI]),
        unit("rpm", "revolution per minute", Dim::FREQUENCY, "1")
            .plural("revolutions per minute")
            .scale(Number::ratio(1, 60)),
        // Mechanics
        unit("N", "newton", Dim::FORCE, "1").prefixes(&[KILO, MEGA, MILLI, MICRO]),
        unit("lbf", "pound-force", Dim::FORCE, "4.4482216152605")
            .plural("pounds-force")
            .aliases(&["pound force", "pounds force"]),
        unit("kgf", "kilogram-force", Dim::FORCE, "9.80665").plural("kilograms-force").aliases(&["kilogram force"]),
        unit("J", "joule", Dim::ENERGY, "1").prefixes(&[KILO, MEGA, GIGA, TERA, MILLI]),
        unit("cal", "calorie", Dim::ENERGY, "4.184").prefixes(&[KILO]),
        unit("eV", "electronvolt", Dim::ENERGY, "0.0000000000000000001602176634")
            .aliases(&["electron volt", "electron volts"])
            .prefixes(&[KILO, MEGA, GIGA, TERA]),
        unit("BTU", "British thermal unit", Dim::ENERGY, "1055.05585262").aliases(&["btu", "btus"]),
        unit("W", "watt", Dim::POWER, "1").prefixes(&[KILO, MEGA, GIGA, TERA, MILLI]),
        unit("hp", "horsepower", Dim::POWER, "745.69987158227022").plural("horsepower"),
        unit("Pa", "pascal", Dim::PRESSURE, "1").prefixes(&[HECTO, KILO, MEGA, GIGA]),
        unit("bar", "bar", Dim::PRESSURE, "100000").prefixes(&[MILLI]),
        unit("atm", "atmosphere", Dim::PRESSURE, "101325"),
        unit("psi", "pound per square inch", Dim::PRESSURE, "6894.757293168361").plural("pounds per square inch"),
        unit("ksi", "kilopound per square inch", Dim::PRESSURE, "6894757.293168361"),
        unit("mmHg", "millimeter of mercury", Dim::PRESSURE, "133.322387415").plural("millimeters of mercury"),
        unit("inHg", "inch of mercury", Dim::PRESSURE, "3386.388640341").plural("inches of mercury"),
        unit("Torr", "torr", Dim::PRESSURE, "1").scale(Number::ratio(101_325, 760)).plural("torr"),
        // Electricity and chemistry
        unit("A", "ampere", Dim::CURRENT, "1").aliases(&["amp", "amps"]).prefixes(&[KILO, MILLI, MICRO, MICRO_GREEK]),
        unit("V", "volt", Dim::VOLTAGE, "1").prefixes(&[KILO, MEGA, MILLI, MICRO]),
        unit("Ω", "ohm", Dim::RESISTANCE, "1").aliases(&["Ω"]).prefixes(&[KILO, MEGA, MILLI]),
        unit("mol", "mole", Dim::AMOUNT, "1").prefixes(&[KILO, MILLI, MICRO, MICRO_GREEK, NANO]),
        // Data (base: bit)
        unit("bit", "bit", Dim::DATA, "1").prefixes(BIG),
        unit("B", "byte", Dim::DATA, "8").aliases(&["octet", "octets"]).prefixes(BYTES),
    ]
}

/// Names for compound units: (symbol shown in answers or "", spellings, definition).
const COMPOUNDS: &[(&str, &[&str], &str)] = &[
    ("mph", &["mph", "miles per hour", "mile per hour"], "mi/h"),
    (
        "",
        &[
            "kph",
            "kmh",
            "kmph",
            "kilometers per hour",
            "kilometres per hour",
            "kilometer per hour",
            "kilometre per hour",
        ],
        "km/h",
    ),
    ("", &["mps", "meters per second", "metres per second", "meter per second", "metre per second"], "m/s"),
    ("", &["feet per second", "foot per second"], "ft/s"),
    ("kn", &["kn", "kt", "kts", "knot", "knots"], "nmi/h"),
    ("mpg", &["mpg", "miles per gallon"], "mi/gal"),
    ("Wh", &["Wh", "watt hour", "watt hours", "watt-hour", "watt-hours"], "W*h"),
    ("kWh", &["kWh", "kwh", "kilowatt hour", "kilowatt hours", "kilowatt-hour", "kilowatt-hours"], "kW*h"),
    ("MWh", &["MWh", "megawatt hour", "megawatt hours"], "MW*h"),
    ("GWh", &["GWh", "gigawatt hour", "gigawatt hours"], "GW*h"),
    ("TWh", &["TWh", "terawatt hour", "terawatt hours"], "TW*h"),
    ("Ah", &["Ah", "amp hour", "amp hours", "ampere hour", "ampere hours"], "A*h"),
    ("mAh", &["mAh", "mah", "milliamp hour", "milliamp hours"], "mA*h"),
    ("bps", &["bps", "bits per second"], "bit/s"),
    ("kbps", &["kbps", "Kbps"], "kbit/s"),
    ("Mbps", &["Mbps", "mbps"], "Mbit/s"),
    ("Gbps", &["Gbps", "gbps"], "Gbit/s"),
    ("Tbps", &["Tbps", "tbps"], "Tbit/s"),
    // A capital B is bytes.
    ("", &["Bps", "bytes per second"], "B/s"),
    ("", &["kBps", "KBps"], "kB/s"),
    ("", &["MBps"], "MB/s"),
    ("", &["GBps"], "GB/s"),
    ("", &["TBps"], "TB/s"),
    ("", &["sqm"], "m^2"),
    ("", &["sqkm"], "km^2"),
    ("", &["sqcm"], "cm^2"),
    ("", &["sqft"], "ft^2"),
    ("", &["sqin"], "in^2"),
    ("", &["sqyd"], "yd^2"),
    ("", &["sqmi"], "mi^2"),
    ("", &["cc", "ccm"], "cm^3"),
    ("", &["cuft"], "ft^3"),
    ("", &["cuin"], "in^3"),
    ("", &["cuyd"], "yd^3"),
];

/// Spellings added on top of the generated ones.
const EXTRA: &[(&str, &str)] = &[
    ("KB", "kB"),
    ("kb", "kB"),
    ("b", "bit"),
    ("Kb", "kbit"),
    ("Mb", "Mbit"),
    ("Gb", "Gbit"),
    ("Tb", "Tbit"),
    ("mb", "MB"),
    ("gb", "GB"),
    ("tb", "TB"),
    ("mcg", "µg"),
    ("micron", "µm"),
    ("microns", "µm"),
    ("Cal", "kcal"),
    ("kcals", "kcal"),
    ("cals", "cal"),
];

struct Builder {
    defs: Vec<UnitDef>,
    exact: TableMap<String, Unit>,
    folded: TableMap<String, Unit>,
}

impl Builder {
    fn push(&mut self, def: UnitDef) -> UnitId {
        self.defs.push(def);
        (self.defs.len() - 1) as UnitId
    }

    /// Registers a spelling; the first registration of a key wins.
    fn spelling(&mut self, text: &str, unit: &Unit) {
        let case_insensitive = text.chars().all(|c| c.is_lowercase() || " '.-".contains(c)) && text.len() > 2;
        let map = if case_insensitive { &mut self.folded } else { &mut self.exact };
        map.entry(text.to_string()).or_insert_with(|| unit.clone());
    }

    fn name(&mut self, text: &str, unit: &Unit) {
        self.folded.entry(text.to_lowercase()).or_insert_with(|| unit.clone());
    }

    fn register(&mut self, id: UnitId, aliases: impl IntoIterator<Item = String>) {
        let unit = Unit::of(id);
        let def = &self.defs[id as usize];
        let (symbol, name, plural) = (def.symbol.clone(), def.name.clone(), def.plural.clone());
        self.exact.entry(symbol).or_insert_with(|| unit.clone());
        self.name(&name, &unit);
        self.name(&plural, &unit);
        for alias in aliases {
            self.spelling(&alias, &unit);
        }
    }
}

pub(super) fn build() -> Registry {
    // Room for every spelling up front, so the tables never grow while filling.
    let mut b = Builder {
        defs: Vec::with_capacity(512),
        exact: TableMap::with_capacity_and_hasher(1024, Default::default()),
        folded: TableMap::with_capacity_and_hasher(2048, Default::default()),
    };
    let defs = definitions();

    let mut ids = Vec::new();
    for d in &defs {
        let id = b.push(UnitDef {
            symbol: d.symbol.into(),
            name: d.name.into(),
            plural: d.plural.map(String::from).unwrap_or_else(|| format!("{}s", d.name)),
            dim: d.dim,
            scale: d.scale,
            offset: d.offset,
            spelled: d.spelled,
            tight: d.tight,
            calendar: d.calendar,
            currency: None,
        });
        b.register(id, d.aliases.iter().map(|a| a.to_string()));
        ids.push(id);
    }

    for c in CURRENCIES {
        let id = b.push(UnitDef {
            symbol: c.code.into(),
            name: c.code.into(),
            plural: c.code.into(),
            dim: Dim::MONEY,
            scale: Number::ONE,
            offset: Number::ZERO,
            spelled: false,
            tight: false,
            calendar: None,
            currency: Some(c),
        });
        let unit = Unit::of(id);
        b.exact.entry(c.code.into()).or_insert_with(|| unit.clone());
        let lower = c.code.to_lowercase();
        if !AMBIGUOUS_CODES.contains(&lower.as_str()) {
            b.folded.entry(lower).or_insert_with(|| unit.clone());
        }
        for alias in c.aliases {
            b.spelling(alias, &unit);
        }
    }
    for &(code, name, plural, aliases) in MINOR_UNITS {
        let currency = find(code).expect("minor unit of a known currency");
        let id = b.push(UnitDef {
            symbol: name.into(),
            name: name.into(),
            plural: plural.into(),
            dim: Dim::MONEY,
            scale: Number::pow10(-(currency.decimals as i32)),
            offset: Number::ZERO,
            spelled: true,
            tight: false,
            calendar: None,
            currency: Some(currency),
        });
        b.register(id, aliases.iter().map(|a| a.to_string()));
    }

    // Prefixed units come last so they never shadow a base unit.
    for (d, &base) in defs.iter().zip(&ids) {
        for p in d.prefixes {
            let scale = Number::from_i64(p.base).powi(p.exp as i64) * d.scale;
            let calendar = match (d.calendar, p.exp) {
                (Some((Cal::Second, _)), -3) => Some((Cal::Millisecond, 1)),
                (Some((Cal::Second, _)), -6) => Some((Cal::Microsecond, 1)),
                (Some((Cal::Second, _)), -9) => Some((Cal::Nanosecond, 1)),
                _ => None,
            };
            let base_def = &b.defs[base as usize];
            let id = b.push(UnitDef {
                symbol: format!("{}{}", p.symbol, base_def.symbol),
                name: format!("{}{}", p.name, base_def.name),
                plural: format!("{}{}", p.name, base_def.plural),
                dim: d.dim,
                scale,
                offset: Number::ZERO,
                spelled: false,
                tight: false,
                calendar,
                currency: None,
            });
            let aliases = d.aliases.iter().map(|a| {
                let is_symbol = a.chars().any(|c| !c.is_lowercase()) || a.len() <= 2;
                format!("{}{a}", if is_symbol { p.symbol } else { p.name })
            });
            b.register(id, aliases.collect::<Vec<_>>());
        }
    }

    for &(text, target) in EXTRA {
        let unit = b.exact[target].clone();
        b.exact.entry(text.into()).or_insert(unit);
    }

    let mut compound_symbols = Vec::new();
    for &(symbol, names, definition) in COMPOUNDS {
        let unit = parse_compound(&b.exact, definition);
        for name in names {
            b.spelling(name, &unit);
        }
        if !symbol.is_empty() {
            compound_symbols.push((unit.key(), symbol.to_string()));
        }
    }

    let max_words =
        b.folded.keys().chain(b.exact.keys()).map(|k| k.bytes().filter(|&b| b == b' ').count() + 1).max().unwrap_or(1);
    Registry { defs: b.defs, exact: b.exact, folded: b.folded, compound_symbols, max_words }
}

/// Parses definitions like "km/h", "W*h" or "m^2" made of exact symbols.
fn parse_compound(exact: &TableMap<String, Unit>, text: &str) -> Unit {
    let (num, den) = text.split_once('/').unwrap_or((text, ""));
    let mut factors = Vec::new();
    for (part, sign) in [(num, 1), (den, -1)] {
        for f in part.split('*').filter(|f| !f.is_empty()) {
            let (symbol, exp) = f.split_once('^').map_or((f, 1), |(s, e)| (s, e.parse::<i8>().unwrap()));
            let unit = exact.get(symbol).unwrap_or_else(|| panic!("unknown unit {symbol} in {text}"));
            factors.extend(unit.pow(exp * sign).factors().iter().copied());
        }
    }
    Unit { factors }
}
