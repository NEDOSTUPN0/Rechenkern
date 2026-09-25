//! Timing of single lines and a whole sheet: `cargo bench -- [lines file]`.
//!
//! Every line is calculated on a cleared calculator, many times over, and the
//! report shows the first pass (with one-time setup), the median and mean
//! time per line, the slowest lines and the best time for `sheet.txt`.
//! `soulver.swift` times SoulverCore the same way.

use std::hint::black_box;
use std::time::{Duration, Instant};

use rechenkern::{Calculator, Number, RateTable};

const DEFAULT_LINES: &str = include_str!("lines.txt");
const SHEET: &str = include_str!("sheet.txt");
/// Time spent on each line.
const BUDGET: Duration = Duration::from_millis(20);

fn calculator() -> Calculator {
    let mut calc = Calculator::new();
    let mut rates = RateTable::new();
    for (code, rate) in [("EUR", "0.9"), ("GBP", "0.8"), ("JPY", "150"), ("RUB", "90"), ("BTC", "0.00001")] {
        rates.insert(code, Number::parse(rate).unwrap());
    }
    calc.set_rates(rates);
    calc
}

/// Mean time of one calculation of `line`, starting from an empty sheet.
fn time_line(calc: &mut Calculator, line: &str) -> Duration {
    let start = Instant::now();
    let mut runs = 0u32;
    while start.elapsed() < BUDGET {
        for _ in 0..10 {
            calc.clear();
            let _ = black_box(calc.calculate(black_box(line)));
        }
        runs += 10;
    }
    start.elapsed() / runs
}

fn micros(d: Duration) -> f64 {
    d.as_nanos() as f64 / 1000.0
}

fn main() {
    // `cargo bench` passes `--bench`; any other argument is a lines file.
    let path = std::env::args().skip(1).find(|a| !a.starts_with('-'));
    let text = match &path {
        Some(path) => std::fs::read_to_string(path).expect("readable lines file"),
        None => DEFAULT_LINES.to_string(),
    };
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("//")).collect();

    let start = Instant::now();
    for line in &lines {
        let _ = calculator().calculate(line);
    }
    println!("first pass (with setup): {:>9.1} µs", micros(start.elapsed()));

    let mut calc = calculator();
    let mut times: Vec<(Duration, &str)> = lines.iter().map(|&l| (time_line(&mut calc, l), l)).collect();
    times.sort();
    let total: Duration = times.iter().map(|(t, _)| *t).sum();
    let mean = total / times.len() as u32;
    println!("lines:                   {:>9}", times.len());
    println!("median:                  {:>9.2} µs", micros(times[times.len() / 2].0));
    println!("mean:                    {:>9.2} µs", micros(mean));
    println!("lines per second:        {:>9.0}", 1.0 / mean.as_secs_f64());
    println!("slowest:");
    for (t, line) in times.iter().rev().take(10) {
        println!("  {:>9.2} µs  {line}", micros(*t));
    }

    let best = (0..100)
        .map(|_| {
            let start = Instant::now();
            black_box(calc.calculate_sheet(black_box(SHEET)));
            start.elapsed()
        })
        .min()
        .expect("runs");
    println!("sheet of {} lines:       {:>9.1} µs", SHEET.lines().count(), micros(best));
}
