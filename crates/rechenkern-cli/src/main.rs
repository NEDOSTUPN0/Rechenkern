//! Command line interface for Rechenkern.

use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use rechenkern::online::{OnlinePlaces, OnlineRates, OnlineWeather};
use rechenkern::{
    AngleUnit, Answer, Calculator, Config, DateOrder, Formatting, Highlight, HtmlColors, HtmlOptions, Notation, Number,
    Sheet, Side, UnitSystem,
};
use rustyline::error::ReadlineError;

#[derive(Parser)]
#[command(name = "rechenkern", version, about = "Natural language calculator: `rechenkern 10 usd to eur`")]
struct Args {
    /// Expression to calculate. Without it, reads lines from stdin or starts a prompt.
    #[arg(allow_hyphen_values = true)]
    expression: Vec<String>,
    /// Calculate every line of a file.
    #[arg(short, long)]
    file: Option<PathBuf>,
    /// Print each line next to its answer.
    #[arg(short, long)]
    annotate: bool,
    /// Print the sheet as an HTML page.
    #[arg(long)]
    html: bool,
    /// Print one JSON object per line: {"input", "answer", "error", "form", "source", "target", "note", "copy",
    /// "alternatives", "tokens"}.
    #[arg(long)]
    json: bool,
    /// Significant digits in answers.
    #[arg(short, long, default_value_t = 10)]
    precision: u32,
    /// Use a 12-hour clock.
    #[arg(long = "12h")]
    twelve_hour: bool,
    /// Don't group thousands with commas.
    #[arg(long)]
    no_separators: bool,
    /// Order of day and month in dates like 03/04/2026 [default: from locale].
    #[arg(long, value_enum)]
    date_order: Option<Order>,
    /// Currency meant by "$" [default: from locale, else USD].
    #[arg(long)]
    dollar: Option<String>,
    /// Trigonometry takes degrees instead of radians.
    #[arg(long)]
    degrees: bool,
    /// What "gallon", "cup" and "ton" mean [default: from locale].
    #[arg(long, value_enum)]
    unit_system: Option<System>,
    /// Pixels in an em or rem.
    #[arg(long, default_value_t = 16)]
    font_size: u32,
    /// Decimal places at most, instead of significant digits.
    #[arg(long)]
    dp: Option<u32>,
    /// When big numbers are written with a scale, as 5M.
    #[arg(long, value_enum, default_value_t = Scales::Auto)]
    notation: Scales,
    /// Local time zone, e.g. America/New_York [default: system].
    #[arg(long)]
    tz: Option<String>,
    /// Never download exchange rates; use cached ones.
    #[arg(long)]
    offline: bool,
    /// Download fresh exchange rates and exit.
    #[arg(long)]
    update_rates: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Scales {
    Auto,
    On,
    Off,
}

#[derive(Clone, Copy, ValueEnum)]
enum System {
    Us,
    Imperial,
    Metric,
}

#[derive(Clone, Copy, ValueEnum)]
enum Order {
    Dmy,
    Mdy,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let rates = OnlineRates {
        offline: args.offline,
        ..OnlineRates::new(dirs::cache_dir().map(|d| d.join("rechenkern/rates.json")))
    };
    if args.update_rates {
        return match rates.refresh() {
            Ok(table) => {
                println!("{} rates from {}", table.len(), table.date.as_deref().unwrap_or("unknown date"));
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        };
    }
    let config = match config(&args) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    let mut calc = Calculator::with_config(config);
    calc.set_rate_provider(rates);
    calc.set_weather_provider(OnlineWeather::default());
    calc.set_place_provider(OnlinePlaces {
        offline: args.offline,
        ..OnlinePlaces::new(dirs::cache_dir().map(|d| d.join("rechenkern/places.json")))
    });

    if !args.expression.is_empty() {
        let line = args.expression.join(" ");
        if args.json {
            let tokens = calc.highlight(&line);
            let result = calc.calculate(&line);
            println!("{}", json_line(&calc, &line, result, &tokens));
            return ExitCode::SUCCESS;
        }
        return match calc.calculate(&line) {
            Ok(Some(answer)) => {
                println!("{answer}");
                ExitCode::SUCCESS
            }
            Ok(None) => fail("nothing to calculate"),
            Err(e) => fail(e.message()),
        };
    }
    let text = match &args.file {
        Some(path) => std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display())),
        None if !io::stdin().is_terminal() => {
            let mut text = String::new();
            io::stdin().read_to_string(&mut text).map(|_| text).map_err(|e| e.to_string())
        }
        None => return repl(&mut calc),
    };
    match text {
        Ok(text) if args.json => {
            calc.clear();
            for line in text.lines() {
                let tokens = calc.highlight(line);
                let result = calc.calculate(line);
                println!("{}", json_line(&calc, line, result, &tokens));
            }
            ExitCode::SUCCESS
        }
        Ok(text) if args.html => {
            let mut sheet = Sheet::new(calc);
            sheet.set_text(text.strip_suffix('\n').unwrap_or(&text));
            print!("{}", sheet.to_html(&HtmlOptions::default(), &HtmlColors::default()));
            ExitCode::SUCCESS
        }
        Ok(text) => sheet(&mut calc, &text, args.annotate),
        Err(e) => fail(&e),
    }
}

/// `{"input": ..., "answer": ..., "error": ...}` for scripts and launchers, with the
/// form of the line, its two sides and the text to copy when there is an answer.
fn json_line(
    calc: &Calculator,
    input: &str,
    result: rechenkern::Result<Option<Answer>>,
    tokens: &[Highlight],
) -> serde_json::Value {
    let (answer, error) = match result {
        Ok(answer) => (answer, None),
        Err(e) => (None, Some(e.to_string())),
    };
    let mut json = serde_json::json!({ "input": input, "answer": answer.as_ref().map(Answer::text), "error": error });
    if let Some(answer) = &answer {
        json["form"] = answer.form().name().into();
        let side = |side: &Side| serde_json::json!({ "text": side.text, "label": side.label });
        if let (Some(source), Some(target)) = (answer.source(), answer.target()) {
            json["source"] = side(source);
            json["target"] = side(target);
        }
        if let Some(note) = answer.note() {
            json["note"] = note.into();
        }
        json["copy"] = answer.copy_text().into();
        let alternatives: Vec<_> = calc
            .alternatives(answer)
            .iter()
            .map(|a| serde_json::json!({ "kind": a.kind.name(), "text": a.text }))
            .collect();
        json["alternatives"] = alternatives.into();
    }
    // Offsets count characters, as hosts index strings.
    let chars = |byte: usize| input[..byte].chars().count();
    let tokens: Vec<_> = tokens
        .iter()
        .map(|t| {
            let (start, end) = (chars(t.range.start), chars(t.range.end));
            serde_json::json!({ "text": &input[t.range.clone()], "kind": t.kind.name(), "start": start, "end": end })
        })
        .collect();
    json["tokens"] = tokens.into();
    json
}

fn config(args: &Args) -> Result<Config, String> {
    let time_zone = match &args.tz {
        Some(name) => Some(jiff::tz::TimeZone::get(name).map_err(|e| format!("unknown time zone {name}: {e}"))?),
        None => None,
    };
    // Numbers follow LC_NUMERIC, dates LC_TIME, the rest LANG.
    let locale = |vars: &[&str]| vars.iter().find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()));
    let base = Config::for_locale(&locale(&["LC_ALL", "LANG"]).unwrap_or_default());
    let numbers = Config::for_locale(&locale(&["LC_ALL", "LC_NUMERIC", "LANG"]).unwrap_or_default());
    let times = Config::for_locale(&locale(&["LC_ALL", "LC_TIME", "LANG"]).unwrap_or_default());
    Ok(Config {
        precision: args.precision.clamp(1, 28),
        thousands_separators: !args.no_separators,
        decimal_comma: numbers.decimal_comma,
        clock_24h: !args.twelve_hour && times.clock_24h,
        date_order: match args.date_order {
            Some(Order::Dmy) => DateOrder::DayFirst,
            Some(Order::Mdy) => DateOrder::MonthFirst,
            None => times.date_order,
        },
        week_start: times.week_start,
        dollar: args.dollar.as_deref().map_or(base.dollar.clone(), str::to_uppercase),
        angle_unit: if args.degrees { AngleUnit::Degrees } else { AngleUnit::Radians },
        unit_system: match args.unit_system {
            Some(System::Us) => UnitSystem::CustomaryUs,
            Some(System::Imperial) => UnitSystem::Imperial,
            Some(System::Metric) => UnitSystem::Metric,
            None => base.unit_system,
        },
        base_font_size: Number::from_i64(args.font_size.into()),
        formatting: Formatting {
            decimal_places: args.dp.map(|dp| dp.min(28)),
            notation: match args.notation {
                Scales::Auto => Notation::Auto,
                Scales::On => Notation::On,
                Scales::Off => Notation::Off,
            },
            ..numbers.formatting
        },
        time_zone,
        ..Config::default()
    })
}

/// Calculates a whole text; each input line gets one output line.
fn sheet(calc: &mut Calculator, text: &str, annotate: bool) -> ExitCode {
    let results = calc.calculate_sheet(text);
    let width = text.lines().map(|l| l.chars().count()).max().unwrap_or(0);
    for (i, (line, result)) in text.lines().zip(results).enumerate() {
        let answer = match result {
            Ok(Some(answer)) => answer.to_string(),
            Ok(None) => String::new(),
            Err(e) if annotate => format!("error: {e}"),
            Err(e) => {
                eprintln!("line {}: {e}", i + 1);
                String::new()
            }
        };
        if annotate {
            let pad = width - line.chars().count();
            println!("{line}{}  │ {answer}", " ".repeat(pad));
        } else {
            println!("{answer}");
        }
    }
    ExitCode::SUCCESS
}

fn repl(calc: &mut Calculator) -> ExitCode {
    let Ok(mut editor) = rustyline::DefaultEditor::new() else { return fail("can't start the prompt") };
    let history = dirs::data_dir().map(|d| d.join("rechenkern/history"));
    if let Some(path) = &history {
        let _ = editor.load_history(path);
    }
    let color = std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal();
    loop {
        match editor.readline("> ") {
            Ok(line) => {
                if matches!(line.trim(), "exit" | "quit") {
                    break;
                }
                if !line.trim().is_empty() {
                    let _ = editor.add_history_entry(line.as_str());
                }
                print_answer(calc.calculate(&line), color);
            }
            Err(ReadlineError::Interrupted) => continue,
            Err(_) => break,
        }
    }
    if let Some(path) = &history {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = editor.save_history(path);
    }
    ExitCode::SUCCESS
}

fn print_answer(result: rechenkern::Result<Option<Answer>>, color: bool) {
    let (green, red, reset) = if color { ("\x1b[32m", "\x1b[31m", "\x1b[0m") } else { ("", "", "") };
    match result {
        Ok(Some(answer)) => println!("{green}= {answer}{reset}"),
        Ok(None) => {}
        Err(e) => println!("{red}error: {e}{reset}"),
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("rechenkern: {message}");
    ExitCode::FAILURE
}
