//! Writing values as text.

use jiff::Zoned;

use crate::config::{Config, Formatting, Notation, Now, ScalarSystem, SymbolPosition, TimespanStyle};
use crate::currency::Currency;
use crate::error::{Result, bail};
use crate::eval::Display;
use crate::number::Number;
use crate::units::{Unit, UnitId, registry};
use crate::value::{Duration, Moment, MomentKind, Quantity, Value};
use crate::zones;

pub(crate) fn render(value: &Value, display: &Display, config: &Config, now: &Now) -> String {
    if let Display::Note(note) = display {
        return format!("{} ({note})", render(value, &Display::Auto, config, now));
    }
    if let Display::Each(values) = display {
        let separator = if config.decimal_comma { "; " } else { ", " };
        return values.iter().map(|v| render(v, &Display::Auto, config, now)).collect::<Vec<_>>().join(separator);
    }
    if let Display::Digits(text) = display {
        return if config.decimal_comma { text.replace('.', ",") } else { text.clone() };
    }
    if let Some(precision) = rounded_precision(value, display).filter(|&p| p > config.precision) {
        return render(value, &Display::Auto, &Config { precision, ..config.clone() }, now);
    }
    if let (Display::Period, Value::Duration(d)) = (display, value)
        && let Some(text) = period(d)
    {
        return text;
    }
    // "5 ft 6 in" for 5.5 ft, if the config asks for it.
    if let (Value::Quantity(q), Display::Auto | Display::Scaled) = (value, display)
        && let Some(parts) = imperial_parts(q, config)
    {
        return quantity(q, &Display::Parts(parts), config);
    }
    match value {
        Value::Quantity(q) => quantity(q, display, config),
        Value::Percent(p) => format!("{}%", number(*p, config)),
        Value::Moment(m) => moment(m, config, now.get()),
        Value::Duration(d) => duration(d, config),
        Value::Bool(b) => b.to_string(),
        Value::Text(t) => t.clone(),
    }
}

/// The value as its answer shows it: rounded to the digits shown, money to its coins.
pub(crate) fn shown(value: &Value, display: &Display, config: &Config) -> Value {
    let round = |n: Number| match config.formatting.decimal_places {
        Some(dp) => n.round_dp(dp.min(28)),
        None if config.formatting.plain_magnitudes().contains(&n.magnitude()) => n.round_sig(config.precision),
        None => n,
    };
    match (value, display) {
        (_, Display::Places(_) | Display::Significant(_) | Display::Digits(_)) => value.clone(),
        (Value::Quantity(q), _) => {
            let number = match q.unit.currency() {
                Some((currency, 1)) if config.formatting.round_currency && q.unit.single().is_some() => {
                    let minor = q.unit.single().map_or(Number::ONE, |d| d.scale);
                    (q.number * minor).round_dp(currency.decimals as u32) / minor
                }
                _ => round(q.number),
            };
            Value::Quantity(Quantity::new(number, q.unit.clone()))
        }
        (Value::Percent(p), _) => Value::Percent(round(*p)),
        _ => value.clone(),
    }
}

/// Significant digits that show every digit a rounding asked for: 15 for "pi to 14 dp".
fn rounded_precision(value: &Value, display: &Display) -> Option<u32> {
    let n = match value {
        Value::Quantity(q) => q.number,
        Value::Percent(p) => *p,
        _ => return None,
    };
    // Scientific notation keeps the configured precision.
    let mag = n.magnitude();
    if !(-9..21).contains(&mag) {
        return None;
    }
    match display {
        Display::Places(dp) => Some((mag + 1 + *dp as i32).max(1) as u32),
        Display::Significant(digits) => Some(*digits),
        _ => None,
    }
}

/// A number with the configured precision and thousands separators.
pub(crate) fn number(n: Number, config: &Config) -> String {
    if let Some(text) = scaled(n, config, false, None) {
        return text;
    }
    unscaled(n, config)
}

/// A number without a scale.
fn unscaled(n: Number, config: &Config) -> String {
    let digits = match config.formatting.decimal_places {
        Some(dp) => places_digits(n, dp, config.formatting.plain_magnitudes()),
        None => plain_digits(n, config.precision, config.formatting.plain_magnitudes()),
    };
    match digits {
        Some(digits) => group(&digits, config),
        None => scientific(n, config),
    }
}

/// Digits rounded to `dp` decimal places, or `None` for scientific notation.
fn places_digits(n: Number, dp: u32, plain: std::ops::Range<i32>) -> Option<String> {
    if !n.is_zero() && !plain.contains(&n.magnitude()) {
        return None;
    }
    let text = match n.round_dp(dp.min(28)) {
        Number::Exact(d) => d.normalize().to_string(),
        Number::Float(f) => trim_zeros(format!("{f:.*}", dp.min(28) as usize)),
    };
    Some(if text == "-0" { "0".into() } else { text })
}

/// Scales after money written with a code, which read back as no unit: "2.5bn MMK".
const SCALES_BEFORE_CODES: &[(i32, &str)] = &[(12, "tn"), (9, "bn"), (6, "M"), (3, "k")];

/// Scales in their systems, biggest first: plain numbers take SI letters, money its own.
fn scales(system: Option<ScalarSystem>) -> &'static [(i32, &'static str)] {
    match system {
        None => &[(12, "T"), (9, "G"), (6, "M"), (3, "k")],
        Some(ScalarSystem::Standard) => &[(12, "T"), (9, "B"), (6, "M"), (3, "K")],
        Some(ScalarSystem::Lowercase) => &[(12, "t"), (9, "b"), (6, "m"), (3, "k")],
        Some(ScalarSystem::British) => &[(12, "tn"), (9, "bn"), (6, "m"), (3, "k")],
        Some(ScalarSystem::Russian) => &[(12, " трлн"), (9, " млрд"), (6, " млн"), (3, " тыс.")],
        Some(ScalarSystem::Chinese) => &[(8, "亿"), (4, "万")],
    }
}

/// `5M`, `2.5k`: a number with a scale, when the config asks for one or the line wrote
/// one (`written`) and the answer stays exact with it: "$5M / 4" is "$1.25M", "$5M / 3" isn't.
fn scaled(n: Number, config: &Config, written: bool, money: Option<ScalarSystem>) -> Option<String> {
    scaled_with(n, config, written, scales(money))
}

fn scaled_with(n: Number, config: &Config, written: bool, scales: &[(i32, &str)]) -> Option<String> {
    let f = &config.formatting;
    let abs = n.abs();
    let reached = f.notation_threshold.is_some_and(|t| abs >= t.value());
    let rounded = f.notation == Notation::On || (f.notation == Notation::Auto && reached);
    if !rounded && !(f.notation == Notation::Auto && written) {
        return None;
    }
    let &(exp, suffix) = scales.iter().find(|&&(exp, _)| abs >= Number::pow10(exp))?;
    let value = n / Number::pow10(exp);
    if !rounded && value.round_dp(2) != value {
        return None;
    }
    let plain = Config { formatting: Formatting { notation: Notation::Off, ..f.clone() }, ..config.clone() };
    Some(format!("{}{suffix}", unscaled(value, &plain)))
}

/// Digits without grouping, or `None` if the number needs scientific notation.
fn plain_digits(n: Number, precision: u32, plain: std::ops::Range<i32>) -> Option<String> {
    if !n.is_finite() {
        return Some(if n.is_negative() { "-∞" } else { "∞" }.into());
    }
    if n.is_zero() {
        return Some("0".into());
    }
    let mag = n.magnitude();
    if !plain.contains(&mag) {
        return None;
    }
    // Keep `precision` significant digits, but never round the whole part.
    let dp = (precision as i32 - 1 - mag).clamp(0, 28) as u32;
    let text = match n.round_dp(dp) {
        Number::Exact(d) => d.normalize().to_string(),
        Number::Float(f) => trim_zeros(format!("{f:.*}", dp as usize)),
    };
    Some(if text == "-0" { "0".into() } else { text })
}

fn trim_zeros(s: String) -> String {
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// Inserts thousands separators into the whole part, in the configured style.
fn group(digits: &str, config: &Config) -> String {
    let (thousands, decimal) = if config.decimal_comma { ('.', ',') } else { (',', '.') };
    let (sign, rest) = digits.strip_prefix('-').map_or(("", digits), |r| ("-", r));
    let (int, frac) = rest.split_once('.').map_or((rest, None), |(i, f)| (i, Some(f)));
    let mut out = String::from(sign);
    for (i, c) in int.chars().enumerate() {
        if config.thousands_separators && i > 0 && (int.len() - i) % 3 == 0 {
            out.push(thousands);
        }
        out.push(c);
    }
    if let Some(f) = frac {
        out.push(decimal);
        out.push_str(f);
    }
    out
}

fn scientific(n: Number, config: &Config) -> String {
    let text = format!("{:.*e}", config.precision.saturating_sub(1) as usize, n.to_f64());
    let text = match text.split_once('e') {
        Some((mantissa, exp)) => format!("{}e{exp}", trim_zeros(mantissa.to_string())),
        None => text,
    };
    if config.decimal_comma { text.replace('.', ",") } else { text }
}

/// Exactly `dp` decimals, as for money: `12.50`.
fn fixed(n: Number, dp: u32, config: &Config) -> String {
    let rounded = n.round_dp(dp);
    let text = match rounded {
        Number::Exact(d) => {
            let mut d = d;
            d.rescale(dp);
            d.to_string()
        }
        Number::Float(f) => format!("{f:.*}", dp as usize),
    };
    group(&text, config)
}

pub(crate) fn radix(n: Number, radix: u32) -> Result<String> {
    let Some(i) = n.to_i128() else { bail!("only whole numbers can be shown in base {radix}") };
    let abs = i.unsigned_abs();
    let digits = match radix {
        16 => format!("0x{abs:X}"),
        2 => format!("0b{abs:b}"),
        _ => format!("0o{abs:o}"),
    };
    Ok(if i < 0 { format!("-{digits}") } else { digits })
}

/// The note nearest a frequency in equal temperament, from A4 = 440 Hz: "C#4".
pub(crate) fn pitch(hz: f64) -> Result<String> {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let midi = (69.0 + 12.0 * (hz / 440.0).log2()).round();
    if !(0.0..=131.0).contains(&midi) {
        bail!("no musical note is that high or low");
    }
    let midi = midi as usize;
    Ok(format!("{}{}", NAMES[midi % 12], midi / 12 - 1))
}

/// Digits of a whole number in any base from 2 to 36, without a prefix: `100110`.
pub(crate) fn digits_in_base(n: Number, base: u32) -> Result<String> {
    let Some(i) = n.to_i128() else { bail!("only whole numbers can be shown in base {base}") };
    let mut abs = i.unsigned_abs();
    let mut digits = Vec::new();
    loop {
        digits.push(char::from_digit((abs % base as u128) as u32, base).unwrap_or('?').to_ascii_uppercase());
        abs /= base as u128;
        if abs == 0 {
            break;
        }
    }
    if i < 0 {
        digits.push('-');
    }
    Ok(digits.iter().rev().collect())
}

/// Closest simple fraction, like `1/3`.
fn fraction(n: Number, config: &Config) -> String {
    if n.is_integer() {
        return number(n, config);
    }
    if let Some(d) = n.as_decimal().map(|d| d.normalize())
        && d.scale() <= 6
    {
        let (num, den) = (d.mantissa().unsigned_abs(), 10u128.pow(d.scale()));
        let g = gcd(num, den);
        return format!("{}{}/{}", if n.is_negative() { "-" } else { "" }, num / g, den / g);
    }
    let x = n.abs().to_f64();
    let (mut h0, mut h1, mut k0, mut k1) = (0i64, 1i64, 1i64, 0i64);
    let mut rest = x;
    for _ in 0..64 {
        let a = rest.floor();
        let (h2, k2) = (a as i64 * h1 + h0, a as i64 * k1 + k0);
        if k2 > 1_000_000 {
            break;
        }
        (h0, h1, k0, k1) = (h1, h2, k1, k2);
        if (h1 as f64 / k1 as f64 - x).abs() < 1e-12 || rest - a < 1e-12 {
            break;
        }
        rest = 1.0 / (rest - a);
    }
    if k1 == 0 || (h1 as f64 / k1 as f64 - x).abs() > 1e-9 {
        return number(n, config);
    }
    format!("{}{h1}/{k1}", if n.is_negative() { "-" } else { "" })
}

fn gcd(a: u128, b: u128) -> u128 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn quantity(q: &Quantity, display: &Display, config: &Config) -> String {
    let digits = match display {
        Display::Radix(r) => radix(q.number, *r).unwrap_or_else(|e| e.to_string()),
        Display::Scientific => scientific(q.number, config),
        Display::Fraction => fraction(q.number, config),
        Display::Multiplier => return format!("{}x", number(q.number, config)),
        Display::Parts(units) => return parts(q, units, config),
        Display::Plain => {
            plain_digits(q.number, config.precision.max(20), -9..21).unwrap_or_else(|| scientific(q.number, config))
        }
        Display::Scaled => {
            if let Some((currency, 1)) = q.unit.currency() {
                return money(q, currency, config, true);
            }
            scaled(q.number, config, true, None).unwrap_or_else(|| number(q.number, config))
        }
        Display::Auto
        | Display::Each(_)
        | Display::Note(_)
        | Display::Period
        | Display::Places(_)
        | Display::Significant(_)
        | Display::Digits(_) => {
            if let Some((currency, 1)) = q.unit.currency() {
                return money(q, currency, config, false);
            }
            number(q.number, config)
        }
    };
    with_unit(&digits, q.number, &q.unit)
}

/// Joins digits and unit: "5 km", "90°", "30/week".
fn with_unit(digits: &str, n: Number, unit: &Unit) -> String {
    if unit.is_none() {
        return digits.to_string();
    }
    let text = unit_text(unit, n.abs() != Number::ONE);
    let tight = unit.single().is_some_and(|d| d.tight) || text.starts_with('/');
    if tight { format!("{digits}{text}") } else { format!("{digits} {text}") }
}

/// Money: "$12.50", "1,200 KZT", "$20.00/hour", "$5M".
fn money(q: &Quantity, currency: &Currency, config: &Config, written_scale: bool) -> String {
    let f = &config.formatting;
    // Cents and satoshis are shown in the whole currency.
    let minor = q.unit.factors().iter().map(|&(id, _)| registry().def(id)).find(|d| d.currency.is_some());
    let n = q.number * minor.map_or(Number::ONE, |d| d.scale);
    let decimals = currency.decimals as u32;
    let custom = f.symbols.iter().find(|s| s.code.eq_ignore_ascii_case(currency.code));
    let (symbol, position) = match (custom, currency.symbol) {
        (Some(custom), _) => (custom.symbol.as_str(), custom.position),
        (None, Some(symbol)) if f.currency_symbols => (symbol, SymbolPosition::Before),
        _ => (currency.code, SymbolPosition::AfterWithSpace),
    };
    // Before a code "K", "B" and "m" would be kelvins, bytes and meters: "2.5bn MMK".
    let after = matches!(position, SymbolPosition::After | SymbolPosition::AfterWithSpace);
    let scales = match f.scalars {
        ScalarSystem::Standard | ScalarSystem::Lowercase | ScalarSystem::British if after => SCALES_BEFORE_CODES,
        system => scales(Some(system)),
    };
    let digits = if let Some(text) = scaled_with(n.abs(), config, written_scale, scales) {
        text
    } else if currency.crypto {
        match plain_digits(n.abs().round_dp(8), config.precision, config.formatting.plain_magnitudes()) {
            Some(digits) => group(&digits, config),
            None => scientific(n.abs(), config),
        }
    } else if !f.round_currency {
        unscaled(n.abs(), config)
    } else if !f.currency_trailing_zeros && n.round_dp(decimals).is_integer() {
        fixed(n.abs(), 0, config)
    } else {
        fixed(n.abs(), decimals, config)
    };
    let sign = if n.is_negative() && !n.round_dp(decimals).is_zero() { "-" } else { "" };
    let amount = match position {
        SymbolPosition::Before => format!("{sign}{symbol}{digits}"),
        SymbolPosition::BeforeWithSpace => format!("{sign}{symbol} {digits}"),
        SymbolPosition::After => format!("{sign}{digits}{symbol}"),
        SymbolPosition::AfterWithSpace => format!("{sign}{digits} {symbol}"),
    };
    // The rest of a rate: "/hour".
    let rest: Vec<(UnitId, i8)> =
        q.unit.factors().iter().copied().filter(|&(id, _)| registry().def(id).currency.is_none()).collect();
    with_unit(&amount, n, &unit_from(&rest))
}

fn unit_from(factors: &[(UnitId, i8)]) -> Unit {
    factors.iter().fold(Unit::none(), |u, &(id, e)| u.product(&Unit::of(id).pow(e)))
}

/// A unit and its power.
type Factor = (UnitId, i8);

fn superscript(e: i8) -> String {
    match e {
        1 => String::new(),
        2 => "²".into(),
        3 => "³".into(),
        e => format!("^{e}"),
    }
}

/// Text for a unit: "km", "hours", "km/h", "m²", "$/hour".
pub(crate) fn unit_text(unit: &Unit, plural: bool) -> String {
    let reg = registry();
    if let Some(symbol) = reg.compound_symbol(unit) {
        return symbol.to_string();
    }
    if let Some(def) = unit.single() {
        return match (def.spelled, plural) {
            (true, true) => def.plural.clone(),
            (true, false) => def.name.clone(),
            _ => def.symbol.clone(),
        };
    }
    let (num, den): (Vec<Factor>, Vec<Factor>) = unit.factors().iter().copied().partition(|&(_, e)| e > 0);
    // Rates of money or counts read better with words: "$/hour", "/week".
    let words = num.is_empty() || (num.len() == 1 && reg.def(num[0].0).currency.is_some());
    let symbol = |id: UnitId, e: i8| {
        let def = reg.def(id);
        // "km/day" reads better than "km/d".
        let long = def.calendar.is_some_and(|(cal, _)| cal >= jiff::Unit::Day);
        // "in" after other units or "/" reads as a word: "N·inch³", "$5/inch".
        let inch = def.symbol == "in" && !matches!(unit.factors(), [(_, e)] if *e > 0);
        let name = if def.spelled && (words || long) || inch { def.name.clone() } else { def.symbol.clone() };
        format!("{name}{}", superscript(e))
    };
    // A compound with a name of its own keeps it: "$/kWh", not "$/(kW·hour)"; "/(N·m)" stays in parentheses.
    let named = |factors: &[Factor], sign: i8| {
        let unit = factors.iter().fold(Unit::none(), |u, &(id, e)| u.product(&Unit::of(id).pow(e * sign)));
        let symbol = reg.compound_symbol(&unit).filter(|_| factors.len() > 1)?;
        Some(match sign < 0 && symbol.contains('·') {
            true => vec![format!("({symbol})")],
            false => vec![symbol.to_string()],
        })
    };
    let num_text = named(&num, 1).unwrap_or_else(|| num.iter().map(|&(id, e)| symbol(id, e)).collect());
    let den_text = named(&den, -1).unwrap_or_else(|| den.iter().map(|&(id, e)| symbol(id, -e)).collect());
    let mut text = num_text.join("·");
    if !den_text.is_empty() {
        text.push('/');
        if den_text.len() > 1 {
            text.push_str(&format!("({})", den_text.join("·")));
        } else {
            text.push_str(&den_text[0]);
        }
    }
    text
}

/// Feet and inches or pounds and ounces for a fractional amount of feet or pounds,
/// if the config asks for compound imperial units.
fn imperial_parts(q: &Quantity, config: &Config) -> Option<Vec<Unit>> {
    if !config.formatting.compound_imperial_units || q.number.is_integer() {
        return None;
    }
    let smaller = match q.unit.single()?.symbol.as_str() {
        "ft" => "in",
        "lb" => "oz",
        _ => return None,
    };
    Some(vec![q.unit.clone(), registry().get(smaller)])
}

/// "5 ft 6 in", "2 lb 3 oz".
fn parts(q: &Quantity, units: &[Unit], config: &Config) -> String {
    let scale = |u: &Unit| u.single().map(|d| d.scale);
    let (Some(first), true) = (scale(&q.unit), units.iter().all(|u| scale(u).is_some())) else {
        return quantity(q, &Display::Auto, config);
    };
    // "in inches and feet" still starts with feet.
    let mut units = units.to_vec();
    units.sort_by(|a, b| scale(b).partial_cmp(&scale(a)).unwrap_or(std::cmp::Ordering::Equal));
    let negative = q.number.is_negative();
    let mut rest = q.number.abs() * first;
    let mut out = Vec::new();
    for (i, u) in units.iter().enumerate() {
        let size = scale(u).unwrap();
        let mut count = rest / size;
        if i + 1 < units.len() {
            // Avoid "4 ft 12 in" from rounding.
            count = (count + Number::parse("1e-9").unwrap()).floor();
            rest = rest - count * size;
            if count.is_zero() {
                continue;
            }
        }
        let count = if i + 1 == units.len() { count.round_dp(2) } else { count };
        if count.is_zero() && !out.is_empty() {
            continue;
        }
        let digits = number(count, config);
        // "40° 26′ 46″": minutes and seconds of arc take their signs.
        out.push(match u.single().map(|d| d.symbol.as_str()) {
            Some("arcmin") => format!("{digits}′"),
            Some("arcsec") => format!("{digits}″"),
            _ => with_unit(&digits, count, u),
        });
    }
    format!("{}{}", if negative { "-" } else { "" }, out.join(" "))
}

fn moment(m: &Moment, config: &Config, now: &Zoned) -> String {
    let kind = match m.kind {
        // A clock time on another day shows the date too.
        MomentKind::Clock if m.time.date() != now.date() => MomentKind::DateTime,
        k => k,
    };
    let mut text = moment_text(m, kind, config);
    let other_zone = m.time.time_zone().iana_name() != now.time_zone().iana_name();
    if kind != MomentKind::Date && (m.zoned || other_zone) {
        text.push(' ');
        text.push_str(&zones::label(&m.time));
    }
    text
}

/// A date, a clock time or both, without the zone.
pub(crate) fn moment_text(m: &Moment, kind: MomentKind, config: &Config) -> String {
    let date = || m.time.strftime("%a, %-d %b %Y").to_string();
    let clock = || clock(&m.time, config.clock_24h, m.seconds);
    match kind {
        MomentKind::Date => date(),
        MomentKind::Clock => clock(),
        MomentKind::DateTime => format!("{} {}", date(), clock()),
    }
}

/// The time between two dates in the configured style: "2 months 14 days", "73 days".
fn restyle(d: &Duration, style: TimespanStyle) -> Option<jiff::Span> {
    use jiff::Unit as Cal;
    let largest = match style {
        TimespanStyle::Automatic => return None,
        TimespanStyle::MonthsAndDays => Cal::Month,
        TimespanStyle::WeeksAndDays => Cal::Week,
        TimespanStyle::Days => Cal::Day,
    };
    let start = d.anchor?;
    let end = start.checked_add(d.span).ok()?;
    let span = start.until((largest, end)).ok()?;
    // Months and days only: no weeks in between.
    match style {
        TimespanStyle::MonthsAndDays => {
            let days = span.get_weeks() as i64 * 7 + span.get_days() as i64;
            jiff::Span::new()
                .years(span.get_years())
                .months(span.get_months())
                .try_days(days)
                .ok()
                .map(|s| s.hours(span.get_hours()).minutes(span.get_minutes()).seconds(span.get_seconds()))
        }
        _ => Some(span),
    }
}

/// The first and last day of an anchored duration: "1 Jul – 30 Sep 2026".
fn period(d: &Duration) -> Option<String> {
    let first = d.anchor?.date();
    let last = first.checked_add(d.span).ok()?.yesterday().ok()?;
    let start = if first.year() == last.year() { first.strftime("%-d %b") } else { first.strftime("%-d %b %Y") };
    Some(format!("{start} – {}", last.strftime("%-d %b %Y")))
}

fn clock(time: &Zoned, h24: bool, seconds: bool) -> String {
    let seconds = seconds && time.second() != 0;
    match (h24, seconds) {
        (true, false) => time.strftime("%H:%M").to_string(),
        (true, true) => time.strftime("%H:%M:%S").to_string(),
        (false, false) => time.strftime("%-I:%M %P").to_string(),
        (false, true) => time.strftime("%-I:%M:%S %P").to_string(),
    }
}

fn duration(d: &Duration, config: &Config) -> String {
    use jiff::Unit as Cal;
    let restyled = restyle(d, config.formatting.timespans);
    let span = restyled.unwrap_or(d.span).abs();
    let sign = if d.span.is_negative() { "-" } else { "" };
    let get = |cal| crate::eval::span_field(&span, cal);
    let seconds = Number::from_i64(get(Cal::Second))
        + Number::from_i64(get(Cal::Millisecond)) / Number::from_i64(1000)
        + Number::from_i64(get(Cal::Microsecond)) / Number::pow10(6)
        + Number::from_i64(get(Cal::Nanosecond)) / Number::pow10(9);
    if d.laptime {
        let total_minutes =
            get(Cal::Week) * 7 * 24 * 60 + get(Cal::Day) * 24 * 60 + get(Cal::Hour) * 60 + get(Cal::Minute);
        let s = seconds.round_dp(3);
        let whole = s.trunc().to_i64().unwrap_or(0);
        let frac = s.fract();
        let point = if config.decimal_comma { "," } else { "." };
        let frac = if frac.is_zero() { String::new() } else { frac.to_string().replacen("0.", point, 1) };
        return format!("{sign}{:02}:{:02}:{whole:02}{frac}", total_minutes / 60, total_minutes % 60);
    }
    let names = [
        (Cal::Year, "year"),
        (Cal::Month, "month"),
        (Cal::Week, "week"),
        (Cal::Day, "day"),
        (Cal::Hour, "hour"),
        (Cal::Minute, "minute"),
    ];
    let mut parts: Vec<String> = names
        .iter()
        .filter(|(cal, _)| get(*cal) != 0)
        .map(|&(cal, name)| {
            let n = get(cal);
            format!("{} {name}{}", number(Number::from_i64(n), config), if n == 1 { "" } else { "s" })
        })
        .collect();
    if !seconds.is_zero() || parts.is_empty() {
        // Every digit there is, down to the nanosecond: "1 hour 0.000000001 seconds".
        let text = group(&plain_digits(seconds, 10, -9..21).unwrap_or_default(), config);
        parts.push(format!("{text} second{}", if seconds == Number::ONE { "" } else { "s" }));
    }
    format!("{sign}{}", parts.join(" "))
}
