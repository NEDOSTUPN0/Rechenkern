//! Command line interface for Rechenkern.

use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use rechenkern::online::OnlineRates;
use rechenkern::{AngleUnit, Answer, Calculator, Config, DateOrder};
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
    /// Currency meant by "$".
    #[arg(long, default_value = "USD")]
    dollar: String,
    /// Trigonometry takes degrees instead of radians.
    #[arg(long)]
    degrees: bool,
    /// Local time zone, e.g. Europe/Berlin [default: system].
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

    if !args.expression.is_empty() {
        let line = args.expression.join(" ");
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
        Ok(text) => sheet(&mut calc, &text, args.annotate),
        Err(e) => fail(&e),
    }
}

fn config(args: &Args) -> Result<Config, String> {
    let time_zone = match &args.tz {
        Some(name) => Some(jiff::tz::TimeZone::get(name).map_err(|e| format!("unknown time zone {name}: {e}"))?),
        None => None,
    };
    Ok(Config {
        precision: args.precision.clamp(1, 28),
        thousands_separators: !args.no_separators,
        clock_24h: !args.twelve_hour,
        date_order: match args.date_order {
            Some(Order::Dmy) => DateOrder::DayFirst,
            Some(Order::Mdy) => DateOrder::MonthFirst,
            None => locale_date_order(),
        },
        dollar: args.dollar.to_uppercase(),
        angle_unit: if args.degrees { AngleUnit::Degrees } else { AngleUnit::Radians },
        time_zone,
        now: None,
    })
}

/// US locales write the month first.
fn locale_date_order() -> DateOrder {
    let locale = ["LC_ALL", "LC_TIME", "LANG"].iter().find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()));
    match locale {
        Some(l) if l.starts_with("en_US") => DateOrder::MonthFirst,
        _ => DateOrder::DayFirst,
    }
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
