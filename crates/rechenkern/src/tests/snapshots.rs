//! Snapshots: many lines and their answers, kept in `snapshots/`. Any changed
//! answer shows up as a diff: review it with `cargo insta review`.
//!
//! Some lines come from the vocabulary tables, so every unit, function, place
//! and holiday is tried; the rest are written by hand in `corpus/`.

use std::path::Path;

use super::{answer, calculator, config};
use crate::config::Config;
use crate::parser::words;
use crate::units::{Unit, registry};
use crate::zones;

/// Lines and their answers, one per row. Lines starting with `//` are headings.
fn table(lines: impl IntoIterator<Item = String>) -> String {
    table_with(config(), lines)
}

fn table_with(config: Config, lines: impl IntoIterator<Item = String>) -> String {
    let mut calc = calculator(config);
    let mut out = String::new();
    for line in lines {
        if line.starts_with("//") {
            out.push_str(&format!("\n{line}\n"));
            continue;
        }
        calc.clear();
        out.push_str(&format!("{line:<44} → {}\n", answer(&mut calc, &line)));
    }
    out
}

fn snapshot(name: &str, text: &str) {
    let mut settings = insta::Settings::clone_current();
    settings.set_prepend_module_to_snapshot(false);
    settings.set_omit_expression(true);
    settings.bind(|| insta::assert_snapshot!(name, text));
}

/// Units each spelling converts to, by kind; the first one of the same kind is used.
const TARGETS: &[&str] = &[
    "m", "kg", "s", "A", "K", "mol", "rad", "B", "EUR", "USD", "workday", "m2", "L", "Hz", "N", "J", "W", "Pa", "V",
    "Ω", "mph", "bps", "L/100km",
];

#[test]
fn units() {
    let reg = registry();
    let targets: Vec<(&str, Unit)> = TARGETS.iter().map(|&t| (t, reg.lookup(t).unwrap())).collect();
    let mut lines = Vec::new();
    for spelling in reg.spellings() {
        let unit = reg.lookup(spelling).unwrap();
        lines.push(format!("2.5 {spelling}"));
        if let Some((target, _)) = targets.iter().find(|(_, t)| t.dim() == unit.dim() && *t != unit) {
            lines.push(format!("2.5 {spelling} in {target}"));
        }
    }
    snapshot("units", &table(lines));
}

#[test]
fn functions_and_formats() {
    let mut lines = vec!["// Functions".to_string()];
    for f in words::FUNCTIONS.phrases() {
        lines.extend([format!("{f}(16)"), format!("{f}(2, 8)"), format!("{f} of 2, 8 and 4")]);
    }
    lines.push("// Formats".into());
    for f in words::FORMATS.phrases() {
        lines.extend([format!("255 in {f}"), format!("0.75 as {f}"), format!("today as {f}")]);
    }
    lines.push("// Constants and powers".into());
    lines.extend(words::PHYSICAL_CONSTANTS.phrases().map(String::from));
    lines.extend(words::POWER_WORDS.phrases().map(|w| format!("3 {w} 4")));
    snapshot("functions_and_formats", &table(lines));
}

#[test]
fn dates() {
    let months = [
        "january",
        "jan",
        "february",
        "feb",
        "march",
        "mar",
        "april",
        "apr",
        "may",
        "june",
        "jun",
        "july",
        "jul",
        "august",
        "aug",
        "september",
        "sep",
        "sept",
        "october",
        "oct",
        "november",
        "nov",
        "december",
        "dec",
    ];
    let weekdays = [
        "monday",
        "mon",
        "tuesday",
        "tue",
        "wednesday",
        "wed",
        "thursday",
        "thu",
        "friday",
        "fri",
        "saturday",
        "sat",
        "sunday",
        "sun",
    ];
    let mut lines = vec!["// Months".to_string()];
    for m in months.into_iter().filter(|m| words::month(m).is_some()) {
        lines.extend([format!("{m} 5"), format!("5 {m} 2027"), format!("days in {m}")]);
    }
    lines.push("// Weekdays".into());
    for w in weekdays.into_iter().filter(|w| words::weekday(w).is_some()) {
        lines.extend([w.to_string(), format!("next {w}"), format!("last {w} at 3pm")]);
    }
    lines.push("// Holidays".into());
    for h in words::HOLIDAYS.phrases() {
        lines.extend([h.to_string(), format!("days until {h}"), format!("{h} 2030")]);
    }
    lines.push("// Date parts".into());
    for p in words::DATE_PARTS.phrases() {
        lines.extend([p.to_string(), format!("{p} of March 9, 2024")]);
    }
    lines.push("// Events".into());
    lines.extend(words::EVENTS.iter().flat_map(|e| e.names.iter().map(|n| n.to_string())));
    lines.push("// Sun and moon".into());
    for s in words::SKY.phrases() {
        lines.extend([s.to_string(), format!("{s} in Tokyo"), format!("{s} on March 21, 2027")]);
    }
    snapshot("dates", &table(lines));
}

#[test]
fn places() {
    let mut lines = vec!["// Listed places and zone cities".to_string()];
    for p in zones::place_names() {
        lines.extend([format!("time in {p}"), format!("sunrise in {p}")]);
    }
    lines.push("// Every 200th city".into());
    for c in zones::city_names().step_by(200) {
        lines.extend([format!("time in {c}"), format!("sunrise in {c}")]);
    }
    snapshot("places", &table(lines));
}

#[test]
fn foods() {
    let lines = words::FOODS.phrases().flat_map(|f| [format!("1 cup of {f} in grams"), format!("100 g {f} in cups")]);
    snapshot("foods", &table(lines));
}

/// Every `corpus/*.txt` file; a `corpus/sheets/*.txt` file is one sheet,
/// where lines see the ones above them.
#[test]
fn corpus() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tests/corpus");
    for (path, sheet) in
        files(&dir).into_iter().map(|p| (p, false)).chain(files(&dir.join("sheets")).into_iter().map(|p| (p, true)))
    {
        let text = std::fs::read_to_string(&path).unwrap();
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        if sheet {
            let mut calc = calculator(config());
            let answers = calc.calculate_sheet(&text);
            let rows: Vec<String> = text
                .lines()
                .zip(answers)
                .map(|(line, a)| {
                    let a = match a {
                        Ok(Some(a)) => a.to_string(),
                        Ok(None) => String::new(),
                        Err(e) => format!("<error: {e}>"),
                    };
                    format!("{line:<44} → {a}\n")
                })
                .collect();
            snapshot(&format!("sheet_{name}"), &rows.concat());
        } else {
            let lines = || text.lines().filter(|l| !l.trim().is_empty()).map(String::from);
            let mut out = table(lines());
            if text.starts_with("// Run in both decimal styles") {
                out.push_str("\n// With a decimal comma\n");
                out.push_str(&table_with(Config { decimal_comma: true, ..config() }, lines()));
            }
            snapshot(&format!("corpus_{name}"), &out);
        }
    }
}

/// The `.txt` files in a directory, sorted.
fn files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .collect();
    files.sort();
    files
}
