//! A sheet of lines that answer together, as SoulverCore's `LineCollection`: variables
//! and earlier answers carry down, an edit recalculates only the lines that depend on
//! it, and a host can mark subtotals, keep undo states and ask for statistics.

mod export;

use std::collections::{HashMap, HashSet};

pub use export::{HtmlColors, HtmlOptions, TextOptions, TextStyle};

use crate::answer::{Answer, Parts};
use crate::ast::{Expr, Func, LineRef};
use crate::calculator::{Assignment, Calculator, Effects, Names, Reads};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::eval::{Display, Env, Line, LineKind};
use crate::parser::highlight::Highlight;
use crate::value::Value;

/// How a line answers, as the host marks it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Behaviour {
    /// The line's own text.
    #[default]
    Expression,
    /// The sum of the lines above, back to the previous subtotal or grand total; the
    /// text is only a caption.
    Subtotal,
    /// The sum of the subtotals above (or of every line, without subtotals), back to the
    /// previous grand total.
    GrandTotal,
    /// The line and the ones below show a running total, to the next blank line or heading.
    RunningTotal,
    /// The line is a budget: the ones below show what is left of it.
    RunningBudget,
}

/// Colors a host paints a part of a line with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Orange,
    Blue,
    Purple,
    Pink,
    Red,
    Yellow,
    Green,
    Grey,
}

/// The part of a line a highlight is on: its start, its text or its answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Portion {
    Head,
    Body,
    Tail,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Alignment {
    #[default]
    Left,
    Center,
    Right,
}

/// Kinds of statistics of lines, as SoulverCore's `StatisticType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Statistic {
    Total,
    Average,
    Count,
    Median,
    /// The smallest answer.
    Min,
    /// The biggest answer.
    Max,
    Lcm,
    Gcd,
    StandardDeviation,
}

/// A line of a sheet: its text, how the host marked it, and its answer.
#[derive(Clone, Debug)]
pub struct SheetLine {
    id: u64,
    text: String,
    behaviour: Behaviour,
    highlights: [Option<Color>; 3],
    pub alignment: Alignment,
    /// Who wrote the line, or where it came from.
    pub attribution: Option<String>,
    answer: Result<Option<Answer>>,
    /// A running total, or what is left of a budget.
    running: Option<Answer>,
    effects: Effects,
}

impl SheetLine {
    fn new(id: u64, text: &str) -> SheetLine {
        SheetLine {
            id,
            text: text.to_string(),
            behaviour: Behaviour::Expression,
            highlights: [None; 3],
            alignment: Alignment::Left,
            attribution: None,
            answer: Ok(None),
            running: None,
            effects: Effects {
                line: Line { value: None, kind: LineKind::Blank, tags: Vec::new() },
                assigned: None,
                defined: None,
                clears_vars: false,
                reads: Reads::default(),
            },
        }
    }

    /// Stays the same while the line moves around the sheet.
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The answer; `Ok(None)` for a line with nothing to calculate.
    pub fn answer(&self) -> std::result::Result<Option<&Answer>, &Error> {
        self.answer.as_ref().map(Option::as_ref)
    }

    /// The running total at this line, or what is left of the budget.
    pub fn running(&self) -> Option<&Answer> {
        self.running.as_ref()
    }

    pub fn behaviour(&self) -> Behaviour {
        self.behaviour
    }

    pub fn highlight(&self, portion: Portion) -> Option<Color> {
        self.highlights[portion as usize]
    }

    pub fn is_heading(&self) -> bool {
        self.effects.line.kind == LineKind::Heading
    }

    pub fn is_divider(&self) -> bool {
        self.effects.line.kind == LineKind::Divider
    }

    /// The variable the line sets: "rent" in `rent = $1,500`.
    pub fn variable(&self) -> Option<&str> {
        self.effects.assigned.as_ref().map(|(name, ..)| name.as_str())
    }

    /// The line reads the clock, so its answer changes with time.
    pub fn reads_clock(&self) -> bool {
        self.effects.reads.clock
    }
}

/// Lines added, removed and changed by going back to a checkpoint, by index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Delta {
    pub added: Vec<usize>,
    pub removed: Vec<usize>,
    pub modified: Vec<usize>,
}

/// A state of the sheet to go back to, for undo and redo.
#[derive(Clone, Debug)]
pub struct Checkpoint {
    lines: Vec<SheetLine>,
}

/// Lets a host stop observing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObserverId(u64);

type Observer = Box<dyn FnMut(&Sheet, &[usize])>;

/// Lines that answer together; see the module docs.
///
/// ```
/// let mut sheet = rechenkern::Sheet::new(rechenkern::Calculator::new());
/// sheet.set_text("rent = $1,500\nfood = $400\nrent + food");
/// assert_eq!(sheet.lines()[2].answer().unwrap().unwrap().text(), "$1,900.00");
/// // Only the lines that read "food" are worked out again.
/// assert_eq!(sheet.set_line(1, "food = $600"), [1, 2]);
/// ```
pub struct Sheet {
    calc: Calculator,
    lines: Vec<SheetLine>,
    next_id: u64,
    observers: Vec<(ObserverId, Observer)>,
    next_observer: u64,
}

impl Default for Sheet {
    fn default() -> Sheet {
        Sheet::new(Calculator::new())
    }
}

impl Sheet {
    /// A sheet that calculates with this calculator's config and providers.
    pub fn new(calc: Calculator) -> Sheet {
        Sheet { calc, lines: Vec::new(), next_id: 1, observers: Vec::new(), next_observer: 1 }
    }

    pub fn with_config(config: Config) -> Sheet {
        Sheet::new(Calculator::with_config(config))
    }

    pub fn calculator(&self) -> &Calculator {
        &self.calc
    }

    /// The calculator, to change its config or providers; every line is worked out again
    /// on the next edit, or with `recalculate`.
    pub fn calculator_mut(&mut self) -> &mut Calculator {
        &mut self.calc
    }

    pub fn lines(&self) -> &[SheetLine] {
        &self.lines
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The lines as text, one per line.
    pub fn text(&self) -> String {
        self.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n")
    }

    /// Replaces every line; returns the lines worked out. Every `\n` starts a line, so
    /// `text()` gives the same text back.
    pub fn set_text(&mut self, text: &str) -> Vec<usize> {
        self.lines.clear();
        for line in text.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let id = self.new_id();
            self.lines.push(SheetLine::new(id, line));
        }
        self.recalculate()
    }

    /// Changes the text of line `i`; returns the lines worked out again.
    pub fn set_line(&mut self, i: usize, text: &str) -> Vec<usize> {
        self.lines[i].text = text.to_string();
        self.update(i, &[i])
    }

    /// Adds a line before line `i` (or at the end); returns the lines worked out.
    pub fn insert(&mut self, i: usize, text: &str) -> Vec<usize> {
        let id = self.new_id();
        let i = i.min(self.lines.len());
        self.lines.insert(i, SheetLine::new(id, text));
        self.update(i, &[])
    }

    pub fn push(&mut self, text: &str) -> Vec<usize> {
        self.insert(self.lines.len(), text)
    }

    pub fn remove(&mut self, i: usize) -> Vec<usize> {
        self.lines.remove(i);
        self.update(i, &[])
    }

    /// Moves line `from` to index `to`.
    pub fn move_line(&mut self, from: usize, to: usize) -> Vec<usize> {
        let line = self.lines.remove(from);
        let to = to.min(self.lines.len());
        self.lines.insert(to, line);
        self.update(from.min(to), &[])
    }

    /// Marks how line `i` answers. Subtotals below may now take other lines, so every
    /// line from it is worked out again.
    pub fn set_behaviour(&mut self, i: usize, behaviour: Behaviour) -> Vec<usize> {
        self.lines[i].behaviour = behaviour;
        self.update(i, &[])
    }

    pub fn set_highlight(&mut self, i: usize, portion: Portion, color: Option<Color>) {
        self.lines[i].highlights[portion as usize] = color;
    }

    /// Works out every line again, as after a change of config or exchange rates.
    pub fn recalculate(&mut self) -> Vec<usize> {
        self.update(0, &[])
    }

    /// Works out again the lines that read the clock, and the ones that depend on them.
    pub fn refresh_time(&mut self) -> Vec<usize> {
        let dirty = self.time_dependent_lines();
        match dirty.first() {
            Some(&first) => self.update_from(first, &dirty, self.lines.len()),
            None => Vec::new(),
        }
    }

    /// Lines whose answers change with time.
    pub fn time_dependent_lines(&self) -> Vec<usize> {
        (0..self.lines.len()).filter(|&i| self.lines[i].effects.reads.clock).collect()
    }

    /// Lines that read a variable, or would read differently without it.
    pub fn lines_reading(&self, variable: &str) -> Vec<usize> {
        let name = variable.to_lowercase();
        (0..self.lines.len()).filter(|&i| self.lines[i].effects.reads.names.contains(&name)).collect()
    }

    /// The variables line `i` sees: the ones set above it.
    pub fn variables_at(&self, i: usize) -> HashMap<String, Value> {
        self.names_at(i).vars
    }

    /// The variables and functions line `i` sees.
    fn names_at(&self, i: usize) -> Names {
        let mut names = Names::default();
        for line in &self.lines[..i.min(self.lines.len())] {
            names.apply(&line.effects);
        }
        names
    }

    /// The variables after the last line.
    pub fn variables(&self) -> HashMap<String, Value> {
        self.variables_at(self.lines.len())
    }

    /// What each part of line `i` is, for syntax highlighting.
    pub fn highlights(&self, i: usize) -> Vec<Highlight> {
        self.calc.highlight_with(&self.lines[i].text, &self.names_at(i))
    }

    /// Calls `f` with the indexes of the lines worked out after every change.
    pub fn observe(&mut self, f: impl FnMut(&Sheet, &[usize]) + 'static) -> ObserverId {
        let id = ObserverId(self.next_observer);
        self.next_observer += 1;
        self.observers.push((id, Box::new(f)));
        id
    }

    pub fn unobserve(&mut self, id: ObserverId) {
        self.observers.retain(|(o, _)| *o != id);
    }

    /// The state to come back to with `revert`.
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint { lines: self.lines.clone() }
    }

    /// Goes back to a checkpoint: what was added since is removed and so on, by line id.
    pub fn revert(&mut self, checkpoint: &Checkpoint) -> Delta {
        let ids = |lines: &[SheetLine]| lines.iter().map(|l| l.id).collect::<HashSet<_>>();
        let (now, then) = (ids(&self.lines), ids(&checkpoint.lines));
        let removed = (0..self.lines.len()).filter(|&i| !then.contains(&self.lines[i].id)).collect();
        let added = (0..checkpoint.lines.len()).filter(|&i| !now.contains(&checkpoint.lines[i].id)).collect();
        let modified = (0..checkpoint.lines.len())
            .filter(|&i| {
                let old = &checkpoint.lines[i];
                self.lines.iter().find(|l| l.id == old.id).is_some_and(|l| {
                    l.text != old.text
                        || l.behaviour != old.behaviour
                        || self.lines.iter().position(|m| m.id == old.id) != Some(i)
                })
            })
            .collect();
        self.lines = checkpoint.lines.clone();
        Delta { added, removed, modified }
    }

    /// A statistic of the answers of some lines (all of them for `None`), leaving out lines
    /// that set a variable and lines other lines read, as SoulverCore's quick statistics do.
    pub fn statistic(&self, statistic: Statistic, lines: Option<&[usize]>) -> Option<Result<Answer>> {
        let all: Vec<usize> = (0..self.lines.len()).collect();
        let lines = lines.unwrap_or(&all);
        let referenced: HashSet<usize> =
            self.lines.iter().flat_map(|l| l.effects.reads.lines.iter().copied()).collect();
        let values: Vec<Value> = lines
            .iter()
            .filter(|&&i| i < self.lines.len() && !referenced.contains(&i))
            .map(|&i| &self.lines[i])
            .filter(|l| l.effects.assigned.is_none() && l.effects.line.kind == LineKind::Value)
            .filter_map(|l| l.effects.line.value.clone())
            .collect();
        if values.is_empty() {
            return None;
        }
        let func = match statistic {
            Statistic::Total => Func::Sum,
            Statistic::Average => Func::Average,
            Statistic::Count => Func::Count,
            Statistic::Median => Func::Median,
            Statistic::Min => Func::Min,
            Statistic::Max => Func::Max,
            Statistic::Lcm => Func::Lcm,
            Statistic::Gcd => Func::Gcd,
            Statistic::StandardDeviation => Func::StdDev,
        };
        let names = Names::default();
        let env = self.calc.env(&names, &[]);
        Some(env.call(func, values).map(|value| answer_of(&env, value)))
    }

    /// The line whose subtotal includes line `i`.
    pub fn subtotal_of(&self, i: usize) -> Option<usize> {
        (i + 1..self.lines.len())
            .find(|&k| matches!(self.lines[k].behaviour, Behaviour::Subtotal | Behaviour::GrandTotal))
    }

    /// Words to finish `fragment` with on line `i`: variables above it, then functions,
    /// units, places and other words the engine knows, shortest first.
    pub fn completions(&self, fragment: &str, i: usize) -> Vec<String> {
        let fragment = fragment.to_lowercase();
        if fragment.is_empty() {
            return Vec::new();
        }
        let mut variables: Vec<String> =
            self.variables_at(i).into_keys().filter(|v| v.starts_with(&fragment) && *v != fragment).collect();
        variables.sort_by_key(|v| (v.len(), v.clone()));
        let mut words: Vec<&str> =
            crate::parser::words::known_words().filter(|w| w.starts_with(&fragment) && *w != fragment).collect();
        words.sort_by_key(|w| (w.len(), *w));
        words.dedup();
        variables.into_iter().chain(words.into_iter().map(String::from)).take(20).collect()
    }

    fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id - 1
    }

    /// Works out the lines from `from` on that need it: `dirty` ones, every one after a
    /// line that came or went at `from`, and the ones that read a changed line or variable.
    fn update(&mut self, from: usize, dirty: &[usize]) -> Vec<usize> {
        // Without dirty lines, lines moved: every line below reads differently.
        let all_from = if dirty.is_empty() { from } else { self.lines.len() };
        self.update_from(from, dirty, all_from)
    }

    fn update_from(&mut self, from: usize, dirty: &[usize], all_from: usize) -> Vec<usize> {
        let mut names = Names::default();
        let mut above: Vec<Line> = Vec::with_capacity(self.lines.len());
        let mut changed_lines = HashSet::new();
        let mut changed_names = HashSet::new();
        let mut all_from = all_from;
        let mut updated = Vec::new();
        for i in 0..self.lines.len() {
            let line = &self.lines[i];
            let needed = i >= all_from
                || dirty.contains(&i)
                || (i >= from
                    && (line.effects.reads.lines.iter().any(|l| changed_lines.contains(l))
                        || line.effects.reads.names.iter().any(|n| changed_names.contains(n))));
            if needed {
                let (answer, effects) = self.work_out(i, &names, &above);
                let old = &self.lines[i].effects;
                if old.line.kind != effects.line.kind || old.clears_vars != effects.clears_vars {
                    // Totals and "prev" below may now take other lines.
                    all_from = all_from.min(i + 1);
                }
                if !same_line(&old.line, &effects.line) {
                    changed_lines.insert(i);
                }
                if !same_assignment(&old.assigned, &effects.assigned) {
                    changed_names.extend(old.assigned.iter().chain(&effects.assigned).map(|(n, ..)| n.clone()));
                    // Subtotals and statistics leave out lines that set a variable.
                    changed_lines.insert(i);
                }
                // A function worked out again may work differently: its callers follow.
                changed_names.extend(old.defined.iter().chain(&effects.defined).map(|(n, _)| n.clone()));
                let line = &mut self.lines[i];
                line.answer = answer;
                line.effects = effects;
                updated.push(i);
            }
            let effects = &self.lines[i].effects;
            names.apply(effects);
            above.push(effects.line.clone());
        }
        self.running_totals();
        if !updated.is_empty() {
            let mut observers = std::mem::take(&mut self.observers);
            for (_, observer) in &mut observers {
                observer(self, &updated);
            }
            observers.append(&mut self.observers);
            self.observers = observers;
        }
        updated
    }

    /// The answer of line `i` after the lines `above`, with the variables they left.
    fn work_out(&self, i: usize, names: &Names, above: &[Line]) -> (Result<Option<Answer>>, Effects) {
        let line = &self.lines[i];
        match line.behaviour {
            Behaviour::Subtotal | Behaviour::GrandTotal => self.subtotal(i, names, above),
            _ => self.calc.run(&line.text, names, above, true),
        }
    }

    /// A subtotal or grand total line: the sum of the lines it takes.
    fn subtotal(&self, i: usize, names: &Names, above: &[Line]) -> (Result<Option<Answer>>, Effects) {
        let grand = self.lines[i].behaviour == Behaviour::GrandTotal;
        let start = (0..i)
            .rev()
            .find(|&k| {
                self.lines[k].behaviour == Behaviour::GrandTotal
                    || (!grand && self.lines[k].behaviour == Behaviour::Subtotal)
            })
            .map_or(0, |k| k + 1);
        let has_subtotals = (start..i).any(|k| self.lines[k].behaviour == Behaviour::Subtotal);
        let taken: Vec<usize> = (start..i)
            .filter(|&k| match grand && has_subtotals {
                true => self.lines[k].behaviour == Behaviour::Subtotal,
                false => above[k].kind == LineKind::Value && self.lines[k].effects.assigned.is_none(),
            })
            .collect();
        let values: Vec<Value> = taken.iter().filter_map(|&k| above[k].value.clone()).collect();
        let env = self.calc.env(names, above);
        let answer = match values.is_empty() {
            true => Ok(Value::number(crate::Number::ZERO)),
            false => env.call(Func::Sum, values),
        }
        .map(|value| answer_of(&env, value));
        let effects = Effects {
            line: Line { value: answer.as_ref().ok().map(Answer::reference), kind: LineKind::Total, tags: Vec::new() },
            assigned: None,
            defined: None,
            clears_vars: false,
            reads: Reads { names: Vec::new(), lines: (start..i).collect(), clock: false },
        };
        (answer.map(Some), effects)
    }

    /// Running totals and budgets, from each line that starts one to the next blank line or heading.
    fn running_totals(&mut self) {
        let names = Names::default();
        let mut current: Option<(Behaviour, Option<Value>)> = None;
        for i in 0..self.lines.len() {
            let line = &self.lines[i];
            if matches!(line.behaviour, Behaviour::RunningTotal | Behaviour::RunningBudget) {
                current = Some((line.behaviour, None));
            } else if matches!(line.effects.line.kind, LineKind::Blank | LineKind::Heading | LineKind::Divider) {
                current = None;
            }
            let value = line.effects.line.value.clone();
            let env = self.calc.env(&names, &[]);
            let running = match (&mut current, value) {
                (Some((kind, total)), Some(value)) => {
                    let next = match (total.take(), *kind) {
                        (None, _) => Ok(value),
                        (Some(sum), Behaviour::RunningBudget) => env.sub(sum, value),
                        (Some(sum), _) => env.add(sum, value),
                    };
                    *total = next.as_ref().ok().cloned();
                    next.ok().map(|v| answer_of(&env, v))
                }
                _ => None,
            };
            self.lines[i].running = running;
        }
    }
}

/// An answer for a value the sheet works out itself.
fn answer_of(env: &Env, value: Value) -> Answer {
    let config = std::sync::Arc::new(env.config.clone());
    env.describe(&Expr::Line(LineRef::Sum), value, Display::Auto, Parts::default(), &config)
}

fn same_line(a: &Line, b: &Line) -> bool {
    // A tag decides which totals take the line.
    a.kind == b.kind
        && a.tags == b.tags
        && match (&a.value, &b.value) {
            (Some(a), Some(b)) => a.same_as(b),
            (None, None) => true,
            _ => false,
        }
}

fn same_assignment(a: &Option<Assignment>, b: &Option<Assignment>) -> bool {
    match (a, b) {
        (Some((n, a, r)), Some((m, b, s))) => n == m && a.same_as(b) && r == s,
        (None, None) => true,
        _ => false,
    }
}
