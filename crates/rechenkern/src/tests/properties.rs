//! Properties of random lines, checked with proptest; a failure is shrunk to
//! the shortest line that shows it. The seed is fixed, so `cargo test` runs the
//! same cases every time. To look further:
//! `PROPTEST_RNG_SEED=$RANDOM PROPTEST_CASES=100000 cargo test --lib properties`.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::LazyLock;

use jiff::tz::TimeZone;
use proptest::prelude::*;
use proptest::sample::select;
use proptest::test_runner::{Config as Runner, RngSeed};

use super::{CLOCK_READS, SHEET_READS, answer, calculator, config};
use crate::config::{Config, Now};
use crate::eval::Env;
use crate::number::Number;
use crate::parser::words;
use crate::rates::Rates;
use crate::units::{Unit, registry};
use crate::value::{Quantity, Value};
use crate::zones;

fn runner() -> Runner {
    let mut runner = Runner::default();
    if std::env::var_os("PROPTEST_RNG_SEED").is_none() {
        runner.rng_seed = RngSeed::Fixed(20260923);
    }
    if std::env::var_os("PROPTEST_CASES").is_none() {
        runner.cases = 2000;
    }
    runner
}

/// Words that random lines are made of: the whole vocabulary.
static VOCABULARY: LazyLock<Vec<String>> = LazyLock::new(|| {
    let phrases = words::FUNCTIONS
        .phrases()
        .chain(words::FORMATS.phrases())
        .chain(words::DATE_PARTS.phrases())
        .chain(words::HOLIDAYS.phrases())
        .chain(words::SKY.phrases())
        .chain(words::FOODS.phrases().step_by(5))
        .chain(words::KEYWORDS.iter().copied())
        .chain(registry().spellings())
        .chain(zones::place_names().into_iter().step_by(5));
    phrases.map(String::from).collect()
});

/// Plain words of notes, which the calculator should take as comments.
const FILLER: &[&str] = &["for", "lunch", "the", "cats", "with", "team", "costs", "paid", "is", "what", "a", "per"];

const SYMBOLS: &[&str] = &["+", "-", "*", "/", "^", "%", "(", ")", "=", ",", ":", "$", "€", "x", "!", ">", "<"];

const TIMES: &[&str] = &["3pm", "14:30", "9:15am", "March 5", "5 May 2027", "2026-09-23", "12/25/2026", "Q3"];

fn number(separators: bool) -> BoxedStrategy<String> {
    let whole = (0u32..2000).prop_map(|n| n.to_string());
    if !separators {
        return whole.boxed();
    }
    prop_oneof![
        3 => whole,
        2 => (0u32..100_000, 0u32..1000).prop_map(|(a, b)| format!("{a}.{b}")),
        1 => select(&["1,500", "1.500", "10,50", "0.25", "1e3", "2.5k", "1/2", "3/4"][..]).prop_map(String::from),
    ]
    .boxed()
}

fn token(separators: bool) -> impl Strategy<Value = String> {
    let symbols = SYMBOLS.iter().copied().filter(move |s| separators || *s != ",");
    prop_oneof![
        4 => select(VOCABULARY.clone()),
        3 => number(separators),
        2 => select(symbols.collect::<Vec<_>>()).prop_map(String::from),
        1 => select(TIMES.iter().copied().filter(move |t| separators || !t.contains([',', '.'])).collect::<Vec<_>>())
            .prop_map(String::from),
        1 => select(FILLER).prop_map(String::from),
    ]
}

/// A line of up to eight tokens, mostly separated by spaces.
fn line_with(separators: bool) -> impl Strategy<Value = String> {
    prop::collection::vec((token(separators), prop::bool::weighted(0.8)), 1..8).prop_map(|tokens| {
        let mut line = String::new();
        for (token, space) in tokens {
            if space && !line.is_empty() {
                line.push(' ');
            }
            line.push_str(&token);
        }
        line
    })
}

fn line() -> impl Strategy<Value = String> {
    line_with(true)
}

/// Lines of a sheet: random ones, and ones that set and read variables and earlier lines.
fn sheet_line() -> impl Strategy<Value = String> {
    const SHEET: &[&str] = &[
        "dbl(a) = a * 2",
        "dbl(a) = a + x",
        "dbl(3)",
        "dbl(x) + 1",
        "x = 5",
        "x = 2 km",
        "y = x * 2",
        "x += 1",
        "x",
        "y + 1",
        "z = $10",
        "z * 3",
        "sum",
        "average",
        "prev",
        "prev + 1",
        "line 1",
        "line 2 * 2",
        "total in EUR",
        "",
        "# Heading",
        "---",
        "$20",
        "3 km",
        "now",
        "x km",
    ];
    prop_oneof![2 => line(), 3 => select(SHEET).prop_map(String::from)]
}

/// An edit of a sheet: set, insert, remove or move a line, by an index taken modulo its length.
#[derive(Clone, Debug)]
enum Edit {
    Set(usize, String),
    Insert(usize, String),
    Remove(usize),
    Move(usize, usize),
    Behaviour(usize, u8),
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        3 => (0..20usize, sheet_line()).prop_map(|(i, t)| Edit::Set(i, t)),
        2 => (0..20usize, sheet_line()).prop_map(|(i, t)| Edit::Insert(i, t)),
        1 => (0..20usize).prop_map(Edit::Remove),
        1 => (0..20usize, 0..20usize).prop_map(|(a, b)| Edit::Move(a, b)),
        1 => (0..20usize, 0..5u8).prop_map(|(i, b)| Edit::Behaviour(i, b)),
    ]
}

/// Answers of a sheet's lines as text.
fn sheet_answers(sheet: &crate::Sheet) -> Vec<String> {
    sheet
        .lines()
        .iter()
        .map(|l| match l.answer() {
            Ok(Some(a)) => a.text().to_string(),
            Ok(None) => "<none>".into(),
            Err(e) => format!("<error: {e}>"),
        })
        .collect()
}

fn count(counter: &'static std::thread::LocalKey<Cell<u64>>) -> u64 {
    counter.with(Cell::get)
}

proptest! {
    #![proptest_config(runner())]

    /// No line makes the calculator panic, alone or in a sheet, nor highlighting it or writing its sides.
    #[test]
    fn no_panics(lines in prop::collection::vec(line(), 1..6)) {
        let mut calc = calculator(config());
        for line in &lines {
            // Highlights are in order, apart and on character boundaries.
            let mut end = 0;
            for h in calc.highlight(line) {
                prop_assert!(h.range.start >= end && h.range.start < h.range.end, "{:?} in {line:?}", h.range);
                prop_assert!(line.get(h.range.clone()).is_some(), "{:?} in {line:?}", h.range);
                end = h.range.end;
            }
            use crate::DataKind::*;
            for found in calc.find(line, &[DateInterval, Date, Money, Length, Quantity, Number, City, Place, Url, Hashtag]) {
                prop_assert!(line.get(found.range.clone()).is_some(), "{:?} in {line:?}", found.range);
            }
            if let Ok(Some(answer)) = calc.calculate(line) {
                let _ = (answer.source(), answer.target(), answer.note(), answer.copy_text());
            }
        }
    }

    /// A sheet that works out only what an edit reaches answers as one worked out from scratch.
    #[test]
    fn sheets_recalculate_as_from_scratch(
        lines in prop::collection::vec(sheet_line(), 1..8),
        edits in prop::collection::vec(edit(), 1..6),
    ) {
        use crate::{Behaviour, Sheet};
        let mut sheet = Sheet::new(calculator(config()));
        sheet.set_text(&lines.join("\n"));
        for edit in edits {
            let n = sheet.len().max(1);
            match edit {
                Edit::Set(i, text) if !sheet.is_empty() => { sheet.set_line(i % n, &text); }
                Edit::Insert(i, text) => { sheet.insert(i % (n + 1), &text); }
                Edit::Remove(i) if sheet.len() > 1 => { sheet.remove(i % n); }
                Edit::Move(a, b) if !sheet.is_empty() => { sheet.move_line(a % n, b % n); }
                Edit::Behaviour(i, b) if !sheet.is_empty() => {
                    let behaviour = [Behaviour::Expression, Behaviour::Subtotal, Behaviour::GrandTotal,
                        Behaviour::RunningTotal, Behaviour::RunningBudget][b as usize];
                    sheet.set_behaviour(i % n, behaviour);
                }
                _ => {}
            }
            let behaviours: Vec<Behaviour> = sheet.lines().iter().map(|l| l.behaviour()).collect();
            let mut fresh = Sheet::new(calculator(config()));
            fresh.set_text(&sheet.text());
            for (i, behaviour) in behaviours.into_iter().enumerate() {
                fresh.set_behaviour(i, behaviour);
            }
            fresh.recalculate();
            prop_assert_eq!(sheet_answers(&sheet), sheet_answers(&fresh), "text:\n{}", sheet.text());
        }
    }

    /// An answer read back as a line gives the same answer: "3.106855961 mi".
    #[test]
    fn answers_read_back(line in line()) {
        let mut calc = calculator(config());
        let Ok(Some(first)) = calc.calculate(&line) else { return Ok(()) };
        // Text is a name ("Wednesday"), not a value; a period shows its first and
        // last day, which read as the time between them.
        prop_assume!(!matches!(first.value(), Value::Text(_)) && !first.text().contains(" – "));
        // Money times a unit that isn't a rate, as in "A$1.00 Mbps", has no way to be written.
        let money = |&(id, e): &(crate::units::UnitId, i8)| e > 0 && registry().def(id).currency.is_some();
        let unwritten = |q: &Quantity| q.unit.factors().iter().any(money) && q.unit.factors().iter().filter(|f| f.1 > 0).count() > 1;
        prop_assume!(!matches!(first.value(), Value::Quantity(q) if unwritten(q)));
        calc.clear();
        // The same text, or the same value written another way: "0xFF" reads
        // back as 255, "1 year·h" as year², a timestamp gets thousands separators.
        let again = calc.calculate(first.text());
        let text = |r: &crate::Result<Option<crate::Answer>>| match r {
            Ok(Some(a)) => a.text().replace(',', ""),
            other => format!("{other:?}"),
        };
        let same_value = match (first.value(), &again) {
            (Value::Quantity(a), Ok(Some(b))) => match b.value() {
                Value::Quantity(b) => convert(b, &a.unit).is_ok_and(|b| close(a.number, b.number)),
                _ => false,
            },
            _ => false,
        };
        prop_assert!(same_value || text(&again) == first.text().replace(',', ""), "{:?} read back as {}, first from {:?}", first.text(), text(&again), line);
    }

    /// A line that doesn't read the clock answers the same at any time and in any zone.
    #[test]
    fn clock_only_when_read(line in line()) {
        let reads = count(&CLOCK_READS);
        let here = answer(&mut calculator(config()), &line);
        if count(&CLOCK_READS) != reads {
            return Ok(());
        }
        let elsewhere = Config {
            now: Some("2031-02-03T04:05:06+09:00[Asia/Tokyo]".parse().unwrap()),
            time_zone: Some(TimeZone::get("Asia/Tokyo").unwrap()),
            ..config()
        };
        prop_assert_eq!(answer(&mut calculator(elsewhere), &line), here);
    }

    /// Without "." or "," in it, a line means the same in both decimal styles;
    /// only the answer is written differently.
    #[test]
    fn decimal_styles_agree(line in line_with(false)) {
        let point = calculator(config()).calculate(&line);
        let comma = calculator(Config { decimal_comma: true, ..config() }).calculate(&line);
        match (point, comma) {
            // Text has numbers too: "5 – 10 kg".
            (Ok(Some(a)), Ok(Some(b))) if !matches!(a.value(), Value::Moment(_) | Value::Bool(_)) => {
                let swapped: String =
                    a.text().chars().map(|c| match c { '.' => ',', ',' => '.', c => c }).collect();
                prop_assert_eq!(swapped.replace(". ", "; "), b.text());
            }
            (Ok(Some(a)), Ok(Some(b))) => prop_assert_eq!(a.text(), b.text()),
            (a, b) => prop_assert_eq!(a.is_ok(), b.is_ok(), "{:?} / {:?}", a, b),
        }
    }

    /// A line answers the same after other lines, unless it reads them.
    #[test]
    fn lines_stand_alone(before in prop::collection::vec(line(), 1..4), line in line()) {
        let alone = answer(&mut calculator(config()), &line);
        let mut calc = calculator(config());
        // Assignments would make words of the line variables.
        for earlier in before.iter().filter(|l| !l.contains('=')) {
            let _ = calc.calculate(earlier);
        }
        let reads = count(&SHEET_READS);
        let after = answer(&mut calc, &line);
        if count(&SHEET_READS) == reads {
            prop_assert_eq!(after, alone);
        }
    }

    /// Converting there and back gives the amount back: miles to km to miles.
    #[test]
    fn conversions_round_trip((from, to) in unit_pair(), thousandths in 1i64..10_000_000) {
        let amount = Number::ratio(thousandths, 1000);
        let there = convert(&Quantity::new(amount, from.clone()), &to);
        prop_assume!(there.is_ok(), "{:?}", there);
        let back = convert(&there.unwrap(), &from).unwrap();
        let error = ((back.number - amount) / amount).abs().to_f64();
        prop_assert!(error < 1e-9, "{amount} {from} -> {to} -> {}", back.number);
    }
}

/// Equal to the ten digits answers show.
fn close(a: Number, b: Number) -> bool {
    a == b || ((a - b) / a).abs().to_f64() < 1e-9
}

/// Two units of the same kind; compound ones are made of two kinds (`mi/h`, `km/h`).
fn unit_pair() -> impl Strategy<Value = (Unit, Unit)> {
    static BY_KIND: LazyLock<Vec<Vec<Unit>>> = LazyLock::new(|| {
        let mut kinds: HashMap<String, Vec<Unit>> = HashMap::new();
        for spelling in registry().spellings() {
            let unit = registry().lookup(spelling).unwrap();
            let Some(def) = unit.single() else { continue };
            let units = kinds.entry(def.dim.to_string()).or_default();
            if !units.contains(&unit) {
                units.push(unit);
            }
        }
        let mut kinds: Vec<(String, Vec<Unit>)> = kinds.into_iter().filter(|(_, u)| u.len() > 1).collect();
        kinds.sort_by(|a, b| a.0.cmp(&b.0));
        kinds.into_iter().map(|(_, units)| units).collect()
    });
    let kinds = BY_KIND.len();
    let pair = move |kind: usize| (select(BY_KIND[kind].clone()), select(BY_KIND[kind].clone()));
    prop_oneof![
        (0..kinds).prop_flat_map(pair),
        (0..kinds, 0..kinds)
            .prop_flat_map(move |(a, b)| (pair(a), pair(b)))
            .prop_map(|((a1, a2), (b1, b2))| { (a1.product(&b1.pow(-1)), a2.product(&b2.pow(-1))) }),
    ]
    // "V/V" is a plain number, which takes any unit as it is.
    .prop_filter("a unit", |(from, to)| !from.is_none() && !to.is_none())
}

/// Converts a quantity as the calculator does, with the test exchange rates.
fn convert(q: &Quantity, to: &Unit) -> crate::Result<Quantity> {
    let config = config();
    let rates = Rates::with_table(super::rates());
    let vars = HashMap::new();
    let env = Env {
        config: &config,
        now: Now::new(&config),
        vars: &vars,
        functions: &HashMap::new(),
        depth: Cell::default(),
        lines: &[],
        rates: &rates,
        food_as: Cell::default(),
        rate_date: Cell::default(),
        inflation: None,
        weather: None,
        reads: Default::default(),
    };
    env.convert_quantity(q, to)
}
