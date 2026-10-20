//! Editing a sheet beyond single lines, as SoulverCore's `LineCollection` offers it:
//! copies, batches of changes, pending answers, renaming a variable, per-line formatting,
//! trips, references that follow their lines.

use std::collections::HashMap;
use std::ops::Range;

use super::{Alignment, Behaviour, Sheet, SheetLine, Statistic};
use crate::answer::Answer;
use crate::config::Formatting;
use crate::error::{Error, ErrorKind, Result};
use crate::eval::LineKind;
use crate::parser::highlight::TokenKind;
use crate::value::Value;

/// What a line looks like, as SoulverCore's `ApparentLineType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineType {
    /// Nothing written.
    Blank,
    /// A calculation, or text with numbers in it.
    Regular,
    /// Words only: nothing to calculate.
    Comment,
    Heading,
    /// `rent = $1,500`, `area(w, h) = w * h`, `widgets = new unit`.
    Declaration,
    /// `---`
    Divider,
    /// `sum`, `total`, `average`, or a subtotal the host marked.
    Total,
}

/// What a sheet defines for others to use, as SoulverCore's `LineCollectionReducer`.
#[derive(Clone, Debug, Default)]
pub struct Definitions {
    /// Variables after the last line.
    pub variables: HashMap<String, Value>,
    /// Units declared with `= new unit`.
    pub units: Vec<String>,
    /// Functions defined in lines: `area(w, h) = w * h`.
    pub functions: Vec<String>,
    /// The answer of the last line that has one.
    pub last_answer: Option<Value>,
}

/// Which lines a statistic leaves out, as `calculateQuickStatistic` asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatisticOptions {
    /// Leave out lines that set a variable.
    pub ignore_declarations: bool,
    /// Leave out lines other lines read (`prev`, `line 2`).
    pub ignore_referenced: bool,
}

impl Default for StatisticOptions {
    fn default() -> StatisticOptions {
        StatisticOptions { ignore_declarations: true, ignore_referenced: true }
    }
}

/// Changes waiting for `end_changes`: where they start, the lines they touched, and from
/// where every line needs working out.
#[derive(Clone, Debug)]
pub(super) struct Batch {
    from: usize,
    dirty: Vec<usize>,
    all_from: usize,
}

impl Clone for Sheet {
    /// A copy with the same lines, answers and calculator; observers stay with the original.
    fn clone(&self) -> Sheet {
        Sheet {
            calc: self.calc.clone(),
            lines: self.lines.clone(),
            next_id: self.next_id,
            observers: Vec::new(),
            next_observer: 1,
            batch: None,
            trip_planning: self.trip_planning,
        }
    }
}

impl SheetLine {
    /// What the line looks like.
    pub fn line_type(&self) -> LineType {
        if matches!(self.behaviour, Behaviour::Subtotal | Behaviour::GrandTotal) {
            return LineType::Total;
        }
        match self.effects.line.kind {
            LineKind::Blank => LineType::Blank,
            LineKind::Heading => LineType::Heading,
            LineKind::Divider => LineType::Divider,
            LineKind::Total => LineType::Total,
            LineKind::Value if self.effects.assigned.is_some() || self.effects.declared.is_some() => {
                LineType::Declaration
            }
            LineKind::Value if self.effects.defined.is_some() => LineType::Declaration,
            LineKind::Value if matches!(self.answer, Ok(None)) => LineType::Comment,
            LineKind::Value => LineType::Regular,
        }
    }

    /// The formatting the host set for this line only.
    pub fn formatting(&self) -> Option<&Formatting> {
        self.formatting.as_ref()
    }

    /// The host marked the line as a stop of a trip.
    pub fn is_waypoint(&self) -> bool {
        self.behaviour == Behaviour::TripWaypoint
    }

    /// The line's answer is still on its way from a provider.
    pub fn is_pending(&self) -> bool {
        self.answer.as_ref().is_err_and(Error::is_pending)
    }
}

impl Sheet {
    /// A copy to change without touching this one, as SoulverCore's `deepCopy`.
    pub fn deep_copy(&self) -> Sheet {
        self.clone()
    }

    /// Holds back working out lines until `end_changes`, for many edits at once.
    pub fn begin_changes(&mut self) {
        if self.batch.is_none() {
            self.batch = Some(Batch { from: usize::MAX, dirty: Vec::new(), all_from: usize::MAX });
        }
    }

    /// Works out what the changes since `begin_changes` need; returns those lines.
    pub fn end_changes(&mut self) -> Vec<usize> {
        match self.batch.take() {
            Some(batch) if batch.from != usize::MAX => {
                let len = self.lines.len();
                let dirty: Vec<usize> = batch.dirty.into_iter().filter(|&i| i < len).collect();
                self.update_from(batch.from.min(len), &dirty, batch.all_from.min(len))
            }
            _ => Vec::new(),
        }
    }

    /// Keeps an edit for `end_changes` while changes are held back.
    pub(super) fn held_back(&mut self, from: usize, dirty: &[usize], all_from: usize) -> bool {
        let Some(batch) = &mut self.batch else { return false };
        batch.from = batch.from.min(from);
        batch.dirty.extend_from_slice(dirty);
        batch.all_from = batch.all_from.min(all_from);
        true
    }

    /// Works out these lines again, and the ones that depend on them.
    pub fn recalculate_lines(&mut self, lines: &[usize]) -> Vec<usize> {
        let dirty: Vec<usize> = lines.iter().copied().filter(|&i| i < self.lines.len()).collect();
        match dirty.iter().min() {
            Some(&first) => self.update(first, &dirty),
            None => Vec::new(),
        }
    }

    /// Adds the lines of another sheet at the end, with how they were marked.
    pub fn append(&mut self, other: &Sheet) -> Vec<usize> {
        let from = self.lines.len();
        for line in &other.lines {
            let id = self.new_id();
            let mut copy = SheetLine::new(id, &line.text);
            copy.behaviour = line.behaviour;
            copy.highlights = line.highlights;
            copy.alignment = line.alignment;
            copy.attribution = line.attribution.clone();
            copy.formatting = line.formatting.clone();
            self.lines.push(copy);
        }
        self.update(from, &[])
    }

    pub fn set_alignment(&mut self, i: usize, alignment: Alignment) {
        self.lines[i].alignment = alignment;
    }

    pub fn set_attribution(&mut self, i: usize, attribution: Option<String>) {
        self.lines[i].attribution = attribution;
    }

    /// Formatting for line `i` only, over the calculator's; `None` goes back to it.
    pub fn set_line_formatting(&mut self, i: usize, formatting: Option<Formatting>) -> Vec<usize> {
        self.lines[i].formatting = formatting;
        self.update(i, &[i])
    }

    /// Lines whose answers wait on a provider (`ErrorKind::Pending`).
    pub fn pending_lines(&self) -> Vec<usize> {
        (0..self.lines.len()).filter(|&i| self.lines[i].is_pending()).collect()
    }

    /// Works out the pending lines again, once the providers have what they waited for.
    pub fn recalculate_pending(&mut self) -> Vec<usize> {
        let pending = self.pending_lines();
        self.recalculate_lines(&pending)
    }

    /// Gives up waiting on line `i`: its answer becomes an error until it is worked out again.
    pub fn cancel_pending(&mut self, i: usize) {
        if self.lines[i].is_pending() {
            self.lines[i].answer = Err(Error::of_kind(ErrorKind::Other, "cancelled"));
        }
    }

    /// Renames a variable everywhere it is written, as `replaceOccurancesInExpressions`.
    pub fn rename_variable(&mut self, old: &str, new: &str) -> Vec<usize> {
        let old = old.to_lowercase();
        let mut changed = Vec::new();
        for i in 0..self.lines.len() {
            let ranges = self.variable_ranges(i, &old);
            if ranges.is_empty() {
                continue;
            }
            let mut text = self.lines[i].text.clone();
            for range in ranges.into_iter().rev() {
                text.replace_range(range, new);
            }
            self.lines[i].text = text;
            changed.push(i);
        }
        match changed.first() {
            Some(&first) => self.update(first, &[]),
            None => Vec::new(),
        }
    }

    /// Where line `i` writes the variable `name` (or sets it).
    pub fn references_to(&self, i: usize, name: &str) -> Vec<Range<usize>> {
        self.variable_ranges(i, &name.to_lowercase())
    }

    fn variable_ranges(&self, i: usize, name: &str) -> Vec<Range<usize>> {
        let text = &self.lines[i].text;
        let mut ranges: Vec<Range<usize>> = self
            .highlights(i)
            .into_iter()
            .filter(|h| h.kind == TokenKind::Variable && text[h.range.clone()].to_lowercase() == name)
            .map(|h| h.range)
            .collect();
        // The name a line sets: "rent = …".
        if self.lines[i].variable() == Some(name)
            && let Some(eq) = text.find(['=', '+', '-', '*', '/'])
        {
            let start = text.len() - text.trim_start().len();
            let end = text[..eq].trim_end().len();
            if text[start..end].to_lowercase() == name && !ranges.iter().any(|r| r.start == start) {
                ranges.insert(0, start..end);
            }
        }
        ranges
    }

    /// The last line that sets `name`, as SoulverCore's `lineFor(variable:)`.
    pub fn line_setting(&self, name: &str) -> Option<usize> {
        let name = name.to_lowercase();
        (0..self.lines.len()).rev().find(|&i| self.lines[i].variable() == Some(name.as_str()))
    }

    /// The nearest line above `i` with some text.
    pub fn first_non_empty_above(&self, i: usize) -> Option<usize> {
        (0..i.min(self.lines.len())).rev().find(|&k| !self.lines[k].text.trim().is_empty())
    }

    /// The lines line `i` reads: `prev`, `line 2`, the lines a total takes.
    pub fn lines_read_by(&self, i: usize) -> Vec<usize> {
        self.lines[i].effects.reads.lines.clone()
    }

    /// Line `i` with `prev` and `line 2` written as the answers they stand for.
    pub fn expanded_text(&self, i: usize) -> String {
        self.expression(i, false, true, false)
    }

    /// The variables line `i` sees, with `prev` and `line N` for the answers above it,
    /// as `includeVariablesReferencingPreviousLine` and `…LineNumbers`.
    pub fn variables_and_references_at(&self, i: usize) -> HashMap<String, Value> {
        let mut vars = self.variables_at(i);
        let value = |k: usize| self.lines[k].effects.line.value.clone();
        if let Some(prev) = (0..i.min(self.lines.len())).rev().find_map(value) {
            vars.insert("prev".into(), prev);
        }
        for k in 0..i.min(self.lines.len()) {
            if let Some(v) = value(k) {
                vars.insert(format!("line {}", k + 1), v);
            }
        }
        vars
    }

    /// What the sheet defines, to hand to another calculator or sheet.
    pub fn definitions(&self) -> Definitions {
        let names = self.names_at(self.lines.len());
        // Each declared unit as its line names it: "widgets".
        let declared = |l: &SheetLine| Some(l.text.split_once('=')?.0.trim().to_lowercase());
        let mut units: Vec<String> =
            self.lines.iter().filter(|l| l.effects.declared.is_some()).filter_map(declared).collect();
        units.dedup();
        let mut functions: Vec<String> = names.functions.keys().cloned().collect();
        functions.sort();
        Definitions {
            variables: names.vars,
            units,
            functions,
            last_answer: self.lines.iter().rev().find_map(|l| l.effects.line.value.clone()),
        }
    }

    /// A statistic of the sheet as a value for another sheet, as `resultsAsVariableWith`.
    pub fn result_as_value(&self, statistic: Statistic) -> Option<Value> {
        Some(self.statistic(statistic, None)?.ok()?.value().clone())
    }

    /// `statistic` with a choice of the lines it leaves out.
    pub fn statistic_with(
        &self,
        statistic: Statistic,
        lines: Option<&[usize]>,
        options: StatisticOptions,
    ) -> Option<Result<Answer>> {
        self.statistic_of(statistic, lines, options)
    }

    /// Trip planning, as SoulverCore's `tripPlanning`: lines marked `Behaviour::TripWaypoint`
    /// are the stops of a trip.
    pub fn set_trip_planning(&mut self, on: bool) {
        self.trip_planning = on;
    }

    pub fn trip_planning(&self) -> bool {
        self.trip_planning
    }

    /// Line `i` is a stop of the trip being planned.
    pub fn is_part_of_trip(&self, i: usize) -> bool {
        self.trip_planning && self.lines[i].is_waypoint()
    }

    /// Moves `line N` references after `shift` lines came at index `at`, or went from it
    /// (negative), so they keep pointing at the same lines.
    pub(super) fn renumber_references(&mut self, at: usize, shift: isize) -> Vec<usize> {
        let mut changed = Vec::new();
        for i in 0..self.lines.len() {
            let text = self.lines[i].text.clone();
            if !text.to_lowercase().contains("line") {
                continue;
            }
            let mut out = text.clone();
            for h in self.highlights(i).into_iter().rev().filter(|h| h.kind == TokenKind::Reference) {
                let part = &text[h.range.clone()];
                let digits: String = part.chars().filter(char::is_ascii_digit).collect();
                let Ok(n) = digits.parse::<usize>() else { continue };
                // "line 3" is line index 2; references above, or to removed lines, stay as they were.
                if n == 0 || n - 1 < at || (shift < 0 && n - 1 < at + shift.unsigned_abs()) {
                    continue;
                }
                let moved = (n as isize + shift).max(1);
                let new = part.replacen(&digits, &moved.to_string(), 1);
                out.replace_range(h.range, &new);
            }
            if out != text {
                self.lines[i].text = out;
                changed.push(i);
            }
        }
        changed
    }

    /// Moves `line N` references after line `from` went to index `to`.
    pub(super) fn renumber_moved(&mut self, from: usize, to: usize) -> Vec<usize> {
        let mut changed = Vec::new();
        let map = |n: usize| -> usize {
            let k = n - 1;
            let moved = if k == from {
                to
            } else if from < to && k > from && k <= to {
                k - 1
            } else if to < from && k >= to && k < from {
                k + 1
            } else {
                k
            };
            moved + 1
        };
        for i in 0..self.lines.len() {
            let text = self.lines[i].text.clone();
            if !text.to_lowercase().contains("line") {
                continue;
            }
            let mut out = text.clone();
            for h in self.highlights(i).into_iter().rev().filter(|h| h.kind == TokenKind::Reference) {
                let part = &text[h.range.clone()];
                let digits: String = part.chars().filter(char::is_ascii_digit).collect();
                let Ok(n) = digits.parse::<usize>() else { continue };
                if n == 0 || n > self.lines.len() {
                    continue;
                }
                out.replace_range(h.range, &part.replacen(&digits, &map(n).to_string(), 1));
            }
            if out != text {
                self.lines[i].text = out;
                changed.push(i);
            }
        }
        changed
    }

    /// Works out lines from `from` on, and the lines whose references were renumbered.
    pub(super) fn update_renumbered(&mut self, from: usize, renumbered: &[usize]) -> Vec<usize> {
        let start = renumbered.iter().copied().min().unwrap_or(from).min(from);
        let all_from = from.min(self.lines.len());
        if self.held_back(start, renumbered, all_from) {
            return Vec::new();
        }
        self.update_from(start, renumbered, all_from)
    }
}

/// An answer written with the line's own formatting.
pub(super) fn formatted(answer: Result<Option<Answer>>, formatting: Option<&Formatting>) -> Result<Option<Answer>> {
    match (answer, formatting) {
        (Ok(Some(answer)), Some(formatting)) => Ok(Some(answer.reformatted(formatting))),
        (answer, _) => answer,
    }
}
