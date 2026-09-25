//! Writing values as text.

use jiff::Zoned;

use crate::config::{Config, Now};
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
    match value {
        Value::Quantity(q) => quantity(q, display, config),
        Value::Percent(p) => format!("{}%", number(*p, config)),
        Value::Moment(m) => moment(m, config, now.get()),
        Value::Duration(d) => duration(d),
        Value::Bool(b) => b.to_string(),
        Value::Text(t) => t.clone(),
    }
}

/// A number with the configured precision and thousands separators.
pub(crate) fn number(n: Number, config: &Config) -> String {
    match plain_digits(n, config.precision) {
        Some(digits) => group(&digits, config),
        None => scientific(n, config.precision),
    }
}

/// Digits without grouping, or `None` if the number needs scientific notation.
fn plain_digits(n: Number, precision: u32) -> Option<String> {
    if !n.is_finite() {
        return Some(if n.is_negative() { "-∞" } else { "∞" }.into());
    }
    if n.is_zero() {
        return Some("0".into());
    }
    let mag = n.magnitude();
    if !(-9..21).contains(&mag) {
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

fn scientific(n: Number, precision: u32) -> String {
    let text = format!("{:.*e}", precision.saturating_sub(1) as usize, n.to_f64());
    match text.split_once('e') {
        Some((mantissa, exp)) => format!("{}e{exp}", trim_zeros(mantissa.to_string())),
        None => text,
    }
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
        Display::Scientific => scientific(q.number, config.precision),
        Display::Fraction => fraction(q.number, config),
        Display::Multiplier => return format!("{}x", number(q.number, config)),
        Display::Parts(units) => return parts(q, units, config),
        Display::Plain => {
            plain_digits(q.number, config.precision.max(20)).unwrap_or_else(|| scientific(q.number, config.precision))
        }
        Display::Auto | Display::Each(_) | Display::Note(_) => {
            if let Some((currency, 1)) = q.unit.currency() {
                return money(q, currency, config);
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
    let text = unit_text(unit, n != Number::ONE);
    let tight = unit.single().is_some_and(|d| d.tight) || text.starts_with('/');
    if tight { format!("{digits}{text}") } else { format!("{digits} {text}") }
}

/// Money: "$12.50", "1,200 KZT", "$20.00/hour".
fn money(q: &Quantity, currency: &Currency, config: &Config) -> String {
    let n = q.number;
    let digits = if currency.crypto {
        group(
            &plain_digits(n.abs().round_dp(8), config.precision)
                .unwrap_or_else(|| scientific(n.abs(), config.precision)),
            config,
        )
    } else {
        fixed(n.abs(), currency.decimals as u32, config)
    };
    let sign = if n.is_negative() && !n.round_dp(currency.decimals as u32).is_zero() { "-" } else { "" };
    let amount = match currency.symbol {
        Some(symbol) => format!("{sign}{symbol}{digits}"),
        None => format!("{sign}{digits} {}", currency.code),
    };
    // The rest of a rate: "/hour".
    let rest: Vec<(UnitId, i8)> =
        q.unit.factors().iter().copied().filter(|&(id, _)| registry().def(id).currency.is_none()).collect();
    if rest.is_empty() {
        return amount;
    }
    format!("{amount}{}", unit_text(&unit_from(&rest), false))
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
        let name = if def.spelled && (words || long) { def.name.clone() } else { def.symbol.clone() };
        format!("{name}{}", superscript(e))
    };
    let num_text: Vec<String> = num.iter().map(|&(id, e)| symbol(id, e)).collect();
    let den_text: Vec<String> = den.iter().map(|&(id, e)| symbol(id, -e)).collect();
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

/// "5 ft 6 in", "2 lb 3 oz".
fn parts(q: &Quantity, units: &[Unit], config: &Config) -> String {
    let scale = |u: &Unit| u.single().map(|d| d.scale);
    let (Some(first), true) = (scale(&q.unit), units.iter().all(|u| scale(u).is_some())) else {
        return quantity(q, &Display::Auto, config);
    };
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
        out.push(with_unit(&number(count, config), count, u));
    }
    format!("{}{}", if negative { "-" } else { "" }, out.join(" "))
}

fn moment(m: &Moment, config: &Config, now: &Zoned) -> String {
    let local = now.with_time_zone(now.time_zone().clone());
    let kind = match m.kind {
        // A clock time on another day shows the date too.
        MomentKind::Clock if m.time.date() != local.date() => MomentKind::DateTime,
        k => k,
    };
    let date = m.time.strftime("%a, %-d %b %Y").to_string();
    let clock = clock(&m.time, config.clock_24h, m.seconds);
    let mut text = match kind {
        MomentKind::Date => date,
        MomentKind::Clock => clock,
        MomentKind::DateTime => format!("{date} {clock}"),
    };
    let other_zone = m.time.time_zone().iana_name() != now.time_zone().iana_name();
    if kind != MomentKind::Date && (m.zoned || other_zone) {
        text.push(' ');
        text.push_str(&zones::label(&m.time));
    }
    text
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

fn duration(d: &Duration) -> String {
    use jiff::Unit as Cal;
    let span = d.span.abs();
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
        let frac = if frac.is_zero() { String::new() } else { frac.to_string().trim_start_matches('0').to_string() };
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
            format!("{n} {name}{}", if n == 1 { "" } else { "s" })
        })
        .collect();
    if !seconds.is_zero() || parts.is_empty() {
        let text = plain_digits(seconds.round_dp(3), 10).unwrap_or_default();
        parts.push(format!("{text} second{}", if seconds == Number::ONE { "" } else { "s" }));
    }
    format!("{sign}{}", parts.join(" "))
}
