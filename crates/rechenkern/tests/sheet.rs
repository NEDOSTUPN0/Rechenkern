//! Sheets: lines that answer together, recalculated only where an edit reaches.

use std::cell::RefCell;
use std::rc::Rc;

use jiff::tz::TimeZone;
use rechenkern::{Behaviour, Calculator, Color, Config, Number, Portion, RateTable, Sheet, Statistic};

/// A sheet at Wed 23 Sep 2026 14:30 in New York, with fixed rates.
fn sheet(text: &str) -> Sheet {
    let config = Config {
        now: Some("2026-09-23T14:30:00-04:00[America/New_York]".parse().unwrap()),
        time_zone: Some(TimeZone::get("America/New_York").unwrap()),
        ..Config::default()
    };
    let mut calc = Calculator::with_config(config);
    let mut rates = RateTable::new();
    rates.insert("EUR", Number::parse("0.9").unwrap());
    calc.set_rates(rates);
    let mut sheet = Sheet::new(calc);
    sheet.set_text(text);
    sheet
}

/// Each line's answer, "" for none and "!" for an error.
fn answers(sheet: &Sheet) -> Vec<String> {
    sheet
        .lines()
        .iter()
        .map(|l| match l.answer() {
            Ok(Some(a)) => a.text().to_string(),
            Ok(None) => String::new(),
            Err(_) => "!".into(),
        })
        .collect()
}

#[test]
fn edits_recalculate_what_depends_on_them() {
    let mut s = sheet("rent = $1,500\nfood = $400\nrent + food\nfun = $100\n2 + 2\nprev * 2");
    assert_eq!(answers(&s), ["$1,500.00", "$400.00", "$1,900.00", "$100.00", "4", "8"]);
    assert_eq!(s.set_line(1, "food = $600"), [1, 2]);
    assert_eq!(answers(&s)[2], "$2,100.00");
    // "prev" reads the line above it.
    assert_eq!(s.set_line(4, "3 + 3"), [4, 5]);
    assert_eq!(answers(&s)[5], "12");
    // The same answer changes nothing below.
    assert_eq!(s.set_line(4, "2 * 3"), [4]);
    // A new variable changes how lines that name it read.
    let mut s = sheet("tax * 2\nlunch tax");
    assert_eq!(answers(&s), ["2", ""]);
    s.insert(0, "tax = 5");
    assert_eq!(answers(&s), ["5", "10", "5"]);
}

#[test]
fn inserting_and_removing_lines() {
    let mut s = sheet("$10\n$20\nsum\nline 1 * 3");
    assert_eq!(answers(&s), ["$10.00", "$20.00", "$30.00", "$30.00"]);
    s.insert(1, "$5");
    assert_eq!(answers(&s), ["$10.00", "$5.00", "$20.00", "$35.00", "$30.00"]);
    s.remove(0);
    assert_eq!(answers(&s), ["$5.00", "$20.00", "$25.00", "$15.00"]);
    s.move_line(0, 1);
    assert_eq!(answers(&s), ["$20.00", "$5.00", "$25.00", "$60.00"]);
    assert_eq!(s.text(), "$20\n$5\nsum\nline 1 * 3");
}

#[test]
fn subtotals_and_running_totals() {
    let mut s = sheet("# Food\n$10\n$20\nsubtotal\n# Travel\n$100\n$200\nsubtotal\ngrand total");
    s.set_behaviour(3, Behaviour::Subtotal);
    s.set_behaviour(7, Behaviour::Subtotal);
    s.set_behaviour(8, Behaviour::GrandTotal);
    assert_eq!(answers(&s), ["", "$10.00", "$20.00", "$30.00", "", "$100.00", "$200.00", "$300.00", "$330.00"]);
    assert_eq!(s.subtotal_of(1), Some(3));
    s.set_line(2, "$25");
    assert_eq!(answers(&s)[8], "$335.00");

    let mut s = sheet("$1,000 budget\n$200 rent\n$50 food\n\n$5");
    s.set_behaviour(0, Behaviour::RunningBudget);
    let left: Vec<_> = s.lines().iter().map(|l| l.running().map(|a| a.text().to_string())).collect();
    assert_eq!(left, [Some("$1,000.00".into()), Some("$800.00".into()), Some("$750.00".into()), None, None]);
    s.set_behaviour(0, Behaviour::RunningTotal);
    assert_eq!(s.lines()[2].running().unwrap().text(), "$1,250.00");
}

#[test]
fn statistics() {
    let s = sheet("$10\n$20\nx = $1,000\n$30\nline 1");
    let stat = |kind, lines: Option<&[usize]>| s.statistic(kind, lines).map(|a| a.unwrap().text().to_string());
    // The variable and line 1, which "line 1" reads, are left out.
    assert_eq!(stat(Statistic::Total, None).as_deref(), Some("$60.00"));
    assert_eq!(stat(Statistic::Average, None).as_deref(), Some("$20.00"));
    assert_eq!(stat(Statistic::Count, None).as_deref(), Some("3"));
    assert_eq!(stat(Statistic::Max, Some(&[1, 3])).as_deref(), Some("$30.00"));
    assert_eq!(stat(Statistic::Total, Some(&[2])), None);
}

#[test]
fn undo_and_metadata() {
    let mut s = sheet("1\n2\n3");
    let before = s.checkpoint();
    s.set_line(1, "20");
    s.push("4");
    s.set_highlight(0, Portion::Head, Some(Color::Green));
    assert_eq!(s.lines()[0].highlight(Portion::Head), Some(Color::Green));
    let delta = s.revert(&before);
    assert_eq!((delta.added, delta.removed, delta.modified), (vec![], vec![3], vec![1]));
    assert_eq!(answers(&s), ["1", "2", "3"]);
    assert_eq!(s.lines()[0].highlight(Portion::Head), None);
}

#[test]
fn observers_and_time() {
    let mut s = sheet("now\n2 + 2\nx = 5\nx * 2");
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let id = s.observe(move |_, lines| log.borrow_mut().push(lines.to_vec()));
    s.set_line(2, "x = 6");
    assert_eq!(*seen.borrow(), [vec![2, 3]]);
    assert_eq!(s.time_dependent_lines(), [0]);
    assert_eq!(s.refresh_time(), [0]);
    assert_eq!(s.lines_reading("x"), [3]);
    s.unobserve(id);
    s.set_line(1, "3");
    assert_eq!(seen.borrow().len(), 2);
}

#[test]
fn completions_and_variables() {
    let s = sheet("monthly rent = $1,500\nmonthly food = $400\n");
    assert_eq!(s.completions("month", 2)[..2], ["monthly food", "monthly rent"]);
    assert!(s.completions("month", 0).iter().all(|c| !c.starts_with("monthly ")));
    assert!(s.completions("sqr", 0).contains(&"sqrt".to_string()));
    assert_eq!(s.variables_at(1).len(), 1);
    assert_eq!(s.variables().len(), 2);
}

#[test]
fn plain_text_and_json() {
    use rechenkern::{TextOptions, TextStyle};
    let s = sheet("# Budget\nrent = $1,500 // monthly\nfood = $400 for lunch\nrent + food\nprev / 2");
    assert_eq!(
        s.to_text(&TextOptions::default()),
        "# Budget\nrent = $1,500 // monthly = $1,500.00\nfood = $400 for lunch = $400.00\nrent + food = $1,900.00\nprev / 2 = $950.00"
    );
    let options = TextOptions {
        style: TextStyle::Pretty,
        separator: " | ".into(),
        hide_comments: true,
        hide_heading_indicator: true,
        substitute_references: true,
        unformatted: true,
        ..TextOptions::default()
    };
    assert_eq!(
        s.to_text(&options),
        "Budget\nrent = $1,500 | 1500 USD\nfood = $400   | 400 USD\nrent + food   | 1900 USD\n$1,900.00 / 2 | 950 USD"
    );
    let numbered = TextOptions {
        include_answers: false,
        line_numbers: true,
        statistic: Some(Statistic::Count),
        ..TextOptions::default()
    };
    assert!(s.to_text(&numbered).starts_with("1. # Budget\n2. rent"), "{}", s.to_text(&numbered));
    let json = s.to_json();
    assert!(json.starts_with(r##"[{"line":1,"text":"# Budget","answer":null"##), "{json}");
    assert!(
        json.contains(r#""text":"rent + food","answer":"$1,900.00","error":null,"raw":"1900 USD","variable":null"#),
        "{json}"
    );
}

#[test]
fn html() {
    use rechenkern::{HtmlColors, HtmlOptions};
    let s = sheet("# Trip\nflights = $1,240\nhotel $870 \"<b>\"\nnotes on Paris");
    let options = HtmlOptions { total_label: Some("Total".into()), line_numbers: true, ..HtmlOptions::default() };
    let html = s.to_html(&options, &HtmlColors::light());
    assert!(html.starts_with("<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>Trip</title>"), "{html}");
    assert!(html.contains("<span class=\"variable\">flights</span>"), "{html}");
    assert!(html.contains("<span class=\"comment\">&quot;&lt;b&gt;&quot;</span>"), "{html}");
    // Lines that set a variable are left out of totals.
    assert!(html.contains(">Total</td><td class=\"answer\" style=\"font-weight:600\">$870.00</td>"), "{html}");
    let email = s.to_email_html(&HtmlOptions { minify: true, ..HtmlOptions::default() }, &HtmlColors::dark());
    assert!(!email.contains("<style>") && !email.contains('\n') && email.contains("style=\"color:"), "{email}");
}

#[test]
fn archives() {
    let mut s = sheet("$10\n$20\nsubtotal\nx = \"quoted\" 5");
    s.set_behaviour(2, Behaviour::Subtotal);
    s.set_highlight(0, Portion::Tail, Some(Color::Orange));
    let archive = s.archive();
    let back = Sheet::unarchive(Calculator::new(), &archive).unwrap();
    assert_eq!(back.text(), s.text());
    assert_eq!(answers(&back), answers(&s));
    assert_eq!(back.lines()[2].behaviour(), Behaviour::Subtotal);
    assert_eq!(back.lines()[0].highlight(Portion::Tail), Some(Color::Orange));
    assert!(Sheet::unarchive(Calculator::new(), "{").is_err());
}
